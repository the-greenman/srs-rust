use srs_repository::attachment_service::{list_attachments, ListAttachmentsFilter};
use srs_repository::JsonStore;

fn srsj_empty() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-list-attachments-empty",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": []
        },
        "data": {}
    })
    .to_string()
}

fn srsj_with_indexed_entry() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-list-attachments-indexed",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": [{
                "documentId": "doc-001",
                "contentPath": "brief.pdf",
                "sidecarPath": "brief.meta.json",
                "title": "Board Brief",
                "contentChecksum": "sha256:abc",
                "sidecarChecksum": "sha256:def"
            }]
        },
        "data": {
            "source-documents/brief.pdf": "pdf-content-placeholder",
            "source-documents/brief.meta.json": {
                "documentId": "doc-001",
                "contentPath": "brief.pdf"
            }
        }
    })
    .to_string()
}

fn srsj_with_unindexed_file() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-list-attachments-unindexed",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": []
        },
        "data": {
            "source-documents/orphan.pdf": "orphan-content-placeholder"
        }
    })
    .to_string()
}

#[test]
fn binding_list_attachments_empty_repo() {
    let store = JsonStore::from_srsj(&srsj_empty()).expect("load store");
    let result =
        list_attachments(&store, ListAttachmentsFilter::default()).expect("list_attachments ok");
    assert!(result.entries.is_empty(), "empty repo must yield no entries");
    assert_eq!(
        result.source_documents_path, "source-documents",
        "default source_documents_path must be 'source-documents'"
    );
}

#[test]
fn binding_list_attachments_with_indexed_entry() {
    let store = JsonStore::from_srsj(&srsj_with_indexed_entry()).expect("load store");
    let result =
        list_attachments(&store, ListAttachmentsFilter::default()).expect("list_attachments ok");
    // One content file (brief.pdf); .meta.json sidecar is filtered out by the service.
    assert_eq!(result.entries.len(), 1, "only content file must appear");
    let entry = &result.entries[0];
    assert_eq!(entry.path, "brief.pdf");
    assert_eq!(entry.document_id.as_deref(), Some("doc-001"));
    assert_eq!(entry.title.as_deref(), Some("Board Brief"));
    assert_eq!(entry.content_checksum.as_deref(), Some("sha256:abc"));
    assert_eq!(entry.sidecar_checksum.as_deref(), Some("sha256:def"));
}

#[test]
fn binding_list_attachments_unindexed_file_appears_path_only() {
    let store = JsonStore::from_srsj(&srsj_with_unindexed_file()).expect("load store");
    let result =
        list_attachments(&store, ListAttachmentsFilter::default()).expect("list_attachments ok");
    assert_eq!(result.entries.len(), 1, "unindexed file must still appear");
    let entry = &result.entries[0];
    assert_eq!(entry.path, "orphan.pdf");
    assert!(
        entry.document_id.is_none(),
        "unindexed file must have no documentId"
    );
    assert!(entry.title.is_none(), "unindexed file must have no title");
    assert!(
        entry.content_checksum.is_none(),
        "unindexed file must have no checksum"
    );
}
