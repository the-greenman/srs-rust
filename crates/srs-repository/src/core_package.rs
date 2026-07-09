//! Embedded `com.semanticops.core` package (RFC-018 — Mechanism A).
//!
//! The canonical bundle is compiled in via `include_str!` and parsed once at
//! startup. `RepositoryStore::load_package` merges these definitions into every
//! repository automatically — callers never need to reference this module
//! directly (use `store.load_package()` instead).

use serde::Deserialize;
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;
use std::sync::OnceLock;

use crate::error::RepositoryError;

const CORE_BUNDLE_JSON: &str = include_str!("../assets/core-bundle.srsj");

/// Parsed representation of the embedded core-bundle artifact.
///
/// Doubles as the serde target for the bundle JSON — `#[serde(rename_all = "camelCase")]`
/// matches the bundle's camelCase keys; `#[serde(rename = "types")]` maps the bundle's
/// `types` array to `record_types`.
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedCorePackage {
    pub package_id: String,
    pub package_name: String,
    pub package_version: String,
    pub fields: Vec<Field>,
    #[serde(rename = "types")]
    pub record_types: Vec<RecordType>,
}

static CORE_PACKAGE: OnceLock<EmbeddedCorePackage> = OnceLock::new();

/// Returns the embedded `com.semanticops.core` package, parsed once.
///
/// Every `load_package()` call merges these fields and types in transparently
/// (ADR-025). Do not call this from service logic — use `store.load_package()`.
pub fn core_package() -> &'static EmbeddedCorePackage {
    CORE_PACKAGE.get_or_init(|| {
        serde_json::from_str(CORE_BUNDLE_JSON)
            .expect("embedded assets/core-bundle.srsj must parse — file is corrupted or invalid")
    })
}

/// Merges core fields and types from the embedded `com.semanticops.core` package
/// into the provided mutable vecs (ADR-025).
///
/// Idempotent: if a field or type is already present with the same id AND the same
/// `com.semanticops.core` namespace/name (i.e. it came from a prior merge — e.g. via a
/// repo-copy that serialised the merged package), it is silently skipped.
///
/// Unlike the sub-package coalescing path (which silently skips identical duplicates across
/// all namespaces), this function errors when a repo-defined field/type has the same id as
/// a core definition but a *different* namespace or name: the `com.semanticops.core`
/// namespace is reserved and repos must not shadow it.
pub(crate) fn merge_core_into_package(
    fields: &mut Vec<Field>,
    record_types: &mut Vec<RecordType>,
) -> Result<(), RepositoryError> {
    let cp = core_package();

    for core_field in &cp.fields {
        if let Some(existing) = fields.iter().find(|f| f.id == core_field.id) {
            // Already present from a prior merge (e.g. a repo-copy) — skip silently.
            if existing.namespace == core_field.namespace && existing.name == core_field.name {
                continue;
            }
            return Err(RepositoryError::CorePackageConflict {
                kind: "field".to_string(),
                id: core_field.id.clone(),
                qualified_name: format!("{}/{}", core_field.namespace, core_field.name),
            });
        }
        fields.push(core_field.clone());
    }

    for core_type in &cp.record_types {
        if let Some(existing) = record_types
            .iter()
            .find(|rt| rt.id == core_type.id && rt.version == core_type.version)
        {
            if existing.namespace == core_type.namespace && existing.name == core_type.name {
                continue;
            }
            return Err(RepositoryError::CorePackageConflict {
                kind: "type".to_string(),
                id: core_type.id.clone(),
                qualified_name: format!("{}/{}", core_type.namespace, core_type.name),
            });
        }
        record_types.push(core_type.clone());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_package_parses_successfully() {
        let cp = core_package();
        assert_eq!(cp.fields.len(), 2);
        assert_eq!(cp.record_types.len(), 1);
    }

    #[test]
    fn core_package_has_expected_purpose_type() {
        let cp = core_package();
        let purpose = cp
            .record_types
            .iter()
            .find(|rt| rt.name == "purpose")
            .expect("core package must contain a 'purpose' type");
        assert_eq!(purpose.namespace, "com.semanticops.core");
        assert_eq!(purpose.version, 1);
    }

    #[test]
    fn core_package_has_expected_fields() {
        let cp = core_package();
        let names: Vec<&str> = cp.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"statement"), "must have statement field");
        assert!(names.contains(&"title"), "must have title field");
        for f in &cp.fields {
            assert_eq!(f.namespace, "com.semanticops.core");
        }
    }

    #[test]
    fn core_package_idempotent() {
        let a = core_package() as *const _;
        let b = core_package() as *const _;
        assert_eq!(a, b, "core_package() must return the same pointer each call");
    }
}
