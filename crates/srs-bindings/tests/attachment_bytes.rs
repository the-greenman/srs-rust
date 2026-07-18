// Integration tests for get_attachment_bytes.
//
// Note: the WASM binding wrapper (SrsRepository::get_attachment_bytes) routes through
// js_sys::Uint8Array which is not meaningful on a native target. These tests validate
// the underlying service functions directly; the wasm32 build gate confirms the binding
// layer compiles and links correctly.

use srs_repository::attachment_service::{
    get_attachment_bytes, AddAttachmentInput, GetAttachmentBytesInput,
    add_attachment,
};
use srs_repository::error::RepositoryError;
use srs_repository::{archive_to_vec, FileStore, JsonStore};

fn filestore_with_repo(tmp: &std::path::Path) -> FileStore {
    std::fs::create_dir_all(tmp.join(".srs")).unwrap();
    std::fs::create_dir_all(tmp.join("package")).unwrap();
    std::fs::write(
        tmp.join("manifest.json"),
        serde_json::json!({"instanceIndex": []}).to_string(),
    )
    .unwrap();
    std::fs::write(
        tmp.join("package/package.json"),
        serde_json::json!({
            "id": "att-bytes-pkg", "namespace": "com.test.attbytes",
            "name": "primary", "version": "1.0.0", "fields": [], "types": []
        })
        .to_string(),
    )
    .unwrap();
    FileStore::new(tmp)
}

#[test]
fn get_attachment_bytes_roundtrip_via_archive() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let store = filestore_with_repo(tmp.path());

    const BYTES: &[u8] = b"\xca\xfe\xba\xbe wasm roundtrip content";

    // add_attachment writes the binary to disk and registers it in the index.
    let added = add_attachment(
        &store,
        AddAttachmentInput {
            file_name: "brief.pdf".to_string(),
            content: BYTES.to_vec(),
            subdir: None,
            title: None,
            content_type: None,
        },
    )
    .expect("add_attachment should succeed");
    let doc_id = added.document_id.clone();

    // Pack to .srs archive bytes (mirrors what load_archive receives in WASM).
    let archive = archive_to_vec(&store).expect("pack archive");

    // Load archive into JsonStore (mirrors the WASM load_archive path).
    let json_store = JsonStore::from_archive(&archive).expect("from_archive");

    // get_attachment_bytes must return the original bytes.
    let result = get_attachment_bytes(
        &json_store,
        GetAttachmentBytesInput {
            document_id: doc_id.clone(),
        },
    )
    .expect("get_attachment_bytes should succeed");

    assert_eq!(result.document_id, doc_id);
    assert_eq!(result.content_path, "brief.pdf");
    assert_eq!(result.bytes, BYTES);
}

#[test]
fn get_attachment_bytes_unknown_document_id() {
    // A .srsj store with no sourceDocumentIndex entries.
    let srsj = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "att-bytes-unknown",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": []
        },
        "data": {}
    })
    .to_string();
    let store = JsonStore::from_srsj(&srsj).expect("load store");

    let err = get_attachment_bytes(
        &store,
        GetAttachmentBytesInput {
            document_id: "no-such-doc".to_string(),
        },
    )
    .expect_err("unknown documentId must return an error");

    assert!(
        matches!(err, RepositoryError::InvalidInput { .. }),
        "unknown documentId must be InvalidInput, got: {err:?}"
    );
}

#[test]
fn get_attachment_bytes_srsj_tombstone() {
    // A .srsj store with an index entry but no binary file stored.
    // JsonStore::load_binary_file returns not-found (tombstone per RFC-017).
    let srsj = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "att-bytes-tombstone",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": [{
                "documentId": "tomb-doc",
                "sidecarPath": "tombstone.meta.json",
                "contentPath": "tombstone.pdf"
            }]
        },
        "data": {}
    })
    .to_string();
    let store = JsonStore::from_srsj(&srsj).expect("load store");

    let err = get_attachment_bytes(
        &store,
        GetAttachmentBytesInput {
            document_id: "tomb-doc".to_string(),
        },
    )
    .expect_err("tombstone must return an error");

    assert!(
        err.is_not_found(),
        "tombstone must be not-found error, got: {err:?}"
    );
}
