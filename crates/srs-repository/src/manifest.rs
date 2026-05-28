use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(rename = "instanceIndex")]
    pub instance_index: Vec<InstanceIndexEntry>,
    // all other manifest fields preserved for round-trip write
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
    // set by loader, not from JSON
    #[serde(skip)]
    pub root: PathBuf,
}

pub fn load_manifest(repo_root: &Path) -> Result<Manifest, RepositoryError> {
    let manifest_path = repo_root.join("manifest.json");

    if !manifest_path.exists() {
        return Err(RepositoryError::ManifestMissing {
            path: manifest_path,
        });
    }

    let content = std::fs::read_to_string(&manifest_path).map_err(|e| RepositoryError::Io {
        path: manifest_path.clone(),
        source: e,
    })?;

    let mut manifest: Manifest =
        serde_json::from_str(&content).map_err(|e| RepositoryError::ManifestParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    manifest.root = repo_root.to_path_buf();
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_load_live_manifest() {
        let repo_root = PathBuf::from("/home/greenman/dev/semanticops/srs");
        let manifest = load_manifest(&repo_root).unwrap();

        assert!(!manifest.instance_index.is_empty());
        assert_eq!(
            manifest.instance_index[0].path(),
            "records/notes/origin-purpose.json"
        );
    }

    #[test]
    fn test_legacy_index_round_trips() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("manifest.json");

        // Create minimal manifest with string-array instanceIndex
        let json = r#"{
            "srsVersion": "2.0-draft",
            "instanceIndex": [
                "records/notes/foo.json",
                "records/notes/bar.json"
            ]
        }"#;

        fs::write(&manifest_path, json).unwrap();

        let manifest = load_manifest(temp.path()).unwrap();
        assert_eq!(manifest.instance_index.len(), 2);
        assert_eq!(manifest.instance_index[0].path(), "records/notes/foo.json");
        assert_eq!(manifest.instance_index[1].path(), "records/notes/bar.json");
    }
}
