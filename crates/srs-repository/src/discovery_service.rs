//! Layer-1 deterministic discovery — the shared `find` entry point for CLI,
//! bindings, and web (`ext:discovery`, RFC-012 / ADR-019).
//!
//! Composes existing services (it does not duplicate them): the structured filter
//! pass reuses [`record_store::list_records_filtered`] for Tier 2 and the manifest
//! instance index (via [`crate::container_service::list_members`] for container
//! scoping) for Tier 0/1; content matching reuses
//! [`text_projection::project_text`] / [`text_projection::project_note_text`] /
//! [`text_projection::project_typed_record_text`]; hit labels reuse
//! [`record_label::record_display_label`] for Tier 2 and the manifest `title`
//! (falling back to `instanceId`) for Tier 0/1. Substring content matching is the
//! recall floor — `score` is always `None` at Layer 1. A future `DiscoveryIndex`
//! (Layer 2) may add recall and ranking but must never drop a Layer-1 match.
//!
//! Discovery spans all three tiers (RFC-012 `R1`/`I-113`, `R11`/`I-123` — see
//! srs-rust#797): `typeId`/`typeNamespace`/`typeName`/`lifecycleState` are
//! Tier-2-only predicates and exclude Tier 0/1 instances outright when specified,
//! since those tiers carry none of those fields; `tag`, `containerId`, and `tier`
//! apply uniformly.

use crate::container_service;
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store::{self, RecordListFilter};
use crate::store::RepositoryStore;
use crate::text_projection::{self, FieldTextIndex, TextSegment};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A conjunction of structured predicates plus an optional content-match floor.
/// Mirrors `DiscoveryQuery` in `docs/schema/2.0/discovery.json`. Unspecified
/// predicates are wildcards.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    /// AND-conjunction: the instance's tags must contain ALL specified values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag: Vec<String>,
    /// Exact match on `Record.lifecycleState` (case-sensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// Exclude instances whose `lifecycleState` matches any listed value (RFC-011
    /// parity; applied after `lifecycle_state`). An empty list excludes nothing —
    /// the "show all" override for an authored default-hidden set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_lifecycle_states: Vec<String>,
    /// Instance tier filter (0=Note, 1=TypedRecord, 2=Record). Unspecified matches
    /// all tiers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    /// Free-text recall-floor predicate over the Text Projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_match: Option<String>,
}

/// Deterministic result: hits in stable order, total, and non-fatal diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub hits: Vec<DiscoveryHit>,
    pub total: usize,
    pub diagnostics: Vec<String>,
}

/// A single matched instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryHit {
    pub instance_id: String,
    pub label: String,
    /// `None` for Tier 0/1 instances, which carry no type binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    /// `None` for Tier 0/1 instances, which carry no type binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// `None` at Layer 1 (deterministic, unranked). Populated only by a Layer-2 index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// First matching segment's raw text, when a content match was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Field names (or sentinels) whose text matched the content predicate.
    pub matched_fields: Vec<String>,
}

/// Run a discovery query against the repository. See module docs for the contract.
pub fn find(
    store: &dyn RepositoryStore,
    query: DiscoveryQuery,
) -> Result<DiscoveryResult, RepositoryError> {
    let diagnostics = Vec::new();

    // One field-metadata pass: the text index also carries the field_id → name map
    // that Tier-2 hit-label resolution needs, so we avoid a second `list_fields` scan.
    let field_text_index = text_projection::build_field_text_index(store)?;

    let needle = query
        .content_match
        .as_deref()
        .map(text_projection::normalize)
        .filter(|q| !q.is_empty());

    let mut hits = Vec::new();

    if query.tier.is_none() || query.tier == Some(2) {
        hits.extend(find_tier2(
            store,
            &query,
            &field_text_index,
            needle.as_deref(),
        )?);
    }

    // Tier 0/1 carry no typeId/typeNamespace/typeName/lifecycleState — a query
    // constraining any of those predicates can never match them.
    let tier2_only_predicate = query.type_id.is_some()
        || query.type_namespace.is_some()
        || query.type_name.is_some()
        || query.lifecycle_state.is_some();

    if !tier2_only_predicate {
        if query.tier.is_none() || query.tier == Some(0) {
            hits.extend(find_tier0(store, &query, needle.as_deref())?);
        }
        if query.tier.is_none() || query.tier == Some(1) {
            hits.extend(find_tier1(store, &query, needle.as_deref())?);
        }
    }

    // Deterministic order independent of index/store iteration order.
    hits.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));

    let total = hits.len();
    Ok(DiscoveryResult {
        hits,
        total,
        diagnostics,
    })
}

/// AND-conjunction: `instance_tags` must contain every value in `query_tags`.
fn tags_match(query_tags: &[String], instance_tags: &[String]) -> bool {
    query_tags
        .iter()
        .all(|t| instance_tags.iter().any(|it| it == t))
}

/// Run the content-match recall floor over a projected segment stream: the first
/// matching segment's raw text becomes the snippet; every distinct matching field
/// name (first-seen order) becomes `matched_fields`.
fn match_content(segments: Vec<TextSegment>, needle: &str) -> (Vec<String>, Option<String>) {
    let mut matched_fields = Vec::new();
    let mut seen_fields = HashSet::new();
    let mut snippet = None;
    for seg in segments {
        if text_projection::normalize(&seg.text).contains(needle) {
            if snippet.is_none() {
                snippet = Some(seg.text.clone());
            }
            if seen_fields.insert(seg.field_name.clone()) {
                matched_fields.push(seg.field_name);
            }
        }
    }
    (matched_fields, snippet)
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
    needle: Option<&str>,
) -> Result<Vec<DiscoveryHit>, RepositoryError> {
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
        if let Some(type_id) = &query.type_id {
            if &record.type_id != type_id {
                continue;
            }
        }

        if !tags_match(&query.tag, record.tags.as_deref().unwrap_or(&[])) {
            continue;
        }

        if let Some(state) = &query.lifecycle_state {
            if record.lifecycle_state.as_deref() != Some(state.as_str()) {
                continue;
            }
        }

        // Exclusion axis: drop records whose lifecycleState is in the hidden set.
        // Records without a lifecycleState are never excluded by this axis.
        if !query.exclude_lifecycle_states.is_empty() {
            if let Some(state) = record.lifecycle_state.as_deref() {
                if query.exclude_lifecycle_states.iter().any(|s| s == state) {
                    continue;
                }
            }
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

        hits.push(DiscoveryHit {
            instance_id: record.instance_id.clone(),
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
        });
    }

    Ok(hits)
}

/// Tier 0 (Note) structured pass + content match. Notes carry no type binding or
/// lifecycle state — only `tag`, `containerId`, and `tier` apply.
fn find_tier0(
    store: &dyn RepositoryStore,
    query: &DiscoveryQuery,
    needle: Option<&str>,
) -> Result<Vec<DiscoveryHit>, RepositoryError> {
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

        hits.push(DiscoveryHit {
            label: note
                .title
                .clone()
                .unwrap_or_else(|| note.instance_id.clone()),
            instance_id: note.instance_id,
            type_namespace: None,
            type_name: None,
            lifecycle_state: None,
            score: None,
            snippet,
            matched_fields,
        });
    }

    Ok(hits)
}

/// Tier 1 (TypedRecord) structured pass + content match. No typed `TypedRecord`
/// struct exists yet, so the body is read via the generic-JSON shim
/// (`load_instance_json` — CLAUDE.md storage boundary rules) rather than a typed
/// logical-id method. TypedRecords carry no type binding or lifecycle state —
/// only `tag`, `containerId`, and `tier` apply.
fn find_tier1(
    store: &dyn RepositoryStore,
    query: &DiscoveryQuery,
    needle: Option<&str>,
) -> Result<Vec<DiscoveryHit>, RepositoryError> {
    let members = member_set(store, &query.container_id)?;
    let cat = store.catalog()?;

    let mut hits = Vec::new();
    for entry in &cat.instances {
        if entry.tier != Some(1) {
            continue;
        }
        if let Some(ref members) = members {
            if !members.contains(entry.id.as_str()) {
                continue;
            }
        }
        let locator = entry.locator.as_deref().unwrap_or_default();
        let value = store.load_instance_json(locator)?;
        let entry_ref =
            crate::store::instance_ref_from_body(entry.id.clone(), entry.tier.unwrap_or(1), &value);
        if !tags_match(&query.tag, &entry_ref.tags) {
            continue;
        }

        let (matched_fields, snippet) = match needle {
            Some(needle) => {
                match_content(text_projection::project_typed_record_text(&value), needle)
            }
            None => (Vec::new(), None),
        };
        if needle.is_some() && matched_fields.is_empty() {
            continue;
        }

        let title = value
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let label = title.unwrap_or_else(|| entry.id.clone());

        hits.push(DiscoveryHit {
            instance_id: entry.id.clone(),
            label,
            type_namespace: None,
            type_name: None,
            lifecycle_state: None,
            score: None,
            snippet,
            matched_fields,
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
    use srs_core::types::note::{Note, NoteSection};
    use srs_core::types::record::{FieldValues, Record};
    use std::path::PathBuf;

    const TITLE: &str = "00000000-0000-4000-8000-00000000f001";
    const STATEMENT: &str = "00000000-0000-4000-8000-00000000f002";

    // Distinct first-8-char prefixes so the file-store canonical path
    // (`<type>-<id[..8]>.json`) does not collide on roundtrip.
    const ID1: &str = "11111111-1111-4111-8111-111111111111";
    const ID2: &str = "22222222-2222-4222-8222-222222222222";
    const ID3: &str = "33333333-3333-4333-8333-333333333333";
    const NOTE1: &str = "44444444-4444-4444-8444-444444444444";
    const TYPED1: &str = "55555555-5555-4555-8555-555555555555";

    fn field(id: &str, name: &str) -> Field {
        Field {
            schema: None,
            id: id.to_string(),
            namespace: "example".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            },
            field_type: FieldType::text(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
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
            ],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn record(id: &str, title: &str, statement: &str, lifecycle: &str, tags: &[&str]) -> Record {
        Record {
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
                federation_path: None,
                upstream_package: None,
                federation_events_path: None,
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
    /// (`NOTE1`, tag `policy`) and one Tier-1 TypedRecord (`TYPED1`, tag
    /// `finance`), built the same way `store_with` builds Tier 2: manual
    /// `InstanceIndexEntry` + `save_instance_json`, since no typed `TypedRecord`
    /// struct exists yet (CLAUDE.md storage boundary rules).
    fn store_with_all_tiers() -> MemoryStore {
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

        let typed_tags = ["finance"];
        let typed = serde_json::json!({
            "instanceId": TYPED1,
            "title": "Budget planning",
            "fields": [
                { "name": "owner", "fieldType": {"datatype": "string"}, "value": "engineering" }
            ],
            "tags": typed_tags,
        });
        let typed_path = format!("records/typed-records/{TYPED1}.json");
        store.save_instance_json(&typed_path, &typed).unwrap();

        store.save_manifest(&manifest).unwrap();
        store
    }

    fn ids(result: &DiscoveryResult) -> Vec<&str> {
        result.hits.iter().map(|h| h.instance_id.as_str()).collect()
    }

    #[test]
    fn no_predicates_returns_all_records() {
        let store = store_with(fixtures());
        let result = find(&store, DiscoveryQuery::default()).unwrap();
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
        )
        .unwrap();
        // ID2 is superseded and must be hidden; ID1 (ratified) + ID3 (draft) remain.
        assert_eq!(ids(&result), vec![ID1, ID3]);
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
        )
        .unwrap();
        assert_eq!(ids(&result), vec![ID3]);
    }

    #[test]
    fn results_are_deterministic() {
        let store = store_with(fixtures());
        let a = find(&store, DiscoveryQuery::default()).unwrap();
        let b = find(&store, DiscoveryQuery::default()).unwrap();
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
        let from_memory = find(&store, query.clone()).unwrap();

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = find(&file_store, query).unwrap();

        assert_eq!(ids(&from_memory), vec![ID1]);
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "DiscoveryResult must be identical across stores (memory -> file)"
        );
    }

    #[test]
    fn tier_filter_note_returns_only_tier_0() {
        let store = store_with_all_tiers();
        let result = find(
            &store,
            DiscoveryQuery {
                tier: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&result), vec![NOTE1]);
        assert_eq!(result.hits[0].label, "Research notes");
        assert!(result.hits[0].type_namespace.is_none());
        assert!(result.hits[0].type_name.is_none());
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn tier_filter_typed_record_returns_only_tier_1() {
        let store = store_with_all_tiers();
        let result = find(
            &store,
            DiscoveryQuery {
                tier: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&result), vec![TYPED1]);
        assert_eq!(result.hits[0].label, "Budget planning");
    }

    #[test]
    fn empty_query_spans_all_three_tiers() {
        let store = store_with_all_tiers();
        let result = find(&store, DiscoveryQuery::default()).unwrap();
        assert_eq!(result.total, 5);
        let hit_ids = ids(&result);
        assert!(hit_ids.contains(&NOTE1));
        assert!(hit_ids.contains(&TYPED1));
    }

    #[test]
    fn type_namespace_predicate_excludes_tier_0_and_1() {
        let store = store_with_all_tiers();
        let result = find(
            &store,
            DiscoveryQuery {
                type_namespace: Some("governance".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total, 3);
        let hit_ids = ids(&result);
        assert!(!hit_ids.contains(&NOTE1));
        assert!(!hit_ids.contains(&TYPED1));
    }

    #[test]
    fn content_match_recalls_note_and_typed_record_text() {
        let store = store_with_all_tiers();
        // "engineering" lives only in the TypedRecord's `owner` field.
        let typed = find(
            &store,
            DiscoveryQuery {
                content_match: Some("engineering".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&typed), vec![TYPED1]);

        // "recall floor" lives only in the Note's `background` section.
        let note = find(
            &store,
            DiscoveryQuery {
                content_match: Some("recall floor".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&note), vec![NOTE1]);
    }

    #[test]
    fn tag_predicate_applies_uniformly_across_tiers() {
        let store = store_with_all_tiers();
        let result = find(
            &store,
            DiscoveryQuery {
                tag: vec!["policy".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
        // ID1 (Record, tags=[policy]), ID2 (Record, tags=[ops, policy]), and NOTE1
        // all carry "policy"; TYPED1 (finance) does not.
        assert_eq!(ids(&result), vec![ID1, ID2, NOTE1]);
    }

    #[test]
    fn all_tiers_are_identical_across_stores_memory_to_file() {
        // Cross-store roundtrip (memory -> file) covering the new Tier 0/1 path,
        // per CLAUDE.md storage rules.
        let store = store_with_all_tiers();
        let query = DiscoveryQuery::default();
        let from_memory = find(&store, query.clone()).unwrap();

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = find(&file_store, query).unwrap();

        assert_eq!(from_memory.total, 5);
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "DiscoveryResult must be identical across stores (memory -> file) across all three tiers"
        );
    }
}
