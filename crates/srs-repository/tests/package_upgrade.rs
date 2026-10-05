//! #1152: `upgrade_package_bundle` (RFC-014 R2/R3/R6; owner rulings 2026-10-05, srs#890).
//! The shape under test is the essay 1.3.0 -> 1.5.0 upgrade.

use serde_json::{json, Value};
use srs_core::extensions::import_tracking::ConflictState;
use srs_repository::package_install_service::{
    install_package_bundle_bytes, upgrade_package_bundle, UpgradeOptions, UpgradePackageResult,
};
use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};
use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::collections::BTreeMap;
use std::path::Path;

const NS: &str = "com.example.upg";
const PKG: &str = "9a1b0c2d-2222-4aaa-8bbb-000000000001";
const F: &str = "9a1b0c2d-0001-4aaa-8bbb-00000000f001";
const G: &str = "9a1b0c2d-0001-4aaa-8bbb-00000000f002";
const A: &str = "9a1b0c30-0003-4aaa-8bbb-000000004001";

fn field(id: &str, name: &str) -> Value {
    json!({"id": id, "namespace": NS, "name": name, "version": 1,
        "description": "Label.", "fieldType": {"datatype": "string"},
        "aiGuidance": {"purpose": "Label."}, "createdAt": "2026-01-01T00:00:00Z"})
}

fn ty(version: u64, description: &str, fields: &[&str]) -> Value {
    let fa: Vec<Value> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| json!({"fieldId": f, "order": i, "required": false}))
        .collect();
    json!({"id": A, "namespace": NS, "name": "essay", "version": version,
        "description": description, "createdAt": "2026-01-01T00:00:00Z", "fields": fa})
}

fn bundle(version: &str, fields: Vec<Value>, types: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": "2.0-draft", "packageId": PKG,
        "packageNamespace": NS, "packageName": "upg", "packageVersion": version,
        "dataModelRevision": 9, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
        "fields": fields, "types": types, "relationTypes": [], "views": [],
        "dependencyRefs": [], "packageDependencies": []
    }))
    .unwrap()
}

fn v_old() -> Vec<u8> {
    bundle("1.3.0", vec![field(F, "title")], vec![ty(1, "Old.", &[F])])
}

fn v_new() -> Vec<u8> {
    bundle(
        "1.5.0",
        vec![field(F, "title"), field(G, "extra")],
        vec![ty(1, "Changed in place.", &[F]), ty(2, "New.", &[F, G])],
    )
}

fn repo() -> (tempfile::TempDir, FileStore) {
    let temp = tempfile::TempDir::new().unwrap();
    let store = FileStore::new(temp.path());
    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "17575e57-0000-4000-8000-175753e57003".to_string(),
                namespace: "com.test.upg".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Upg".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "upg-primary".to_string(),
                namespace: "com.test.upg".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .unwrap();
    (temp, store)
}

fn installed_old() -> (tempfile::TempDir, FileStore) {
    let (t, s) = repo();
    install_package_bundle_bytes(&s, &v_old(), Default::default()).unwrap();
    (t, s)
}

fn upgrade(store: &FileStore, bytes: &[u8], dry_run: bool) -> UpgradePackageResult {
    upgrade_package_bundle(
        store,
        bytes,
        UpgradeOptions {
            dry_run,
            boundary_path: None,
        },
    )
    .unwrap()
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn names(items: &[srs_repository::package_install_service::UpgradeItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| format!("{}:{}@{}", i.kind, i.name, i.version))
        .collect()
}

fn assert_valid(store: &FileStore) {
    let r = validate_repository(store).unwrap();
    assert_eq!(r.summary.errors, 0, "{:?}", r.diagnostics);
}

fn type_v1_path(store: &FileStore) -> String {
    let b = store.list_package_boundaries().unwrap().pop().unwrap();
    let sel = b.selector.clone().unwrap();
    let p = b
        .type_paths
        .iter()
        .find(|p| !p.contains("-v2"))
        .unwrap()
        .clone();
    format!("{sel}/{p}")
}

#[test]
fn essay_shape_upgrade() {
    let (t, store) = installed_old();
    srs_repository::record_store::create_record(
        &store,
        A,
        1,
        serde_json::from_value(json!({"title": "x"})).unwrap(),
        None,
        None,
    )
    .unwrap();
    let records_before = snapshot(&t.path().join("records"));
    assert!(!records_before.is_empty());

    let r = upgrade(&store, &v_new(), false);
    assert_eq!(names(&r.added), vec!["field:extra@1"]);
    assert_eq!(names(&r.new_versions), vec!["type:essay@2"]);
    assert_eq!(names(&r.updated), vec!["type:essay@1"]);
    assert_eq!(names(&r.unchanged), vec!["field:title@1"]);
    assert!(r.conflicts.is_empty() && r.removed_upstream.is_empty());
    assert_eq!(
        (r.previous_version.as_str(), r.version.as_str()),
        ("1.3.0", "1.5.0")
    );
    assert!(r.upgraded && !r.dry_run);

    let b = store.list_package_boundaries().unwrap().pop().unwrap();
    assert_eq!(b.version, "1.5.0");
    assert_eq!(b.type_paths.len(), 2);
    assert_eq!(snapshot(&t.path().join("records")), records_before);
    assert_valid(&store);

    // Import records: replaced not duplicated, all clean, new source version.
    let s = list_package_imports(&store, ListPackageImportsFilter::default()).unwrap();
    assert_eq!(s.types.len(), 2);
    assert_eq!(s.fields.len(), 2);
    assert!(s
        .types
        .iter()
        .chain(&s.fields)
        .all(|r| r.conflict_state == Some(ConflictState::Clean)));
    let v1 = s.types.iter().find(|r| r.version == 1).unwrap();
    assert_eq!(v1.source_package_version, "1.5.0");

    // Re-run: everything unchanged, nothing written.
    let before = snapshot(t.path());
    let r2 = upgrade(&store, &v_new(), false);
    assert!(r2.added.is_empty() && r2.new_versions.is_empty() && r2.updated.is_empty());
    assert_eq!(r2.unchanged.len(), 4);
    assert!(!r2.upgraded);
    assert_eq!(snapshot(t.path()), before);
}

#[test]
fn local_edit_is_a_conflict_and_kept() {
    let (t, store) = installed_old();
    let rel = type_v1_path(&store);
    let file = t.path().join(&rel);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    v["description"] = json!("Edited locally.");
    std::fs::write(&file, serde_json::to_vec_pretty(&v).unwrap()).unwrap();

    let r = upgrade(&store, &v_new(), false);
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].conflict_kind, "local-edit");
    assert_eq!(r.conflicts[0].item.name, "essay");
    assert!(r.updated.is_empty());
    let kept: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(kept["description"], "Edited locally.");
    // The rest of the upgrade still applied.
    assert_eq!(names(&r.added), vec!["field:extra@1"]);
    assert_eq!(
        store
            .list_package_boundaries()
            .unwrap()
            .pop()
            .unwrap()
            .version,
        "1.5.0"
    );
}

#[test]
fn downgrade_is_refused() {
    let (t, store) = installed_old();
    upgrade(&store, &v_new(), false);
    let before = snapshot(t.path());
    let err = upgrade_package_bundle(&store, &v_old(), UpgradeOptions::default()).unwrap_err();
    assert!(err.to_string().contains("downgrade refused"), "{err}");
    assert_eq!(snapshot(t.path()), before);
}

#[test]
fn same_version_is_a_content_sync() {
    let (_t, store) = installed_old();
    let changed = bundle(
        "1.3.0",
        vec![field(F, "title")],
        vec![ty(1, "Synced.", &[F])],
    );
    let r = upgrade(&store, &changed, false);
    assert_eq!(names(&r.updated), vec!["type:essay@1"]);
    assert!(!r.upgraded);
    assert_eq!(
        store
            .list_package_boundaries()
            .unwrap()
            .pop()
            .unwrap()
            .version,
        "1.3.0"
    );
    assert_valid(&store);
}

#[test]
fn removed_upstream_is_reported_and_kept() {
    let (t, store) = installed_old();
    let dropped = bundle("1.4.0", vec![field(F, "title")], vec![]);
    let r = upgrade(&store, &dropped, false);
    assert_eq!(names(&r.removed_upstream), vec!["type:essay@1"]);
    assert!(t.path().join(type_v1_path(&store)).exists());
    assert_eq!(
        store
            .list_package_boundaries()
            .unwrap()
            .pop()
            .unwrap()
            .type_paths
            .len(),
        1
    );
}

#[test]
fn dry_run_writes_nothing_and_returns_the_same_plan() {
    let (t, store) = installed_old();
    let before = snapshot(t.path());
    let dry = upgrade(&store, &v_new(), true);
    assert_eq!(snapshot(t.path()), before);
    assert!(dry.dry_run && dry.upgraded);
    let real = upgrade(&store, &v_new(), false);
    for (a, b) in [
        (&dry.added, &real.added),
        (&dry.new_versions, &real.new_versions),
        (&dry.updated, &real.updated),
        (&dry.unchanged, &real.unchanged),
    ] {
        assert_eq!(names(a), names(b));
    }
}

#[test]
fn not_installed_is_an_error() {
    let (_t, store) = repo();
    let err = upgrade_package_bundle(&store, &v_new(), UpgradeOptions::default()).unwrap_err();
    assert!(
        err.to_string().contains("not installed; use install"),
        "{err}"
    );
}
