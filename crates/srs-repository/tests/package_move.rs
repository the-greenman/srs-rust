//! srs-rust#1639: `move_definition` moves every version of a definition to another package
//! boundary, keeping its id, so the repository stays valid; it refuses what would break an
//! installed package, and a dry run writes nothing.
//!
//! srs-rust#1654: the lineage-move-together case, the dry-run case and the refusal-code cases
//! also run against `MemoryStore` (`memory_store_*` tests, sharing `run_..._suite` fns with the
//! `file_store_*` tests, the idiom established in `delete_membership_cascade.rs`) — CLAUDE.md's
//! Storage Boundary Rules require a cross-store test for a service feature, and a `FileStore`-only
//! suite tests the adapter, not the service. The remaining tests below (file-rename collision,
//! partial-move resume, installed-package refusals) stay `FileStore`-only for now (srs-rust#1655).

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
use srs_repository::store::memory::MemoryStore;
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

/// The `MemoryStore` counterpart to [`repo`]: `MemoryStore::empty()` already carries a valid
/// manifest/primary package in memory (`create_repository`/`initialize_repository` is not
/// required — see `store_conformance.rs`), so this populates it with the same site package,
/// field, type and relation type as the `FileStore` fixture, through the same store-generic
/// `&dyn RepositoryStore` service calls.
fn memory_repo() -> MemoryStore {
    let store = MemoryStore::empty();
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
    store
}

fn input(id: &str, to: Option<&str>, dry_run: bool) -> MoveDefinitionInput {
    MoveDefinitionInput {
        id: id.to_string(),
        to: to.map(str::to_string),
        dry_run,
    }
}

fn list(store: &dyn RepositoryStore, prefix: &str, key: &str) -> Vec<String> {
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

/// Shared with `file_store_moves_every_version_of_a_type_together` and
/// `memory_store_moves_every_version_of_a_type_together` (srs-rust#1654): one assertion body,
/// run against both backends, so the claim is about the service, not the adapter.
///
/// Uses `store.catalog()` rather than `validate_repository` for the "repository stays valid"
/// check, as `assert_loads_clean` does in `delete_membership_cascade.rs`: `validate_repository`
/// also reads the raw `manifest.json` text, which only a disk-backed store carries by default
/// (a `MemoryStore` manifest is typed, not additionally mirrored as text) — `catalog()` is the
/// store-agnostic validity check RFC-038 and ADR-045 already treat as authoritative.
fn run_moves_every_version_of_a_type_together(store: &dyn RepositoryStore) {
    let out = move_definition(store, &input(T, Some(SITE), false)).unwrap();
    let mut versions: Vec<u64> = out.moved.iter().map(|m| m.version).collect();
    versions.sort();
    assert_eq!(versions, vec![1, 2]);
    assert!(list(store, "package", "types").is_empty());
    assert_eq!(list(store, SITE, "types").len(), 2);
    let cat = store.catalog().unwrap();
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    // the type still references the field it uses, which stays in the primary package
    assert!(out.referenced_by.is_empty());
}

#[test]
fn file_store_moves_every_version_of_a_type_together() {
    let (_t, store) = repo();
    run_moves_every_version_of_a_type_together(&store);
}

#[test]
fn memory_store_moves_every_version_of_a_type_together() {
    run_moves_every_version_of_a_type_together(&memory_repo());
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

/// `MemoryStore` counterpart (srs-rust#1654): `snapshot()` above is a filesystem walk, so it has
/// no `MemoryStore` equivalent — this asserts the same "writes nothing" claim through the
/// store's own read surface instead: the boundary lists are unchanged before and after a dry run
/// that still reports what it *would* move.
#[test]
fn memory_store_a_dry_run_writes_nothing() {
    let store = memory_repo();
    let before_package = list(&store, "package", "types");
    let before_site = list(&store, SITE, "types");
    let out = move_definition(&store, &input(T, Some(SITE), true)).unwrap();
    assert!(out.dry_run);
    assert_eq!(out.moved.len(), 2);
    assert_eq!(list(&store, "package", "types"), before_package);
    assert_eq!(list(&store, SITE, "types"), before_site);
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

/// Shared with `file_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary` and
/// `memory_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary` (srs-rust#1654):
/// the refusal codes are a claim about the service, so they must hold against either backend.
fn run_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary(store: &dyn RepositoryStore) {
    let missing = "5a1b0c2d-0009-4aaa-8bbb-000000000000";
    assert_eq!(
        code(move_definition(store, &input(missing, Some(SITE), false)).unwrap_err()),
        "definition-not-found"
    );
    assert_eq!(
        code(move_definition(store, &input(RT, Some("packages/none"), false)).unwrap_err()),
        "package-not-found"
    );
    assert_eq!(
        code(move_definition(store, &input(RT, None, false)).unwrap_err()),
        "move-same-boundary"
    );
}

#[test]
fn file_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary() {
    let (_t, store) = repo();
    run_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary(&store);
}

#[test]
fn memory_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary() {
    run_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary(&memory_repo());
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

#[test]
fn moves_a_type_that_records_use_and_reports_what_it_leaves_behind() {
    let (_t, store) = repo();
    let fv: srs_core::types::record::FieldValues =
        serde_json::from_value(json!({"title": "Home"})).unwrap();
    srs_repository::record_store::create_record(&store, T, 2, fv, None, None).unwrap();
    let out = move_definition(&store, &input(T, Some(SITE), false)).unwrap();
    // the type uses the field, which stays in the primary package
    assert_eq!(out.references.len(), 1, "{:?}", out.references);
    assert!(out.references[0].starts_with("package/fields/"));
    let report = validate_repository(&store).unwrap();
    assert!(report.is_ok(), "{:?}", report.diagnostics);
}

#[test]
fn renames_the_file_when_the_target_holds_another_definition_under_its_name() {
    let (_t, store) = repo();
    let rel = list(&store, "package", "relationTypes")[0].clone();
    // another definition already sits at that path in the target
    store
        .ensure_instance_dir(&format!("{SITE}/relation-types"))
        .unwrap();
    store
        .save_instance_json(
            &format!("{SITE}/{rel}"),
            &json!({"id": "5a1b0c2d-0003-4aaa-8bbb-0000000000ff", "version": 1, "key": "other",
                "namespace": NS, "label": "Other", "category": "association"}),
        )
        .unwrap();
    let out = move_definition(&store, &input(RT, Some(SITE), false)).unwrap();
    assert_ne!(out.moved[0].path, rel);
    assert_eq!(
        store
            .load_instance_json(&format!("{SITE}/{}", out.moved[0].path))
            .unwrap()["id"],
        RT
    );
}

#[test]
fn running_a_move_again_finishes_one_that_failed_partway() {
    let (_t, store) = repo();
    // a half-done move: version 1 of the type already written into the target and listed,
    // the source not yet emptied
    let v1 = list(&store, "package", "types")
        .into_iter()
        .find(|r| store.load_instance_json(&format!("package/{r}")).unwrap()["version"] == 1)
        .unwrap();
    let value = store.load_instance_json(&format!("package/{v1}")).unwrap();
    store.ensure_instance_dir(&format!("{SITE}/types")).unwrap();
    store
        .save_instance_json(&format!("{SITE}/{v1}"), &value)
        .unwrap();
    store
        .add_definition_to_boundary(
            &Some(SITE.to_string()),
            srs_repository::package_types::DefinitionKind::Type,
            &v1,
        )
        .unwrap();
    let out = move_definition(&store, &input(T, Some(SITE), false)).unwrap();
    assert_eq!(out.moved.len(), 2);
    assert!(list(&store, "package", "types").is_empty());
    assert_eq!(list(&store, SITE, "types").len(), 2);
    assert!(validate_repository(&store).unwrap().is_ok());
}

#[test]
fn refuses_a_split_it_did_not_make() {
    let (_t, store) = repo();
    // the target holds a different version-1 copy of the id: not a half-done move
    let v1 = list(&store, "package", "relationTypes")[0].clone();
    let mut changed = store.load_instance_json(&format!("package/{v1}")).unwrap();
    changed["label"] = json!("Changed");
    store
        .ensure_instance_dir(&format!("{SITE}/relation-types"))
        .unwrap();
    store
        .save_instance_json(&format!("{SITE}/{v1}"), &changed)
        .unwrap();
    store
        .add_definition_to_boundary(
            &Some(SITE.to_string()),
            srs_repository::package_types::DefinitionKind::RelationType,
            &v1,
        )
        .unwrap();
    assert_eq!(
        code(move_definition(&store, &input(RT, Some(SITE), false)).unwrap_err()),
        "move-split-lineage"
    );
}

#[test]
fn package_names_the_primary_boundary() {
    let (_t, store) = repo();
    move_definition(&store, &input(RT, Some(SITE), false)).unwrap();
    let out = move_definition(&store, &input(RT, Some("package"), false)).unwrap();
    assert_eq!(out.moved[0].to, "package");
}
