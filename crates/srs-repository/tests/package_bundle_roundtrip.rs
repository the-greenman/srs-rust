//! `.srspkg` round trip (srs-rust#632/#690, ADR-050): export a boundary carrying
//! all ten definition kinds, install the bytes into a fresh repository, and every
//! definition comes back identical with zero validation errors.

use serde_json::Value;
use srs_core::extensions::import_tracking::ConflictState;
use srs_repository::package_bundle::{export_package_bundle, ExportPackageInput};
use srs_repository::package_install_service::{
    install_package, install_package_bundle_bytes, InstallPackageInput,
};
use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};
use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tempfile::TempDir;

const FIXTURE: &str = "packages/install-fixture";
const AT: &str = "2026-10-03T00:00:00Z";
const KIND_KEYS: [&str; 10] = [
    "fields",
    "types",
    "relationTypes",
    "lifecycles",
    "vocabularies",
    "views",
    "compositions",
    "themes",
    "blueprints",
    "protocols",
];

fn fresh(repo_id: &str) -> (TempDir, FileStore) {
    let temp = TempDir::new().unwrap();
    let store = FileStore::new(temp.path());
    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: repo_id.to_string(),
                namespace: "com.test.roundtrip".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Round Trip".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: format!("{repo_id}-primary"),
                namespace: "com.test.roundtrip".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .unwrap();
    (temp, store)
}

/// Repo A with the fixture installed from its directory.
fn repo_a() -> (TempDir, FileStore) {
    let (t, store) = fresh("17575e57-0000-4000-8000-0000000000a1");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/install-package");
    install_package(
        &store,
        InstallPackageInput {
            source_dir: dir.display().to_string(),
            boundary_path: None,
            strict: false,
        },
    )
    .unwrap();
    (t, store)
}

fn export(store: &FileStore) -> srs_repository::package_bundle::PackageBundleExport {
    export_package_bundle(
        store,
        ExportPackageInput {
            selector: Some(FIXTURE.to_string()),
            published_at: Some(AT.to_string()),
            publisher: None,
        },
    )
    .unwrap()
}

/// Every definition listed by a boundary's package.json, keyed by id.
fn definitions(store: &FileStore, boundary: &str) -> HashMap<String, Value> {
    let index = store
        .load_instance_json(&format!("{boundary}/package.json"))
        .unwrap();
    let mut out = HashMap::new();
    for key in KIND_KEYS {
        for rel in index[key].as_array().into_iter().flatten() {
            let v = store
                .load_instance_json(&format!("{boundary}/{}", rel.as_str().unwrap()))
                .unwrap();
            out.insert(v["id"].as_str().unwrap().to_string(), v);
        }
    }
    out
}

fn assert_zero_errors(store: &FileStore) {
    let report = validate_repository(store).unwrap();
    assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
}

/// Repo A, its export, and fresh repo B with the bundle installed.
fn round_trip() -> (TempDir, FileStore, String, TempDir, FileStore) {
    let (ta, a) = repo_a();
    let e = export(&a);
    assert_eq!(e.summary.kinds.len(), 10);
    let (tb, b) = fresh("17575e57-0000-4000-8000-0000000000b1");
    let r = install_package_bundle_bytes(&b, e.text.as_bytes(), Default::default()).unwrap();
    assert_eq!(r.installed, 11);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    (ta, a, e.text, tb, b)
}

#[test]
fn bundle_roundtrip_all_ten_kinds_definitions_identical() {
    let (_ta, a, _text, _tb, b) = round_trip();
    let from_a = definitions(&a, FIXTURE);
    let from_b = definitions(&b, FIXTURE);
    assert_eq!(from_a.len(), 11);
    assert_eq!(from_a, from_b);
    assert_zero_errors(&a);
    assert_zero_errors(&b);
}

#[test]
fn bundle_roundtrip_boundary_metadata_identical() {
    let (_ta, a, _text, _tb, b) = round_trip();
    let sel = Some(FIXTURE.to_string());
    let (ba, bb) = (
        a.load_package_boundary(&sel).unwrap(),
        b.load_package_boundary(&sel).unwrap(),
    );
    assert_eq!(
        (&ba.id, &ba.namespace, &ba.name, &ba.version),
        (&bb.id, &bb.namespace, &bb.name, &bb.version)
    );
}

/// `ImportSummary` has no theme, vocabulary, lifecycle or composition lists, so
/// this covers fields, types, views, blueprints, protocols and relation types
/// only; the other four are covered by the definitions-identical test.
#[test]
fn bundle_roundtrip_imports_report_clean() {
    let (_ta, _a, _text, _tb, b) = round_trip();
    let s = list_package_imports(&b, ListPackageImportsFilter::default()).unwrap();
    let all: Vec<_> = [
        &s.fields,
        &s.types,
        &s.views,
        &s.blueprints,
        &s.protocols,
        &s.relation_types,
    ]
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(all.len(), 7);
    for r in all {
        assert_eq!(r.conflict_state, Some(ConflictState::Clean), "{r:?}");
    }
}

#[test]
fn bundle_roundtrip_reexport_is_byte_identical() {
    let (_ta, _a, text, _tb, b) = round_trip();
    assert_eq!(export(&b).text, text);
}

fn read_tree(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            read_tree(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy();
            out.insert(rel.replace('\\', "/"), std::fs::read(&path).unwrap());
        }
    }
}

#[test]
fn bundle_roundtrip_tree_session_matches_disk() {
    let (ta, a) = repo_a();
    let mut files = BTreeMap::new();
    read_tree(ta.path(), ta.path(), &mut files);
    let tree = srs_repository::tree_session::open_tree(files).unwrap();
    assert_eq!(export(&tree).text, export(&a).text);
}
