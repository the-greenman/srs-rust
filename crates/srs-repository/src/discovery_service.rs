//! Layer-1 deterministic discovery — the shared `find` entry point for CLI,
//! bindings, and web (`ext:discovery`, RFC-012 / ADR-019).
//!
//! Composes existing services (it does not duplicate them): the structured filter
//! pass reuses [`record_store::list_records_filtered`]; content matching reuses
//! [`text_projection::project_text`]; hit labels reuse
//! [`record_label::record_display_label`]. Substring content matching is the
//! recall floor — `score` is always `None` at Layer 1. A future `DiscoveryIndex`
//! (Layer 2) may add recall and ranking but must never drop a Layer-1 match.

use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store::{self, RecordListFilter};
use crate::store::RepositoryStore;
use crate::text_projection;
use serde::{Deserialize, Serialize};

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
    /// Instance tier (0=Note, 1=TypedRecord, 2=Record). Phase 1 serves Tier 2.
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
    pub type_namespace: String,
    pub type_name: String,
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
    let mut diagnostics = Vec::new();

    // Tier predicate: Phase 1 composes `list_records_filtered`, which yields Tier-2
    // Records only. Tier 0/1 projection is deferred (ADR-019).
    if let Some(tier) = query.tier {
        if tier != 2 {
            diagnostics.push(format!(
                "tier {tier} discovery is deferred; Phase 1 serves Tier 2 only"
            ));
            return Ok(DiscoveryResult {
                hits: Vec::new(),
                total: 0,
                diagnostics,
            });
        }
    }

    // 1. Structured pass — push type ns/name, container, and the first tag into the
    //    store query; the remaining predicates are applied in-service below.
    let records = record_store::list_records_filtered(
        store,
        RecordListFilter {
            type_namespace: query.type_namespace.clone(),
            type_name: query.type_name.clone(),
            container_id: query.container_id.clone(),
            tag: query.tag.first().cloned(),
        },
    )?;

    // One field-metadata pass: the text index also carries the field_id → name map
    // that hit-label resolution needs, so we avoid a second `list_fields` scan.
    let field_text_index = text_projection::build_field_text_index(store)?;

    let needle = query
        .content_match
        .as_deref()
        .map(text_projection::normalize)
        .filter(|q| !q.is_empty());

    let mut hits = Vec::new();
    for record in &records {
        if let Some(type_id) = &query.type_id {
            if &record.type_id != type_id {
                continue;
            }
        }

        // Remaining tags (AND) beyond the one pushed to the store query.
        if query.tag.len() > 1 {
            let record_tags = record.tags.as_deref().unwrap_or(&[]);
            let has_all = query
                .tag
                .iter()
                .all(|t| record_tags.iter().any(|rt| rt == t));
            if !has_all {
                continue;
            }
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

        // 2 + 3. Content match (recall floor) over the text projection.
        let mut matched_fields = Vec::new();
        let mut seen_fields = std::collections::HashSet::new();
        let mut snippet = None;
        if let Some(needle) = &needle {
            for seg in text_projection::project_text(record, &field_text_index) {
                if text_projection::normalize(&seg.text).contains(needle) {
                    if snippet.is_none() {
                        snippet = Some(seg.text.clone());
                    }
                    if seen_fields.insert(seg.field_name.clone()) {
                        matched_fields.push(seg.field_name);
                    }
                }
            }
            if matched_fields.is_empty() {
                continue;
            }
        }

        hits.push(DiscoveryHit {
            instance_id: record.instance_id.clone(),
            label: record_label::record_display_label(record, field_text_index.names()),
            type_namespace: record.type_namespace.clone(),
            type_name: record.type_name.clone(),
            lifecycle_state: record.lifecycle_state.clone(),
            score: None,
            snippet,
            matched_fields,
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::InstanceIndexEntry;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use crate::store::RepositoryStore;
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record::{FieldValue, Record};
    use std::collections::HashMap;
    use std::path::PathBuf;

    const TITLE: &str = "00000000-0000-4000-8000-00000000f001";
    const STATEMENT: &str = "00000000-0000-4000-8000-00000000f002";

    // Distinct first-8-char prefixes so the file-store canonical path
    // (`<type>-<id[..8]>.json`) does not collide on roundtrip.
    const ID1: &str = "11111111-1111-4111-8111-111111111111";
    const ID2: &str = "22222222-2222-4222-8222-222222222222";
    const ID3: &str = "33333333-3333-4333-8333-333333333333";

    fn field(id: &str, name: &str) -> Field {
        Field {
            id: id.to_string(),
            namespace: "example".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            ai_guidance: serde_json::json!({}),
            value_type: ValueType::Text,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
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
            instance_id: id.to_string(),
            type_id: "00000000-0000-4000-8000-00000000d100".to_string(),
            type_version: 1,
            type_namespace: "governance".to_string(),
            type_name: "decision".to_string(),
            field_values: vec![
                FieldValue {
                    field_id: TITLE.to_string(),
                    value: serde_json::json!(title),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: STATEMENT.to_string(),
                    value: serde_json::json!(statement),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            group_values: None,
            lifecycle_state: Some(lifecycle.to_string()),
            tags: (!tags.is_empty()).then(|| tags.iter().map(|t| t.to_string()).collect()),
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
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
                instance_index: vec![],
                extra: HashMap::new(),
                root: PathBuf::from("/memory"),
            },
            package(),
        );
        let mut manifest = store.load_manifest().unwrap();
        for record in &records {
            let path = format!("records/{}.json", record.instance_id);
            manifest.instance_index.push(InstanceIndexEntry {
                instance_id: record.instance_id.clone(),
                tier: 2,
                path: path.clone(),
                title: None,
                tags: record.tags.clone(),
            });
            store
                .save_instance_json(&path, &serde_json::to_value(record).unwrap())
                .unwrap();
        }
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
    fn non_tier2_query_is_deferred_with_diagnostic() {
        let store = store_with(fixtures());
        let result = find(
            &store,
            DiscoveryQuery {
                tier: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total, 0);
        assert!(result.diagnostics.iter().any(|d| d.contains("tier 0")));
    }
}
