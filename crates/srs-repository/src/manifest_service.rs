use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::writer::write_manifest;
use serde_json::json;

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
}
