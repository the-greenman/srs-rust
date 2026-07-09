use crate::error::RepositoryError;
use crate::record_store::list_all_records;
use crate::relation_service::{list_relations, ListRelationsFilter};
use crate::store::RepositoryStore;
use crate::writer::write_manifest;
use serde_json::json;
use std::collections::HashMap;

const EXT_ADDRESSABILITY: &str = "ext:addressability";
const EXT_DISCOVERY: &str = "ext:discovery";
const EXT_FIELD_GROUPS: &str = "ext:field-groups";
const EXT_LIFECYCLE: &str = "ext:lifecycle";
const EXT_RELATIONS: &str = "ext:relations";
const EXT_REPOSITORY: &str = "ext:repository";
const EXT_TYPE_INHERITANCE: &str = "ext:type-inheritance";

/// Extension IDs actively implemented by this version of the SRS engine.
/// This is the single authoritative list — do not add `ext:` literals elsewhere.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    EXT_ADDRESSABILITY,
    EXT_DISCOVERY,
    EXT_FIELD_GROUPS,
    EXT_LIFECYCLE,
    EXT_RELATIONS,
    EXT_REPOSITORY,
    EXT_TYPE_INHERITANCE,
];

/// Conformance report: declared vs supported vs content-detected extension usage.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredExtensionsReport {
    /// Extension IDs declared in `manifest.extra.declaredExtensions`.
    pub declared: Vec<String>,
    /// Extension IDs this implementation actively handles (from `SUPPORTED_EXTENSIONS`).
    pub supported: Vec<String>,
    /// Declared IDs that are not in the supported set.
    pub declared_but_unsupported: Vec<String>,
    /// Supported IDs detected in repo content but absent from the declared list.
    pub used_but_undeclared: Vec<String>,
}

/// Return a conformance report comparing the manifest's `declaredExtensions` against the
/// implementation's supported set and the repo's actual content usage.
pub fn declared_extensions_conformance(
    store: &dyn RepositoryStore,
) -> Result<DeclaredExtensionsReport, RepositoryError> {
    let declared = list_declared_extensions(store)?;
    let supported: Vec<String> = SUPPORTED_EXTENSIONS.iter().map(|s| s.to_string()).collect();

    let declared_but_unsupported: Vec<String> = declared
        .iter()
        .filter(|id| !supported.contains(id))
        .cloned()
        .collect();

    let used_extensions = detect_used_extensions(store)?;
    let used_but_undeclared: Vec<String> = used_extensions
        .into_iter()
        .filter(|id| !declared.contains(id))
        .collect();

    Ok(DeclaredExtensionsReport {
        declared,
        supported,
        declared_but_unsupported,
        used_but_undeclared,
    })
}

/// Detect which supported extension IDs are actively in use by this repo's content.
///
/// Only extensions with a detectable content signal are checked. `ext:repository` and
/// `ext:discovery` have no absence signal (they are structural/always-available) and are
/// excluded from detection — they will never appear in `used_but_undeclared`.
fn detect_used_extensions(
    store: &dyn RepositoryStore,
) -> Result<Vec<String>, RepositoryError> {
    let mut used = Vec::new();

    // ext:lifecycle — any Tier 2 record has lifecycleState set
    match list_all_records(store) {
        Ok(records) if records.iter().any(|r| r.lifecycle_state.is_some()) => {
            used.push(EXT_LIFECYCLE.to_string());
        }
        Ok(_) => {}
        Err(e) => return Err(e),
    }

    // ext:relations — relations collection is non-empty
    match list_relations(store, ListRelationsFilter::default()) {
        Ok(relations) if !relations.is_empty() => {
            used.push(EXT_RELATIONS.to_string());
        }
        Ok(_) => {}
        Err(e) => return Err(e),
    }

    // ext:type-inheritance — any package type declares an extends base type
    // ext:field-groups — any package type declares field groups
    match store.load_package() {
        Ok(package) => {
            let mut has_inheritance = false;
            let mut has_field_groups = false;
            for record_type in &package.record_types {
                if record_type.extends_type_id.is_some() {
                    has_inheritance = true;
                }
                if record_type
                    .field_groups
                    .as_ref()
                    .map(|g| !g.is_empty())
                    .unwrap_or(false)
                {
                    has_field_groups = true;
                }
            }
            if has_inheritance {
                used.push(EXT_TYPE_INHERITANCE.to_string());
            }
            if has_field_groups {
                used.push(EXT_FIELD_GROUPS.to_string());
            }
        }
        Err(RepositoryError::Io { .. } | RepositoryError::PackageLoad { .. }) => {}
        Err(e) => return Err(e),
    }

    // ext:addressability — any .revisions.json sidecar file exists
    if store.has_revision_sidecars() {
        used.push(EXT_ADDRESSABILITY.to_string());
    }

    used.sort();
    Ok(used)
}

/// List declared extension IDs from the manifest
pub fn list_declared_extensions(
    store: &dyn RepositoryStore,
) -> Result<Vec<String>, RepositoryError> {
    let manifest = store.load_manifest()?;

    let extensions = manifest
        .extra
        .get("declaredExtensions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(extensions)
}

/// Add an extension ID to the declared extensions list
pub fn add_declared_extension(
    store: &dyn RepositoryStore,
    extension_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let mut extensions: Vec<String> = manifest
        .extra
        .get("declaredExtensions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if !extensions.contains(&extension_id.to_string()) {
        extensions.push(extension_id.to_string());
        extensions.sort();

        manifest
            .extra
            .insert("declaredExtensions".to_string(), json!(extensions));
        write_manifest(store, &manifest)?;
    }

    Ok(extensions)
}

/// Remove an extension ID from the declared extensions list
pub fn remove_declared_extension(
    store: &dyn RepositoryStore,
    extension_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let mut extensions: Vec<String> = manifest
        .extra
        .get("declaredExtensions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let was_present = extensions.contains(&extension_id.to_string());

    if was_present {
        extensions.retain(|e| e != extension_id);

        if extensions.is_empty() {
            manifest.extra.remove("declaredExtensions");
        } else {
            manifest
                .extra
                .insert("declaredExtensions".to_string(), json!(extensions));
        }
        write_manifest(store, &manifest)?;
    }

    Ok(extensions)
}

/// A reference to a local sub-package declared in the manifest
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PackageRef {
    pub mode: String,
    pub path: String,
}

/// List declared package refs from the manifest
pub fn list_package_refs(store: &dyn RepositoryStore) -> Result<Vec<PackageRef>, RepositoryError> {
    let manifest = store.load_manifest()?;

    let refs = manifest
        .extra
        .get("packageRefs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let mode = v.get("mode").and_then(|m| m.as_str())?;
                    let path = v.get("path").and_then(|p| p.as_str())?;
                    Some(PackageRef {
                        mode: mode.to_string(),
                        path: path.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(refs)
}

/// Add a local package ref to the manifest (deduplicates by path)
pub fn add_package_ref(
    store: &dyn RepositoryStore,
    path: &str,
) -> Result<Vec<PackageRef>, RepositoryError> {
    store.validate_package_ref_path(path)?;

    let mut manifest = store.load_manifest()?;

    let mut refs = manifest
        .extra
        .get("packageRefs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let mode = v.get("mode").and_then(|m| m.as_str())?;
                    let p = v.get("path").and_then(|p| p.as_str())?;
                    Some(PackageRef {
                        mode: mode.to_string(),
                        path: p.to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if !refs.iter().any(|r| r.path == path) {
        refs.push(PackageRef {
            mode: "local".to_string(),
            path: path.to_string(),
        });
        refs.sort_by(|a, b| a.path.cmp(&b.path));

        let json_refs: Vec<serde_json::Value> = refs
            .iter()
            .map(|r| json!({"mode": r.mode, "path": r.path}))
            .collect();
        manifest
            .extra
            .insert("packageRefs".to_string(), json!(json_refs));
        write_manifest(store, &manifest)?;
    }

    Ok(refs)
}

/// Remove a package ref from the manifest by path
pub fn remove_package_ref(
    store: &dyn RepositoryStore,
    path: &str,
) -> Result<Vec<PackageRef>, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let mut refs: Vec<PackageRef> = manifest
        .extra
        .get("packageRefs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let mode = v.get("mode").and_then(|m| m.as_str())?;
                    let p = v.get("path").and_then(|p| p.as_str())?;
                    Some(PackageRef {
                        mode: mode.to_string(),
                        path: p.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let was_present = refs.iter().any(|r| r.path == path);

    if was_present {
        refs.retain(|r| r.path != path);

        if refs.is_empty() {
            manifest.extra.remove("packageRefs");
        } else {
            let json_refs: Vec<serde_json::Value> = refs
                .iter()
                .map(|r| json!({"mode": r.mode, "path": r.path}))
                .collect();
            manifest
                .extra
                .insert("packageRefs".to_string(), json!(json_refs));
        }
        write_manifest(store, &manifest)?;
    }

    Ok(refs)
}

/// Input for `set_manifest_root_container`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetManifestRootContainerInput {
    pub container_id: String,
    pub identity_instance_id: String,
}

/// Result of `set_manifest_root_container`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetManifestRootContainerResult {
    pub container_id: String,
    pub identity_instance_id: String,
}

/// Write manifest.container — sets the root container embed used by the navigation service.
pub fn set_manifest_root_container(
    store: &dyn RepositoryStore,
    input: SetManifestRootContainerInput,
) -> Result<SetManifestRootContainerResult, RepositoryError> {
    if input.container_id.is_empty() {
        return Err(RepositoryError::InvalidInput {
            message: "container_id must not be empty".to_string(),
        });
    }
    if input.identity_instance_id.is_empty() {
        return Err(RepositoryError::InvalidInput {
            message: "identity_instance_id must not be empty".to_string(),
        });
    }

    let mut manifest = store.load_manifest()?;

    manifest.container = Some(srs_core::types::container::Container {
        container_id: input.container_id.clone(),
        // title is intentionally empty — manifest.container is a navigation pointer;
        // the display title is read from the container record by container_id.
        title: String::new(),
        identity_instance_id: Some(input.identity_instance_id.clone()),
        namespace: None,
        name: None,
        description: None,
        container_type: None,
        root_instance_ids: None,
        member_instance_ids: None,
        tags: None,
        created_at: None,
        updated_at: None,
        meta: None,
        extra: HashMap::new(),
    });

    write_manifest(store, &manifest)?;

    Ok(SetManifestRootContainerResult {
        container_id: input.container_id,
        identity_instance_id: input.identity_instance_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_store() -> MemoryStore {
        MemoryStore::default()
    }

    fn make_store_with_extensions() -> MemoryStore {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "declaredExtensions".to_string(),
            json!(["ext:repository", "ext:relations"]),
        );
        store.save_manifest(&manifest).unwrap();
        store
    }

    #[test]
    fn list_declared_extensions_empty_when_none() {
        let store = make_store();
        let extensions = list_declared_extensions(&store).unwrap();
        assert!(extensions.is_empty());
    }

    #[test]
    fn list_declared_extensions_returns_extensions() {
        let store = make_store_with_extensions();
        let extensions = list_declared_extensions(&store).unwrap();
        assert_eq!(extensions.len(), 2);
        assert!(extensions.contains(&"ext:repository".to_string()));
        assert!(extensions.contains(&"ext:relations".to_string()));
    }

    #[test]
    fn add_declared_extension_adds_new() {
        let store = make_store();
        let extensions = add_declared_extension(&store, "ext:new").unwrap();
        assert_eq!(extensions.len(), 1);
        assert!(extensions.contains(&"ext:new".to_string()));

        let manifest = store.load_manifest().unwrap();
        let declared = manifest.extra["declaredExtensions"].as_array().unwrap();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0], "ext:new");
    }

    #[test]
    fn add_declared_extension_dedupes() {
        let store = make_store_with_extensions();
        let extensions = add_declared_extension(&store, "ext:repository").unwrap();
        assert_eq!(extensions.len(), 2);
    }

    #[test]
    fn remove_declared_extension_removes_existing() {
        let store = make_store_with_extensions();
        let extensions = remove_declared_extension(&store, "ext:repository").unwrap();
        assert_eq!(extensions.len(), 1);
        assert!(!extensions.contains(&"ext:repository".to_string()));
        assert!(extensions.contains(&"ext:relations".to_string()));
    }

    #[test]
    fn remove_declared_extension_noop_when_not_present() {
        let store = make_store_with_extensions();
        let extensions = remove_declared_extension(&store, "ext:nonexistent").unwrap();
        assert_eq!(extensions.len(), 2);
    }

    #[test]
    fn remove_last_extension_removes_field() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("declaredExtensions".to_string(), json!(["ext:single"]));
        store.save_manifest(&manifest).unwrap();

        let extensions = remove_declared_extension(&store, "ext:single").unwrap();
        assert!(extensions.is_empty());

        let manifest = store.load_manifest().unwrap();
        assert!(!manifest.extra.contains_key("declaredExtensions"));
    }

    fn create_package_dir(temp: &TempDir, rel_path: &str) {
        let pkg_dir = temp.path().join(rel_path);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let pkg_json = json!({
            "id": "test-pkg",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
            "fields": [],
            "types": []
        });
        std::fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string_pretty(&pkg_json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn add_package_ref_rejects_missing_path() {
        let temp = TempDir::new().unwrap();
        let store = crate::FileStore::new(temp.path());
        // Write minimal manifest so load_manifest succeeds after validation
        std::fs::write(
            temp.path().join("manifest.json"),
            r#"{"srsVersion":"2.0-draft","repositoryId":"test","instanceIndex":[]}"#,
        )
        .unwrap();

        let result = add_package_ref(&store, "package/nonexistent");
        assert!(
            matches!(result, Err(RepositoryError::PackageRefMissing { .. })),
            "expected PackageRefMissing, got {result:?}"
        );
    }

    #[test]
    fn add_package_ref_rejects_traversal_outside_repo() {
        let temp = TempDir::new().unwrap();
        let store = crate::FileStore::new(temp.path());
        std::fs::write(
            temp.path().join("manifest.json"),
            r#"{"srsVersion":"2.0-draft","repositoryId":"test","instanceIndex":[]}"#,
        )
        .unwrap();

        let outside = TempDir::new().unwrap();
        create_package_dir(&outside, ".");
        let traversal = format!("../../../{}", outside.path().display());

        let result = add_package_ref(&store, &traversal);
        assert!(
            matches!(
                result,
                Err(RepositoryError::PackageRefOutsideRepo { .. })
                    | Err(RepositoryError::PackageRefMissing { .. })
            ),
            "expected scope or missing error, got {result:?}"
        );
    }

    #[test]
    fn add_package_ref_succeeds_for_valid_local_package() {
        let temp = TempDir::new().unwrap();
        let store = crate::FileStore::new(temp.path());
        std::fs::write(
            temp.path().join("manifest.json"),
            r#"{"srsVersion":"2.0-draft","repositoryId":"test","instanceIndex":[]}"#,
        )
        .unwrap();
        create_package_dir(&temp, "package/sub");

        let refs = add_package_ref(&store, "package/sub").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "package/sub");
        assert_eq!(refs[0].mode, "local");
    }

    #[test]
    fn add_package_ref_dedupes() {
        let temp = TempDir::new().unwrap();
        let store = crate::FileStore::new(temp.path());
        std::fs::write(
            temp.path().join("manifest.json"),
            r#"{"srsVersion":"2.0-draft","repositoryId":"test","instanceIndex":[]}"#,
        )
        .unwrap();
        create_package_dir(&temp, "package/sub");

        add_package_ref(&store, "package/sub").unwrap();
        let refs = add_package_ref(&store, "package/sub").unwrap();
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn list_package_refs_empty_when_none() {
        let store = make_store();
        let refs = list_package_refs(&store).unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn remove_package_ref_removes_existing() {
        let temp = TempDir::new().unwrap();
        let store = crate::FileStore::new(temp.path());
        std::fs::write(
            temp.path().join("manifest.json"),
            r#"{"srsVersion":"2.0-draft","repositoryId":"test","instanceIndex":[]}"#,
        )
        .unwrap();
        create_package_dir(&temp, "package/sub");

        add_package_ref(&store, "package/sub").unwrap();
        let refs = remove_package_ref(&store, "package/sub").unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn remove_package_ref_noop_when_not_present() {
        let store = make_store();
        let refs = remove_package_ref(&store, "package/nonexistent").unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn declared_extensions_enable_disable_updates_manifest() {
        let store = make_store();

        let ext1 = add_declared_extension(&store, "ext:repository").unwrap();
        assert_eq!(ext1.len(), 1);

        let ext2 = add_declared_extension(&store, "ext:relations").unwrap();
        assert_eq!(ext2.len(), 2);

        let manifest = store.load_manifest().unwrap();
        let declared = manifest.extra["declaredExtensions"].as_array().unwrap();
        assert_eq!(declared.len(), 2);

        let ext3 = remove_declared_extension(&store, "ext:repository").unwrap();
        assert_eq!(ext3.len(), 1);

        let ext4 = remove_declared_extension(&store, "ext:relations").unwrap();
        assert!(ext4.is_empty());

        let manifest = store.load_manifest().unwrap();
        assert!(!manifest.extra.contains_key("declaredExtensions"));
    }

    const VALID_CONTAINER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const VALID_IDENTITY_ID: &str = "aaaaaaaa-0000-4000-8000-aaaaaaaaaaaa";

    #[test]
    fn set_manifest_root_container_writes_and_reads_back() {
        let store = MemoryStore::default();
        let result = set_manifest_root_container(
            &store,
            SetManifestRootContainerInput {
                container_id: VALID_CONTAINER_ID.to_string(),
                identity_instance_id: VALID_IDENTITY_ID.to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.container_id, VALID_CONTAINER_ID);
        assert_eq!(result.identity_instance_id, VALID_IDENTITY_ID);

        let manifest = store.load_manifest().unwrap();
        let container = manifest.container.as_ref().unwrap();
        assert_eq!(container.container_id, VALID_CONTAINER_ID);
        assert_eq!(
            container.identity_instance_id.as_deref(),
            Some(VALID_IDENTITY_ID)
        );
        assert_eq!(container.title, "");
    }

    #[test]
    fn set_manifest_root_container_empty_container_id_returns_error() {
        let store = MemoryStore::default();
        let err = set_manifest_root_container(
            &store,
            SetManifestRootContainerInput {
                container_id: "".to_string(),
                identity_instance_id: VALID_IDENTITY_ID.to_string(),
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput, got {err:?}"
        );
    }

    #[test]
    fn set_manifest_root_container_empty_identity_id_returns_error() {
        let store = MemoryStore::default();
        let err = set_manifest_root_container(
            &store,
            SetManifestRootContainerInput {
                container_id: VALID_CONTAINER_ID.to_string(),
                identity_instance_id: "".to_string(),
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput, got {err:?}"
        );
    }

    #[test]
    fn set_manifest_root_container_roundtrips_through_json() {
        // Write via MemoryStore, serialise manifest to JSON, deserialise, assert fields survive.
        let store = MemoryStore::default();
        set_manifest_root_container(
            &store,
            SetManifestRootContainerInput {
                container_id: VALID_CONTAINER_ID.to_string(),
                identity_instance_id: VALID_IDENTITY_ID.to_string(),
            },
        )
        .unwrap();

        let manifest = store.load_manifest().unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let reparsed: crate::manifest::Manifest = serde_json::from_str(&json).unwrap();

        let container = reparsed.container.as_ref().unwrap();
        assert_eq!(container.container_id, VALID_CONTAINER_ID);
        assert_eq!(
            container.identity_instance_id.as_deref(),
            Some(VALID_IDENTITY_ID)
        );
        assert_eq!(container.title, "");
    }

    // ── Conformance tests ─────────────────────────────────────────────────────

    #[test]
    fn conformance_empty_repo_reports_nothing_used_or_declared() {
        let store = MemoryStore::default();
        let report = declared_extensions_conformance(&store).unwrap();
        assert!(report.declared.is_empty());
        assert_eq!(report.supported.len(), SUPPORTED_EXTENSIONS.len());
        assert!(report.declared_but_unsupported.is_empty());
        assert!(report.used_but_undeclared.is_empty());
    }

    #[test]
    fn conformance_declared_but_unsupported_extension_is_flagged() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("declaredExtensions".to_string(), json!(["ext:nonexistent"]));
        store.save_manifest(&manifest).unwrap();

        let report = declared_extensions_conformance(&store).unwrap();
        assert_eq!(report.declared, vec!["ext:nonexistent"]);
        assert_eq!(report.declared_but_unsupported, vec!["ext:nonexistent"]);
        assert!(report.used_but_undeclared.is_empty());
    }

    #[test]
    fn conformance_supported_declared_extension_not_flagged() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "declaredExtensions".to_string(),
            json!(["ext:lifecycle"]),
        );
        store.save_manifest(&manifest).unwrap();

        let report = declared_extensions_conformance(&store).unwrap();
        assert!(
            report.declared_but_unsupported.is_empty(),
            "ext:lifecycle is supported; should not appear in declared_but_unsupported"
        );
    }

    #[test]
    fn conformance_lifecycle_state_detected_as_used() {
        use crate::index::InstanceIndexEntry;
        use crate::manifest::Manifest;
        use crate::store::memory::MemoryStore;
        use std::path::PathBuf;

        let record_path = "records/abc123.json";
        let record_json = json!({
            "instanceId": "abc123",
            "typeId": "test-type-id",
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "note",
            "fieldValues": [],
            "lifecycleState": "active"
        });
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: "abc123".to_string(),
                tier: 2,
                path: record_path.to_string(),
                title: None,
                tags: None,
            }],
            container: None,
            container_index: None,
            extra: std::collections::HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
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
            },
        )
        .with_data(record_path, record_json);

        let report = declared_extensions_conformance(&store).unwrap();
        assert!(
            report.used_but_undeclared.contains(&"ext:lifecycle".to_string()),
            "ext:lifecycle should be detected as used: {:?}",
            report.used_but_undeclared
        );
    }

    #[test]
    fn conformance_declared_lifecycle_not_in_undeclared() {
        use crate::index::InstanceIndexEntry;
        use crate::manifest::Manifest;
        use crate::store::memory::MemoryStore;
        use std::path::PathBuf;

        let record_path = "records/abc123.json";
        let record_json = json!({
            "instanceId": "abc123",
            "typeId": "test-type-id",
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "note",
            "fieldValues": [],
            "lifecycleState": "active"
        });
        let mut extra = std::collections::HashMap::new();
        extra.insert("declaredExtensions".to_string(), json!(["ext:lifecycle"]));
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: "abc123".to_string(),
                tier: 2,
                path: record_path.to_string(),
                title: None,
                tags: None,
            }],
            container: None,
            container_index: None,
            extra,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
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
            },
        )
        .with_data(record_path, record_json);

        let report = declared_extensions_conformance(&store).unwrap();
        assert!(
            !report.used_but_undeclared.contains(&"ext:lifecycle".to_string()),
            "ext:lifecycle is declared; must not appear in used_but_undeclared"
        );
    }

    #[test]
    fn conformance_lifecycle_used_roundtrip_filestore() {
        // Cross-store roundtrip: write a record with lifecycleState to FileStore,
        // assert that conformance detects ext:lifecycle as used-but-undeclared.
        let temp = TempDir::new().unwrap();
        let repo = temp.path();

        // Minimal manifest with one Tier 2 record entry
        let manifest_json = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": [
                {
                    "instanceId": "rec001",
                    "tier": 2,
                    "path": "records/rec001.json"
                }
            ]
        });
        std::fs::write(
            repo.join("manifest.json"),
            serde_json::to_string_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        // Record with lifecycleState — implies ext:lifecycle is in use
        std::fs::create_dir_all(repo.join("records")).unwrap();
        let record_json = json!({
            "instanceId": "rec001",
            "typeId": "test-type-id",
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "note",
            "fieldValues": [],
            "lifecycleState": "active"
        });
        std::fs::write(
            repo.join("records/rec001.json"),
            serde_json::to_string_pretty(&record_json).unwrap(),
        )
        .unwrap();

        let store = crate::FileStore::new(repo);
        let report = declared_extensions_conformance(&store).unwrap();

        assert!(
            report.used_but_undeclared.contains(&"ext:lifecycle".to_string()),
            "FileStore: ext:lifecycle should be detected via record lifecycleState: {:?}",
            report.used_but_undeclared
        );
        assert!(
            report.declared.is_empty(),
            "no extensions declared in manifest"
        );
    }
}
