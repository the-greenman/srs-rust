//! `.srspkg` reader + bytes install (srs-rust#690, ADR-050). Bundles are built
//! inline, modeled on the `install-package` fixture; no spec content.

use serde_json::{json, Value};
use srs_repository::error::RepositoryError;
use srs_repository::package_bundle::read_package_bundle;
use srs_repository::package_install_service::{
    install_package_bundle_bytes, InstallBundleOptions, InstallPackageResult,
};
use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::memory::MemoryStore;
use srs_repository::store::{FileStore, RepositoryStore};

const NS: &str = "com.example.bundle";

fn field() -> Value {
    json!({"id": "9a1b0c2d-0001-4aaa-8bbb-00000000f001", "namespace": NS, "name": "title",
        "version": 1, "description": "Short label.", "fieldType": {"datatype": "string"},
        "aiGuidance": {"purpose": "Short label."}, "createdAt": "2026-01-01T00:00:00Z"})
}

fn ty() -> Value {
    json!({"id": "9a1b0c30-0003-4aaa-8bbb-000000004001", "namespace": NS, "name": "entry",
        "version": 1, "description": "An entry.", "createdAt": "2026-01-01T00:00:00Z",
        "fields": [{"fieldId": "9a1b0c2d-0001-4aaa-8bbb-00000000f001", "order": 0, "required": true}]})
}

fn relation_type() -> Value {
    json!({"id": "9a1b0c40-0004-4aaa-8bbb-00000000e001", "namespace": NS, "key": "follows",
        "version": 1, "label": "Follows", "description": "Source follows target.",
        "category": "sequence", "createdAt": "2026-01-01T00:00:00Z"})
}

fn view() -> Value {
    json!({"id": "9a1b0c60-0006-4aaa-8bbb-000000007001", "namespace": NS, "name": "entry-view",
        "version": 1, "description": "Entry view.", "createdAt": "2026-01-01T00:00:00Z",
        "fieldViews": [{"fieldId": "9a1b0c2d-0001-4aaa-8bbb-00000000f001", "order": 0,
            "required": true, "visible": true}]})
}

fn bundle(rev: u64) -> Value {
    json!({
        "schemaVersion": "2.0-draft",
        "packageId": "9a1b0c2d-1111-4aaa-8bbb-000000000001",
        "packageNamespace": NS, "packageName": "bundle-fixture", "packageVersion": "1.0.0",
        "dataModelRevision": rev, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
        "fields": [field()], "types": [ty()], "relationTypes": [relation_type()],
        "views": [view()], "dependencyRefs": [], "packageDependencies": []
    })
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap()
}

fn code_of(err: RepositoryError) -> (&'static str, String) {
    match err {
        RepositoryError::InvalidPackageBundle { code, message } => (code, message),
        other => panic!("expected InvalidPackageBundle, got {other:?}"),
    }
}

fn refused(v: &Value) -> (&'static str, String) {
    code_of(read_package_bundle(&bytes(v)).unwrap_err())
}

fn file_repo() -> (tempfile::TempDir, FileStore) {
    let temp = tempfile::TempDir::new().unwrap();
    let store = FileStore::new(temp.path());
    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "17575e57-0000-4000-8000-175753e57001".to_string(),
                namespace: "com.test.bundle".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Bundle Test".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "bundle-test-primary".to_string(),
                namespace: "com.test.bundle".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .unwrap();
    (temp, store)
}

/// A composition whose section is not container-subset but carries memberOrder:
/// the RFC-043 transformer strips it and reports `migration-memberorder-dropped`.
fn composition_with_member_order(scope: Option<&str>) -> Value {
    let source = match scope {
        Some(s) => json!({"type": "container-subset",
            "containerId": "11111111-0000-4000-8000-0000000000c1", "containerScope": s}),
        None => json!({"type": "fixed-instances",
            "instanceIds": ["22222222-0000-4000-8000-000000000011"]}),
    };
    json!({"id": "9a1b0c70-0007-4aaa-8bbb-000000008001", "namespace": NS, "name": "entry-log",
        "version": 1, "description": "d", "createdAt": "2026-01-01T00:00:00Z",
        "sections": [{"sectionId": "s", "order": 0, "source": source,
            "ordering": {"memberOrder": ["22222222-0000-4000-8000-000000000011"]}}]})
}

#[test]
fn read_bundle_rejects_non_json() {
    let (code, _) = code_of(read_package_bundle(b"not json {").unwrap_err());
    assert_eq!(code, "bundle-not-json");
    assert_eq!(refused(&json!([1, 2])).0, "bundle-not-json");
}

#[test]
fn read_bundle_refuses_readme_until_1164() {
    let mut b = bundle(9);
    b["readme"] = json!({"path": "README.md", "content": "hi"});
    let (code, msg) = refused(&b);
    assert_eq!(code, "bundle-readme-unsupported");
    assert!(msg.contains("#1164"), "{msg}");
}

#[test]
fn read_bundle_refuses_newer_revision() {
    let (code, msg) = refused(&bundle(10));
    assert_eq!(code, "bundle-revision-too-new");
    assert!(msg.contains("10") && msg.contains('9'), "{msg}");
}

#[test]
fn read_bundle_rejects_schema_invalid() {
    let mut b = bundle(9);
    b.as_object_mut().unwrap().remove("packageId");
    assert_eq!(refused(&b).0, "bundle-schema-invalid");
}

#[test]
fn read_bundle_schema_error_on_old_bundle_names_revision() {
    let mut b = bundle(5);
    b["documentViews"] = json!([]);
    let (code, msg) = refused(&b);
    assert_eq!(code, "bundle-schema-invalid");
    assert!(
        msg.contains("re-export") && msg.contains("dataModelRevision 5"),
        "{msg}"
    );
}

#[test]
fn read_bundle_rejects_invalid_definition() {
    let mut b = bundle(9);
    b["fields"][0]["descriptionn"] = json!("typo");
    let (code, msg) = refused(&b);
    assert_eq!(code, "bundle-definition-invalid");
    assert!(msg.contains("descriptionn"), "{msg}");
}

#[test]
fn read_bundle_rejects_definition_without_id() {
    let mut b = bundle(9);
    b["views"][0].as_object_mut().unwrap().remove("id");
    assert_eq!(refused(&b).0, "bundle-definition-invalid");
}

#[test]
fn read_bundle_rejects_definition_without_name() {
    let mut b = bundle(9);
    b["views"][0].as_object_mut().unwrap().remove("name");
    assert_eq!(refused(&b).0, "bundle-definition-invalid");
}

#[test]
fn read_bundle_migrates_pre_rev8_member_order() {
    let mut b = bundle(7);
    b["compositions"] = json!([composition_with_member_order(None)]);
    let read = read_package_bundle(&bytes(&b)).unwrap();
    assert!(
        read.notes
            .iter()
            .any(|n| n.starts_with("migration-memberorder-dropped")),
        "{:?}",
        read.notes
    );
    let comp = read
        .bundle
        .definitions
        .iter()
        .find(|d| d.rel_path.starts_with("compositions/"))
        .unwrap();
    assert!(comp.value["sections"][0]["ordering"]
        .get("memberOrder")
        .is_none());

    let (_t, store) = file_repo();
    let result = install_package_bundle_bytes(&store, &bytes(&b), Default::default()).unwrap();
    assert!(result
        .notes
        .iter()
        .any(|n| n.starts_with("migration-memberorder-dropped")));
    assert_eq!(result.installed, 5);
}

#[test]
fn read_bundle_current_revision_passes_through_unchanged() {
    let b = bundle(9);
    let read = read_package_bundle(&bytes(&b)).unwrap();
    assert!(read.notes.is_empty());
    let values: Vec<&Value> = read.bundle.definitions.iter().map(|d| &d.value).collect();
    assert_eq!(values, vec![&field(), &ty(), &relation_type(), &view()]);
    assert_eq!(read.bundle.id, "9a1b0c2d-1111-4aaa-8bbb-000000000001");
    assert_eq!(read.bundle.name, "bundle-fixture");
    assert!(read.bundle.package_dependencies.is_none());
}

#[test]
fn read_bundle_member_order_conflict_refuses() {
    let mut b = bundle(7);
    b["compositions"] = json!([composition_with_member_order(Some("subtree"))]);
    let (_t, store) = file_repo();
    let before = store.list_package_boundaries().unwrap().len();
    let err = install_package_bundle_bytes(&store, &bytes(&b), Default::default()).unwrap_err();
    assert!(
        err.to_string().contains("migration-memberorder-conflict"),
        "{err}"
    );
    assert_eq!(store.list_package_boundaries().unwrap().len(), before);
}

#[test]
fn read_bundle_derives_rel_paths_by_kind() {
    let read = read_package_bundle(&bytes(&bundle(9))).unwrap();
    let paths: Vec<&str> = read
        .bundle
        .definitions
        .iter()
        .map(|d| d.rel_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "fields/title-9a1b0c2d.json",
            "types/entry-9a1b0c30.json",
            "relation-types/follows-9a1b0c40.json",
            "views/entry-view-9a1b0c60.json",
        ]
    );
}

#[test]
fn install_options_json_maps_to_install_bundle_options() {
    let d: InstallBundleOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(d.boundary_path, None);
    assert!(!d.strict);
    let s: InstallBundleOptions =
        serde_json::from_str(r#"{"boundaryPath":"packages/x","strict":true}"#).unwrap();
    assert_eq!(s.boundary_path.as_deref(), Some("packages/x"));
    assert!(s.strict);
}

fn comparable(r: &InstallPackageResult) -> Value {
    let mut v = serde_json::to_value(r).unwrap();
    // installedAt is a wall-clock stamp taken at install time: differs by construction.
    v.as_object_mut().unwrap().remove("installedAt");
    v
}

#[test]
fn install_bundle_bytes_memory_and_file_store_agree() {
    let b = bytes(&bundle(9));
    let mem = MemoryStore::default();
    let from_mem = install_package_bundle_bytes(&mem, &b, Default::default()).unwrap();
    let (_t, disk) = file_repo();
    let from_disk = install_package_bundle_bytes(&disk, &b, Default::default()).unwrap();
    assert_eq!(comparable(&from_mem), comparable(&from_disk));
    assert_eq!(from_disk.installed, 4);
}

#[test]
fn install_bundle_bytes_reinstall_is_idempotent() {
    let b = bytes(&bundle(9));
    let (_t, store) = file_repo();
    let first = install_package_bundle_bytes(&store, &b, Default::default()).unwrap();
    let second = install_package_bundle_bytes(&store, &b, Default::default()).unwrap();
    assert_eq!(second.installed, 0);
    assert_eq!(second.skipped_identical, first.installed);
}
