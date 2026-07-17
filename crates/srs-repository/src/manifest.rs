use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use serde::{Deserialize, Serialize};
use srs_core::extensions::import_tracking::UpstreamPackage;
use srs_core::types::container::{Container, ContainerIndexEntry};
use srs_core::types::source_document::SourceDocumentIndexEntry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(rename = "instanceIndex")]
    pub instance_index: Vec<InstanceIndexEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<Container>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_index: Option<Vec<ContainerIndexEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federation_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federation_events_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_package: Option<UpstreamPackage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_documents_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_document_index: Option<Vec<SourceDocumentIndexEntry>>,
    // all other manifest fields preserved for round-trip write
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
    // set by loader, not from JSON
    #[serde(skip)]
    pub root: PathBuf,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            instance_index: Vec::new(),
            container: None,
            container_index: None,
            federation_path: None,
            federation_events_path: None,
            upstream_package: None,
            source_documents_path: None,
            source_document_index: None,
            extra: HashMap::new(),
            root: PathBuf::new(),
        }
    }
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

    let mut raw: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| RepositoryError::ManifestParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    migrate_upstream_package(&mut raw);

    let mut manifest: Manifest =
        serde_json::from_value(raw).map_err(|e| RepositoryError::ManifestParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    manifest.root = repo_root.to_path_buf();
    Ok(manifest)
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

    #[test]
    fn manifest_with_container_roundtrips() {
        let json = r#"{
            "instanceIndex": [],
            "container": {
                "containerId": "550e8400-e29b-41d4-a716-446655440000",
                "identityInstanceId": "aaaaaaaa-0000-4000-8000-aaaaaaaaaaaa"
            }
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        let container = manifest.container.as_ref().unwrap();
        assert_eq!(
            container.container_id,
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_eq!(
            container.identity_instance_id.as_deref(),
            Some("aaaaaaaa-0000-4000-8000-aaaaaaaaaaaa")
        );
        // must not appear in extra
        assert!(!manifest.extra.contains_key("container"));

        // serialise and re-parse
        let serialised = serde_json::to_string(&manifest).unwrap();
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(
            reparsed.container.as_ref().unwrap().identity_instance_id,
            container.identity_instance_id
        );
    }

    #[test]
    fn manifest_with_container_index_roundtrips() {
        let json = r#"{
            "instanceIndex": [],
            "containerIndex": [
                {"containerId": "c1", "title": "Alpha", "path": "containers/alpha.json"},
                {"containerId": "c2", "path": "containers/beta.json"}
            ]
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        let index = manifest.container_index.as_ref().unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index[0].container_id, "c1");
        assert_eq!(index[0].title.as_deref(), Some("Alpha"));
        assert_eq!(index[0].path.as_deref(), Some("containers/alpha.json"));
        assert_eq!(index[1].container_id, "c2");
        assert_eq!(index[1].title, None);
        // must not appear in extra
        assert!(!manifest.extra.contains_key("containerIndex"));

        // serialise and re-parse
        let serialised = serde_json::to_string(&manifest).unwrap();
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(reparsed.container_index, manifest.container_index);
    }

    #[test]
    fn manifest_absent_container_fields_are_none() {
        let json = r#"{"instanceIndex": []}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.container.is_none());
        assert!(manifest.container_index.is_none());
    }

    #[test]
    fn manifest_federation_fields_roundtrip() {
        let json = r#"{
            "instanceIndex": [],
            "federationPath": "custom/registry.json",
            "federationEventsPath": "custom/events.json"
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(
            manifest.federation_path.as_deref(),
            Some("custom/registry.json")
        );
        assert_eq!(
            manifest.federation_events_path.as_deref(),
            Some("custom/events.json")
        );
        // must not appear in extra
        assert!(!manifest.extra.contains_key("federationPath"));
        assert!(!manifest.extra.contains_key("federationEventsPath"));

        // serialise and re-parse
        let serialised = serde_json::to_string(&manifest).unwrap();
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(reparsed.federation_path, manifest.federation_path);
        assert_eq!(
            reparsed.federation_events_path,
            manifest.federation_events_path
        );
    }

    #[test]
    fn manifest_absent_federation_fields_are_none() {
        let json = r#"{"instanceIndex": []}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.federation_path.is_none());
        assert!(manifest.federation_events_path.is_none());
    }

    #[test]
    fn manifest_upstream_package_roundtrips() {
        let json = r#"{
            "instanceIndex": [],
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
        let json = r#"{"instanceIndex": []}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.upstream_package.is_none());
    }

    #[test]
    fn migrate_upstream_package_renames_id_to_package_id() {
        let mut raw = serde_json::json!({
            "instanceIndex": [],
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
            "instanceIndex": [],
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
            "instanceIndex": [],
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
            "instanceIndex": [],
            "sourceDocumentsPath": "attachments"
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.source_documents_path.as_deref(), Some("attachments"));
        // must not appear in extra
        assert!(!manifest.extra.contains_key("sourceDocumentsPath"));

        let serialised = serde_json::to_string(&manifest).unwrap();
        assert!(serialised.contains("\"sourceDocumentsPath\""));
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(reparsed.source_documents_path, manifest.source_documents_path);
    }

    #[test]
    fn manifest_source_document_index_roundtrips() {
        let json = r#"{
            "instanceIndex": [],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": [
                {
                    "documentId": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
                    "sidecarPath": "my-doc.meta.json",
                    "contentPath": "my-doc.pdf"
                }
            ]
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        let idx = manifest.source_document_index.as_ref().unwrap();
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].document_id, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
        assert_eq!(idx[0].sidecar_path, "my-doc.meta.json");
        assert_eq!(idx[0].content_path, "my-doc.pdf");
        assert!(idx[0].title.is_none());
        // must not appear in extra
        assert!(!manifest.extra.contains_key("sourceDocumentIndex"));

        let serialised = serde_json::to_string(&manifest).unwrap();
        let reparsed: Manifest = serde_json::from_str(&serialised).unwrap();
        assert_eq!(reparsed.source_document_index, manifest.source_document_index);
    }

    #[test]
    fn manifest_absent_source_doc_fields_are_none() {
        let json = r#"{"instanceIndex": []}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.source_documents_path.is_none());
        assert!(manifest.source_document_index.is_none());
    }
}
