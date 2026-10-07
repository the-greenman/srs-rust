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
        Self::from_entries(
            map.into_iter().map(|(value, count)| (value, None, count)),
            FACET_TOP_N,
        )
    }

    /// `(value, typeId, count)` entries; `cap` 0 keeps every value.
    fn from_entries(
        entries: impl Iterator<Item = (String, Option<String>, usize)>,
        cap: usize,
    ) -> Self {
        let mut values: Vec<FacetCount> = entries
            .map(|(value, type_id, count)| FacetCount {
                value,
                type_id,
                count,
            })
            .collect();
        // Stable sort over BTreeMap order: count descending, then value ascending.
        values.sort_by_key(|a| std::cmp::Reverse(a.count));
        let keep = if cap == 0 {
            values.len()
        } else {
            values.len().min(cap)
        };
        let other = values.split_off(keep);
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
    /// The Type's id; set on `byType` values only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
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

fn build_facets(candidates: &[Candidate], by_type_limit: usize) -> DiscoveryFacets {
    let mut by_type: BTreeMap<String, (Option<String>, usize)> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut fields: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut notes = 0;
    for c in candidates {
        match (&c.hit.type_namespace, &c.hit.type_name) {
            (Some(ns), Some(name)) => {
                let e = by_type.entry(format!("{ns}/{name}")).or_default();
                e.0 = e.0.take().or_else(|| c.hit.type_id.clone());
                e.1 += 1;
            }
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
        by_type: FacetCounts::from_entries(
            by_type.into_iter().map(|(v, (id, n))| (v, id, n)),
            by_type_limit,
        ),
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
/// matches are returned; only [`FindPage::match_mode`] widens which instances match,
/// and always to a superset of the RFC-012 recall floor (I-114).
/// `limit: None` means every match; any default cap is the adapter's choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindPage {
    pub limit: Option<usize>,
    pub offset: usize,
    /// Order content-match hits by BM25 relevance and fill `score` (srs-rust#1228)
    /// instead of by `instanceId`. Never changes which instances match. Off by
    /// default; the MCP `find` tool turns it on. Ignored without a `contentMatch`.
    pub rank: bool,
    /// How the words of a `contentMatch` combine (srs-rust#1284). Ignored without one.
    pub match_mode: MatchMode,
    /// Cap on `facets.byType` values (the rest are summed into `other`). `None` = the
    /// default [`FACET_TOP_N`]; `Some(0)` = every type. Never affects hits or `total`.
    pub by_type_limit: Option<usize>,
}

/// How the words of a `contentMatch` combine. Both modes return a superset of the
/// contiguous-phrase recall floor (RFC-012 I-114 permits extra recall; I-117 keeps the
/// structured predicates conjunctive either way), so the choice is retrieval policy,
/// not a spec predicate — which is why it lives on [`FindPage`], not [`DiscoveryQuery`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    /// Every word must occur, in any field and any order (srs-rust#1218).
    #[default]
    All,
    /// For questions typed as sentences: an instance matches when it contains at least one
    /// *significant* query word as a whole token, or every query word (the all-words set,
    /// so the phrase recall floor holds). A significant word has at least
    /// [`MIN_SIGNIFICANT_CHARS`] chars and occurs as a token in at most
    /// [`MAX_COMMON_FRACTION`] of instances, so "the", "what", "how" never match on their
    /// own, in any language, without a stopword list. Always ranked (BM25 over the
    /// significant words): the set is wide by design. A query with no significant word
    /// behaves as [`MatchMode::All`].
    Any,
}

impl std::str::FromStr for MatchMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "all" => Ok(MatchMode::All),
            "any" => Ok(MatchMode::Any),
            other => Err(format!("invalid match mode '{other}' (expected all|any)")),
        }
    }
}

/// Shortest word that counts in [`MatchMode::Any`]: shorter words ("a", "in", "of") are
/// noise. The BM25 index's own term threshold, so the two agree on what a term is.
pub const MIN_SIGNIFICANT_CHARS: usize = crate::discovery_index::MIN_TERM_CHARS;

/// A word occurring as a token in more than this share of instances is too common to
/// select anything in [`MatchMode::Any`] (corpus-derived, so language-neutral).
pub const MAX_COMMON_FRACTION: f64 = 0.5;

/// The content predicate of one query, built by [`content_needle`].
struct ContentNeedle {
    /// Every normalized query word: the all-words (substring) match.
    words: Vec<String>,
    /// [`MatchMode::Any`]: the significant words, matched as whole tokens. `None` = all-words.
    any: Option<Vec<String>>,
}

impl ContentNeedle {
    /// The words BM25 ranks by: the significant ones in any-mode, all of them otherwise.
    fn rank_words(&self) -> &[String] {
        self.any.as_deref().unwrap_or(&self.words)
    }
}

fn content_needle(
    store: &dyn RepositoryStore,
    field_text_index: &FieldTextIndex,
    content_match: Option<&str>,
    mode: MatchMode,
) -> Result<Option<ContentNeedle>, RepositoryError> {
    let words: Vec<String> = content_match
        .map(|q| {
            text_projection::normalize(q)
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if words.is_empty() {
        return Ok(None);
    }
    let mut any = None;
    if mode == MatchMode::Any {
        let index = discovery_index(store, field_text_index)?;
        let mut significant: Vec<String> = Vec::new();
        for w in &words {
            // The index's own tokenisation, so a word like "co-op" yields its parts.
            for t in crate::discovery_index::tokens(w) {
                if index.token_document_fraction(t) <= MAX_COMMON_FRACTION
                    && !significant.iter().any(|s| s == t)
                {
                    significant.push(t.to_string());
                }
            }
        }
        if !significant.is_empty() {
            any = Some(significant);
        }
    }
    Ok(Some(ContentNeedle { words, any }))
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

    // All-words (srs-rust#1218) or any-significant-word (srs-rust#1284) match: both a
    // superset of the phrase match, so the RFC-012 recall floor holds.
    let content = content_needle(
        store,
        &field_text_index,
        query.content_match.as_deref(),
        page.match_mode,
    )?;
    let needle = content.as_ref().map(|c| Needle {
        words: &c.words,
        any: c.any.as_deref(),
    });

    let mut candidates = collect_candidates(store, &query, &field_text_index, needle)?;

    // Deterministic order independent of index/store iteration order.
    candidates.sort_by(|a, b| a.hit.instance_id.cmp(&b.hit.instance_id));

    // Facets count the Layer-1 match set: independent of ranking and paging.
    let facets = build_facets(&candidates, page.by_type_limit.unwrap_or(FACET_TOP_N));
    let mut hits: Vec<DiscoveryHit> = candidates.into_iter().map(|c| c.hit).collect();
    if let Some(c) = &content {
        // Any-mode is always ranked: its match set is wide by design (srs-rust#1284).
        if page.rank || c.any.is_some() {
            rank_hits(store, &field_text_index, c.rank_words(), &mut hits)?;
        }
    }

    finish_page(store, hits, facets, page, diagnostics)
}

/// Apply paging, then fill the navigation fields (uri, containerIds) for the returned page only.
fn finish_page(
    store: &dyn RepositoryStore,
    hits: Vec<DiscoveryHit>,
    facets: DiscoveryFacets,
    page: FindPage,
    diagnostics: Vec<String>,
) -> Result<DiscoveryResult, RepositoryError> {
    let total = hits.len();
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

/// The store-cached [`DiscoveryIndex`], built on first use.
fn discovery_index(
    store: &dyn RepositoryStore,
    field_text_index: &FieldTextIndex,
) -> Result<Rc<dyn DiscoveryIndex>, RepositoryError> {
    let cached = store
        .discovery_index_cache()
        .and_then(|c| c.borrow().clone());
    Ok(match cached {
        Some(index) => index,
        None => {
            let index: Rc<dyn DiscoveryIndex> = Rc::new(Bm25Index::build(store, field_text_index)?);
            if let Some(slot) = store.discovery_index_cache() {
                *slot.borrow_mut() = Some(index.clone());
            }
            index
        }
    })
}

/// Terms of the source instance used as the "more like this" query.
const SIMILAR_TERMS: usize = 10;

/// "More like this" (srs-rust#1230): instances whose text overlaps the most characteristic terms of
/// `instance_id`, ranked by the same [`DiscoveryIndex`] as `find --rank`, in the normal hit shape.
///
/// The structured predicates of `query` compose exactly as in [`find`]; `content_match` is not
/// accepted (the source is the query). The source itself is never returned, `score` is always
/// filled, and `total` counts the instances sharing at least one term. Deterministic; ties by
/// `instanceId`. `page.rank` is irrelevant: similar hits are always ranked.
pub fn similar(
    store: &dyn RepositoryStore,
    instance_id: &str,
    query: DiscoveryQuery,
    page: FindPage,
) -> Result<DiscoveryResult, RepositoryError> {
    if query.content_match.is_some() {
        return Err(RepositoryError::InvalidInput {
            message: "similar takes no contentMatch: the source instance is the query".into(),
        });
    }
    if store.find_instance(instance_id)?.is_none() {
        return Err(RepositoryError::InstanceNotFound {
            id: instance_id.to_string(),
        });
    }
    let mut diagnostics = Vec::new();
    if !unresolved_filters(store, &query, &mut diagnostics)? {
        return Ok(DiscoveryResult {
            hits: Vec::new(),
            total: 0,
            facets: DiscoveryFacets::default(),
            diagnostics,
        });
    }
    let field_text_index = text_projection::build_field_text_index(store)?;
    let index = discovery_index(store, &field_text_index)?;
    let words = index.top_terms(instance_id, SIMILAR_TERMS).ok_or_else(|| {
        RepositoryError::InstanceNotFound {
            id: instance_id.to_string(),
        }
    })?;

    if words.is_empty() {
        diagnostics.push(format!(
            "warning: {instance_id} has no similarity terms (no word of 3+ characters)"
        ));
    }
    let mut candidates = collect_candidates(store, &query, &field_text_index, None)?;
    candidates.retain(|c| c.hit.instance_id != instance_id);
    candidates.sort_by(|a, b| a.hit.instance_id.cmp(&b.hit.instance_id));
    let ids: Vec<&str> = candidates
        .iter()
        .map(|c| c.hit.instance_id.as_str())
        .collect();
    let scores = index.score(&words, &ids);
    for (c, score) in candidates.iter_mut().zip(scores) {
        c.hit.score = Some(score);
    }
    candidates.retain(|c| c.hit.score.unwrap_or(0.0) > 0.0);
    // Stable sort over an id-ordered list: equal scores keep id order.
    candidates.sort_by(|a, b| {
        b.hit
            .score
            .unwrap_or(0.0)
            .total_cmp(&a.hit.score.unwrap_or(0.0))
    });
    // Facets count the similar set, as `find`'s count its match set.
    let facets = build_facets(&candidates, page.by_type_limit.unwrap_or(FACET_TOP_N));
    let hits = candidates.into_iter().map(|c| c.hit).collect();
    finish_page(store, hits, facets, page, diagnostics)
}

/// Every instance passing the structured predicates (and the content words, if any), Tier 2 then Tier 0.
fn collect_candidates(
    store: &dyn RepositoryStore,
    query: &DiscoveryQuery,
    field_text_index: &FieldTextIndex,
    needle: Option<Needle>,
) -> Result<Vec<Candidate>, RepositoryError> {
    let mut hits = Vec::new();

    if query.tier.is_none() || query.tier == Some(2) {
        hits.extend(find_tier2(store, query, field_text_index, needle)?);
    }

    // Tier 0 carries no typeId/typeNamespace/typeName/lifecycleState — a query
    // constraining any of those predicates can never match it.
    let tier2_only_predicate = query.type_id.is_some()
        || query.type_namespace.is_some()
        || query.type_name.is_some()
        || query.lifecycle_state.is_some()
        || !query.lifecycle_states.is_empty();

    if !tier2_only_predicate && (query.tier.is_none() || query.tier == Some(0)) {
        hits.extend(find_tier0(store, query, needle)?);
    }
    Ok(hits)
}

/// Fill `score` from the (store-cached) [`DiscoveryIndex`] and reorder by score,
/// ties by `instanceId`. `hits` must already be in `instanceId` order.
fn rank_hits(
    store: &dyn RepositoryStore,
    field_text_index: &FieldTextIndex,
    words: &[String],
    hits: &mut [DiscoveryHit],
) -> Result<(), RepositoryError> {
    let index = discovery_index(store, field_text_index)?;
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

/// The content predicate of one query, borrowed from a [`ContentNeedle`].
#[derive(Clone, Copy)]
struct Needle<'a> {
    /// Every query word, matched as a substring: the all-words match.
    words: &'a [String],
    /// [`MatchMode::Any`]: significant words, any one of which suffices as a whole token.
    any: Option<&'a [String]>,
}

/// Matched fields and snippet collected while scanning segments.
#[derive(Default)]
struct SegmentHits {
    fields: Vec<String>,
    seen: HashSet<String>,
    snippet: Option<String>,
}

impl SegmentHits {
    fn add(&mut self, seg: &TextSegment, word: &str) {
        if self.snippet.is_none() {
            self.snippet = Some(snippet_window(&seg.text, word));
        }
        if self.seen.insert(seg.field_name.clone()) {
            self.fields.push(seg.field_name.clone());
        }
    }
}

/// Run the content-match recall floor over a projected segment stream. A record
/// matches when every word of `needle.words` occurs in some segment (any field, any
/// order), or, with `needle.any`, when one of those significant words occurs as a whole
/// token. The first matching segment supplies the snippet (windowed on that word); every
/// field with a matching segment becomes `matched_fields` (first-seen order). When both
/// hold, the token match decides the fields and snippet. Empty result means no match.
fn match_content(segments: Vec<TextSegment>, needle: Needle) -> (Vec<String>, Option<String>) {
    let words = needle.words;
    let mut found = vec![false; words.len()];
    let (mut substring, mut token) = (SegmentHits::default(), SegmentHits::default());
    for seg in &segments {
        let norm = text_projection::normalize(&seg.text);
        let mut first_hit = None;
        for (i, w) in words.iter().enumerate() {
            if norm.contains(w.as_str()) {
                found[i] = true;
                first_hit.get_or_insert(w);
            }
        }
        if let Some(w) = first_hit {
            substring.add(seg, w);
        }
        if let Some(significant) = needle.any {
            let toks: HashSet<&str> = crate::discovery_index::tokens(&norm).collect();
            if let Some(w) = significant.iter().find(|w| toks.contains(w.as_str())) {
                token.add(seg, w);
            }
        }
    }
    if !token.fields.is_empty() {
        (token.fields, token.snippet)
    } else if found.iter().all(|f| *f) {
        (substring.fields, substring.snippet)
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
    needle: Option<Needle>,
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
    needle: Option<Needle>,
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
                ..Default::default()
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

    fn typed_candidate(i: usize) -> Candidate {
        Candidate {
            hit: DiscoveryHit {
                instance_id: format!("i{i}"),
                uri: String::new(),
                label: String::new(),
                type_id: Some(format!("tid-{}", i % 25)),
                container_ids: vec![],
                type_namespace: Some("ns".into()),
                type_name: Some(format!("t{:02}", i % 25)),
                lifecycle_state: None,
                score: None,
                snippet: None,
                matched_fields: vec![],
            },
            tags: vec![],
            selects: vec![],
        }
    }

    #[test]
    fn by_type_cap_is_a_request_option_and_other_stays_correct() {
        let cands: Vec<Candidate> = (0..60).map(typed_candidate).collect(); // 25 types
        let capped = build_facets(&cands, FACET_TOP_N).by_type;
        assert_eq!(capped.values.len(), FACET_TOP_N);
        assert_eq!(capped.total(), 60);
        assert!(capped.other > 0);
        let all = build_facets(&cands, 0).by_type;
        assert_eq!((all.values.len(), all.other, all.total()), (25, 0, 60));
        // count desc, then name; typeId rides along with its value.
        assert_eq!(all.values[0].value, "ns/t00");
        assert_eq!(all.values[0].type_id.as_deref(), Some("tid-0"));
        let three = build_facets(&cands, 3).by_type;
        assert_eq!(three.values.len(), 3);
        assert_eq!(three.total(), 60);
    }

    #[test]
    fn by_type_carries_type_id_and_sums_to_total() {
        let store = faceted_store();
        let r = find(
            &store,
            DiscoveryQuery::default(),
            FindPage {
                limit: Some(1),
                by_type_limit: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        let sum: usize = r.facets.by_type.values.iter().map(|v| v.count).sum();
        assert_eq!(sum, r.total);
        assert!(r.facets.by_type.values.iter().all(|v| v.type_id.is_some()));
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
    fn match_any_needs_one_significant_word_and_keeps_the_recall_floor() {
        let store = store_with(fixtures());
        let q = |m: &str| DiscoveryQuery {
            content_match: Some(m.to_string()),
            ..Default::default()
        };
        let any = FindPage {
            match_mode: MatchMode::Any,
            ..Default::default()
        };
        // A sentence: "zzz" and the other words are absent, "consent" is enough.
        let sentence = find(&store, q("how is consent zzz handled"), any).unwrap();
        assert_eq!(ids(&sentence), vec![ID1]);
        // The same query under the default all-words mode finds nothing.
        let all = find(&store, q("how is consent zzz handled"), FindPage::default()).unwrap();
        assert_eq!(all.total, 0);
        // Recall floor: every all-words (hence every phrase) match is an any-word match.
        for m in ["consent process", "adopt changes", "consent"] {
            let all = find(&store, q(m), FindPage::default()).unwrap();
            let any = find(&store, q(m), any).unwrap();
            assert!(ids(&all).iter().all(|id| ids(&any).contains(id)), "{m}");
        }
        // Words shorter than MIN_SIGNIFICANT_CHARS never match on their own...
        let short = find(&store, q("zzz of a"), any).unwrap();
        assert_eq!(short.total, 0);
        // ...unless the query has no significant word: then every word must occur (all-words).
        let only_short = find(&store, q("of"), any).unwrap();
        let only_short_all = find(&store, q("of"), FindPage::default()).unwrap();
        assert_eq!(ids(&only_short), ids(&only_short_all));
    }

    /// A corpus where "consent" is in 2 of 6 records (significant) and "the" in 5 of 6 (common).
    fn any_mode_corpus() -> MemoryStore {
        const ID4: &str = "44444444-4444-4444-8444-444444444444";
        const ID5: &str = "55555555-5555-4555-8555-555555555555";
        const ID6: &str = "66666666-6666-4666-8666-666666666666";
        store_with(vec![
            record(ID1, "Consent", "the short statement", "draft", &[]),
            record(
                ID2,
                "Consent process",
                "adopt the changes by consent",
                "draft",
                &[],
            ),
            record(ID3, "Aaa", "the other unrelated", "draft", &[]),
            record(ID4, "Bbb", "the minutes", "draft", &[]),
            record(ID5, "Ccc", "the roles", "draft", &[]),
            record(ID6, "Ddd", "another thing", "draft", &[]),
        ])
    }

    #[test]
    fn match_any_ranks_records_matching_more_words_first() {
        let store = any_mode_corpus();
        let q = DiscoveryQuery {
            content_match: Some("what is the consent process".to_string()),
            ..Default::default()
        };
        let page = FindPage {
            rank: true,
            match_mode: MatchMode::Any,
            ..Default::default()
        };
        let ranked = find(&store, q, page).unwrap();
        // "the" is in 5 of 6 records: too common to select anything on its own.
        assert_eq!(ids(&ranked), vec![ID2, ID1]);
        assert!(ranked.hits.iter().all(|h| h.score.is_some()));
    }

    #[test]
    fn match_any_matches_whole_tokens_not_substrings_and_is_always_ranked() {
        let store = any_mode_corpus();
        let q = |m: &str| DiscoveryQuery {
            content_match: Some(m.to_string()),
            ..Default::default()
        };
        let any = FindPage {
            match_mode: MatchMode::Any,
            ..Default::default()
        };
        // "her" is a substring of "other" (ID3) but not a token anywhere: no match.
        assert_eq!(find(&store, q("her zzz"), any).unwrap().total, 0);
        // "other" is a token of ID3 only — and not a substring hit on "another" (ID6).
        assert_eq!(ids(&find(&store, q("other zzz"), any).unwrap()), vec![ID3]);
        // Ranked even though `rank` is false: scores filled, best first.
        let ranked = find(&store, q("consent process"), any).unwrap();
        assert_eq!(ids(&ranked), vec![ID2, ID1]);
        assert!(ranked.hits.iter().all(|h| h.score.is_some()));
        // A query of only common words behaves as all-words.
        let common = find(&store, q("the"), any).unwrap();
        let all = find(&store, q("the"), FindPage::default()).unwrap();
        assert_eq!(common.total, all.total);
    }

    #[test]
    fn match_mode_parses_wire_spellings() {
        assert_eq!("all".parse(), Ok(MatchMode::All));
        assert_eq!("any".parse(), Ok(MatchMode::Any));
        assert!("some".parse::<MatchMode>().is_err());
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
                ..Default::default()
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
    fn similar_excludes_source_and_ranks_related_first() {
        let store = store_with(fixtures());
        let result = similar(&store, ID1, DiscoveryQuery::default(), FindPage::default()).unwrap();
        // ID2 shares "process" and the "policy" tag; ID3 shares nothing; the source is excluded.
        assert_eq!(ids(&result), vec![ID2]);
        assert!(result.hits[0].score.unwrap() > 0.0);
        assert!(result.hits[0].uri.ends_with(ID2));
    }

    #[test]
    fn similar_is_deterministic() {
        let store = store_with_note();
        let run = || similar(&store, ID1, DiscoveryQuery::default(), FindPage::default()).unwrap();
        let (a, b) = (run(), run());
        assert_eq!(ids(&a), ids(&b));
        assert!(!ids(&a).contains(&ID1));
    }

    #[test]
    fn similar_composes_structured_predicates() {
        let store = store_with_note();
        let all = similar(&store, ID1, DiscoveryQuery::default(), FindPage::default()).unwrap();
        assert!(ids(&all).contains(&NOTE1) && ids(&all).contains(&ID2));
        let tier2 = similar(
            &store,
            ID1,
            DiscoveryQuery {
                tier: Some(2),
                ..Default::default()
            },
            FindPage::default(),
        )
        .unwrap();
        assert_eq!(ids(&tier2), vec![ID2]);
        // A note can be the source too.
        let from_note = similar(
            &store,
            NOTE1,
            DiscoveryQuery::default(),
            FindPage::default(),
        );
        assert!(from_note
            .unwrap()
            .hits
            .iter()
            .all(|h| h.instance_id != NOTE1));
    }

    #[test]
    fn similar_unknown_id_and_contentmatch_rejected() {
        let store = store_with(fixtures());
        let unknown = similar(
            &store,
            "nope",
            DiscoveryQuery::default(),
            FindPage::default(),
        );
        assert!(matches!(
            unknown,
            Err(RepositoryError::InstanceNotFound { .. })
        ));
        let with_text = similar(
            &store,
            ID1,
            DiscoveryQuery {
                content_match: Some("x".into()),
                ..Default::default()
            },
            FindPage::default(),
        );
        assert!(matches!(
            with_text,
            Err(RepositoryError::InvalidInput { .. })
        ));
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
                    ..Default::default()
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
