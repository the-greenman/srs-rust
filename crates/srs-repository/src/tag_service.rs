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

/// Cross-tier tag query — returns all instances (Notes, TypedRecords, Records)
/// whose entity body carries `tag_key`. One catalog snapshot, then one body
/// read per instance (RFC-038: tags are catalog-derived, not a cached index
/// column).
pub fn query_by_tag(
    store: &dyn RepositoryStore,
    tag_key: &str,
    container_id: Option<&str>,
) -> Result<TagQueryResult, RepositoryError> {
    let filtered_ids: Option<std::collections::HashSet<String>> = if let Some(cid) = container_id {
        let members = crate::container_service::list_container_members(store, cid)?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    let cat = store.catalog()?;
    let mut hits: Vec<TagQueryHit> = Vec::new();
    for entry in &cat.instances {
        if let Some(ref ids) = filtered_ids {
            if !ids.contains(entry.id.as_str()) {
                continue;
            }
        }
        let Some(locator) = entry.locator.as_deref() else {
            continue;
        };
        let Ok(body) = store.load_instance_json(locator) else {
            continue;
        };
        let r =
            crate::store::instance_ref_from_body(entry.id.clone(), entry.tier.unwrap_or(2), &body);
        if !r.tags.iter().any(|t| t == tag_key) {
            continue;
        }
        hits.push(TagQueryHit {
            instance_id: r.instance_id,
            tier: r.tier,
            path: locator.to_string(),
            title: r.title,
            tags: r.tags,
        });
    }

    Ok(TagQueryResult {
        key: tag_key.to_string(),
        hits,
    })
}

/// Advisory tag audit — checks tier-2 Records for missing required facets.
/// Never causes validation to fail; findings are informational only. One
/// catalog snapshot, then one body read per tier-2 record (RFC-038: tags are
/// catalog-derived, not a cached index column).
pub fn audit_tags(
    store: &dyn RepositoryStore,
    filter: AuditTagsFilter,
) -> Result<AuditTagsResult, RepositoryError> {
    let cat = store.catalog()?;
    let records: Vec<_> = cat.instances.iter().filter(|e| e.tier == Some(2)).collect();

    let records_checked = records.len();
    let mut findings: Vec<AuditFinding> = Vec::new();

    for entry in records {
        let Some(locator) = entry.locator.as_deref() else {
            continue;
        };
        let Ok(body) = store.load_instance_json(locator) else {
            continue;
        };
        let r = crate::store::instance_ref_from_body(entry.id.clone(), 2, &body);
        let tags: Vec<&str> = r.tags.iter().map(|s| s.as_str()).collect();

        for facet in &filter.required_facets {
            let prefix = format!("{}:", facet);
            let has_facet = tags.iter().any(|t| t.starts_with(&prefix));
            if !has_facet {
                findings.push(AuditFinding {
                    instance_id: entry.id.clone(),
                    path: locator.to_string(),
                    title: r.title.clone(),
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
