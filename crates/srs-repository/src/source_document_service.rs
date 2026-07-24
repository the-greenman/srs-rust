use std::path::PathBuf;

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::source_document_meta::SourceDocumentMeta;

/// A parse error collected in lenient mode.
#[derive(Debug, Clone)]
pub struct SourceDocumentParseError {
    /// Repo-relative path of the sidecar that failed to parse.
    pub path: PathBuf,
    /// Display string of the underlying parse error.
    pub message: String,
}

/// Result returned by `list_source_documents`.
///
/// In strict mode (`lenient: false`) `errors` is always empty — the function returns `Err` on the
/// first parse failure instead. In lenient mode (`lenient: true`) `errors` collects every sidecar
/// that failed to parse while `entries` contains all valid entries found.
#[derive(Debug)]
pub struct ListSourceDocumentsResult {
    pub entries: Vec<SourceDocumentMeta>,
    pub errors: Vec<SourceDocumentParseError>,
}

/// Filter parameters for `list_source_documents`.
#[derive(Debug, Clone, Default)]
pub struct ListSourceDocumentsFilter {
    /// When `false` (the default), the first malformed sidecar causes the function to return
    /// `Err`. When `true`, malformed sidecars are skipped and collected in
    /// `ListSourceDocumentsResult.errors`.
    pub lenient: bool,
}

/// Enumerate all source-document sidecars in the repository.
///
/// Scans the directory configured by `sourceDocumentsPath` in `manifest.json` (defaulting to
/// `"source-documents"`) recursively for `*.meta.json` files and parses each.
///
/// - **Strict mode** (`filter.lenient = false`, the default): returns `Err` if any sidecar fails
///   to parse. `ListSourceDocumentsResult.errors` is always empty on success.
/// - **Lenient mode** (`filter.lenient = true`): skips malformed sidecars and collects them in
///   `ListSourceDocumentsResult.errors`; never returns `Err` for a parse failure.
pub fn list_source_documents(
    store: &dyn RepositoryStore,
    filter: ListSourceDocumentsFilter,
) -> Result<ListSourceDocumentsResult, RepositoryError> {
    let sidecar_paths = store.list_source_document_sidecar_paths();
    let mut entries = Vec::with_capacity(sidecar_paths.len());
    let mut errors: Vec<SourceDocumentParseError> = Vec::new();
    for sidecar_path in sidecar_paths {
        let json_str = store.load_text_file(&sidecar_path)?;
        match serde_json::from_str::<SourceDocumentMeta>(&json_str) {
            Ok(meta) => entries.push(meta),
            Err(source) => {
                if filter.lenient {
                    errors.push(SourceDocumentParseError {
                        path: PathBuf::from(&sidecar_path),
                        message: source.to_string(),
                    });
                } else {
                    return Err(RepositoryError::SourceDocumentMetaLoad {
                        path: PathBuf::from(&sidecar_path),
                        source,
                    });
                }
            }
        }
    }
    Ok(ListSourceDocumentsResult { entries, errors })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RepositoryStore;

    #[test]
    fn memory_store_list_source_documents_empty() {
        let store = crate::store::memory::MemoryStore::empty();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert!(result.entries.is_empty());
        assert!(result.errors.is_empty());
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
        assert_eq!(result.entries.len(), 1);
        assert!(result.errors.is_empty());
        assert_eq!(
            result.entries[0].document_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
        assert_eq!(result.entries[0].content_type, "text/markdown");
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
        assert_eq!(result.entries.len(), 2);
        assert!(result.errors.is_empty());
        let ids: Vec<_> = result
            .entries
            .iter()
            .map(|e| e.document_id.as_str())
            .collect();
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
            result.entries.len(),
            4,
            "expected 4 sidecars, got {} entries",
            result.entries.len()
        );
        assert!(result.errors.is_empty());
        for entry in &result.entries {
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
        assert_eq!(
            result.entries.len(),
            1,
            "expected sidecar from custom path 'attachments'"
        );
        assert!(result.errors.is_empty());
        assert_eq!(
            result.entries[0].document_id,
            "cccccccc-0000-4000-8000-000000000001"
        );

        // Nothing from the default "source-documents/" path when none exist there
        store
            .save_text_file("source-documents/should-not-appear.md.meta.json", meta_json)
            .unwrap();
        let result2 = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result2.entries.len(),
            1,
            "only the sidecar from the manifest path should be returned"
        );
        assert!(result2.errors.is_empty());
    }

    #[test]
    fn memory_store_no_source_path_in_manifest_falls_back_to_default() {
        let store = crate::store::memory::MemoryStore::empty();
        let meta_json = r#"{"documentId":"dddddddd-0000-4000-8000-000000000002","contentPath":"fallback.md","contentType":"text/plain","createdAt":"2026-01-01T00:00:00Z"}"#;
        store
            .save_text_file("source-documents/fallback.md.meta.json", meta_json)
            .unwrap();

        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result.entries.len(),
            1,
            "should find sidecars in default 'source-documents' when manifest has no override"
        );
        assert!(result.errors.is_empty());
        assert_eq!(
            result.entries[0].document_id,
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
            result.entries.len(),
            1,
            "expected 1 sidecar from 'attachments/' (manifest override), got {}",
            result.entries.len()
        );
        assert!(result.errors.is_empty());
        assert_eq!(
            result.entries[0].document_id,
            "cccccccc-0000-4000-8000-000000000001"
        );
        assert_eq!(result.entries[0].content_type, "text/markdown");
    }

    // ── lenient mode tests ────────────────────────────────────────────────────────

    #[test]
    fn lenient_mode_skips_malformed_returns_err_in_side_channel() {
        let store = crate::store::memory::MemoryStore::empty();
        let valid_json = r#"{"documentId":"eeeeeeee-0000-4000-8000-000000000001","contentPath":"valid.md","contentType":"text/markdown","createdAt":"2026-01-01T00:00:00Z"}"#;
        store
            .save_text_file("source-documents/valid.md.meta.json", valid_json)
            .unwrap();
        store
            .save_text_file("source-documents/corrupt.md.meta.json", "not-valid-json")
            .unwrap();

        let result =
            list_source_documents(&store, ListSourceDocumentsFilter { lenient: true }).unwrap();

        assert_eq!(result.entries.len(), 1, "should return the valid entry");
        assert_eq!(result.errors.len(), 1, "should collect the parse error");
        assert_eq!(
            result.entries[0].document_id,
            "eeeeeeee-0000-4000-8000-000000000001"
        );
        assert!(result.errors[0]
            .path
            .to_str()
            .unwrap_or("")
            .ends_with("corrupt.md.meta.json"));
        assert!(!result.errors[0].message.is_empty());
    }

    #[test]
    fn lenient_mode_all_malformed_returns_empty_entries() {
        let store = crate::store::memory::MemoryStore::empty();
        store
            .save_text_file("source-documents/a.md.meta.json", "not-json")
            .unwrap();
        store
            .save_text_file("source-documents/b.md.meta.json", "{}")
            .unwrap();

        let result =
            list_source_documents(&store, ListSourceDocumentsFilter { lenient: true }).unwrap();

        assert!(result.entries.is_empty(), "no valid entries expected");
        assert_eq!(result.errors.len(), 2, "both sidecars should error");
    }

    #[test]
    fn strict_mode_still_returns_err_on_malformed() {
        let store = crate::store::memory::MemoryStore::empty();
        store
            .save_text_file("source-documents/bad.md.meta.json", "not-valid-json")
            .unwrap();

        let result = list_source_documents(&store, ListSourceDocumentsFilter { lenient: false });
        assert!(
            result.is_err(),
            "strict mode must return Err on malformed sidecar"
        );
    }

    #[test]
    fn file_store_lenient_mode_skips_malformed_sidecar() {
        use crate::store::FileStore;
        // Fixture has 1 valid + 1 corrupt sidecar; see tests/fixtures/malformed-sidecar-repo/
        let repo_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/malformed-sidecar-repo"
        );
        let store = FileStore::new(repo_path);
        let result =
            list_source_documents(&store, ListSourceDocumentsFilter { lenient: true }).unwrap();

        assert_eq!(
            result.entries.len(),
            1,
            "should return the 1 valid sidecar; got {} entries",
            result.entries.len()
        );
        assert_eq!(
            result.errors.len(),
            1,
            "should collect the 1 corrupt sidecar as an error; got {} errors",
            result.errors.len()
        );
        assert_eq!(
            result.entries[0].document_id,
            "bbbbbbbb-2222-4000-8000-000000000001"
        );
        assert!(!result.errors[0].message.is_empty());
    }
}
