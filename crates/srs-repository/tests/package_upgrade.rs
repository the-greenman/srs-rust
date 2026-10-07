//! #1152: `upgrade_package_bundle` (RFC-014 R2/R3/R6; owner rulings 2026-10-05, srs#890).
//! The shape under test is the essay 1.3.0 -> 1.5.0 upgrade.

use serde_json::{json, Value};
use srs_core::extensions::import_tracking::ConflictState;
use srs_repository::package_install_service::{
    install_package_bundle, install_package_bundle_bytes, upgrade_package_bundle,
    upgrade_package_source, PackageSourceBundle, PackageSourceDefinition, UpgradeOptions,
    UpgradePackageResult,
};
use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};
use srs_repository::package_types::DefinitionKind;
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
    bundle_with(version, fields, types, json!({}))
}

/// `extra` keys are merged over the bundle (e.g. `lifecycles`, `packageDependencies`).
fn bundle_with(version: &str, fields: Vec<Value>, types: Vec<Value>, extra: Value) -> Vec<u8> {
    let mut b = json!({
        "schemaVersion": "2.0-draft", "packageId": PKG,
        "packageNamespace": NS, "packageName": "upg", "packageVersion": version,
        "dataModelRevision": 9, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
        "fields": fields, "types": types, "relationTypes": [], "views": [],
        "dependencyRefs": [], "packageDependencies": []
    });
    b.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::to_vec(&b).unwrap()
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
            ..Default::default()
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

fn lifecycle(description: &str) -> Value {
    json!({"id": "9a1b0c50-0005-4aaa-8bbb-000000001001", "version": 1, "namespace": NS,
        "name": "simple_lifecycle", "description": description,
        "states": [
            {"id": "9a1b0c51-0005-4aaa-8bbb-000000005001", "key": "draft", "label": "Draft", "isInitial": true},
            {"id": "9a1b0c52-0005-4aaa-8bbb-000000005002", "key": "final", "label": "Final", "isFinal": true}],
        "transitions": [{"id": "9a1b0c53-0005-4aaa-8bbb-000000006001", "name": "finalize", "from": "draft", "to": "final"}],
        "initialState": "draft", "createdAt": "2026-01-01T00:00:00Z"})
}

fn source(version: &str, defs: Vec<(DefinitionKind, &str, Value)>) -> PackageSourceBundle {
    PackageSourceBundle {
        id: PKG.to_string(),
        namespace: NS.to_string(),
        name: "upg".to_string(),
        version: version.to_string(),
        package_dependencies: None,
        definitions: defs
            .into_iter()
            .map(|(kind, rel, value)| PackageSourceDefinition {
                kind,
                rel_path: rel.to_string(),
                value,
            })
            .collect(),
    }
}

fn boundary_of(store: &FileStore) -> srs_repository::package_types::PackageBoundary {
    store.list_package_boundaries().unwrap().pop().unwrap()
}

#[test]
fn install_never_overwrites_a_different_version_at_the_same_path() {
    let (t, store) = repo();
    let b = source(
        "1.0.0",
        vec![
            (
                DefinitionKind::Field,
                "fields/title.json",
                field(F, "title"),
            ),
            (
                DefinitionKind::Type,
                "types/essay.json",
                ty(1, "One.", &[F]),
            ),
            (
                DefinitionKind::Type,
                "types/essay.json",
                ty(2, "Two.", &[F]),
            ),
        ],
    );
    let r = install_package_bundle(&store, &b, Default::default()).unwrap();
    assert_eq!(r.installed, 3);
    let bd = boundary_of(&store);
    assert_eq!(bd.type_paths.len(), 2, "{:?}", bd.type_paths);
    let versions: Vec<u64> = bd
        .type_paths
        .iter()
        .map(|p| {
            let v: Value = serde_json::from_slice(
                &std::fs::read(t.path().join("packages/upg").join(p)).unwrap(),
            )
            .unwrap();
            v["version"].as_u64().unwrap()
        })
        .collect();
    assert!(
        versions.contains(&1) && versions.contains(&2),
        "{versions:?}"
    );
    let pkg = store.load_package().unwrap();
    assert!(pkg.resolve_type(A, 1).is_some() && pkg.resolve_type(A, 2).is_some());
    assert_valid(&store);
}

#[test]
fn upgrade_never_overwrites_a_different_version_at_the_same_path() {
    let (_t, store) = installed_old();
    let v1_rel = {
        let bd = boundary_of(&store);
        bd.type_paths[0].clone()
    };
    let b = source(
        "1.4.0",
        vec![
            (DefinitionKind::Field, "fields/x.json", field(F, "title")),
            (DefinitionKind::Type, &v1_rel, ty(1, "Old.", &[F])),
            (DefinitionKind::Type, &v1_rel, ty(2, "Two.", &[F])),
        ],
    );
    let r = upgrade_package_source(&store, &b, UpgradeOptions::default()).unwrap();
    assert_eq!(names(&r.new_versions), vec!["type:essay@2"]);
    assert_eq!(boundary_of(&store).type_paths.len(), 2);
    let pkg = store.load_package().unwrap();
    assert_eq!(pkg.resolve_type(A, 1).unwrap().description, "Old.");
    assert_eq!(pkg.resolve_type(A, 2).unwrap().description, "Two.");
    assert_valid(&store);
}

#[test]
fn local_edit_of_a_definition_upstream_did_not_change_is_unchanged() {
    let (t, store) = installed_old();
    let file = t.path().join(type_v1_path(&store));
    let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    v["description"] = json!("Edited locally.");
    std::fs::write(&file, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let r = upgrade(&store, &v_old(), false);
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(names(&r.unchanged).len(), 2);
    let kept: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(kept["description"], "Edited locally.");
}

#[test]
fn missing_reference_copy_is_a_no_reference_copy_conflict() {
    let (t, store) = installed_old();
    let rel = type_v1_path(&store).replacen("packages/upg/", "", 1);
    std::fs::remove_file(t.path().join("packages/upg/.srs-import/refs").join(&rel)).unwrap();
    let changed = bundle(
        "1.3.0",
        vec![field(F, "title")],
        vec![ty(1, "Synced.", &[F])],
    );
    let r = upgrade(&store, &changed, false);
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].conflict_kind, "no-reference-copy");
    assert!(r.updated.is_empty());
}

#[test]
fn same_key_different_uuid_is_a_key_collision() {
    let (_t, store) = installed_old();
    let other = "9a1b0c2d-0001-4aaa-8bbb-00000000f0ff";
    let b = bundle(
        "1.4.0",
        vec![field(F, "title"), field(other, "title")],
        vec![ty(1, "Old.", &[F])],
    );
    let r = upgrade(&store, &b, false);
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].conflict_kind, "key-collision");
    assert_eq!(r.conflicts[0].item.id, other);
    assert!(r.added.is_empty());
}

#[test]
fn semver_prerelease_and_build_metadata() {
    let (_t, store) = installed_old(); // 1.3.0
    let with = |v: &str| bundle(v, vec![field(F, "title")], vec![ty(1, "Old.", &[F])]);
    // A pre-release of the installed release is lower.
    let up = |v: &str| upgrade_package_bundle(&store, &with(v), UpgradeOptions::default());
    assert!(up("1.3.0-rc.1")
        .unwrap_err()
        .to_string()
        .contains("downgrade refused"));
    // Build metadata never takes part in comparison: equal, a content sync, no bump.
    let r = up("1.3.0+build.7").unwrap();
    assert!(!r.upgraded);
    assert_eq!(boundary_of(&store).version, "1.3.0");
    // A pre-release above the installed version is an upgrade.
    let r = up("1.4.0-rc.1").unwrap();
    assert!(r.upgraded);
    assert_eq!(boundary_of(&store).version, "1.4.0-rc.1");
    // The release is above its own pre-release.
    assert!(up("1.4.0").unwrap().upgraded);
    // Not SemVer: refused.
    assert!(up("1.4").is_err());
}

#[test]
fn unsatisfied_requirements_are_warnings_not_errors() {
    let (_t, store) = installed_old();
    let missing = "e0000007-0000-4000-a000-000000000007";
    let b = bundle_with(
        "1.4.0",
        vec![field(F, "title")],
        vec![ty(1, "Old.", &[F])],
        json!({"packageDependencies": [
            {"packageId": missing, "namespace": "com.example.other", "name": "other", "version": "1.0.0"}]}),
    );
    let r = upgrade(&store, &b, false);
    assert!(r.upgraded);
    assert_eq!(r.dependency_warnings.len(), 1);
    let j = serde_json::to_value(&r.dependency_warnings[0]).unwrap();
    assert_eq!(j["reason"], "missing");
    for k in [
        "packageId",
        "namespace",
        "name",
        "version",
        "satisfied",
        "candidateVersions",
        "mismatchedLabels",
    ] {
        assert!(j.get(k).is_some(), "{k} in {j}");
    }
    assert_eq!(
        r.dependency_warnings[0].package_id.as_deref(),
        Some(missing)
    );
    assert!(!r.dependency_warnings[0].satisfied);
}

#[test]
fn a_kind_with_no_import_record_is_updated_and_reference_copied() {
    let (t, store) = repo();
    let with = |d: &str| {
        bundle_with(
            "1.0.0",
            vec![field(F, "title")],
            vec![],
            json!({"lifecycles": [lifecycle(d)]}),
        )
    };
    install_package_bundle_bytes(&store, &with("First."), Default::default()).unwrap();
    // Install writes a reference copy for every kind.
    let refs = t.path().join("packages/upg/.srs-import/refs");
    assert!(std::fs::read_dir(refs.join("lifecycles")).unwrap().count() == 1);
    let r = upgrade(&store, &with("Second."), false);
    assert_eq!(r.updated.len(), 1);
    assert_eq!(r.updated[0].kind, "lifecycle");
    let again = upgrade(&store, &with("Second."), false);
    assert!(again.updated.is_empty() && again.repaired.is_empty());
    assert_valid(&store);
}

#[test]
fn rerun_after_a_partial_failure_converges() {
    let (t, store) = installed_old();
    upgrade(&store, &v_new(), false);
    // Simulate a mid-loop failure: a reference copy and an import record are gone.
    let rel = type_v1_path(&store).replacen("packages/upg/", "", 1);
    std::fs::remove_file(t.path().join("packages/upg/.srs-import/refs").join(&rel)).unwrap();
    let sp = t
        .path()
        .join("packages/upg/.srs-import/import-records.json");
    let mut sum: Value = serde_json::from_slice(&std::fs::read(&sp).unwrap()).unwrap();
    sum["fields"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["name"] != "extra");
    std::fs::write(&sp, serde_json::to_vec(&sum).unwrap()).unwrap();

    let r = upgrade(&store, &v_new(), false);
    assert_eq!(names(&r.repaired), vec!["field:extra@1", "type:essay@1"]);
    let s = list_package_imports(&store, ListPackageImportsFilter::default()).unwrap();
    assert_eq!((s.fields.len(), s.types.len()), (2, 2));
    assert!(s
        .types
        .iter()
        .chain(&s.fields)
        .all(|r| r.conflict_state == Some(ConflictState::Clean)));
    let before = snapshot(t.path());
    let r = upgrade(&store, &v_new(), false);
    assert!(r.repaired.is_empty() && r.updated.is_empty());
    assert_eq!(snapshot(t.path()), before);
}

#[test]
fn dry_run_fails_on_a_corrupt_import_summary_like_a_real_run() {
    let (t, store) = installed_old();
    std::fs::write(
        t.path()
            .join("packages/upg/.srs-import/import-records.json"),
        "{\"fields\": 3}",
    )
    .unwrap();
    for dry_run in [true, false] {
        let opts = UpgradeOptions {
            dry_run,
            boundary_path: None,
            ..Default::default()
        };
        assert!(upgrade_package_bundle(&store, &v_new(), opts).is_err());
    }
}

#[test]
fn boundary_option_selects_and_ambiguity_is_refused() {
    let (t, store) = installed_old();
    let at = |p: &str| UpgradeOptions {
        dry_run: true,
        boundary_path: Some(p.to_string()),
        ..Default::default()
    };
    assert!(upgrade_package_bundle(&store, &v_new(), at("packages/upg")).is_ok());
    let err = upgrade_package_bundle(&store, &v_new(), at("packages/elsewhere")).unwrap_err();
    assert!(
        err.to_string()
            .contains("not installed at 'packages/elsewhere'"),
        "{err}"
    );

    // The same packageId at a second boundary (create_package refuses a duplicate id, so
    // retarget a fresh boundary's id by hand): no default is guessed.
    srs_repository::package_service::create_package(
        &store,
        srs_repository::package_service::CreatePackageInput {
            id: "other-id".to_string(),
            namespace: NS.to_string(),
            name: "upg".to_string(),
            version: "1.3.0".to_string(),
            boundary_path: Some("packages/upg2".to_string()),
        },
    )
    .unwrap();
    let pj = t.path().join("packages/upg2/package.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&pj).unwrap()).unwrap();
    v["id"] = json!(PKG);
    std::fs::write(&pj, serde_json::to_vec(&v).unwrap()).unwrap();
    let err = upgrade_package_bundle(&store, &v_new(), UpgradeOptions::default()).unwrap_err();
    let m = err.to_string();
    assert!(
        m.contains("packages/upg") && m.contains("packages/upg2"),
        "{m}"
    );
    assert!(upgrade_package_bundle(&store, &v_new(), at("packages/upg")).is_ok());
}

#[test]
fn repair_keeps_local_edit_provenance_in_the_import_record() {
    // A definition whose reference copy is gone and whose record has local provenance:
    // repaired, provenance kept.
    let (t, store) = installed_old();
    let rel = type_v1_path(&store).replacen("packages/upg/", "", 1);
    let sp = t
        .path()
        .join("packages/upg/.srs-import/import-records.json");
    let mut sum: Value = serde_json::from_slice(&std::fs::read(&sp).unwrap()).unwrap();
    sum["types"][0]["conflictState"] = json!("local-ahead");
    sum["types"][0]["localVersion"] = json!(7);
    sum["types"][0]["localEditedAt"] = json!("2026-02-02T00:00:00Z");
    std::fs::write(&sp, serde_json::to_vec(&sum).unwrap()).unwrap();
    std::fs::remove_file(t.path().join("packages/upg/.srs-import/refs").join(&rel)).unwrap();
    let r = upgrade(&store, &v_old(), false);
    assert_eq!(names(&r.repaired), vec!["type:essay@1"]);
    let after: Value = serde_json::from_slice(&std::fs::read(&sp).unwrap()).unwrap();
    let rec = &after["types"][0];
    assert_eq!(rec["localVersion"], 7);
    assert_eq!(rec["localEditedAt"], "2026-02-02T00:00:00Z");
    assert_eq!(rec["conflictState"], "local-ahead");
}

#[test]
fn refresh_of_a_locally_edited_definition_keeps_its_local_provenance() {
    let (t, store) = installed_old();
    let rel = type_v1_path(&store).replacen("packages/upg/", "", 1);
    let file = t.path().join("packages/upg").join(&rel);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    v["description"] = json!("Edited locally.");
    std::fs::write(&file, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let sp = t
        .path()
        .join("packages/upg/.srs-import/import-records.json");
    let mut sum: Value = serde_json::from_slice(&std::fs::read(&sp).unwrap()).unwrap();
    sum["types"][0]["conflictState"] = json!("local-ahead");
    sum["types"][0]["localVersion"] = json!(7);
    sum["types"][0]["localEditedAt"] = json!("2026-02-02T00:00:00Z");
    std::fs::write(&sp, serde_json::to_vec(&sum).unwrap()).unwrap();
    // Upstream did not change this definition, but the package version moved: the
    // record's sourcePackageVersion is refreshed and the local provenance survives.
    let newer = bundle("1.4.0", vec![field(F, "title")], vec![ty(1, "Old.", &[F])]);
    let r = upgrade(&store, &newer, false);
    assert!(r.conflicts.is_empty());
    let after: Value = serde_json::from_slice(&std::fs::read(&sp).unwrap()).unwrap();
    let rec = &after["types"][0];
    assert_eq!(rec["sourcePackageVersion"], "1.4.0");
    assert_eq!(rec["localVersion"], 7);
    assert_eq!(rec["conflictState"], "local-ahead");
    let kept: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(kept["description"], "Edited locally.");
}

// ── #1325: proof by prior bundle, per-definition consent ────────────────────

fn synced() -> Vec<u8> {
    bundle(
        "1.4.0",
        vec![field(F, "title")],
        vec![ty(1, "Synced.", &[F])],
    )
}

fn no_ref_copy(t: &tempfile::TempDir, store: &FileStore) {
    let rel = type_v1_path(store).replacen("packages/upg/", "", 1);
    std::fs::remove_file(t.path().join("packages/upg/.srs-import/refs").join(&rel)).unwrap();
}

fn with_opts(
    store: &FileStore,
    new: &[u8],
    prior: &[Vec<u8>],
    adopt: &[&str],
    dry: bool,
) -> UpgradePackageResult {
    upgrade_package_bundle(
        store,
        new,
        UpgradeOptions {
            dry_run: dry,
            prior_bundles: prior
                .iter()
                .map(|b| String::from_utf8(b.clone()).unwrap())
                .collect(),
            adopt: adopt.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn prior_bundle_proves_a_missing_reference_copy_clean() {
    let (t, store) = installed_old();
    no_ref_copy(&t, &store);
    let before = snapshot(t.path());
    let dry = with_opts(&store, &synced(), &[v_old()], &[], true);
    assert_eq!(snapshot(t.path()), before, "dry run wrote");
    assert!(dry.conflicts.is_empty(), "{:?}", dry.conflicts);
    assert_eq!(dry.updated.len(), 1);
    assert_eq!(dry.updated[0].proven_by.as_deref(), Some("1.3.0"));

    let r = with_opts(&store, &synced(), &[v_old()], &[], false);
    assert_eq!(r.updated.len(), 1);
    assert_eq!(r.updated[0].proven_by.as_deref(), Some("1.3.0"));
    let rel = type_v1_path(&store).replacen("packages/upg/", "", 1);
    let refc: Value = serde_json::from_slice(
        &std::fs::read(t.path().join("packages/upg/.srs-import/refs").join(&rel)).unwrap(),
    )
    .unwrap();
    assert_eq!(refc["description"], "Synced.");
    assert_valid(&store);
    // Converges: the next run has nothing to do.
    let again = with_opts(&store, &synced(), &[v_old()], &[], false);
    assert!(again.conflicts.is_empty() && again.updated.is_empty() && again.adopted.is_empty());
}

#[test]
fn prior_bundle_does_not_prove_an_edited_definition() {
    let (t, store) = installed_old();
    no_ref_copy(&t, &store);
    let file = t.path().join(type_v1_path(&store));
    let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    v["description"] = json!("Edited locally.");
    std::fs::write(&file, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let r = with_opts(&store, &synced(), &[v_old()], &[], false);
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].conflict_kind, "no-reference-copy");
    assert!(r.updated.is_empty());
}

#[test]
fn adopt_overwrites_a_no_reference_copy_conflict() {
    let (t, store) = installed_old();
    no_ref_copy(&t, &store);
    let dry = with_opts(&store, &synced(), &[], &[A], true);
    assert_eq!(dry.adopted.len(), 1);
    assert!(dry.conflicts.is_empty());
    let r = with_opts(&store, &synced(), &[], &[A], false);
    assert_eq!(r.adopted.len(), 1);
    assert!(r.updated.is_empty());
    let pkg = store.load_package().unwrap();
    assert_eq!(pkg.resolve_type(A, 1).unwrap().description, "Synced.");
    assert_valid(&store);
}

#[test]
fn adopt_is_refused_for_a_local_edit() {
    let (t, store) = installed_old();
    let file = t.path().join(type_v1_path(&store));
    let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    v["description"] = json!("Edited locally.");
    std::fs::write(&file, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let r = with_opts(&store, &synced(), &[], &[A], false);
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].conflict_kind, "local-edit");
    assert!(r.adopted.is_empty());
    assert!(
        r.notes.iter().any(|n| n.contains("not adoptable")),
        "{:?}",
        r.notes
    );
    let kept: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(kept["description"], "Edited locally.");
}

#[test]
fn prior_bundle_of_another_package_is_refused() {
    let (_t, store) = installed_old();
    let other = bundle_with(
        "1.0.0",
        vec![field(F, "title")],
        vec![ty(1, "Old.", &[F])],
        json!({"packageId": "9a1b0c2d-2222-4aaa-8bbb-0000000000ff"}),
    );
    let err = upgrade_package_bundle(
        &store,
        &synced(),
        UpgradeOptions {
            prior_bundles: vec![String::from_utf8(other).unwrap()],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("prior bundle"), "{err}");
}

fn record_versions(t: &tempfile::TempDir) -> Vec<String> {
    let v: Value = serde_json::from_slice(
        &std::fs::read(
            t.path()
                .join("packages/upg/.srs-import/import-records.json"),
        )
        .unwrap(),
    )
    .unwrap();
    v["types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["sourcePackageVersion"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn proof_and_adopt_refresh_import_records_and_adopt_converges() {
    let newer = bundle(
        "1.4.0",
        vec![field(F, "title")],
        vec![ty(1, "Synced.", &[F])],
    );
    for adopt in [false, true] {
        let (t, store) = installed_old();
        no_ref_copy(&t, &store);
        let (prior, ids): (Vec<Vec<u8>>, &[&str]) = if adopt {
            (vec![], &[A])
        } else {
            (vec![v_old()], &[])
        };
        with_opts(&store, &newer, &prior, ids, false);
        assert_eq!(record_versions(&t), vec!["1.4.0"], "adopt={adopt}");
        let again = with_opts(&store, &newer, &prior, ids, false);
        assert!(again.conflicts.is_empty() && again.adopted.is_empty() && again.updated.is_empty());
        assert!(again.notes.iter().any(|n| n.contains("nothing to adopt")) == adopt);
    }
}

#[test]
fn adopt_is_refused_for_a_key_collision() {
    let (_t, store) = installed_old();
    let other = "9a1b0c2d-0001-4aaa-8bbb-00000000f0ff";
    let b = bundle(
        "1.4.0",
        vec![field(F, "title"), field(other, "title")],
        vec![ty(1, "Old.", &[F])],
    );
    let r = with_opts(&store, &b, &[], &[other], false);
    assert_eq!(r.conflicts[0].conflict_kind, "key-collision");
    assert!(r.adopted.is_empty());
    assert!(
        r.notes.iter().any(|n| n.contains("not adoptable")),
        "{:?}",
        r.notes
    );
}

#[test]
fn prior_with_a_different_definition_version_is_not_proof() {
    let (t, store) = installed_old();
    no_ref_copy(&t, &store);
    // Prior holds type v2 only; installed is v1.
    let prior = bundle("1.2.0", vec![field(F, "title")], vec![ty(2, "Old.", &[F])]);
    let r = with_opts(&store, &synced(), &[prior], &[], false);
    assert_eq!(r.conflicts[0].conflict_kind, "no-reference-copy");
}

#[test]
fn prior_version_sanity_checks() {
    let (_t, store) = installed_old(); // 1.3.0
    let run = |prior: Vec<u8>, new: &[u8]| {
        upgrade_package_bundle(
            &store,
            new,
            UpgradeOptions {
                dry_run: true,
                prior_bundles: vec![String::from_utf8(prior).unwrap()],
                ..Default::default()
            },
        )
    };
    // Not older than the new bundle.
    let e = run(v_old(), &v_old()).unwrap_err();
    assert!(e.to_string().contains("not older"), "{e}");
    // Newer than the installed version.
    let newer_prior = bundle("1.4.0", vec![field(F, "title")], vec![ty(1, "Old.", &[F])]);
    let e = run(newer_prior, &v_new()).unwrap_err();
    assert!(e.to_string().contains("newer than the installed"), "{e}");
}
