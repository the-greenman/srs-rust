//! #1267: a definition's identity is (kind, uuid, version): a bundle carrying one
//! type id at v1 and v2 installs both, each in its own file.

use serde_json::{json, Value};
use srs_core::extensions::import_tracking::ConflictState;
use srs_repository::package_install_service::{
    install_package_bundle_bytes, InstallBundleOptions, InstallPackageResult,
};
use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};
use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;

const NS: &str = "com.example.multiver";
const FIELD: &str = "9a1b0c2d-0001-4aaa-8bbb-00000000f001";
const TYPE: &str = "9a1b0c30-0003-4aaa-8bbb-000000004001";

fn ty(version: u64) -> Value {
    json!({"id": TYPE, "namespace": NS, "name": "state", "version": version,
        "description": "A state.", "createdAt": "2026-01-01T00:00:00Z",
        "fields": [{"fieldId": FIELD, "order": 0, "required": true}]})
}

fn bundle_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": "2.0-draft",
        "packageId": "9a1b0c2d-1111-4aaa-8bbb-000000000001",
        "packageNamespace": NS, "packageName": "multiver", "packageVersion": "1.0.0",
        "dataModelRevision": 9, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
        "fields": [{"id": FIELD, "namespace": NS, "name": "title", "version": 1,
            "description": "Label.", "fieldType": {"datatype": "string"},
            "aiGuidance": {"purpose": "Label."}, "createdAt": "2026-01-01T00:00:00Z"}],
        "types": [ty(1), ty(2)], "relationTypes": [], "views": [],
        "dependencyRefs": [], "packageDependencies": []
    }))
    .unwrap()
}

fn repo() -> (tempfile::TempDir, FileStore) {
    let temp = tempfile::TempDir::new().unwrap();
    let store = FileStore::new(temp.path());
    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "17575e57-0000-4000-8000-175753e57002".to_string(),
                namespace: "com.test.multiver".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Multiver".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "multiver-primary".to_string(),
                namespace: "com.test.multiver".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .unwrap();
    (temp, store)
}

fn install(store: &FileStore) -> InstallPackageResult {
    install_package_bundle_bytes(store, &bundle_bytes(), InstallBundleOptions::default()).unwrap()
}

#[test]
fn both_versions_install_and_resolve() {
    let (_t, store) = repo();
    let r = install(&store);
    assert_eq!((r.installed, r.skipped_identical), (3, 0));

    let pkg = store.load_package().unwrap();
    assert!(pkg.resolve_type(TYPE, 1).is_some());
    assert!(pkg.resolve_type(TYPE, 2).is_some());

    // A record bound to v1 validates cleanly; each version kept its own file.
    srs_repository::record_store::create_record(
        &store,
        TYPE,
        1,
        serde_json::from_value(json!({"title": "x"})).unwrap(),
        None,
        None,
    )
    .unwrap();
    let report = validate_repository(&store).unwrap();
    assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
    let boundary = store.list_package_boundaries().unwrap().pop().unwrap();
    assert_eq!(boundary.type_paths.len(), 2, "{:?}", boundary.type_paths);
}

#[test]
fn reinstall_is_a_full_noop() {
    let (_t, store) = repo();
    install(&store);
    let before = list_package_imports(&store, ListPackageImportsFilter::default()).unwrap();
    let r = install(&store);
    assert_eq!((r.installed, r.skipped_identical), (0, 3));
    let after = list_package_imports(&store, ListPackageImportsFilter::default()).unwrap();
    assert_eq!(before.types.len(), after.types.len());
}

#[test]
fn imports_report_both_versions_clean() {
    let (_t, store) = repo();
    install(&store);
    let s = list_package_imports(&store, ListPackageImportsFilter::default()).unwrap();
    let mut versions: Vec<u32> = s.types.iter().map(|t| t.version).collect();
    versions.sort();
    assert_eq!(versions, vec![1, 2]);
    assert!(s
        .types
        .iter()
        .all(|t| t.conflict_state == Some(ConflictState::Clean)));
}
