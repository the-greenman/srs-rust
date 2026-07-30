//! Embedded `com.semanticops.core` package (RFC-018 — Mechanism A).
//!
//! The canonical bundle is compiled in via `include_str!` and parsed once at
//! startup. `RepositoryStore::load_package` merges these definitions into every
//! repository automatically — callers never need to reference this module
//! directly (use `store.load_package()` instead).

use serde::Deserialize;
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use std::sync::OnceLock;

use crate::error::RepositoryError;

const CORE_BUNDLE_JSON: &str = include_str!("../assets/core-bundle.srsj");

/// Parsed representation of the embedded core-bundle artifact.
pub struct EmbeddedCorePackage {
    pub package_id: String,
    pub package_name: String,
    pub package_version: String,
    pub fields: Vec<Field>,
    pub record_types: Vec<RecordType>,
    pub relation_types: Vec<RelationTypeDefinition>,
}

/// The serde target for the bundle JSON — `#[serde(rename_all = "camelCase")]`
/// matches the bundle's camelCase keys; `#[serde(rename = "types")]` maps the
/// bundle's `types` array to `record_types`.
///
/// Fields land in [`FieldJson`], not [`Field`], so the embedded bundle goes
/// through the **same** data-model-revision compatibility path as every other
/// package source (`FileStore`, `JsonStore`). A bundle authored before RFC-032
/// therefore still loads, upgraded in memory — see `field_json`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddedCorePackageJson {
    package_id: String,
    package_name: String,
    package_version: String,
    #[serde(deserialize_with = "crate::field_json::deserialize_fields_compat")]
    fields: Vec<Field>,
    #[serde(rename = "types")]
    record_types: Vec<RecordType>,
    #[serde(default)]
    relation_types: Vec<RelationTypeDefinition>,
}

static CORE_PACKAGE: OnceLock<EmbeddedCorePackage> = OnceLock::new();

/// Returns the embedded `com.semanticops.core` package, parsed once.
///
/// Every `load_package()` call merges these fields and types in transparently
/// (ADR-025). Do not call this from service logic — use `store.load_package()`.
pub fn core_package() -> &'static EmbeddedCorePackage {
    CORE_PACKAGE.get_or_init(|| {
        let raw: EmbeddedCorePackageJson = serde_json::from_str(CORE_BUNDLE_JSON)
            .expect("embedded assets/core-bundle.srsj must parse — file is corrupted or invalid");
        EmbeddedCorePackage {
            package_id: raw.package_id,
            package_name: raw.package_name,
            package_version: raw.package_version,
            fields: raw.fields,
            record_types: raw.record_types,
            relation_types: raw.relation_types,
        }
    })
}

/// Merges core fields, types, and relation types from the embedded `com.semanticops.core`
/// package into the provided mutable vecs (ADR-025).
///
/// Idempotent: if a field, type, or relation type is already present with the same id AND the
/// same namespace/name (or namespace/key for relation types) — i.e. it came from a prior merge,
/// or (for the seven canonical relation types) from a repo's own package explicitly declaring
/// them with the same canonical identity — it is silently skipped.
///
/// Unlike the sub-package coalescing path (which silently skips identical duplicates across
/// all namespaces), this function errors when a repo-defined field/type/relation type has the
/// same id as a core definition but *different* namespace/name/key content: that id is reserved
/// by the embedded core package and repos must not shadow it with different content.
pub(crate) fn merge_core_into_package(
    fields: &mut Vec<Field>,
    record_types: &mut Vec<RecordType>,
    relation_types: &mut Vec<RelationTypeDefinition>,
) -> Result<(), RepositoryError> {
    let cp = core_package();

    for core_field in &cp.fields {
        // Match by id + version so both field and type lookups behave consistently when the
        // core bundle gains new versions in the future.
        if let Some(existing) = fields
            .iter()
            .find(|f| f.id == core_field.id && f.version == core_field.version)
        {
            // Already present from a prior merge (e.g. a repo-copy) — skip silently.
            if existing.namespace == core_field.namespace && existing.name == core_field.name {
                continue;
            }
            return Err(RepositoryError::CorePackageConflict {
                kind: "field".to_string(),
                id: core_field.id.clone(),
                qualified_name: format!("{}/{}", existing.namespace, existing.name),
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
                qualified_name: format!("{}/{}", existing.namespace, existing.name),
            });
        }
        record_types.push(core_type.clone());
    }

    // Relation types resolve by bare `key` (`resolve_definition` in srs-core), not by id —
    // two definitions sharing a key but differing in id/namespace/content produce an E1Conflict
    // at relation-validation time, regardless of which package they came from. So unlike fields
    // and types (whose reserved-namespace conflict is an id collision), the safe merge rule here
    // is: skip a canonical relation type whenever the repo already has *any* definition — its
    // own, or the same canonical one carried over by a prior merge — using that key. A repo's
    // own definition always wins; this also covers repos that pre-date this fix and worked
    // around the missing canonical types by declaring their own (srs-rust#685).
    for core_rt in &cp.relation_types {
        if relation_types.iter().any(|rt| rt.key == core_rt.key) {
            continue;
        }
        relation_types.push(core_rt.clone());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_field(id: &str, namespace: &str, name: &str, version: u32) -> Field {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "namespace": namespace,
            "name": name,
            "version": version,
            "description": "",
            "aiGuidance": {"purpose": ""},
            "fieldType": {"datatype": "string"},
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn make_type(id: &str, namespace: &str, name: &str, version: u32) -> RecordType {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "namespace": namespace,
            "name": name,
            "version": version,
            "description": "",
            "aiGuidance": null,
            "fields": [],
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn make_relation_type(
        id: &str,
        namespace: &str,
        key: &str,
        version: u32,
    ) -> RelationTypeDefinition {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "namespace": namespace,
            "key": key,
            "version": version,
            "label": key,
            "description": "",
            "category": "other",
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn merge_core_into_empty_vecs_appends_core_definitions() {
        let mut fields = vec![];
        let mut types = vec![];
        let mut relation_types = vec![];
        merge_core_into_package(&mut fields, &mut types, &mut relation_types).unwrap();
        let cp = core_package();
        assert_eq!(fields.len(), cp.fields.len());
        assert_eq!(types.len(), cp.record_types.len());
        assert_eq!(relation_types.len(), cp.relation_types.len());
        assert!(fields
            .iter()
            .any(|f| f.namespace == "com.semanticops.core" && f.name == "statement"));
        assert!(types
            .iter()
            .any(|t| t.namespace == "com.semanticops.core" && t.name == "purpose"));
        assert!(relation_types.iter().any(|rt| rt.key == "depends-on"));
    }

    #[test]
    fn merge_core_idempotent_when_core_already_present() {
        let cp = core_package();
        // Pre-populate with the actual core definitions (as a repo-copy would serialise them).
        let mut fields = cp.fields.clone();
        let mut types = cp.record_types.clone();
        let mut relation_types = cp.relation_types.clone();
        let field_count_before = fields.len();
        let type_count_before = types.len();
        let relation_type_count_before = relation_types.len();

        merge_core_into_package(&mut fields, &mut types, &mut relation_types).unwrap();

        assert_eq!(
            fields.len(),
            field_count_before,
            "idempotent: fields must not be duplicated"
        );
        assert_eq!(
            types.len(),
            type_count_before,
            "idempotent: types must not be duplicated"
        );
        assert_eq!(
            relation_types.len(),
            relation_type_count_before,
            "idempotent: relation types must not be duplicated"
        );
    }

    #[test]
    fn merge_core_errors_when_repo_shadows_core_field_id() {
        let cp = core_package();
        // A field with the same id as the core statement field but a different namespace/name.
        let shadow = make_field(&cp.fields[0].id, "com.shadow", "shadow-field", 1);
        let mut fields = vec![shadow];
        let mut types = vec![];
        let mut relation_types = vec![];

        let err =
            merge_core_into_package(&mut fields, &mut types, &mut relation_types).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::CorePackageConflict { kind, qualified_name, .. }
                if kind == "field" && qualified_name.starts_with("com.shadow/")),
            "expected CorePackageConflict for field with repo's qualified_name, got: {err:?}"
        );
    }

    #[test]
    fn merge_core_errors_when_repo_shadows_core_type_id() {
        let cp = core_package();
        let shadow = make_type(
            &cp.record_types[0].id,
            "com.shadow",
            "shadow-type",
            cp.record_types[0].version,
        );
        let mut fields = vec![];
        let mut types = vec![shadow];
        let mut relation_types = vec![];

        let err =
            merge_core_into_package(&mut fields, &mut types, &mut relation_types).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::CorePackageConflict { kind, qualified_name, .. }
                if kind == "type" && qualified_name.starts_with("com.shadow/")),
            "expected CorePackageConflict for type with repo's qualified_name, got: {err:?}"
        );
    }

    #[test]
    fn merge_core_skips_canonical_key_when_repo_has_its_own_conflicting_definition() {
        // Relation types resolve by bare `key`, not by id (srs-core's resolve_definition) — two
        // definitions sharing a key with different id/content is an E1Conflict at relation-
        // validation time. Repos that pre-date this fix worked around the missing canonical
        // types (srs-rust#685) by declaring their own "contains"/"depends-on"/etc under a
        // different id and namespace. The merge must never introduce that conflict: the repo's
        // own definition wins and the canonical one is skipped, not appended alongside it.
        let cp = core_package();
        let own_contains = make_relation_type(
            "00000000-0000-4000-8000-000000000999",
            "com.example",
            &cp.relation_types[0].key,
            1,
        );
        let mut fields = vec![];
        let mut types = vec![];
        let mut relation_types = vec![own_contains.clone()];

        merge_core_into_package(&mut fields, &mut types, &mut relation_types).unwrap();

        let matching: Vec<_> = relation_types
            .iter()
            .filter(|rt| rt.key == own_contains.key)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "must not introduce a second definition under an already-declared key"
        );
        assert_eq!(
            matching[0].id, own_contains.id,
            "repo's own definition wins"
        );
    }

    #[test]
    fn merge_core_skips_relation_type_already_declared_with_same_identity() {
        // The srs/srs spec repo's own package already declares the seven canonical relation
        // types with the same id/namespace/key the core bundle carries — the merge must treat
        // that as already-present and skip, not duplicate or conflict.
        let cp = core_package();
        let mut fields = vec![];
        let mut types = vec![];
        let mut relation_types = cp.relation_types.clone();

        merge_core_into_package(&mut fields, &mut types, &mut relation_types).unwrap();

        assert_eq!(relation_types.len(), cp.relation_types.len());
    }

    #[test]
    fn core_package_parses_successfully() {
        let cp = core_package();
        assert_eq!(cp.fields.len(), 2);
        assert_eq!(cp.record_types.len(), 1);
        assert_eq!(cp.relation_types.len(), 7);
    }

    #[test]
    fn core_package_has_expected_relation_types() {
        let cp = core_package();
        let keys: Vec<&str> = cp.relation_types.iter().map(|rt| rt.key.as_str()).collect();
        for expected in [
            "contains",
            "depends-on",
            "supersedes",
            "refines",
            "derived-from",
            "evidences",
            "precedes",
        ] {
            assert!(
                keys.contains(&expected),
                "must have '{expected}' relation type"
            );
        }
        for rt in &cp.relation_types {
            assert_eq!(rt.namespace, "com.semanticops.srs");
        }
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
        assert_eq!(
            a, b,
            "core_package() must return the same pointer each call"
        );
    }
}
