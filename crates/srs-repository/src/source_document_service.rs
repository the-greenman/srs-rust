use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::source_document_meta::SourceDocumentMeta;

/// Filter parameters for `list_source_documents`. Currently empty; extended in future issues
/// without breaking the service boundary (per ADR-010: list functions accept a filter struct).
#[derive(Debug, Clone, Default)]
pub struct ListSourceDocumentsFilter {}

/// Enumerate all source-document sidecars in the repository.
///
/// Scans the directory configured by `sourceDocumentsPath` in `manifest.json` (defaulting to
/// `"source-documents"`) recursively for `*.meta.json` files and parses each.
/// Returns `Err` if any sidecar fails to parse (malformed JSON or missing required fields).
pub fn list_source_documents(
    store: &dyn RepositoryStore,
    _filter: ListSourceDocumentsFilter,
) -> Result<Vec<SourceDocumentMeta>, RepositoryError> {
    let sidecar_paths = store.list_source_document_sidecar_paths();
    let mut entries = Vec::with_capacity(sidecar_paths.len());
    for sidecar_path in sidecar_paths {
        let json_str = store.load_text_file(&sidecar_path)?;
        let meta = serde_json::from_str::<SourceDocumentMeta>(&json_str).map_err(|source| {
            RepositoryError::SourceDocumentMetaLoad {
                path: std::path::PathBuf::from(&sidecar_path),
                source,
            }
        })?;
        entries.push(meta);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RepositoryStore;

    #[test]
    fn memory_store_list_source_documents_empty() {
        let store = crate::store::memory::MemoryStore::empty();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn memory_store_list_source_documents_single() {
        let store = crate::store::memory::MemoryStore::empty();
        let meta_json = r#"{
            "documentId": "aaaaaaaa-0000-4000-8000-000000000001",
            "contentPath": "test.md",
            "contentType": "text/markdown",
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        store
            .save_text_file("source-documents/test.md.meta.json", meta_json)
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].document_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
        assert_eq!(result[0].content_type, "text/markdown");
    }

    #[test]
    fn memory_store_list_source_documents_subdirectory() {
        let store = crate::store::memory::MemoryStore::empty();
        let meta_json_a = r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000001","contentPath":"a.md","contentType":"text/markdown","createdAt":"2026-01-01T00:00:00Z"}"#;
        let meta_json_b = r#"{"documentId":"bbbbbbbb-0000-4000-8000-000000000002","contentPath":"b.pdf","contentType":"application/pdf","createdAt":"2026-01-01T00:00:00Z"}"#;
        store
            .save_text_file("source-documents/sub/a.md.meta.json", meta_json_a)
            .unwrap();
        store
            .save_text_file("source-documents/sub/b.pdf.meta.json", meta_json_b)
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.len(), 2);
        let ids: Vec<_> = result.iter().map(|e| e.document_id.as_str()).collect();
        assert!(ids.contains(&"aaaaaaaa-0000-4000-8000-000000000001"));
        assert!(ids.contains(&"bbbbbbbb-0000-4000-8000-000000000002"));
    }

    #[test]
    fn memory_store_malformed_sidecar_returns_err() {
        let store = crate::store::memory::MemoryStore::empty();
        store
            .save_text_file("source-documents/bad.md.meta.json", "not-valid-json")
            .unwrap();
        assert!(list_source_documents(&store, ListSourceDocumentsFilter::default()).is_err());
    }

    #[test]
    fn memory_store_valid_json_missing_required_field_returns_err() {
        let store = crate::store::memory::MemoryStore::empty();
        store
            .save_text_file(
                "source-documents/incomplete.meta.json",
                r#"{"documentId":"test","contentPath":"test.md"}"#,
            )
            .unwrap();
        assert!(list_source_documents(&store, ListSourceDocumentsFilter::default()).is_err());
    }

    #[test]
    fn file_store_list_source_documents_spec_repo() {
        use crate::store::FileStore;
        let repo_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/spec-repo"
        );
        let store = FileStore::new(repo_path);
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        // The spec-repo fixture has 4 .meta.json sidecars:
        //   source-documents/spec/srs-spec.md.meta.json
        //   source-documents/ai-sessions/chatgpt-origin.md.meta.json
        //   source-documents/ai-sessions/chatgpt-spec-review.md.meta.json
        //   source-documents/ai-sessions/claude-collaborative-document.md.meta.json
        assert_eq!(
            result.len(),
            4,
            "expected 4 sidecars, got {} entries",
            result.len()
        );
        for entry in &result {
            assert!(!entry.document_id.is_empty());
            assert!(!entry.content_type.is_empty());
        }
    }

    #[test]
    fn memory_store_custom_source_documents_path() {
        use crate::manifest::Manifest;
        use crate::store::RepositoryStore;

        let store = crate::store::memory::MemoryStore::empty();
        let mut manifest = Manifest::default();
        manifest.source_documents_path = Some("attachments".to_string());
        store.save_manifest(&manifest).unwrap();

        let meta_json = r#"{"documentId":"cccccccc-0000-4000-8000-000000000001","contentPath":"doc.md","contentType":"text/markdown","createdAt":"2026-01-01T00:00:00Z"}"#;
        store
            .save_text_file("attachments/doc.md.meta.json", meta_json)
            .unwrap();

        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.len(), 1, "expected sidecar from custom path 'attachments'");
        assert_eq!(
            result[0].document_id,
            "cccccccc-0000-4000-8000-000000000001"
        );

        // Nothing from the default "source-documents/" path when none exist there
        store
            .save_text_file(
                "source-documents/should-not-appear.md.meta.json",
                meta_json,
            )
            .unwrap();
        let result2 = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result2.len(),
            1,
            "only the sidecar from the manifest path should be returned"
        );
    }

    #[test]
    fn memory_store_missing_manifest_falls_back_to_default() {
        let store = crate::store::memory::MemoryStore::empty();
        let meta_json = r#"{"documentId":"dddddddd-0000-4000-8000-000000000002","contentPath":"fallback.md","contentType":"text/plain","createdAt":"2026-01-01T00:00:00Z"}"#;
        store
            .save_text_file("source-documents/fallback.md.meta.json", meta_json)
            .unwrap();

        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result.len(),
            1,
            "should find sidecars in default 'source-documents' when manifest has no override"
        );
        assert_eq!(
            result[0].document_id,
            "dddddddd-0000-4000-8000-000000000002"
        );
    }

    #[test]
    fn file_store_custom_source_documents_path() {
        use crate::store::FileStore;
        let repo_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/custom-source-path-repo"
        );
        let store = FileStore::new(repo_path);
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result.len(),
            1,
            "expected 1 sidecar from 'attachments/' (manifest override), got {}",
            result.len()
        );
        assert_eq!(
            result[0].document_id,
            "cccccccc-0000-4000-8000-000000000001"
        );
        assert_eq!(result[0].content_type, "text/markdown");
    }
}
