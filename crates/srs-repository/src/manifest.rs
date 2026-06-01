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

    fn srs_spec_repo() -> PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
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
    fn live_manifest_loads_and_has_correct_first_entry() {
        let repo_root = srs_spec_repo();
        let manifest = load_manifest(&repo_root).unwrap();

        assert!(!manifest.instance_index.is_empty());
        assert_eq!(
            manifest.instance_index[0].path(),
            "records/notes/origin-purpose.json"
        );
    }

    #[test]
    fn string_index_entries_are_rejected() {
        let result: Result<Manifest, _> = serde_json::from_str(
            r#"{
                "instanceIndex": [
                    "records/notes/foo.json"
                ]
            }"#,
        );

        assert!(result.is_err());
    }
}
