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
