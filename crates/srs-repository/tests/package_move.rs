//! srs-rust#1639: `move_definition` moves every version of a definition to another package
//! boundary, keeping its id, so the repository stays valid; it refuses what would break an
//! installed package, and a dry run writes nothing.

use serde_json::{json, Value};
use srs_repository::error::RepositoryError;
use srs_repository::package_install_service::install_package_bundle_bytes;
use srs_repository::package_move_service::{move_definition, MoveDefinitionInput};
use srs_repository::package_service::{
    create_field_normalized, create_package, create_relation_type_normalized,
    create_type_normalized, CreatePackageInput,
};
use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::collections::BTreeMap;
use std::path::Path;

const NS: &str = "com.test.move";
const F: &str = "5a1b0c2d-0001-4aaa-8bbb-00000000f001";
const T: &str = "5a1b0c2d-0002-4aaa-8bbb-00000000a001";
const RT: &str = "5a1b0c2d-0003-4aaa-8bbb-00000000b001";
const SITE: &str = "packages/site";

fn repo() -> (tempfile::TempDir, FileStore) {
    let temp = tempfile::TempDir::new().unwrap();
    let store = FileStore::new(temp.path());
    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "5a1b0c2d-0000-4000-8000-000000000001".to_string(),
                namespace: NS.to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Move".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "move-primary".to_string(),
                namespace: NS.to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .unwrap();
    create_package(
        &store,
        CreatePackageInput {
            id: "5a1b0c2d-0000-4000-8000-0000000000aa".to_string(),
            namespace: "com.test.site".to_string(),
            name: "site".to_string(),
            version: "0.1.0".to_string(),
            boundary_path: Some(SITE.to_string()),
        },
    )
    .unwrap();
    create_field_normalized(
        &store,
        json!({"id": F, "namespace": NS, "name": "title", "version": 1,
            "fieldType": {"datatype": "string"}, "aiGuidance": {"purpose": "A title."}}),
        None,
    )
    .unwrap();
    for version in [1, 2] {
        create_type_normalized(
            &store,
            json!({"id": T, "namespace": NS, "name": "page", "version": version,
                "fields": [{"fieldId": F, "order": 0, "required": false}]}),
            None,
        )
        .unwrap();
    }
    create_relation_type_normalized(
        &store,
        json!({"id": RT, "version": 1, "key": "uses", "namespace": NS, "label": "Uses",
            "category": "association"}),
        None,
    )
    .unwrap();
    (temp, store)
}

fn input(id: &str, to: Option<&str>, dry_run: bool) -> MoveDefinitionInput {
    MoveDefinitionInput {
        id: id.to_string(),
        to: to.map(str::to_string),
        dry_run,
    }
}

fn list(store: &FileStore, prefix: &str, key: &str) -> Vec<String> {
    store
        .load_instance_json(&format!("{prefix}/package.json"))
        .unwrap()[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn code(e: RepositoryError) -> &'static str {
    e.code()
}

/// Every file except the derived catalog cache.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if rel.starts_with(".srs/cache") {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn moves_a_relation_type_keeping_its_id_and_the_repository_valid() {
    let (_t, store) = repo();
    let out = move_definition(&store, &input(RT, Some(SITE), false)).unwrap();
    assert_eq!(out.moved.len(), 1);
    assert_eq!(out.moved[0].from, "package");
    assert_eq!(out.moved[0].to, SITE);
    let path = &out.moved[0].path;
    assert!(list(&store, SITE, "relationTypes").contains(path));
    assert!(!list(&store, "package", "relationTypes").contains(path));
    assert!(store
        .load_instance_json(&format!("package/{path}"))
        .is_err());
    let moved = store.load_instance_json(&format!("{SITE}/{path}")).unwrap();
    assert_eq!(moved["id"], RT);
    assert_eq!(moved["key"], "uses");
    assert!(validate_repository(&store).unwrap().is_ok());
}

#[test]
fn moves_every_version_of_a_type_together() {
    let (_t, store) = repo();
    let out = move_definition(&store, &input(T, Some(SITE), false)).unwrap();
    let mut versions: Vec<u64> = out.moved.iter().map(|m| m.version).collect();
    versions.sort();
    assert_eq!(versions, vec![1, 2]);
    assert!(list(&store, "package", "types").is_empty());
    assert_eq!(list(&store, SITE, "types").len(), 2);
    let report = validate_repository(&store).unwrap();
    assert!(report.is_ok(), "{:?}", report.diagnostics);
    // the type still references the field it uses, which stays in the primary package
    assert!(out.referenced_by.is_empty());
}

#[test]
fn reports_what_in_the_source_mentions_the_moved_definition() {
    let (_t, store) = repo();
    let out = move_definition(&store, &input(F, Some(SITE), true)).unwrap();
    assert_eq!(
        out.referenced_by.len(),
        2,
        "both versions of the type use the field"
    );
    assert!(out
        .referenced_by
        .iter()
        .all(|p| p.starts_with("package/types/")));
}

#[test]
fn a_dry_run_writes_nothing() {
    let (t, store) = repo();
    let before = snapshot(t.path());
    let out = move_definition(&store, &input(T, Some(SITE), true)).unwrap();
    assert!(out.dry_run);
    assert_eq!(out.moved.len(), 2);
    assert_eq!(snapshot(t.path()), before);
}

#[test]
fn moves_back_into_the_primary_package() {
    let (_t, store) = repo();
    move_definition(&store, &input(RT, Some(SITE), false)).unwrap();
    let out = move_definition(&store, &input(RT, None, false)).unwrap();
    assert_eq!(out.moved[0].to, "package");
    assert!(list(&store, SITE, "relationTypes").is_empty());
    assert!(validate_repository(&store).unwrap().is_ok());
}

#[test]
fn refuses_an_unknown_id_an_unknown_target_and_the_same_boundary() {
    let (_t, store) = repo();
    let missing = "5a1b0c2d-0009-4aaa-8bbb-000000000000";
    assert_eq!(
        code(move_definition(&store, &input(missing, Some(SITE), false)).unwrap_err()),
        "definition-not-found"
    );
    assert_eq!(
        code(move_definition(&store, &input(RT, Some("packages/none"), false)).unwrap_err()),
        "package-not-found"
    );
    assert_eq!(
        code(move_definition(&store, &input(RT, None, false)).unwrap_err()),
        "move-same-boundary"
    );
}

fn bundle() -> Vec<u8> {
    let field = |id: &str, name: &str| -> Value {
        json!({"id": id, "namespace": "com.test.inst", "name": name, "version": 1,
            "description": "Label.", "fieldType": {"datatype": "string"},
            "aiGuidance": {"purpose": "Label."}, "createdAt": "2026-01-01T00:00:00Z"})
    };
    serde_json::to_vec(&json!({
        "schemaVersion": "2.0-draft", "packageId": "5a1b0c2d-2222-4aaa-8bbb-000000000001",
        "packageNamespace": "com.test.inst", "packageName": "inst", "packageVersion": "1.0.0",
        "dataModelRevision": 9, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
        "fields": [field("5a1b0c2d-2222-4aaa-8bbb-00000000f001", "label")], "types": [],
        "relationTypes": [], "views": [], "dependencyRefs": [], "packageDependencies": []
    }))
    .unwrap()
}

#[test]
fn refuses_to_move_into_or_out_of_an_installed_package() {
    let (_t, store) = repo();
    install_package_bundle_bytes(&store, &bundle(), Default::default()).unwrap();
    let installed = "packages/inst";
    assert_eq!(
        code(move_definition(&store, &input(RT, Some(installed), false)).unwrap_err()),
        "move-target-installed"
    );
    assert_eq!(
        code(
            move_definition(
                &store,
                &input("5a1b0c2d-2222-4aaa-8bbb-00000000f001", Some(SITE), false)
            )
            .unwrap_err()
        ),
        "move-definition-imported"
    );
}
