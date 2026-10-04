//! Layer-1 deterministic discovery — the shared `find` entry point for CLI,
//! bindings, and web (`ext:discovery`, RFC-012 / ADR-019).
//!
//! Composes existing services (it does not duplicate them): the structured filter
//! pass reuses [`record_store::list_records_filtered`] for Tier 2 and the manifest
//! instance index (via [`crate::container_service::list_members`] for container
//! scoping) for Tier 0; content matching reuses
//! [`text_projection::project_text`] / [`text_projection::project_note_text`];
//! hit labels reuse [`record_label::record_display_label`] for Tier 2 and the
//! manifest `title` (falling back to `instanceId`) for Tier 0. Substring content
//! matching is the recall floor — `score` is `None` at Layer 1. With
//! [`FindPage::rank`] the same hits are ordered (and scored) by a
//! [`crate::discovery_index::DiscoveryIndex`] (Layer 2, srs-rust#1228), which may
//! reorder but never add or drop a Layer-1 match.
//!
//! Discovery spans both remaining tiers (RFC-012 `R1`/`I-113`, `R11`/`I-123` —
//! see srs-rust#797; Tier 1 / TypedRecord was retired, srs#448/rfc-decision-53635966,
//! srs-rust#888): `typeId`/`typeNamespace`/`typeName`/`lifecycleState` are
//! Tier-2-only predicates and exclude Tier 0 instances outright when specified,
//! since Tier 0 carries none of those fields; `tag`, `containerId`, and `tier`
//! apply uniformly.

use crate::container_service;
use crate::discovery_index::{Bm25Index, DiscoveryIndex};
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store::{self, RecordListFilter};
use crate::resource_uri;
use crate::store::RepositoryStore;
use crate::text_projection::{self, FieldTextIndex, TextSegment};
use serde::{Deserialize, Serialize};
use srs_core::types::record::Record;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

/// The canonical query shape now lives in `srs-core` (`srs_core::types::discovery`)
/// so it can be the one type [`crate::render_service`]'s `SectionSource::DiscoveryQuery`
/// consumes too (srs#525 / srs-rust#924 — "one query engine", not a divergent
/// re-implementation). Re-exported here so existing `discovery_service::DiscoveryQuery`
/// call sites (CLI `find`, MCP, bindings) are unaffected.
pub use srs_core::types::discovery::DiscoveryQuery;

/// Deterministic result: hits in stable order, total, and non-fatal diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub hits: Vec<DiscoveryHit>,
    pub total: usize,
    /// Counts over the whole match set, before paging (same rule as `total`).
    pub facets: DiscoveryFacets,
    pub diagnostics: Vec<String>,
}

/// Values kept per facet; the rest are summed into `other`.
const FACET_TOP_N: usize = 20;
/// Closed-field facets kept; the rest are dropped. Both caps keep a reply well under
/// the ~128 KB browser-relay limit however large the repository.
const FACET_MAX_FIELDS: usize = 25;

/// The derived repository map (srs-rust#1219): what the match set holds, so an agent can
/// narrow a query without guessing. `find {limit: 0}` with no filters is the whole
/// repository; with a type filter it is that type's keyword map. Every count is
/// one per instance per value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFacets {
    /// `namespace/name` of the bound Type, for Tier 2 records.
    #[serde(default, skip_serializing_if = "FacetCounts::is_empty")]
    pub by_type: FacetCounts,
    /// Tier 0 notes in the match set (they carry no type).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub notes: usize,
    #[serde(default, skip_serializing_if = "FacetCounts::is_empty")]
    pub tags: FacetCounts,
    /// One entry per closed string field (the successor of select/multiselect, RFC-032
    /// R3) that matched records carry, keyed by `Field.name`: the key records store
    /// values under (RFC-039), so two fields sharing a name share a facet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldFacet>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Value counts, most frequent first (ties by value), at most [`FACET_TOP_N`] entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetCounts {
    pub values: Vec<FacetCount>,
    /// Occurrences under the values left out of `values`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub other: usize,
}

impl FacetCounts {
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn from_map(map: BTreeMap<String, usize>) -> Self {
        let mut values: Vec<FacetCount> = map
            .into_iter()
            .map(|(value, count)| FacetCount { value, count })
            .collect();
        // Stable sort over BTreeMap order: count descending, then value ascending.
        values.sort_by_key(|a| std::cmp::Reverse(a.count));
        let other = values.split_off(values.len().min(FACET_TOP_N));
        Self {
            values,
            other: other.iter().map(|c| c.count).sum(),
        }
    }

    fn total(&self) -> usize {
        self.values.iter().map(|c| c.count).sum::<usize>() + self.other
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetCount {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldFacet {
    pub field: String,
    #[serde(flatten)]
    pub counts: FacetCounts,
}

/// A hit plus the material the facets count, so the match set is walked once.
struct Candidate {
    hit: DiscoveryHit,
    tags: Vec<String>,
    /// `(Field.name, value)` for each closed string field value the record carries.
    selects: Vec<(String, String)>,
}

fn build_facets(candidates: &[Candidate]) -> DiscoveryFacets {
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut fields: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut notes = 0;
    for c in candidates {
        match (&c.hit.type_namespace, &c.hit.type_name) {
            (Some(ns), Some(name)) => *by_type.entry(format!("{ns}/{name}")).or_default() += 1,
            _ => notes += 1,
        }
        for t in c.tags.iter().collect::<HashSet<_>>() {
            *tags.entry(t.clone()).or_default() += 1;
        }
        for (field, value) in &c.selects {
            *fields
                .entry(field.clone())
                .or_default()
                .entry(value.clone())
                .or_default() += 1;
        }
    }
    let mut fields: Vec<FieldFacet> = fields
        .into_iter()
        .map(|(field, m)| FieldFacet {
            field,
            counts: FacetCounts::from_map(m),
        })
        .collect();
    fields.sort_by_key(|f| std::cmp::Reverse(f.counts.total()));
    fields.truncate(FACET_MAX_FIELDS);
    DiscoveryFacets {
        by_type: FacetCounts::from_map(by_type),
        notes,
        tags: FacetCounts::from_map(tags),
        fields,
    }
}

/// A single matched instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryHit {
    pub instance_id: String,
    /// `srs://<repo>/record/<id>`: readable as-is by the MCP `read` tool / resource.
    pub uri: String,
    pub label: String,
    /// The bound Type's id (readable at `srs://<repo>/type/{typeId}`); `None` for Tier 0 notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    /// Ids of the containers that declare this instance as a member.
    pub container_ids: Vec<String>,
    /// `None` for Tier 0 instances, which carry no type binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    /// `None` for Tier 0 instances, which carry no type binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// `None` at Layer 1 (deterministic, unranked). Populated only when ranked ([`FindPage::rank`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// A ~200-char window around the first match in the first matching segment, when a content match was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Field names (or sentinels) whose text matched the content predicate.
    pub matched_fields: Vec<String>,
}

/// Result shaping for [`find`] (srs-rust#1217). Deliberately not part of
/// [`DiscoveryQuery`], which mirrors the spec schema: paging selects which of the
/// matches are returned, never which instances match.
/// `limit: None` means every match; any default cap is the adapter's choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindPage {
    pub limit: Option<usize>,
    pub offset: usize,
    /// Order content-match hits by BM25 relevance and fill `score` (srs-rust#1228)
    /// instead of by `instanceId`. Never changes which instances match. Off by
    /// default; the MCP `find` tool turns it on. Ignored without a `contentMatch`.
    pub rank: bool,
}

/// Characters of text around the first match kept in a hit snippet.
const SNIPPET_WINDOW: usize = 200;
/// Characters of leading context before the match inside the window.
const SNIPPET_LEAD: usize = 40;

/// A window of about [`SNIPPET_WINDOW`] chars of `text` around the first
/// occurrence of `needle` (compared after normalization), `…` marking each cut.
/// Whole text when it fits. Counts and cuts on char boundaries.
fn snippet_window(text: &str, needle: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= SNIPPET_WINDOW {
        return text.to_string();
    }
    // Match position in chars. `normalize` may change length (e.g. case folding), so
    // locate the match on the normalized text and clamp; the window is approximate.
    let norm = text_projection::normalize(text);
    let pos = norm
        .find(needle)
        .map_or(0, |byte| norm[..byte].chars().count());
    let start = pos
        .saturating_sub(SNIPPET_LEAD)
        .min(chars.len() - SNIPPET_WINDOW);
    let end = start + SNIPPET_WINDOW;
    let body: String = chars[start..end].iter().collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        body,
        if end < chars.len() { "…" } else { "" }
    )
}

/// Run a discovery query against the repository. See module docs for the contract.
/// `page` is applied after the deterministic sort; `total` is the pre-paging match count.
pub fn find(
    store: &dyn RepositoryStore,
    query: DiscoveryQuery,
    page: FindPage,
) -> Result<DiscoveryResult, RepositoryError> {
    let mut diagnostics = Vec::new();
    if !unresolved_filters(store, &query, &mut diagnostics)? {
        return Ok(DiscoveryResult {
            hits: Vec::new(),
            total: 0,
            facets: DiscoveryFacets::default(),
            diagnostics,
        });
    }

    // One field-metadata pass: the text index also carries the field_id → name map
    // that Tier-2 hit-label resolution needs, so we avoid a second `list_fields` scan.
    let field_text_index = text_projection::build_field_text_index(store)?;

    // All-words match (srs-rust#1218): a superset of the phrase match, so the
    // RFC-012 recall floor holds.
    let words: Vec<String> = query
        .content_match
        .as_deref()
        .map(|q| {
            text_projection::normalize(q)
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let needle = (!words.is_empty()).then_some(words.as_slice());

    let mut candidates = Vec::new();

    if query.tier.is_none() || query.tier == Some(2) {
        candidates.extend(find_tier2(store, &query, &field_text_index, needle)?);
    }

    // Tier 0 carries no typeId/typeNamespace/typeName/lifecycleState — a query
    // constraining any of those predicates can never match it.
    let tier2_only_predicate = query.type_id.is_some()
        || query.type_namespace.is_some()
        || query.type_name.is_some()
        || query.lifecycle_state.is_some()
        || !query.lifecycle_states.is_empty();

    if !tier2_only_predicate && (query.tier.is_none() || query.tier == Some(0)) {
        candidates.extend(find_tier0(store, &query, needle)?);
    }

    // Deterministic order independent of index/store iteration order.
    candidates.sort_by(|a, b| a.hit.instance_id.cmp(&b.hit.instance_id));

    let total = candidates.len();
    // Facets count the Layer-1 match set: independent of ranking and paging.
    let facets = build_facets(&candidates);
    let mut hits: Vec<DiscoveryHit> = candidates.into_iter().map(|c| c.hit).collect();
    if page.rank && !words.is_empty() {
        rank_hits(store, &field_text_index, &words, &mut hits)?;
    }
    let mut hits: Vec<DiscoveryHit> = hits
        .into_iter()
        .skip(page.offset)
        .take(page.limit.unwrap_or(usize::MAX))
        .collect();

    // Navigation fields, filled for the returned page only.
    let manifest = store.load_manifest()?;
    let repo_id = resource_uri::repository_id(&manifest).unwrap_or_default();
    let memberships = if hits.is_empty() {
        Default::default()
    } else {
        container_service::membership_index(store)?
    };
    for hit in &mut hits {
        hit.uri = resource_uri::record_uri(repo_id, &hit.instance_id);
        hit.container_ids = memberships
            .get(&hit.instance_id)
            .cloned()
            .unwrap_or_default();
    }
    Ok(DiscoveryResult {
        hits,
        total,
        facets,
        diagnostics,
    })
}

/// Fill `score` from the (store-cached) [`DiscoveryIndex`] and reorder by score,
/// ties by `instanceId`. `hits` must already be in `instanceId` order.
fn rank_hits(
    store: &dyn RepositoryStore,
    field_text_index: &FieldTextIndex,
    words: &[String],
    hits: &mut [DiscoveryHit],
) -> Result<(), RepositoryError> {
    let cached = store
        .discovery_index_cache()
        .and_then(|c| c.borrow().clone());
    let index = match cached {
        Some(index) => index,
        None => {
            let index: Rc<dyn DiscoveryIndex> = Rc::new(Bm25Index::build(store, field_text_index)?);
            if let Some(slot) = store.discovery_index_cache() {
                *slot.borrow_mut() = Some(index.clone());
            }
            index
        }
    };
    let ids: Vec<&str> = hits.iter().map(|h| h.instance_id.as_str()).collect();
    let scores = index.score(words, &ids);
    for (hit, score) in hits.iter_mut().zip(scores) {
        hit.score = Some(score);
    }
    // Stable sort over an id-ordered slice: equal scores keep id order.
    hits.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
    Ok(())
}

/// AND-conjunction: `instance_tags` must contain every value in `query_tags`.
fn tags_match(query_tags: &[String], instance_tags: &[String]) -> bool {
    query_tags
        .iter()
        .all(|t| instance_tags.iter().any(|it| it == t))
}

/// Applies every structured (non-content-match) `DiscoveryQuery` predicate to
/// one Tier-2 Record: `typeId`, `tag`, `lifecycleState` (exact),
/// `lifecycleStates` (OR-inclusion), `excludeLifecycleStates` (exclusion).
/// `typeNamespace`/`typeName`/`containerId`/the first `tag` are expected to
/// already be pushed down via [`RecordListFilter`] by the caller — this
/// re-applies `tag` in full (the pushdown only uses the first element as an
/// optimization) plus the remaining predicates that filter never covers.
///
/// The **one** place these axes are evaluated (srs#525 / srs-rust#924's "one
/// query engine" collapse) — consumed by both [`find_tier2`] and
/// [`crate::render_service`]'s `SectionSource::DiscoveryQuery` section
/// resolution, so a Composition section no longer re-implements them.
pub(crate) fn record_matches_structured_predicates(
    record: &Record,
    query: &DiscoveryQuery,
) -> bool {
    if let Some(type_id) = &query.type_id {
        if &record.type_id != type_id {
            return false;
        }
    }

    if !tags_match(&query.tag, record.tags.as_deref().unwrap_or(&[])) {
        return false;
    }

    if let Some(state) = &query.lifecycle_state {
        if record.lifecycle_state.as_deref() != Some(state.as_str()) {
            return false;
        }
    }

    // Inclusion axis (RFC-012 Rev 11): only records whose lifecycleState
    // matches any listed value. Records with no lifecycleState are excluded
    // when this predicate is present and non-empty.
    if !query.lifecycle_states.is_empty() {
        let matches = record
            .lifecycle_state
            .as_deref()
            .is_some_and(|s| query.lifecycle_states.iter().any(|v| v == s));
        if !matches {
            return false;
        }
    }

    // Exclusion axis: drop records whose lifecycleState is in the hidden set.
    // Records without a lifecycleState are never excluded by this axis.
    if !query.exclude_lifecycle_states.is_empty() {
        if let Some(state) = record.lifecycle_state.as_deref() {
            if query.exclude_lifecycle_states.iter().any(|s| s == state) {
                return false;
            }
        }
    }

    true
}

/// Run the content-match recall floor over a projected segment stream. A record
/// matches when every word of `words` occurs in some segment (any field, any order).
/// The first segment containing a word supplies the snippet (windowed on that word);
/// every field with a matching segment becomes `matched_fields` (first-seen order).
/// Empty result means no match.
fn match_content(segments: Vec<TextSegment>, words: &[String]) -> (Vec<String>, Option<String>) {
    let mut matched_fields = Vec::new();
    let mut seen_fields = HashSet::new();
    let mut found = vec![false; words.len()];
    let mut snippet = None;
    for seg in segments {
        let norm = text_projection::normalize(&seg.text);
        let mut first_hit = None;
        for (i, w) in words.iter().enumerate() {
            if norm.contains(w.as_str()) {
                found[i] = true;
                first_hit.get_or_insert(w);
            }
        }
        if let Some(w) = first_hit {
            if snippet.is_none() {
                snippet = Some(snippet_window(&seg.text, w));
            }
            if seen_fields.insert(seg.field_name.clone()) {
                matched_fields.push(seg.field_name);
            }
        }
    }
    if found.iter().all(|f| *f) {
        (matched_fields, snippet)
    } else {
        (Vec::new(), None)
    }
}

/// Warn (never error) when a type or container filter names nothing, so a typo is
/// not indistinguishable from "no matches". Returns false when the query cannot
/// match anything (an unknown container), true otherwise.
fn unresolved_filters(
    store: &dyn RepositoryStore,
    query: &DiscoveryQuery,
    diagnostics: &mut Vec<String>,
) -> Result<bool, RepositoryError> {
    let mut resolvable = true;
    if let Some(cid) = &query.container_id {
        let missing = match container_service::get_container(store, cid) {
            Ok(_) => false,
            Err(RepositoryError::ContainerNotFound { .. }) => true,
            Err(e) => return Err(e),
        };
        if missing {
            diagnostics.push(format!(
                "warning: containerId '{cid}' names no container; no instances can match"
            ));
            resolvable = false;
        }
    }
    if query.type_id.is_some() || query.type_namespace.is_some() || query.type_name.is_some() {
        let types = crate::package_service::list_types(store)?;
        let known = |f: &dyn Fn(&crate::package_service::TypeSummary) -> bool| types.iter().any(f);
        if let Some(id) = &query.type_id {
            if !known(&|t| &t.id == id) {
                diagnostics.push(format!("warning: typeId '{id}' names no type"));
            }
        }
        if query.type_namespace.is_some() || query.type_name.is_some() {
            let hit = known(&|t| {
                query
                    .type_namespace
                    .as_ref()
                    .is_none_or(|n| &t.namespace == n)
                    && query.type_name.as_ref().is_none_or(|n| &t.name == n)
            });
            if !hit {
                diagnostics.push(format!(
                    "warning: type '{}/{}' names no type (expected namespace/name)",
                    query.type_namespace.as_deref().unwrap_or("*"),
                    query.type_name.as_deref().unwrap_or("*")
                ));
            }
        }
    }
    Ok(resolvable)
}

/// Resolve the container membership set once, if `container_id` is scoped.
fn member_set(
    store: &dyn RepositoryStore,
    container_id: &Option<String>,
) -> Result<Option<HashSet<String>>, RepositoryError> {
    match container_id {
        Some(cid) => Ok(Some(
            container_service::list_members(store, cid)?
                .into_iter()
                .collect(),
        )),
        None => Ok(None),
    }
}

/// Tier 2 (Record) structured pass + content match.
fn find_tier2(
    store: &dyn RepositoryStore,
    query: &DiscoveryQuery,
    field_text_index: &FieldTextIndex,
    needle: Option<&[String]>,
) -> Result<Vec<Candidate>, RepositoryError> {
    // Push type ns/name, container, and the first tag into the store query; the
    // remaining predicates are applied in-service below.
    let records = record_store::list_records_filtered(
        store,
        RecordListFilter {
            type_namespace: query.type_namespace.clone(),
            type_name: query.type_name.clone(),
            container_id: query.container_id.clone(),
            tag: query.tag.first().cloned(),
        },
    )?;

    let mut hits = Vec::new();
    for record in &records {
        if !record_matches_structured_predicates(record, query) {
            continue;
        }

        let (matched_fields, snippet) = match needle {
            Some(needle) => match_content(
                text_projection::project_text(record, field_text_index),
                needle,
            ),
            None => (Vec::new(), None),
        };
        if needle.is_some() && matched_fields.is_empty() {
            continue;
        }

        let mut selects = Vec::new();
        for (name, value) in record.field_values.iter() {
            if !field_text_index.is_closed_name(name) {
                continue;
            }
            let values: Vec<&str> = match value {
                serde_json::Value::String(s) => vec![s.as_str()],
                serde_json::Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
                _ => Vec::new(),
            };
            for v in values.into_iter().collect::<HashSet<_>>() {
                selects.push((name.clone(), v.to_string()));
            }
        }
        let hit = DiscoveryHit {
            instance_id: record.instance_id.clone(),
            uri: String::new(),
            type_id: Some(record.type_id.clone()),
            container_ids: Vec::new(),
            label: record_label::record_display_label(
                record,
                field_text_index.identity_field_ids(),
                field_text_index.names(),
            ),
            type_namespace: Some(record.type_namespace.clone()),
            type_name: Some(record.type_name.clone()),
            lifecycle_state: record.lifecycle_state.clone(),
            score: None,
            snippet,
            matched_fields,
        };
        hits.push(Candidate {
            hit,
            tags: record.tags.clone().unwrap_or_default(),
            selects,
        });
    }

    Ok(hits)
}

/// Tier 0 (Note) structured pass + content match. Notes carry no type binding or
/// lifecycle state — only `tag`, `containerId`, and `tier` apply.
fn find_tier0(
    store: &dyn RepositoryStore,
    query: &DiscoveryQuery,
    needle: Option<&[String]>,
) -> Result<Vec<Candidate>, RepositoryError> {
    let members = member_set(store, &query.container_id)?;
    let cat = store.catalog()?;

    let mut hits = Vec::new();
    for entry in &cat.instances {
        if entry.tier != Some(0) {
            continue;
        }
        if let Some(ref members) = members {
            if !members.contains(entry.id.as_str()) {
                continue;
            }
        }
        let locator = entry.locator.as_deref().unwrap_or_default();
        let body = store.load_instance_json(locator)?;
        let entry_ref =
            crate::store::instance_ref_from_body(entry.id.clone(), entry.tier.unwrap_or(0), &body);
        if !tags_match(&query.tag, &entry_ref.tags) {
            continue;
        }

        let note = crate::store::note_from_value(body, locator)?;
        let (matched_fields, snippet) = match needle {
            Some(needle) => match_content(text_projection::project_note_text(&note), needle),
            None => (Vec::new(), None),
        };
        if needle.is_some() && matched_fields.is_empty() {
            continue;
        }

        let hit = DiscoveryHit {
            label: note
                .title
                .clone()
                .unwrap_or_else(|| note.instance_id.clone()),
            instance_id: note.instance_id,
            uri: String::new(),
            type_id: None,
            container_ids: Vec::new(),
            type_namespace: None,
            type_name: None,
            lifecycle_state: None,
            score: None,
            snippet,
            matched_fields,
        };
        hits.push(Candidate {
            hit,
            tags: entry_ref.tags,
            selects: Vec::new(),
        });
    }

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use crate::store::RepositoryStore;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::field_type::{Cardinality, Datatype, ValueDomain};
    use srs_core::types::note::{Note, NoteSection};
    use srs_core::types::record::FieldValues;
    use std::path::PathBuf;

    const TITLE: &str = "00000000-0000-4000-8000-00000000f001";
    const STATEMENT: &str = "00000000-0000-4000-8000-00000000f002";

    // Distinct first-8-char prefixes so the file-store canonical path
    // (`<type>-<id[..8]>.json`) does not collide on roundtrip.
    const ID1: &str = "11111111-1111-4111-8111-111111111111";
    const ID2: &str = "22222222-2222-4222-8222-222222222222";
    const ID3: &str = "33333333-3333-4333-8333-333333333333";
    const NOTE1: &str = "44444444-4444-4444-8444-444444444444";

    fn field(id: &str, name: &str) -> Field {
        Field {
            schema: None,
            id: id.to_string(),
            namespace: "example".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            field_type: FieldType::text(),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn closed_field(id: &str, name: &str, list: bool) -> Field {
        let mut f = field(id, name);
        f.field_type = FieldType {
            value_domain: Some(ValueDomain::Closed),
            allowed_values: Some(vec!["a".into(), "b".into(), "x".into(), "y".into()]),
            cardinality: list.then_some(Cardinality::List),
            ..FieldType::new(Datatype::String)
        };
        f
    }

    /// `record` plus closed-field values: `kind` (string) and `areas` (list).
    fn with_values(mut r: Record, values: &[(&str, serde_json::Value)]) -> Record {
        for (k, v) in values {
            r.field_values.insert(*k, v.clone());
        }
        r
    }

    fn facet_values(c: &FacetCounts) -> Vec<(&str, usize)> {
        c.values
            .iter()
            .map(|v| (v.value.as_str(), v.count))
            .collect()
    }

    fn faceted_store() -> MemoryStore {
        let mut recs = fixtures();
        recs[0] = with_values(
            recs[0].clone(),
            &[
                ("kind", "a".into()),
                ("areas", serde_json::json!(["x", "y", "x"])),
            ],
        );
        recs[1] = with_values(
            recs[1].clone(),
            &[
                ("kind", "b".into()),
                ("areas", serde_json::json!([])),
                ("mixed", "free".into()),
            ],
        );
        // non-string values are skipped, not counted.
        recs[2] = with_values(
            recs[2].clone(),
            &[
                ("kind", serde_json::json!(5)),
                ("areas", serde_json::json!([1, "x"])),
            ],
        );
        store_with(recs)
    }

    fn package() -> Package {
        Package {
            id: "pkg-discovery".to_string(),
            namespace: "example".to_string(),
            name: "discovery".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                field(TITLE, "title"),
                field(STATEMENT, "decision_statement"),
                closed_field("00000000-0000-4000-8000-00000000f003", "kind", false),
                closed_field("00000000-0000-4000-8000-00000000f004", "areas", true),
                // Same name closed and open: never a facet.
                closed_field("00000000-0000-4000-8000-00000000f005", "mixed", false),
                field("00000000-0000-4000-8000-00000000f006", "mixed"),
            ],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn record(id: &str, title: &str, statement: &str, lifecycle: &str, tags: &[&str]) -> Record {
        Record {
            created_by: None,
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "00000000-0000-4000-8000-00000000d100".to_string(),
            type_version: 1,
            type_namespace: "governance".to_string(),
            type_name: "decision".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert("title", serde_json::json!(title));
                fv.insert("decision_statement", serde_json::json!(statement));
                fv
            },
            lifecycle_state: Some(lifecycle.to_string()),
            tags: (!tags.is_empty()).then(|| tags.iter().map(|t| t.to_string()).collect()),
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn fixtures() -> Vec<Record> {
        vec![
            record(
                ID1,
                "Adopt consent process",
                "We will use consent for changes",
                "ratified",
                &["policy"],
            ),
            record(
                ID2,
                "Retire pilot",
                "The pilot is replaced by the standing process",
                "superseded",
                &["ops", "policy"],
            ),
            record(
                ID3,
                "Budget cadence",
                "Review the budget monthly",
                "draft",
                &["finance"],
            ),
        ]
    }

    fn store_with(records: Vec<Record>) -> MemoryStore {
        let store = MemoryStore::new(
            Manifest {
                container: None,
                upstream_package: None,
                extra: std::collections::BTreeMap::new(),
                source_documents_path: None,
                root: PathBuf::from("/memory"),
            },
            package(),
        );
        let manifest = store.load_manifest().unwrap();
        for record in &records {
            let path = format!("records/{}.json", record.instance_id);
            store
                .save_instance_json(&path, &serde_json::to_value(record).unwrap())
                .unwrap();
        }
        store.save_manifest(&manifest).unwrap();
        store
    }

    fn note_fixture(id: &str, title: &str, sections: &[(&str, &str)], tags: &[&str]) -> Note {
        Note {
            created_by: None,
            instance_id: id.to_string(),
            title: Some(title.to_string()),
            tags: (!tags.is_empty()).then(|| tags.iter().map(|t| t.to_string()).collect()),
            sections: sections
                .iter()
                .map(|(name, content)| NoteSection {
                    name: name.to_string(),
                    label: None,
                    content: content.to_string(),
                    content_hint: None,
                    tags: None,
                })
                .collect(),
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    /// Extends [`store_with`]'s three Tier-2 fixtures with one Tier-0 Note
    /// (`NOTE1`, tag `policy`), built the same way `store_with` builds Tier 2:
    /// manual `InstanceIndexEntry` + `save_instance_json`.
    ///
    /// This used to also add a Tier-1 TypedRecord fixture (`TYPED1`); Tier 1 is
    /// retired (srs#448/rfc-decision-53635966, srs-rust#888) and its raw-JSON
    /// shape no longer classifies at catalog build — adding it here would make
    /// `store.catalog()` fatal for every test below ([R24]).
    fn store_with_note() -> MemoryStore {
        let store = store_with(fixtures());
        let manifest = store.load_manifest().unwrap();

        let note = note_fixture(
            NOTE1,
            "Research notes",
            &[(
                "background",
                "Full-text search needs a portable recall floor.",
            )],
            &["policy"],
        );
        let note_path = format!("records/notes/{NOTE1}.json");
        store
            .save_instance_json(&note_path, &serde_json::to_value(&note).unwrap())
            .unwrap();

        store.save_manifest(&manifest).unwrap();
        store
    }

    fn ids(result: &DiscoveryResult) -> Vec<&str> {
        result.hits.iter().map(|h| h.instance_id.as_str()).collect()
    }

    #[test]
    fn facets_count_the_whole_match_set_independent_of_paging() {
        let store = faceted_store();
        let full = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        let map = find(
            &store,
            DiscoveryQuery::default(),
            FindPage {
                limit: Some(0),
                offset: 5,
            },
        )
        .unwrap();
        assert!(map.hits.is_empty());
        assert_eq!(map.total, 3);
        assert_eq!(
            serde_json::to_value(&map.facets).unwrap(),
            serde_json::to_value(&full.facets).unwrap()
        );
        let f = &full.facets;
        assert_eq!(facet_values(&f.by_type), vec![("governance/decision", 3)]);
        assert_eq!(f.notes, 0);
        // policy on ID1+ID2, then ops, finance by value order.
        assert_eq!(
            facet_values(&f.tags),
            vec![("policy", 2), ("finance", 1), ("ops", 1)]
        );
        // `mixed` is closed in one field and open in another: not a facet.
        let names: Vec<&str> = f.fields.iter().map(|x| x.field.as_str()).collect();
        assert_eq!(names, vec!["areas", "kind"]);
        let areas = &f.fields[0].counts;
        // x counted once per instance despite the duplicate; the number is skipped.
        assert_eq!(facet_values(areas), vec![("x", 2), ("y", 1)]);
        assert_eq!(facet_values(&f.fields[1].counts), vec![("a", 1), ("b", 1)]);
    }

    #[test]
    fn facets_follow_the_filters_and_count_notes() {
        let store = store_with_note();
        let all = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        assert_eq!(all.facets.notes, 1);
        assert_eq!(facet_values(&all.facets.tags)[0], ("policy", 3));
        let typed = find(
            &store,
            DiscoveryQuery {
                type_name: Some("decision".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(typed.facets.notes, 0);
        assert_eq!(facet_values(&typed.facets.tags)[0], ("policy", 2));
        let json = serde_json::to_value(&typed.facets).unwrap();
        assert!(json.get("notes").is_none() && json.get("fields").is_none());
    }

    #[test]
    fn facet_values_are_bounded_with_an_other_count() {
        let map: BTreeMap<String, usize> = (0..FACET_TOP_N + 5)
            .map(|i| (format!("v{i:02}"), if i == 24 { 9 } else { 1 }))
            .collect();
        let c = FacetCounts::from_map(map);
        assert_eq!(c.values.len(), FACET_TOP_N);
        assert_eq!(c.values[0].value, "v24");
        assert_eq!(c.other, 5);
        assert_eq!(c.total(), FACET_TOP_N + 5 + 8);
    }

    #[test]
    fn no_predicates_returns_all_records() {
        let store = store_with(fixtures());
        let result = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        assert_eq!(result.total, 3);
    }

    #[test]
    fn lifecycle_state_filters_to_exact_include() {
        let store = store_with(fixtures());
        let result = find(
            &store,
            DiscoveryQuery {
                lifecycle_state: Some("ratified".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(ids(&result), vec![ID1]);
    }

    #[test]
    fn exclude_lifecycle_states_hides_listed_states() {
        let store = store_with(fixtures());
        // Hide superseded + closed (the governance default-hidden set); empty list
        // would be the "show all" override.
        let result = find(
            &store,
            DiscoveryQuery {
                exclude_lifecycle_states: vec!["superseded".to_string(), "closed".to_string()],
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        // ID2 is superseded and must be hidden; ID1 (ratified) + ID3 (draft) remain.
        assert_eq!(ids(&result), vec![ID1, ID3]);
    }

    #[test]
    fn content_match_is_all_words_any_order_any_field_and_superset_of_phrase() {
        let store = store_with(fixtures());
        let q = |m: &str| DiscoveryQuery {
            content_match: Some(m.to_string()),
            ..Default::default()
        };
        // Phrase match (title only) stays a match.
        let phrase = find(&store, q("consent process"), FindPage::default()).unwrap();
        assert_eq!(ids(&phrase), vec![ID1]);
        // Reversed order, and words split across title + statement fields.
        let rev = find(&store, q("process consent"), FindPage::default()).unwrap();
        assert_eq!(ids(&rev), vec![ID1]);
        let split = find(&store, q("adopt changes"), FindPage::default()).unwrap();
        assert_eq!(ids(&split), vec![ID1]);
        assert!(split.hits[0].matched_fields.len() >= 2);
        assert!(split.hits[0].snippet.is_some());
        // One missing word => no match.
        let none = find(&store, q("consent zzz"), FindPage::default()).unwrap();
        assert_eq!(none.total, 0);
    }

    #[test]
    fn rank_orders_by_score_with_title_above_body_and_keeps_the_hit_set() {
        let long_body = format!("{} consent", "filler ".repeat(300));
        let store = store_with(vec![
            // consent only in a long body field
            record(ID1, "Zzz", &long_body, "draft", &[]),
            // consent in the title
            record(ID2, "Consent", "short statement", "draft", &[]),
            // consent in a short statement
            record(ID3, "Aaa", "use consent", "draft", &[]),
        ]);
        let q = || DiscoveryQuery {
            content_match: Some("consent".to_string()),
            ..Default::default()
        };
        let ranked = |rank| FindPage {
            rank,
            ..Default::default()
        };
        let plain = find(&store, q(), ranked(false)).unwrap();
        assert_eq!(ids(&plain), vec![ID1, ID2, ID3]);
        assert!(plain.hits.iter().all(|h| h.score.is_none()));

        let result = find(&store, q(), ranked(true)).unwrap();
        // title > short statement > long body
        assert_eq!(ids(&result), vec![ID2, ID3, ID1]);
        assert!(result.hits.iter().all(|h| h.score.is_some_and(|s| s > 0.0)));
        assert_eq!(result.total, plain.total);
        // Deterministic.
        let again = find(&store, q(), ranked(true)).unwrap();
        assert_eq!(ids(&again), ids(&result));
        // Paging applies after ranking.
        let page = find(
            &store,
            q(),
            FindPage {
                limit: Some(1),
                offset: 1,
                rank: true,
            },
        )
        .unwrap();
        assert_eq!(ids(&page), vec![ID3]);
        // No contentMatch: rank is a no-op, id order, no score.
        let none = find(&store, DiscoveryQuery::default(), ranked(true)).unwrap();
        assert_eq!(ids(&none), vec![ID1, ID2, ID3]);
        assert!(none.hits.iter().all(|h| h.score.is_none()));
    }

    #[test]
    fn rank_ties_break_by_instance_id_and_notes_are_ranked() {
        let store = store_with_note();
        let result = find(
            &store,
            DiscoveryQuery {
                tag: vec!["policy".to_string()],
                content_match: Some("policy".to_string()),
                ..Default::default()
            },
            FindPage {
                rank: true,
                ..Default::default()
            },
        )
        .unwrap();
        // Same query, ranked and unranked, return the same set including the Tier-0 note.
        assert_eq!(result.total, 3);
        assert!(ids(&result).contains(&NOTE1));
        // Equal scores keep instanceId order.
        let scores: Vec<f32> = result.hits.iter().map(|h| h.score.unwrap()).collect();
        for w in result.hits.windows(2).zip(scores.windows(2)) {
            if w.1[0] == w.1[1] {
                assert!(w.0[0].instance_id < w.0[1].instance_id);
            }
            assert!(w.1[0] >= w.1[1]);
        }
    }

    #[test]
    fn unknown_type_and_container_filters_warn() {
        let store = store_with(fixtures());
        let r = find(
            &store,
            DiscoveryQuery {
                type_id: Some("nope".to_string()),
                container_id: Some("nada".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(r.total, 0);
        assert!(r
            .diagnostics
            .iter()
            .any(|d| d.contains("containerId 'nada'")));
        let r = find(
            &store,
            DiscoveryQuery {
                type_id: Some("nope".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert!(r.diagnostics.iter().any(|d| d.contains("typeId 'nope'")));
        let r = find(
            &store,
            DiscoveryQuery {
                type_namespace: Some("x".to_string()),
                type_name: Some("y".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert!(r.diagnostics.iter().any(|d| d.contains("x/y")));
    }

    #[test]
    fn content_match_searches_non_title_field_case_insensitively() {
        let store = store_with(fixtures());
        // "consent" lives only in the decision_statement (non-title) field — the
        // recall the removed web filter and projection service missed on body text.
        let result = find(
            &store,
            DiscoveryQuery {
                content_match: Some("CONSENT".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(ids(&result), vec![ID1]);
        assert!(result.hits[0]
            .matched_fields
            .contains(&"decision_statement".to_string()));
    }

    #[test]
    fn tag_predicate_is_and_conjunction() {
        let store = store_with(fixtures());
        let result = find(
            &store,
            DiscoveryQuery {
                tag: vec!["policy".to_string(), "ops".to_string()],
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        // Only the record carrying BOTH policy AND ops.
        assert_eq!(ids(&result), vec![ID2]);
    }

    #[test]
    fn type_and_container_compose_with_content() {
        let store = store_with(fixtures());
        let result = find(
            &store,
            DiscoveryQuery {
                type_namespace: Some("governance".to_string()),
                type_name: Some("decision".to_string()),
                content_match: Some("budget".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(ids(&result), vec![ID3]);
    }

    #[test]
    fn results_are_deterministic() {
        let store = store_with(fixtures());
        let a = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        let b = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        assert_eq!(ids(&a), ids(&b));
    }

    #[test]
    fn content_match_is_identical_across_stores_memory_to_file() {
        // Cross-store roundtrip (memory -> file) per CLAUDE.md storage rules, with a
        // match on the non-title `decision_statement` field.
        let store = store_with(fixtures());
        let query = DiscoveryQuery {
            content_match: Some("consent".to_string()),
            ..Default::default()
        };
        let from_memory = find(&store, query.clone(), FindPage::default()).unwrap();

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = find(&file_store, query, FindPage::default()).unwrap();

        assert_eq!(ids(&from_memory), vec![ID1]);
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "DiscoveryResult must be identical across stores (memory -> file)"
        );
    }

    #[test]
    fn hits_are_navigable() {
        let store = store_with_note();
        let r = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        let manifest = store.load_manifest().unwrap();
        let repo = resource_uri::repository_id(&manifest).unwrap_or_default();
        for h in &r.hits {
            assert_eq!(h.uri, format!("srs://{repo}/record/{}", h.instance_id));
        }
        let note = r.hits.iter().find(|h| h.instance_id == NOTE1).unwrap();
        assert!(note.type_id.is_none());
        let rec = r.hits.iter().find(|h| h.type_id.is_some()).unwrap();
        assert!(!rec.type_id.as_deref().unwrap().is_empty());
    }

    #[test]
    fn tier_filter_note_returns_only_tier_0() {
        let store = store_with_note();
        let result = find(
            &store,
            DiscoveryQuery {
                tier: Some(0),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(ids(&result), vec![NOTE1]);
        assert_eq!(result.hits[0].label, "Research notes");
        assert!(result.hits[0].type_namespace.is_none());
        assert!(result.hits[0].type_name.is_none());
        assert!(result.diagnostics.is_empty());
    }

    // tier_filter_typed_record_returns_only_tier_1 retired by srs-rust#888
    // (Tier 1 / TypedRecord retirement, srs#448/rfc-decision-53635966): a
    // `tier: 1` query can never match anything again — no instance ever
    // classifies into that tier — so there is nothing left to assert.

    #[test]
    fn empty_query_spans_both_tiers() {
        let store = store_with_note();
        let result = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        assert_eq!(result.total, 4);
        let hit_ids = ids(&result);
        assert!(hit_ids.contains(&NOTE1));
    }

    #[test]
    fn type_namespace_predicate_excludes_tier_0() {
        let store = store_with_note();
        let result = find(
            &store,
            DiscoveryQuery {
                type_namespace: Some("governance".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(result.total, 3);
        let hit_ids = ids(&result);
        assert!(!hit_ids.contains(&NOTE1));
    }

    #[test]
    fn content_match_recalls_note_text() {
        let store = store_with_note();
        // "recall floor" lives only in the Note's `background` section.
        let note = find(
            &store,
            DiscoveryQuery {
                content_match: Some("recall floor".to_string()),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(ids(&note), vec![NOTE1]);
    }

    #[test]
    fn tag_predicate_applies_uniformly_across_tiers() {
        let store = store_with_note();
        let result = find(
            &store,
            DiscoveryQuery {
                tag: vec!["policy".to_string()],
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        // ID1 (Record, tags=[policy]), ID2 (Record, tags=[ops, policy]), and NOTE1
        // all carry "policy".
        assert_eq!(ids(&result), vec![ID1, ID2, NOTE1]);
    }

    #[test]
    fn both_tiers_are_identical_across_stores_memory_to_file() {
        // Cross-store roundtrip (memory -> file) covering the Tier 0 path, per
        // CLAUDE.md storage rules.
        let store = store_with_note();
        let query = DiscoveryQuery::default();
        let from_memory = find(&store, query.clone(), FindPage::default()).unwrap();

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = find(&file_store, query, FindPage::default()).unwrap();

        assert_eq!(from_memory.total, 4);
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "DiscoveryResult must be identical across stores (memory -> file) across both tiers"
        );
    }

    #[test]
    fn paging_applies_after_sort_and_total_is_pre_paging() {
        let store = store_with(fixtures());
        let page = |limit, offset| {
            find(
                &store,
                DiscoveryQuery::default(),
                FindPage {
                    limit: Some(limit),
                    offset,
                    rank: false,
                },
            )
            .unwrap()
        };
        let all = find(&store, DiscoveryQuery::default(), FindPage::default()).unwrap();
        assert_eq!((all.total, all.hits.len()), (3, 3), "default is unbounded");
        let first = page(2, 0);
        let second = page(2, 2);
        assert_eq!((first.total, second.total), (3, 3));
        assert_eq!(ids(&first), ids(&all)[..2]);
        assert_eq!(ids(&second), ids(&all)[2..]);
        let none = page(0, 0);
        assert!(none.hits.is_empty());
        assert_eq!(none.total, 3);
        assert!(page(5, 10).hits.is_empty());
    }

    #[test]
    fn snippet_window_cases() {
        let short = "a short text with needle in it";
        assert_eq!(snippet_window(short, "needle"), short);

        let long = |before: usize, after: usize| {
            format!("{}needle{}", "x".repeat(before), "y".repeat(after))
        };
        // Middle: both cuts, match kept, window size bounded.
        let mid = snippet_window(&long(500, 500), "needle");
        assert!(mid.starts_with('…') && mid.ends_with('…') && mid.contains("needle"));
        assert_eq!(mid.chars().count(), SNIPPET_WINDOW + 2);
        // Near the start: no leading cut.
        let start = snippet_window(&long(5, 500), "needle");
        assert!(!start.starts_with('…') && start.ends_with('…') && start.contains("needle"));
        // Near the end: no trailing cut.
        let end = snippet_window(&long(500, 5), "needle");
        assert!(end.starts_with('…') && !end.ends_with('…') && end.contains("needle"));
        assert_eq!(end.chars().count(), SNIPPET_WINDOW + 1);
    }

    #[test]
    fn snippet_window_is_multibyte_safe() {
        let text = format!("{}démocratie{}", "é".repeat(300), "日本".repeat(300));
        let w = snippet_window(&text, "démocratie");
        assert!(w.contains("démocratie") && w.chars().count() <= SNIPPET_WINDOW + 2);
        // Normalization that changes byte length must not panic either.
        let w = snippet_window(&format!("{}İ needle", "İ".repeat(300)), "needle");
        assert!(w.chars().count() <= SNIPPET_WINDOW + 2);
    }
}
