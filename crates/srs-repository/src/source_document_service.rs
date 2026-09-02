use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::source_document_meta::SourceDocumentMeta;

/// A source-document sidecar entry returned by `list_source_documents`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDocumentEntry {
    /// Path relative to the source-documents directory (e.g. "spec/srs-spec.md.meta.json").
    /// Matches the convention of `SourceDocumentIndexEntry.sidecar_path`.
    pub sidecar_path: String,
    pub meta: SourceDocumentMeta,
}

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
    pub entries: Vec<SourceDocumentEntry>,
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
/// Resolves the configured `sourceDocumentsPath` from `manifest.json` (defaulting to
/// `"source-documents"`) and scans it recursively for `*.meta.json` files using
/// `RepositoryStore::list_files_recursive`. Each valid sidecar is returned as a
/// `SourceDocumentEntry` with `sidecar_path` relative to the base directory.
///
/// - **Strict mode** (`filter.lenient = false`, the default): returns `Err` if any sidecar fails
///   to parse. `ListSourceDocumentsResult.errors` is always empty on success.
/// - **Lenient mode** (`filter.lenient = true`): skips malformed sidecars and collects them in
///   `ListSourceDocumentsResult.errors`; never returns `Err` for a parse failure.
///
/// Returns `Err` if the manifest cannot be loaded (unlike the prior `list_source_document_sidecar_paths`
/// store method, which silently fell back to the default path on manifest errors).
pub fn list_source_documents(
    store: &dyn RepositoryStore,
    filter: ListSourceDocumentsFilter,
) -> Result<ListSourceDocumentsResult, RepositoryError> {
    let manifest = store.load_manifest()?;
    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();
    let prefix = format!("{}/", src_docs_base);
    let sidecar_repo_paths: Vec<(String, String)> = store
        .list_files_recursive(&src_docs_base)
        .into_iter()
        .filter(|p| p.ends_with(".meta.json"))
        .filter_map(|repo_rel| {
            repo_rel
                .strip_prefix(&prefix)
                .map(|rel| (repo_rel.clone(), rel.to_string()))
        })
        .collect();
    let mut entries = Vec::with_capacity(sidecar_repo_paths.len());
    let mut errors: Vec<SourceDocumentParseError> = Vec::new();
    for (repo_relative_path, sidecar_path) in sidecar_repo_paths {
        let json_str = store.load_text_file(&repo_relative_path)?;
        match serde_json::from_str::<SourceDocumentMeta>(&json_str) {
            Ok(meta) => entries.push(SourceDocumentEntry { sidecar_path, meta }),
            Err(source) => {
                if filter.lenient {
                    errors.push(SourceDocumentParseError {
                        path: PathBuf::from(&repo_relative_path),
                        message: source.to_string(),
                    });
                } else {
                    return Err(RepositoryError::SourceDocumentMetaLoad {
                        path: PathBuf::from(&repo_relative_path),
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

    // ── memory store tests ────────────────────────────────────────────────────────

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
            result.entries[0].meta.document_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
        assert_eq!(result.entries[0].meta.content_type, "text/markdown");
        assert_eq!(result.entries[0].sidecar_path, "test.md.meta.json");
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
            .map(|e| e.meta.document_id.as_str())
            .collect();
        assert!(ids.contains(&"aaaaaaaa-0000-4000-8000-000000000001"));
        assert!(ids.contains(&"bbbbbbbb-0000-4000-8000-000000000002"));
        let sidecar_paths: Vec<_> = result
            .entries
            .iter()
            .map(|e| e.sidecar_path.as_str())
            .collect();
        assert!(sidecar_paths.contains(&"sub/a.md.meta.json"));
        assert!(sidecar_paths.contains(&"sub/b.pdf.meta.json"));
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
        // The spec-repo fixture has 5 .meta.json sidecars (srs-rust#825
        // re-vendor added source-documents/spec/srs-purpose-and-scope.md).
        assert_eq!(
            result.entries.len(),
            5,
            "expected 5 sidecars, got {} entries",
            result.entries.len()
        );
        assert!(result.errors.is_empty());
        for entry in &result.entries {
            assert!(!entry.meta.document_id.is_empty());
            assert!(!entry.meta.content_type.is_empty());
            assert!(!entry.sidecar_path.is_empty());
        }
    }

    #[test]
    fn memory_store_custom_source_documents_path() {
        use crate::manifest::Manifest;
        use crate::store::RepositoryStore;

        let store = crate::store::memory::MemoryStore::empty();
        let manifest = Manifest {
            source_documents_path: Some("attachments".to_string()),
            ..Default::default()
        };
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
            result.entries[0].meta.document_id,
            "cccccccc-0000-4000-8000-000000000001"
        );
        assert_eq!(result.entries[0].sidecar_path, "doc.md.meta.json");

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
            result.entries[0].meta.document_id,
            "dddddddd-0000-4000-8000-000000000002"
        );
        assert_eq!(result.entries[0].sidecar_path, "fallback.md.meta.json");
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
            result.entries[0].meta.document_id,
            "cccccccc-0000-4000-8000-000000000001"
        );
        assert_eq!(result.entries[0].meta.content_type, "text/markdown");
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
            result.entries[0].meta.document_id,
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
            result.entries[0].meta.document_id,
            "bbbbbbbb-2222-4000-8000-000000000001"
        );
        assert!(!result.errors[0].message.is_empty());
    }

    // ── migrated from attachment_service tests ────────────────────────────────────

    #[test]
    fn attachment_service_list_source_documents_empty() {
        use crate::manifest::Manifest;
        use crate::store::memory::MemoryStore;
        let store = MemoryStore::new(Manifest::default(), test_package());
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert!(result.entries.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn attachment_service_list_source_documents_single() {
        use crate::manifest::Manifest;
        use crate::store::memory::MemoryStore;
        let store = MemoryStore::new(Manifest::default(), test_package());
        store
            .save_text_file(
                "source-documents/my-doc.meta.json",
                r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000001","contentPath":"my-doc.pdf","contentType":"text/plain"}"#,
            )
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].sidecar_path, "my-doc.meta.json");
        assert_eq!(
            result.entries[0].meta.document_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
    }

    #[test]
    fn attachment_service_list_source_documents_subdirectory() {
        use crate::manifest::Manifest;
        use crate::store::memory::MemoryStore;
        let store = MemoryStore::new(Manifest::default(), test_package());
        store
            .save_text_file(
                "source-documents/sub/a.meta.json",
                r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000002","contentPath":"sub/a.pdf","contentType":"text/plain"}"#,
            )
            .unwrap();
        store
            .save_text_file(
                "source-documents/sub/b.meta.json",
                r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000003","contentPath":"sub/b.pdf","contentType":"text/plain"}"#,
            )
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.entries.len(), 2);
        let paths: Vec<&str> = result
            .entries
            .iter()
            .map(|e| e.sidecar_path.as_str())
            .collect();
        assert!(
            paths.contains(&"sub/a.meta.json"),
            "expected sub/a.meta.json, got {paths:?}"
        );
        assert!(
            paths.contains(&"sub/b.meta.json"),
            "expected sub/b.meta.json, got {paths:?}"
        );
    }

    #[test]
    fn attachment_service_list_source_documents_malformed_returns_err() {
        use crate::manifest::Manifest;
        use crate::store::memory::MemoryStore;
        let store = MemoryStore::new(Manifest::default(), test_package());
        store
            .save_text_file("source-documents/bad.meta.json", "not valid json")
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default());
        assert!(
            matches!(result, Err(RepositoryError::SourceDocumentMetaLoad { .. })),
            "expected SourceDocumentMetaLoad error, got {result:?}"
        );
    }

    #[test]
    fn attachment_service_file_store_list_source_documents_spec_repo() {
        use crate::store::FileStore;
        let repo_root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/spec-repo");
        let store = FileStore::new(&repo_root);
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result.entries.len(),
            5,
            "expected 5 sidecars, got {:?}",
            result
                .entries
                .iter()
                .map(|e| &e.sidecar_path)
                .collect::<Vec<_>>()
        );
        for entry in &result.entries {
            assert!(
                !entry.meta.document_id.is_empty(),
                "document_id missing in {}",
                entry.sidecar_path
            );
            assert!(
                !entry.meta.content_type.is_empty(),
                "content_type missing in {}",
                entry.sidecar_path
            );
        }
    }

    // ── cross-store roundtrip test ────────────────────────────────────────────────

    #[test]
    fn file_store_list_source_documents_tempdir_repo() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Minimal repo scaffold.
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        std::fs::write(
            root.join("manifest.json"),
            serde_json::json!({"dataModelRevision": 2}).to_string(),
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

        // Two sidecars: one top-level, one in a subdirectory.
        std::fs::create_dir_all(root.join("source-documents/sub")).unwrap();
        std::fs::write(
            root.join("source-documents/top.md.meta.json"),
            r#"{"documentId":"roundtrip-top-001","contentPath":"top.md","contentType":"text/markdown"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("source-documents/sub/nested.pdf.meta.json"),
            r#"{"documentId":"roundtrip-nested-002","contentPath":"sub/nested.pdf","contentType":"application/pdf"}"#,
        )
        .unwrap();

        let store = FileStore::new(root);
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();

        assert_eq!(result.entries.len(), 2, "expected 2 sidecars");
        assert!(result.errors.is_empty());

        let ids: Vec<&str> = result
            .entries
            .iter()
            .map(|e| e.meta.document_id.as_str())
            .collect();
        assert!(ids.contains(&"roundtrip-top-001"), "missing top sidecar");
        assert!(
            ids.contains(&"roundtrip-nested-002"),
            "missing nested sidecar"
        );

        let paths: Vec<&str> = result
            .entries
            .iter()
            .map(|e| e.sidecar_path.as_str())
            .collect();
        assert!(
            paths.contains(&"top.md.meta.json"),
            "sidecar_path must be relative to src_docs_base, got {paths:?}"
        );
        assert!(
            paths.contains(&"sub/nested.pdf.meta.json"),
            "subdirectory sidecar_path must be relative to src_docs_base, got {paths:?}"
        );
    }

    #[test]
    fn file_store_corrupt_manifest_returns_err() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        // Write a syntactically invalid manifest.json to force load_manifest() -> Err.
        std::fs::write(root.join("manifest.json"), "not-valid-json").unwrap();

        let store = FileStore::new(root);
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default());
        assert!(
            result.is_err(),
            "expected Err on corrupt manifest, got {result:?}"
        );
    }

    fn test_package() -> crate::package::Package {
        crate::package::Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }
}
