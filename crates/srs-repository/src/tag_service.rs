//! # Tag Service
//!
//! Public API for tag query operations.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::vocabulary_service;

/// A single hit returned by `query_by_tag`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagQueryHit {
    pub instance_id: String,
    pub tier: u8,
    pub path: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

/// Result of `query_by_tag`.
pub struct TagQueryResult {
    pub key: String,
    pub hits: Vec<TagQueryHit>,
}

/// Filter for `audit_tags`.
pub struct AuditTagsFilter {
    /// Facet prefixes that every tier-2 Record must have at least one tag for.
    /// E.g. `["construct", "layer"]` requires at least one tag beginning with `"construct:"`.
    pub required_facets: Vec<String>,
}

/// A single advisory finding from `audit_tags`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFinding {
    pub instance_id: String,
    pub path: String,
    pub title: Option<String>,
    pub kind: AuditFindingKind,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditFindingKind {
    /// No tag found for a required facet prefix.
    MissingFacet { facet: String },
    /// Tag present but no vocab is declared, so resolution cannot be verified.
    /// (Informational — not an error; included when the repo carries no vocabulary.)
    NoVocabDeclared,
}

/// Result of `audit_tags`.
pub struct AuditTagsResult {
    /// All audit findings (advisory only — never causes a validation failure).
    pub findings: Vec<AuditFinding>,
    /// Number of tier-2 Records examined.
    pub records_checked: usize,
}

/// List all Terms from vocabularies in the package (RFC-006).
pub fn list_terms(
    store: &dyn RepositoryStore,
) -> Result<Vec<srs_core::types::term::Term>, RepositoryError> {
    vocabulary_service::list_terms(store)
}

/// Find a Term by id across all vocabularies in the package (RFC-006).
pub fn get_term_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<srs_core::types::term::Term>, RepositoryError> {
    vocabulary_service::get_term_by_id(store, id)
}

/// Cross-tier tag query — returns all instances (Notes, TypedRecords, Records) whose
/// manifest index entry carries `tag_key`. Reads the manifest only; no per-record file loads.
pub fn query_by_tag(
    store: &dyn RepositoryStore,
    tag_key: &str,
    container_id: Option<&str>,
) -> Result<TagQueryResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let filtered_ids: Option<std::collections::HashSet<String>> = if let Some(cid) = container_id {
        let members = crate::container_service::list_container_members(store, cid)?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    let hits: Vec<TagQueryHit> = manifest
        .instance_index
        .iter()
        .filter(|e| {
            e.tags
                .as_ref()
                .map(|tags| tags.iter().any(|t| t == tag_key))
                .unwrap_or(false)
        })
        .filter(|e| {
            filtered_ids
                .as_ref()
                .map(|ids| ids.contains(e.instance_id()))
                .unwrap_or(true)
        })
        .map(|e| TagQueryHit {
            instance_id: e.instance_id().to_string(),
            tier: e.tier(),
            path: e.path().to_string(),
            title: e.title(),
            tags: e.tags.clone().unwrap_or_default(),
        })
        .collect();

    Ok(TagQueryResult {
        key: tag_key.to_string(),
        hits,
    })
}

/// Advisory tag audit — checks tier-2 Records for missing required facets.
/// Never causes validation to fail; findings are informational only.
/// Reads the manifest only; no per-record file loads.
pub fn audit_tags(
    store: &dyn RepositoryStore,
    filter: AuditTagsFilter,
) -> Result<AuditTagsResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let records: Vec<_> = manifest
        .instance_index
        .iter()
        .filter(|e| e.tier() == 2)
        .collect();

    let records_checked = records.len();
    let mut findings: Vec<AuditFinding> = Vec::new();

    for entry in records {
        let tags: Vec<&str> = entry
            .tags
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str())
            .collect();

        for facet in &filter.required_facets {
            let prefix = format!("{}:", facet);
            let has_facet = tags.iter().any(|t| t.starts_with(&prefix));
            if !has_facet {
                findings.push(AuditFinding {
                    instance_id: entry.instance_id().to_string(),
                    path: entry.path().to_string(),
                    title: entry.title(),
                    kind: AuditFindingKind::MissingFacet {
                        facet: facet.clone(),
                    },
                });
            }
        }
    }

    Ok(AuditTagsResult {
        findings,
        records_checked,
    })
}
