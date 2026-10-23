use serde::{Deserialize, Serialize};
use srs_core::extensions::import_tracking::UpstreamPackage;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    // The RFC-038 Change-K retired properties (`instanceIndex`,
    // `containerIndex`, `sourceDocumentIndex`, `relationsChecksums`,
    // `relationsPath`) have no typed fields: membership is the tree via the
    // catalog, and `rfc038::check_manifest` denies the raw keys at load so
    // the `extra` catch-all can never silently absorb one.
    /// RFC-052 [R1]: the `containerId` of the root container, which lives in
    /// the container set (`containers/`) like every other container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) root_container_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_package: Option<UpstreamPackage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_documents_path: Option<String>,
    // all other manifest fields preserved for round-trip write
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    // set by loader, not from JSON
    #[serde(skip)]
    pub root: PathBuf,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            root_container_id: None,
            upstream_package: None,
            source_documents_path: None,
            extra: std::collections::BTreeMap::new(),
            root: PathBuf::new(),
        }
    }
}

/// The root container pointer (RFC-052, dataModelRevision 10).
///
/// The manifest names the root container by `rootContainerId` and holds no
/// container content ([R3]). The body is an ordinary member of the container
/// set: read it with [`crate::container_service::resolve_root_container`] and
/// write it with `RepositoryStore::save_container`. The raw-tree migration
/// readers read `manifest.json` as JSON and do not use these methods.
impl Manifest {
    /// The id of the root container this manifest names (RFC-052 [R1]), if any.
    pub fn root_container_id(&self) -> Option<&str> {
        self.root_container_id.as_deref()
    }

    /// The one write path for the pointer: replace (or, with `None`, remove)
    /// the root container id, returning the previous one.
    pub fn set_root_container_id(&mut self, id: Option<String>) -> Option<String> {
        std::mem::replace(&mut self.root_container_id, id)
    }
}

/// RFC-038 [R2] retired-property deny + [R21] generation gate — active for
/// every reader since the srs-rust#783 Phase-6 flip. The only exempt readers
/// are stores constructed through [`crate::store::FileStore`]'s named
/// exemption (the migration tooling entry points and the discovery
/// conformance fixture loader — see `with_rfc038_exemption`).
/// The data-model generation this build requires ([R21]). Below it, a
/// repository is refused with [`crate::error::RepositoryError::StorageGenerationUnsupported`].
/// Surfaced to MCP clients via `serverInfo` (srs-rust#858) so a stale binary
/// and a stale repository are distinguishable at the protocol level — `pub`
/// (not `pub(crate)`, unlike the `rfc038` module it governs) precisely so
/// `srs-mcp` can reuse it instead of hardcoding its own copy of the floor.
pub const MIN_SUPPORTED_DATA_MODEL_REVISION: u64 = 2;

pub(crate) mod rfc038 {
    use super::MIN_SUPPORTED_DATA_MODEL_REVISION;
    use crate::error::RepositoryError;

    /// The manifest properties retired by RFC-038 Change K.
    pub(crate) const RETIRED_PROPERTIES: &[&str] = &[
        "instanceIndex",
        "containerIndex",
        "sourceDocumentIndex",
        "relationsChecksums",
        "relationsPath",
    ];

    /// Apply the [R21] generation gate then the [R2] retired-property deny to
    /// a raw `manifest.json` value. Checked against the raw JSON (not the
    /// typed struct) so the `extra` catch-all can never silently absorb a
    /// retired key.
    pub(crate) fn check_manifest(raw: &serde_json::Value) -> Result<(), RepositoryError> {
        let declared = raw
            .get("dataModelRevision")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if declared < MIN_SUPPORTED_DATA_MODEL_REVISION {
            return Err(RepositoryError::StorageGenerationUnsupported { declared });
        }
        for prop in RETIRED_PROPERTIES {
            if raw.get(*prop).is_some() {
                return Err(RepositoryError::RetiredManifestProperty {
                    property: (*prop).to_string(),
                });
            }
        }
        super::rfc052::check_root_container(raw, declared)
    }
}

/// RFC-052 [R1]/[R7]: the root-container load gate.
pub(crate) mod rfc052 {
    use crate::error::RepositoryError;
    use crate::field_type_migration_service::RFC052_ROOT_CONTAINER_REVISION;

    /// Detection is by shape below revision 10: a manifest that still carries
    /// the inline `container` is refused naming the migration ([R7]); a
    /// revision-10 manifest must name its root and must not embed it ([R1]).
    /// A pre-10 manifest with no inline root (a rootless legacy corpus, which
    /// `repo validate` already reports under I-79) keeps loading as before.
    pub(crate) fn check_root_container(
        raw: &serde_json::Value,
        declared: u64,
    ) -> Result<(), RepositoryError> {
        let inline = raw.get("container").is_some();
        if declared < RFC052_ROOT_CONTAINER_REVISION {
            if inline {
                return Err(RepositoryError::ContainerMigrationNeeded {
                    declared,
                    legacy_rfc043_shape:
                        crate::rfc043_container_entries_migration_service::raw_manifest_container_is_legacy(raw),
                });
            }
            return Ok(());
        }
        if inline {
            return Err(RepositoryError::ManifestRootContainer {
                code: "manifest-root-container-inline",
                message: format!(
                    "manifest.json declares dataModelRevision {declared} and carries the inline \
                     `container`; at revision {RFC052_ROOT_CONTAINER_REVISION} the root container \
                     lives in the container set and the manifest names it by `rootContainerId` \
                     (RFC-052 [R1])"
                ),
            });
        }
        if !raw
            .get("rootContainerId")
            .is_some_and(serde_json::Value::is_string)
        {
            return Err(RepositoryError::ManifestRootContainer {
                code: "manifest-root-container-missing",
                message: format!(
                    "manifest.json declares dataModelRevision {declared} and names no root \
                     container: `rootContainerId` is required (RFC-052 [R1])"
                ),
            });
        }
        Ok(())
    }
}

/// RFC-044 Change D item 1: `packageRefs` (when present) wins over the
/// singular `packageRef` entirely — it is not merged with it, so a manifest
/// carrying both (`repo()` in `package_dependency_check.rs`) resolves the
/// plural list only. A manifest carrying only the singular form (schema-valid
/// for single-package repositories) resolves to that one ref, same as a
/// one-element `packageRefs` list. The one resolver every `packageRefs`
/// reader must call (srs-rust#1225): before this helper existed,
/// `FileStore::list_package_boundaries` and `FileStore::load_package` read
/// `packageRefs` directly and silently ignored a singular-only `packageRef`,
/// while `package_dependency_service::installed_set` already applied this
/// fallback — so a package declared only via singular `packageRef` counted as
/// "installed" for dependency checks but was never actually loaded into the
/// catalog.
pub(crate) fn resolve_package_refs(manifest: &Manifest) -> Vec<serde_json::Value> {
    match manifest.extra.get("packageRefs") {
        Some(v) if !v.is_null() => v.as_array().cloned().unwrap_or_default(),
        _ => manifest
            .extra
            .get("packageRef")
            .filter(|v| v.is_object())
            .cloned()
            .into_iter()
            .collect(),
    }
}

/// Normalize upstreamPackage across legacy formats so the typed field deserialises cleanly.
///
/// Two legacy formats exist:
/// 1. Old `"id"` key (bug in install_package_bundle before #246): rename to `"packageId"`.
/// 2. Pre-RFC-014 seeds: `upstreamPackage` nested under `meta` — lift to top level.
pub(crate) fn migrate_upstream_package(raw: &mut serde_json::Value) {
    let obj = match raw.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    // Lift from meta.upstreamPackage if absent at top level.
    if !obj.contains_key("upstreamPackage") {
        if let Some(up) = obj
            .get("meta")
            .and_then(|m| m.get("upstreamPackage"))
            .cloned()
        {
            obj.insert("upstreamPackage".to_string(), up);
        }
    }

    // Rename "id" → "packageId" (old provenance-stamp bug).
    if let Some(up) = obj
        .get_mut("upstreamPackage")
        .and_then(|v| v.as_object_mut())
    {
        if let Some(id_val) = up.remove("id") {
            up.entry("packageId").or_insert(id_val);
        }
    }
}

#[cfg(test)]
mod rfc038_tests {
    use super::rfc038::check_manifest;
    use crate::error::RepositoryError;

    #[test]
    fn r2_denies_a_rev2_manifest_still_carrying_instance_index() {
        let raw = serde_json::json!({
            "dataModelRevision": 2,
            "instanceIndex": [],
        });
        let err = check_manifest(&raw).unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::RetiredManifestProperty { ref property } if property == "instanceIndex"
        ));
        // [R2]: the diagnostic names the file and the property.
        let msg = err.to_string();
        assert!(msg.contains("manifest.json") && msg.contains("instanceIndex"));
    }

    #[test]
    fn r2_names_every_retired_property_not_just_instance_index() {
        for prop in [
            "containerIndex",
            "sourceDocumentIndex",
            "relationsChecksums",
            "relationsPath",
        ] {
            let raw = serde_json::json!({ "dataModelRevision": 2, prop: [] });
            let err = check_manifest(&raw).unwrap_err();
            assert!(
                matches!(err, RepositoryError::RetiredManifestProperty { ref property } if property == prop),
                "expected {prop} to be denied"
            );
        }
    }

    #[test]
    fn r21_denies_generation_below_2() {
        // Absent dataModelRevision ⇒ generation 0.
        let err = check_manifest(&serde_json::json!({})).unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::StorageGenerationUnsupported { declared: 0 }
        ));

        let err = check_manifest(&serde_json::json!({"dataModelRevision": 1})).unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::StorageGenerationUnsupported { declared: 1 }
        ));
    }

    #[test]
    fn rfc052_r7_refuses_an_embedded_root_below_revision_10() {
        let raw = serde_json::json!({
            "dataModelRevision": 9,
            "container": {"containerId": "c", "title": "Root"},
        });
        let err = check_manifest(&raw).unwrap_err();
        assert_eq!(err.code(), "container-migration-needed");
        assert!(err.to_string().contains("rfc052-root-container"));
    }

    #[test]
    fn rfc052_r1_refuses_an_inline_or_missing_root_at_revision_10() {
        let inline = serde_json::json!({
            "dataModelRevision": 10,
            "rootContainerId": "c",
            "container": {"containerId": "c", "title": "Root"},
        });
        assert_eq!(
            check_manifest(&inline).unwrap_err().code(),
            "manifest-root-container-inline"
        );
        let missing = serde_json::json!({ "dataModelRevision": 10 });
        assert_eq!(
            check_manifest(&missing).unwrap_err().code(),
            "manifest-root-container-missing"
        );
        let ok = serde_json::json!({ "dataModelRevision": 10, "rootContainerId": "c" });
        assert!(check_manifest(&ok).is_ok());
    }

    #[test]
    fn r21_and_r2_pass_together_on_a_clean_generation_2_manifest() {
        let raw = serde_json::json!({ "dataModelRevision": 2 });
        assert!(check_manifest(&raw).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srs_spec_repo() -> PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let vendored = manifest.join("../../tests/fixtures/spec-repo");
        if let Ok(c) = vendored.canonicalize() {
            if c.join(".srs").exists() {
                return c;
            }
        }
        let mut dir = manifest.to_path_buf();
        loop {
            let candidate = dir.join("../srs/srs");
            if let Ok(c) = candidate.canonicalize() {
                if c.join(".srs").exists() {
                    return c;
                }
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        manifest.join("../../../srs/srs")
    }

    #[test]
    fn live_manifest_loads_with_enforcement_active() {
        // The vendored spec-repo fixture is migrated: loading it through the
        // enforcing path proves [R2]/[R21] pass on final-format data, and the
        // retired keys are gone rather than riding in `extra`.
        let repo_root = srs_spec_repo();
        let manifest =
            crate::store::RepositoryStore::load_manifest(&crate::store::FileStore::new(&repo_root))
                .unwrap();
        for prop in super::rfc038::RETIRED_PROPERTIES {
            assert!(!manifest.extra.contains_key(*prop), "{prop} survived");
        }
        assert!(manifest.root_container_id().is_some());
    }

    #[test]
    fn manifest_root_container_id_roundtrips() {
        let json = r#"{ "rootContainerId": "550e8400-e29b-41d4-a716-446655440000" }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(
            manifest.root_container_id(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        // must not appear in extra
        assert!(!manifest.extra.contains_key("rootContainerId"));
        let reparsed: Manifest =
            serde_json::from_str(&serde_json::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(reparsed.root_container_id(), manifest.root_container_id());
    }

    #[test]
    fn manifest_absent_root_container_id_is_none() {
        let manifest: Manifest = serde_json::from_str("{}").unwrap();
        assert!(manifest.root_container_id().is_none());
    }

    #[test]
    fn manifest_upstream_package_roundtrips() {
        let json = r#"{
            "upstreamPackage": {
                "packageId": "1cd9622e-3d05-4214-a683-4cb81d0c44d9",
                "namespace": "com.mudemocracy.governance",
                "name": "governance",
                "version": "1.0.0",
                "installedAt": "2026-06-28T12:00:00Z"
            }
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        let up = manifest.upstream_package.as_ref().unwrap();
        assert_eq!(up.package_id, "1cd9622e-3d05-4214-a683-4cb81d0c44d9");
        assert_eq!(up.namespace, "com.mudemocracy.governance");
        assert!(!manifest.extra.contains_key("upstreamPackage"));

        let serialised = serde_json::to_string(&manifest).unwrap();
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(
            reparsed.upstream_package.as_ref().unwrap().package_id,
            up.package_id
        );
    }

    #[test]
    fn manifest_absent_upstream_package_is_none() {
        let json = r#"{}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.upstream_package.is_none());
    }

    #[test]
    fn migrate_upstream_package_renames_id_to_package_id() {
        let mut raw = serde_json::json!({
            "upstreamPackage": {
                "id": "old-id-value",
                "namespace": "com.example",
                "name": "pkg",
                "version": "1.0.0",
                "installedAt": "2026-01-01T00:00:00Z"
            }
        });
        migrate_upstream_package(&mut raw);
        let up = &raw["upstreamPackage"];
        assert_eq!(up["packageId"].as_str(), Some("old-id-value"));
        assert!(up.get("id").is_none());
    }

    #[test]
    fn migrate_upstream_package_lifts_from_meta() {
        let mut raw = serde_json::json!({
            "meta": {
                "upstreamPackage": {
                    "packageId": "meta-pkg-id",
                    "namespace": "com.example",
                    "name": "pkg",
                    "version": "1.0.0",
                    "installedAt": "2026-01-01T00:00:00Z"
                }
            }
        });
        migrate_upstream_package(&mut raw);
        let up = &raw["upstreamPackage"];
        assert_eq!(up["packageId"].as_str(), Some("meta-pkg-id"));
    }

    #[test]
    fn migrate_upstream_package_top_level_wins_over_meta() {
        let mut raw = serde_json::json!({
            "upstreamPackage": {
                "packageId": "top-level-id",
                "namespace": "com.example",
                "name": "pkg",
                "version": "1.0.0",
                "installedAt": "2026-01-01T00:00:00Z"
            },
            "meta": {
                "upstreamPackage": {
                    "packageId": "meta-id",
                    "namespace": "com.example",
                    "name": "pkg",
                    "version": "1.0.0",
                    "installedAt": "2026-01-01T00:00:00Z"
                }
            }
        });
        migrate_upstream_package(&mut raw);
        assert_eq!(
            raw["upstreamPackage"]["packageId"].as_str(),
            Some("top-level-id")
        );
    }

    #[test]
    fn manifest_source_documents_path_roundtrips() {
        let json = r#"{
            "sourceDocumentsPath": "attachments"
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(
            manifest.source_documents_path.as_deref(),
            Some("attachments")
        );
        // must not appear in extra
        assert!(!manifest.extra.contains_key("sourceDocumentsPath"));

        let serialised = serde_json::to_string(&manifest).unwrap();
        assert!(serialised.contains("\"sourceDocumentsPath\""));
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(
            reparsed.source_documents_path,
            manifest.source_documents_path
        );
    }

    #[test]
    fn manifest_absent_source_doc_fields_are_none() {
        let json = r#"{}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.source_documents_path.is_none());
    }
}
