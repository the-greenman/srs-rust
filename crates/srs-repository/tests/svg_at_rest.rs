//! An unsafe SVG that arrives by import or merge is kept and reported by
//! `repo validate` as `svg-unsafe` (decision 866167c1, srs-rust#1617).

use serde_json::{json, Value};
use srs_core::types::record::FieldValues;
use srs_repository::attachment_service::{
    add_attachment, link_attachment, AddAttachmentInput, LinkAttachmentInput,
};
use srs_repository::merge_service::{merge_apply, merge_preview, MergeSelection};
use srs_repository::record_store::{self, CreateRecordInput};
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::repository_portability::{
    export_repository_snapshot_with_options, import_repository_snapshot, ExportSnapshotOptions,
};
use srs_repository::validation::validate_repository;
use srs_repository::{archive_to_tree, archive_to_vec, archive_unpack, FileStore, RepositoryStore};
use std::collections::BTreeMap;
use std::io::Cursor;
use tempfile::TempDir;

const REPO: &str = "16170000-0000-4000-8000-000000000001";
const HOSTILE: &[u8] =
    br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;

fn origin() -> (TempDir, FileStore) {
    let dir = TempDir::new().unwrap();
    let store = FileStore::new(dir.path());
    store.set_session_actor(Some(json!({"kind": "human", "id": "user-1"})));
    create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: REPO.to_string(),
                namespace: "com.test.svg".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "16170000-0000-4000-8000-000000000002".to_string(),
                namespace: "com.test.svg".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    (dir, store)
}

/// Attach a safe SVG to a new record, then replace its bytes on disk with a
/// hostile one: the state a copy made elsewhere (or by an older build) is in.
fn plant_hostile_svg(store: &FileStore) {
    let mut field_values = FieldValues::new();
    field_values.insert("statement", Value::String("has a picture".to_string()));
    let record = record_store::create_record_in_context(
        store,
        "com.semanticops.core/purpose",
        None,
        CreateRecordInput {
            field_values,
            field_meta: None,
            tags: None,
            lifecycle_state: None,
            extra: BTreeMap::new(),
        },
        None,
        None,
    )
    .unwrap()
    .record;
    let doc = add_attachment(
        store,
        AddAttachmentInput {
            file_name: "picture.svg".to_string(),
            content: br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.to_vec(),
            subdir: None,
            title: None,
            content_type: Some("image/svg+xml".to_string()),
            enforce_policy: false,
        },
    )
    .unwrap();
    link_attachment(
        store,
        LinkAttachmentInput {
            instance_id: record.instance_id,
            document_id: doc.document_id,
        },
    )
    .unwrap();
    store
        .save_binary_file(
            &format!("{}/{}", doc.source_documents_path, doc.content_path),
            HOSTILE,
        )
        .unwrap();
}

fn svg_unsafe_warnings(store: &dyn RepositoryStore) -> usize {
    validate_repository(store)
        .unwrap()
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("svg-unsafe"))
        .count()
}

#[test]
fn a_clean_repository_has_no_svg_warning() {
    let (_d, store) = origin();
    assert_eq!(svg_unsafe_warnings(&store), 0);
}

#[test]
fn archive_import_keeps_and_reports_it() {
    let (_d, store) = origin();
    plant_hostile_svg(&store);
    let dir = TempDir::new().unwrap();
    let target = FileStore::new(dir.path());
    archive_unpack(Cursor::new(archive_to_vec(&store).unwrap()), &target).unwrap();
    assert_eq!(svg_unsafe_warnings(&target), 1);
}

#[test]
fn snapshot_import_keeps_and_reports_it() {
    let (_d, store) = origin();
    plant_hostile_svg(&store);
    let snapshot = export_repository_snapshot_with_options(
        &store,
        ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )
    .unwrap();
    let dir = TempDir::new().unwrap();
    let target = FileStore::new(dir.path());
    import_repository_snapshot(&target, &snapshot).unwrap();
    assert_eq!(svg_unsafe_warnings(&target), 1);
}

#[test]
fn merge_keeps_and_reports_it() {
    let (_d, store) = origin();
    let target = archive_to_tree(Cursor::new(archive_to_vec(&store).unwrap())).unwrap();
    plant_hostile_svg(&store);
    let archive = archive_to_vec(&store).unwrap();
    let preview = merge_preview(&target, Cursor::new(archive.clone()), None).unwrap();
    let selection = MergeSelection {
        instance_ids: preview
            .bring
            .instances
            .iter()
            .map(|i| i.instance_id.clone())
            .collect(),
        ..MergeSelection::default()
    };
    merge_apply(&target, Cursor::new(archive), &selection).unwrap();
    assert_eq!(svg_unsafe_warnings(&target), 1);
}
