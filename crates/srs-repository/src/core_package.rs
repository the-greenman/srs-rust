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
/// package source (disk or a `.srsj` tree session). A bundle authored before RFC-032
/// therefore still loads, upgraded in memory — see `field_json`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddedCorePackageJson {
    package_id: String,
    package_name: String,
    package_version: String,
    /// RFC-038 [R21]: a *package* has a generation too — absent ⇒ 0, and a
    /// generation-2 reader rejects an unstamped bundle (acceptance test 17).
    #[serde(default)]
    data_model_revision: u64,
    #[serde(deserialize_with = "crate::field_json::deserialize_fields_compat")]
    fields: Vec<Field>,
    #[serde(
        rename = "types",
        deserialize_with = "crate::type_json::deserialize_types_compat"
    )]
    record_types: Vec<RecordType>,
    #[serde(default)]
    relation_types: Vec<RelationTypeDefinition>,
}

/// Parse a package bundle, applying the RFC-038 [R21] generation gate: a
/// bundle below `dataModelRevision: 2` (absent ⇒ 0) is rejected, not coerced.
pub(crate) fn parse_package_bundle(json: &str) -> Result<EmbeddedCorePackage, RepositoryError> {
    let raw: EmbeddedCorePackageJson =
        serde_json::from_str(json).map_err(|e| RepositoryError::InvalidSnapshotData {
            message: format!("package bundle does not parse: {e}"),
        })?;
    if raw.data_model_revision < 2 {
        return Err(RepositoryError::StorageGenerationUnsupported {
            declared: raw.data_model_revision,
        });
    }
    Ok(EmbeddedCorePackage {
        package_id: raw.package_id,
        package_name: raw.package_name,
        package_version: raw.package_version,
        fields: raw.fields,
        record_types: raw.record_types,
        relation_types: raw.relation_types,
    })
}

static CORE_PACKAGE: OnceLock<EmbeddedCorePackage> = OnceLock::new();

/// Returns the embedded `com.semanticops.core` package, parsed once.
///
/// Every `load_package()` call merges these fields and types in transparently
/// (ADR-025). Do not call this from service logic — use `store.load_package()`.
pub fn core_package() -> &'static EmbeddedCorePackage {
    CORE_PACKAGE.get_or_init(|| {
        parse_package_bundle(CORE_BUNDLE_JSON)
            .expect("embedded assets/core-bundle.srsj must parse at generation 2 ([R21])")
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

    // RFC-048 ruling 8 / [R11] (srs-rust#1341): for every relation-type key the embedded
    // core package defines, the *core* definition governs — never a repo's local one. A
    // local definition identical to core (full JSON-value equality, which `PartialEq`
    // covers field-for-field including `createdAt`/`updatedAt`/`meta`) is a reference copy:
    // left in place, no diagnostic. Any other local definition of a core key is replaced by
    // core's — it never governs resolution or validation — and draws the warning
    // `relation-type-core-key-shadow` (never an error, never a load failure, never reported
    // as an installation conflict). That diagnostic is computed separately, by
    // `relation_type_shadow_diagnostics`, from the on-disk definition *before* this
    // replacement overwrites it — this function only needs to make core win.
    //
    // This corrects PR #738 (2026-07-24), which skipped the core definition whenever the
    // repo had *any* definition with the same key, letting a repo's own (possibly stale or
    // weaker — e.g. missing `irreflexive`) copy silently shadow core with no diagnostic.
    for core_rt in &cp.relation_types {
        match relation_types.iter().position(|rt| rt.key == core_rt.key) {
            Some(idx) if &relation_types[idx] != core_rt => {
                relation_types[idx] = core_rt.clone();
            }
            Some(_) => {
                // Identical reference copy — already in place, nothing to do.
            }
            None => relation_types.push(core_rt.clone()),
        }
    }

    Ok(())
}

/// One local relation-type definition that shares a key with the embedded core package but
/// is not identical to it (RFC-048 [R11], `relation-type-core-key-shadow`). Re-derived from
/// the on-disk definition directly — by the time `merge_core_into_package` has run, the
/// local copy's own id/version is gone (overwritten by core's), so this reads the repo's own
/// relation-type files again rather than the merged `Package`.
pub(crate) struct RelationTypeShadowFinding {
    /// Repo-relative path to the local definition file that shadows a core key.
    pub path: String,
    pub message: String,
}

/// Scans every package root's own `relationTypes[]` entries for a local definition that
/// shares a key with the embedded core package but differs from it (RFC-048 [R11]). Never
/// fails — an unreadable or unparsable definition is skipped, since catalog/package-load
/// validation already reports that separately.
pub(crate) fn relation_type_shadow_diagnostics(
    store: &dyn crate::store::RepositoryStore,
    package_roots: &[String],
) -> Vec<RelationTypeShadowFinding> {
    let cp = core_package();
    let mut out = Vec::new();
    for root in package_roots {
        let manifest_rel = if root.is_empty() {
            "package.json".to_string()
        } else {
            format!("{}/package.json", root.trim_end_matches('/'))
        };
        let Ok(manifest) = store.load_instance_json(&manifest_rel) else {
            continue;
        };
        let Some(rt_paths) = manifest.get("relationTypes").and_then(|v| v.as_array()) else {
            continue;
        };
        for p in rt_paths.iter().filter_map(|v| v.as_str()) {
            let rel = if root.is_empty() {
                p.to_string()
            } else {
                format!("{root}/{p}")
            };
            let Ok(def_json) = store.load_instance_json(&rel) else {
                continue;
            };
            let Ok(def) = serde_json::from_value::<RelationTypeDefinition>(def_json) else {
                continue;
            };
            if let Some(core_rt) = cp.relation_types.iter().find(|c| c.key == def.key) {
                if core_rt != &def {
                    out.push(RelationTypeShadowFinding {
                        path: rel.clone(),
                        message: format!(
                            "RFC-048 [R11] relation-type-core-key-shadow: local definition \
                             of core key '{}' ({} v{}) differs from the governing core \
                             definition ({} v{}) — core governs resolution and validation; \
                             this copy is a reference only",
                            def.key, def.id, def.version, core_rt.id, core_rt.version
                        ),
                    });
                }
            }
        }
    }
    out
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
    fn merge_core_governs_when_repo_has_its_own_conflicting_definition() {
        // RFC-048 ruling 8 / [R11] (srs-rust#1341): relation types resolve by bare `key`, not
        // by id (srs-core's `resolve_definition`), so two definitions sharing a key would be
        // an E1Conflict at relation-validation time if both were kept. Repos that pre-date
        // this fix worked around the missing canonical types (srs-rust#685) by declaring
        // their own "contains"/"depends-on"/etc under a different id and namespace — PR #738
        // then made that repo-local copy win silently. That is exactly what ruling 8
        // reverses: the core definition governs, and the repo's differing copy never does,
        // whatever its id/namespace. (The `relation-type-core-key-shadow` warning this draws
        // is asserted separately in validation.rs, which has the on-disk fixture this needs.)
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
            matching[0].id, cp.relation_types[0].id,
            "core's definition governs — the repo's differing copy must not win"
        );
        assert_eq!(
            matching[0].version, cp.relation_types[0].version,
            "core's definition governs in full, including its version"
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

#[cfg(test)]
mod rfc038_bundle_gate_tests {
    use super::*;

    /// RFC-038 acceptance test 17, reader half: the stamped bundle is
    /// accepted; the same bundle unstamped is rejected under [R21].
    #[test]
    fn unstamped_bundle_is_rejected_and_stamped_bundle_accepted() {
        // The embedded (stamped) bundle parses.
        let stamped = core_package();
        assert_eq!(stamped.package_name, "core");

        // The identical bundle with the stamp removed is generation 0 — rejected.
        let mut raw: serde_json::Value = serde_json::from_str(CORE_BUNDLE_JSON).unwrap();
        raw.as_object_mut().unwrap().remove("dataModelRevision");
        let err = match parse_package_bundle(&raw.to_string()) {
            Err(e) => e,
            Ok(_) => panic!("unstamped bundle must be rejected"),
        };
        assert!(matches!(
            err,
            RepositoryError::StorageGenerationUnsupported { declared: 0 }
        ));
    }
}
