use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_checksum: Option<String>,
}

pub struct ListAttachmentsResult {
    pub source_documents_path: String,
    pub entries: Vec<AttachmentEntry>,
}

/// List source document attachments by walking `source_documents_path` recursively.
///
/// Sidecar files (`.meta.json`) are excluded from the listing; their metadata is
/// surfaced through the index fields (`document_id`, `title`, etc.) on the content entry.
/// Files not present in `manifest.sourceDocumentIndex` appear with only `path` populated.
pub fn list_attachments(
    store: &dyn RepositoryStore,
) -> Result<ListAttachmentsResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();

    // Build index map keyed on content_path (relative to src_docs_base).
    let empty = Vec::new();
    let index_entries = manifest.source_document_index.as_deref().unwrap_or(&empty);
    let index_map: HashMap<&str, _> = index_entries
        .iter()
        .map(|e| (e.content_path.as_str(), e))
        .collect();

    // Walk the filesystem. list_files_recursive returns paths relative to repo root,
    // prefixed with src_docs_base + "/".
    let prefix = format!("{}/", src_docs_base);
    let all_files = store.list_files_recursive(&src_docs_base);

    let mut entries: Vec<AttachmentEntry> = all_files
        .into_iter()
        // Strip the base prefix to get the path relative to source-documents/.
        .filter_map(|repo_rel| {
            repo_rel
                .strip_prefix(&prefix)
                .map(|s| s.to_string())
        })
        // Exclude .meta.json sidecars.
        .filter(|rel| !rel.ends_with(".meta.json"))
        .map(|rel| {
            if let Some(idx) = index_map.get(rel.as_str()) {
                AttachmentEntry {
                    path: rel,
                    document_id: Some(idx.document_id.clone()).filter(|s| !s.is_empty()),
                    title: idx.title.clone(),
                    content_checksum: idx.content_checksum.clone(),
                    sidecar_checksum: idx.sidecar_checksum.clone(),
                }
            } else {
                AttachmentEntry {
                    path: rel,
                    document_id: None,
                    title: None,
                    content_checksum: None,
                    sidecar_checksum: None,
                }
            }
        })
        .collect();

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(ListAttachmentsResult {
        source_documents_path: src_docs_base,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use srs_core::types::source_document::SourceDocumentIndexEntry;
    use std::path::PathBuf;

    fn test_package() -> Package {
        Package {
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
        }
    }

    fn store_with_manifest(manifest: Manifest) -> MemoryStore {
        MemoryStore::new(manifest, test_package())
    }

    fn manifest_with_index(
        source_documents_path: Option<String>,
        entries: Vec<SourceDocumentIndexEntry>,
    ) -> Manifest {
        Manifest {
            source_documents_path,
            source_document_index: Some(entries),
            ..Manifest::default()
        }
    }

    // MemoryStore::list_files_recursive only scans the text-data store.
    // In tests, use save_text_file (empty string suffices) to register a file path.
    fn touch(store: &MemoryStore, path: &str) {
        store.save_text_file(path, "").unwrap();
    }

    #[test]
    fn list_attachments_empty_store() {
        let store = store_with_manifest(Manifest::default());
        let result = list_attachments(&store).unwrap();
        assert_eq!(result.source_documents_path, "source-documents");
        assert!(result.entries.is_empty());
    }

    #[test]
    fn list_attachments_indexed_file() {
        let store = store_with_manifest(manifest_with_index(
            None,
            vec![SourceDocumentIndexEntry {
                document_id: "doc-uuid-1".to_string(),
                sidecar_path: "my-doc.meta.json".to_string(),
                content_path: "my-doc.pdf".to_string(),
                title: Some("My Document".to_string()),
                sidecar_checksum: Some("sha256:aaa".to_string()),
                content_checksum: Some("sha256:bbb".to_string()),
            }],
        ));
        touch(&store, "source-documents/my-doc.pdf");
        touch(&store, "source-documents/my-doc.meta.json");

        let result = list_attachments(&store).unwrap();
        assert_eq!(result.entries.len(), 1);
        let e = &result.entries[0];
        assert_eq!(e.path, "my-doc.pdf");
        assert_eq!(e.document_id.as_deref(), Some("doc-uuid-1"));
        assert_eq!(e.title.as_deref(), Some("My Document"));
        assert_eq!(e.content_checksum.as_deref(), Some("sha256:bbb"));
        assert_eq!(e.sidecar_checksum.as_deref(), Some("sha256:aaa"));
    }

    #[test]
    fn list_attachments_unindexed_file() {
        let store = store_with_manifest(Manifest::default());
        touch(&store, "source-documents/unknown.docx");

        let result = list_attachments(&store).unwrap();
        assert_eq!(result.entries.len(), 1);
        let e = &result.entries[0];
        assert_eq!(e.path, "unknown.docx");
        assert!(e.document_id.is_none());
        assert!(e.title.is_none());
        assert!(e.content_checksum.is_none());
    }

    #[test]
    fn list_attachments_walks_subdirs() {
        let store = store_with_manifest(Manifest::default());
        touch(&store, "source-documents/subdir/nested.pdf");
        touch(&store, "source-documents/top.pdf");

        let result = list_attachments(&store).unwrap();
        assert_eq!(result.entries.len(), 2);
        let paths: Vec<&str> = result.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.contains(&"subdir/nested.pdf"),
            "expected subdir/nested.pdf, got {paths:?}"
        );
        assert!(
            paths.contains(&"top.pdf"),
            "expected top.pdf, got {paths:?}"
        );
    }

    #[test]
    fn list_attachments_excludes_sidecars() {
        let store = store_with_manifest(Manifest::default());
        touch(&store, "source-documents/doc.pdf");
        touch(&store, "source-documents/doc.meta.json");

        let result = list_attachments(&store).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].path, "doc.pdf");
    }

    #[test]
    fn list_attachments_custom_path() {
        let manifest = Manifest {
            source_documents_path: Some("attachments".to_string()),
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        touch(&store, "attachments/report.pdf");

        let result = list_attachments(&store).unwrap();
        assert_eq!(result.source_documents_path, "attachments");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].path, "report.pdf");
    }

    // Cross-store roundtrip: MemoryStore setup → FileStore exercised (CLAUDE.md requirement).
    #[test]
    fn list_attachments_filestore_roundtrip() {
        use crate::store::FileStore;
        use srs_core::types::source_document::SourceDocumentIndexEntry;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Minimal repo scaffold.
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        let manifest_json = serde_json::json!({
            "instanceIndex": [],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": [
                {
                    "documentId": "roundtrip-uuid",
                    "sidecarPath": "brief.meta.json",
                    "contentPath": "brief.pdf",
                    "title": "Roundtrip Brief",
                    "contentChecksum": "sha256:ccc"
                }
            ]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "rt-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();
        // Write a real file (content) and sidecar.
        std::fs::create_dir_all(root.join("source-documents")).unwrap();
        std::fs::write(root.join("source-documents/brief.pdf"), b"pdf bytes").unwrap();
        std::fs::write(
            root.join("source-documents/brief.meta.json"),
            r#"{"documentId":"roundtrip-uuid"}"#,
        )
        .unwrap();
        // Write a file in a subdirectory (not in index).
        std::fs::create_dir_all(root.join("source-documents/annexes")).unwrap();
        std::fs::write(root.join("source-documents/annexes/annex-a.pdf"), b"annex").unwrap();

        let store = FileStore::new(root);
        let result = list_attachments(&store).unwrap();

        assert_eq!(result.source_documents_path, "source-documents");
        let paths: Vec<&str> = result.entries.iter().map(|e| e.path.as_str()).collect();
        // content file is indexed → metadata populated
        let brief = result.entries.iter().find(|e| e.path == "brief.pdf").unwrap();
        assert_eq!(brief.document_id.as_deref(), Some("roundtrip-uuid"));
        assert_eq!(brief.title.as_deref(), Some("Roundtrip Brief"));
        // subdirectory file is present but not indexed
        assert!(
            paths.contains(&"annexes/annex-a.pdf"),
            "expected annexes/annex-a.pdf, got {paths:?}"
        );
        let annex = result.entries.iter().find(|e| e.path == "annexes/annex-a.pdf").unwrap();
        assert!(annex.document_id.is_none());
        // sidecar excluded
        assert!(!paths.contains(&"brief.meta.json"));
    }
}
