use crate::error::RepositoryError;
use crate::manifest::load_manifest;
use crate::writer::write_manifest;
use serde_json::json;
use std::path::Path;

/// List declared extension IDs from the manifest
pub fn list_declared_extensions(repo_root: &Path) -> Result<Vec<String>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;

    // declaredExtensions is stored in the extra HashMap
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
    repo_root: &Path,
    extension_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Get current extensions or create new array
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

    // Check if already present
    if !extensions.contains(&extension_id.to_string()) {
        extensions.push(extension_id.to_string());
        // Sort for consistency
        extensions.sort();

        // Update manifest
        manifest
            .extra
            .insert("declaredExtensions".to_string(), json!(extensions));
        write_manifest(&manifest)?;
    }

    Ok(extensions)
}

/// Remove an extension ID from the declared extensions list
pub fn remove_declared_extension(
    repo_root: &Path,
    extension_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Get current extensions
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

    // Check if present before removing
    let was_present = extensions.contains(&extension_id.to_string());

    if was_present {
        extensions.retain(|e| e != extension_id);

        // Update manifest
        if extensions.is_empty() {
            manifest.extra.remove("declaredExtensions");
        } else {
            manifest
                .extra
                .insert("declaredExtensions".to_string(), json!(extensions));
        }
        write_manifest(&manifest)?;
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
pub fn list_package_refs(repo_root: &Path) -> Result<Vec<PackageRef>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;

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
pub fn add_package_ref(repo_root: &Path, path: &str) -> Result<Vec<PackageRef>, RepositoryError> {
    // Validate the path is scoped to the repo and points to a real package.
    let repo_root_canonical = repo_root.canonicalize().map_err(|e| RepositoryError::Io {
        path: repo_root.to_path_buf(),
        source: e,
    })?;

    // Resolve relative to repo_root; reject anything with ../ that escapes.
    let candidate = repo_root.join(path);
    let candidate_canonical =
        candidate
            .canonicalize()
            .map_err(|_| RepositoryError::PackageRefMissing {
                path: path.to_string(),
            })?;
    if !candidate_canonical.starts_with(&repo_root_canonical) {
        return Err(RepositoryError::PackageRefOutsideRepo {
            path: path.to_string(),
        });
    }
    if !candidate_canonical.join("package.json").exists() {
        return Err(RepositoryError::PackageRefMissing {
            path: path.to_string(),
        });
    }

    let mut manifest = load_manifest(repo_root)?;

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
        write_manifest(&manifest)?;
    }

    Ok(refs)
}

/// Remove a package ref from the manifest by path
pub fn remove_package_ref(
    repo_root: &Path,
    path: &str,
) -> Result<Vec<PackageRef>, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

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
        write_manifest(&manifest)?;
    }

    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_minimal_manifest(temp: &TempDir) {
        let manifest = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": []
        });
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn create_manifest_with_extensions(temp: &TempDir) {
        let manifest = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": [],
            "declaredExtensions": ["ext:repository", "ext:relations"]
        });
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_declared_extensions_empty_when_none() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);

        let extensions = list_declared_extensions(temp.path()).unwrap();
        assert!(extensions.is_empty());
    }

    #[test]
    fn list_declared_extensions_returns_extensions() {
        let temp = TempDir::new().unwrap();
        create_manifest_with_extensions(&temp);

        let extensions = list_declared_extensions(temp.path()).unwrap();
        assert_eq!(extensions.len(), 2);
        assert!(extensions.contains(&"ext:repository".to_string()));
        assert!(extensions.contains(&"ext:relations".to_string()));
    }

    #[test]
    fn add_declared_extension_adds_new() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);

        let extensions = add_declared_extension(temp.path(), "ext:new").unwrap();
        assert_eq!(extensions.len(), 1);
        assert!(extensions.contains(&"ext:new".to_string()));

        // Verify it was written to manifest
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let declared = manifest["declaredExtensions"].as_array().unwrap();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0], "ext:new");
    }

    #[test]
    fn add_declared_extension_dedupes() {
        let temp = TempDir::new().unwrap();
        create_manifest_with_extensions(&temp);

        let extensions = add_declared_extension(temp.path(), "ext:repository").unwrap();
        assert_eq!(extensions.len(), 2); // No duplicate added
    }

    #[test]
    fn remove_declared_extension_removes_existing() {
        let temp = TempDir::new().unwrap();
        create_manifest_with_extensions(&temp);

        let extensions = remove_declared_extension(temp.path(), "ext:repository").unwrap();
        assert_eq!(extensions.len(), 1);
        assert!(!extensions.contains(&"ext:repository".to_string()));
        assert!(extensions.contains(&"ext:relations".to_string()));
    }

    #[test]
    fn remove_declared_extension_noop_when_not_present() {
        let temp = TempDir::new().unwrap();
        create_manifest_with_extensions(&temp);

        let extensions = remove_declared_extension(temp.path(), "ext:nonexistent").unwrap();
        assert_eq!(extensions.len(), 2); // Unchanged
    }

    #[test]
    fn remove_last_extension_removes_field() {
        let temp = TempDir::new().unwrap();
        let manifest = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": [],
            "declaredExtensions": ["ext:single"]
        });
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let extensions = remove_declared_extension(temp.path(), "ext:single").unwrap();
        assert!(extensions.is_empty());

        // Verify field was removed from manifest
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(manifest["declaredExtensions"].is_null());
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
        create_minimal_manifest(&temp);

        let result = add_package_ref(temp.path(), "package/nonexistent");
        assert!(
            matches!(result, Err(RepositoryError::PackageRefMissing { .. })),
            "expected PackageRefMissing, got {result:?}"
        );
    }

    #[test]
    fn add_package_ref_rejects_traversal_outside_repo() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);

        // Create a package outside the temp dir to ensure it exists on disk.
        let outside = TempDir::new().unwrap();
        create_package_dir(&outside, ".");
        let traversal = format!("../../../{}", outside.path().display());

        let result = add_package_ref(temp.path(), &traversal);
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
        create_minimal_manifest(&temp);
        create_package_dir(&temp, "package/sub");

        let refs = add_package_ref(temp.path(), "package/sub").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "package/sub");
        assert_eq!(refs[0].mode, "local");
    }

    #[test]
    fn add_package_ref_dedupes() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);
        create_package_dir(&temp, "package/sub");

        add_package_ref(temp.path(), "package/sub").unwrap();
        let refs = add_package_ref(temp.path(), "package/sub").unwrap();
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn list_package_refs_empty_when_none() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);

        let refs = list_package_refs(temp.path()).unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn remove_package_ref_removes_existing() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);
        create_package_dir(&temp, "package/sub");

        add_package_ref(temp.path(), "package/sub").unwrap();
        let refs = remove_package_ref(temp.path(), "package/sub").unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn remove_package_ref_noop_when_not_present() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);

        let refs = remove_package_ref(temp.path(), "package/nonexistent").unwrap();
        assert!(refs.is_empty());
    }

    // Acceptance Criteria Test for Phase 2

    #[test]
    fn declared_extensions_enable_disable_updates_manifest() {
        let temp = TempDir::new().unwrap();
        create_minimal_manifest(&temp);

        // ENABLE: Add multiple extensions
        let ext1 = add_declared_extension(temp.path(), "ext:repository").unwrap();
        assert_eq!(ext1.len(), 1);
        assert!(ext1.contains(&"ext:repository".to_string()));

        let ext2 = add_declared_extension(temp.path(), "ext:relations").unwrap();
        assert_eq!(ext2.len(), 2);
        assert!(ext2.contains(&"ext:repository".to_string()));
        assert!(ext2.contains(&"ext:relations".to_string()));

        // Verify manifest was updated
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let declared = manifest["declaredExtensions"].as_array().unwrap();
        assert_eq!(declared.len(), 2);

        // DISABLE: Remove extensions
        let ext3 = remove_declared_extension(temp.path(), "ext:repository").unwrap();
        assert_eq!(ext3.len(), 1);
        assert!(!ext3.contains(&"ext:repository".to_string()));

        let ext4 = remove_declared_extension(temp.path(), "ext:relations").unwrap();
        assert!(ext4.is_empty());

        // Verify field was removed when empty
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(manifest["declaredExtensions"].is_null());
    }
}
