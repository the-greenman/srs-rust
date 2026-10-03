//! `.srspkg` round trip (srs-rust#632/#690, ADR-050): export a boundary carrying
//! all ten definition kinds, install the bytes into a fresh repository, and every
//! definition comes back identical with zero validation errors.

use serde_json::json;
use serde_json::Value;
use srs_core::extensions::import_tracking::ConflictState;
use srs_repository::package_bundle::BundleMode;
use srs_repository::package_bundle::{export_package_bundle, ExportPackageInput};
use srs_repository::package_install_service::{
    install_package, install_package_bundle_bytes, InstallPackageInput,
};
use srs_repository::package_service::{create_package, CreatePackageInput};
use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};
use srs_repository::package_types::DefinitionKind;
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    create_repository(
        &store,
        &InitializeRepositoryInput {
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
        },
    )
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
            ..Default::default()
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

// ---------------------------------------------------------------------------
// RFC-003 Revision 10: dependencyRefs and the Package invariants (srs-rust#1212)
// ---------------------------------------------------------------------------

/// Core Field and Type ids, read from the embedded core bundle file itself
/// (independent of the exporter's own core handling).
fn core_ids() -> BTreeSet<String> {
    let core: Value = serde_json::from_str(include_str!("../assets/core-bundle.srsj")).unwrap();
    ["fields", "types"]
        .iter()
        .flat_map(|k| core[*k].as_array().unwrap().iter())
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect()
}

/// The Package invariants a bundle's `dependencyRefs` must satisfy, written from
/// the invariant texts and independent of the exporter's reference-site table.
/// A `bundled` bundle must also carry every non-core definition it must list;
/// a `standalone` one lists without carrying.
fn assert_package_invariants(b: &Value) {
    let core = core_ids();
    let bundled = b["mode"] == "bundled";
    let listed: BTreeSet<(String, u64, String)> = b["dependencyRefs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["id"].as_str().unwrap().to_string(),
                r["version"].as_u64().unwrap(),
                r["definitionType"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let items = |key: &str| b[key].as_array().cloned().unwrap_or_default();
    let carried = |key: &str, id: &str, ver: Option<u64>| {
        items(key)
            .iter()
            .any(|d| d["id"] == id && ver.is_none_or(|v| d["version"] == v))
    };
    let need = |id: &str, ver: Option<u64>, ty: &str, key: &str, inv: &str| {
        assert!(
            listed
                .iter()
                .any(|(i, v, t)| i == id && t == ty && ver.is_none_or(|w| *v == w)),
            "{inv}: {ty} {id} {ver:?} is not in dependencyRefs"
        );
        if bundled && !core.contains(id) {
            assert!(
                carried(key, id, ver),
                "{inv}: {ty} {id} {ver:?} is not carried"
            );
        }
    };
    // I-8: every Field a Type uses is listed; every listed Field is carried or core.
    for t in items("types") {
        for f in t["fields"].as_array().into_iter().flatten() {
            need(
                f["fieldId"].as_str().unwrap(),
                None,
                "field",
                "fields",
                "I-8",
            );
        }
    }
    if bundled {
        for (id, v, _) in listed.iter().filter(|r| r.2 == "field") {
            assert!(
                core.contains(id) || carried("fields", id, Some(*v)),
                "I-8: field {id}"
            );
        }
    }
    // I-15 is vacuous against view.json: a View holds no Type UUID (only the KEYED
    // compatibleTypes), so there is nothing to check.
    // I-35: a section's renderViewId is listed and carried (no core views exist).
    for c in items("compositions") {
        for sec in c["sections"].as_array().into_iter().flatten() {
            if let Some(v) = sec["renderViewId"].as_str() {
                need(v, None, "view", "views", "I-35");
            }
        }
    }
    // I-36: Blueprint type references, version-exact.
    for bp in items("blueprints") {
        let refs = ["rootTypes", "requiredTypes"]
            .iter()
            .flat_map(|k| bp[*k].as_array().cloned().unwrap_or_default())
            .chain(
                bp["structure"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|s| [s["sourceType"].clone(), s["targetType"].clone()]),
            )
            .collect::<Vec<_>>();
        for r in refs {
            need(
                r["typeId"].as_str().unwrap(),
                r["typeVersion"].as_u64(),
                "type",
                "types",
                "I-36",
            );
        }
    }
    // I-37: Protocol target/output types and contributed fields.
    for p in items("protocols") {
        let mut types: Vec<String> = p["targetType"]
            .as_str()
            .into_iter()
            .map(str::to_string)
            .collect();
        for st in p["stages"].as_array().into_iter().flatten() {
            types.extend(st["outputType"].as_str().map(str::to_string));
            for c in st["contributesTo"].as_array().into_iter().flatten() {
                types.extend(c["typeId"].as_str().map(str::to_string));
                if let Some(f) = c["fieldId"].as_str() {
                    need(f, None, "field", "fields", "I-37");
                }
            }
        }
        for t in types.iter().filter(|t| !t.is_empty()) {
            need(t, None, "type", "types", "I-37");
        }
    }
    // I-43: the base-Type closure (each carried Type's base is listed and carried).
    for t in items("types") {
        if let Some(base) = t["extendsTypeId"].as_str() {
            need(
                base,
                t["extendsTypeVersion"].as_u64(),
                "type",
                "types",
                "I-43",
            );
        }
    }
}

fn export_mode(store: &FileStore, selector: &str, mode: BundleMode) -> String {
    export_package_bundle(
        store,
        ExportPackageInput {
            selector: Some(selector.to_string()),
            published_at: Some(AT.to_string()),
            mode,
            ..Default::default()
        },
    )
    .unwrap()
    .text
}

fn sub_package(store: &FileStore, path: &str, id: &str) {
    create_package(
        store,
        CreatePackageInput {
            id: id.to_string(),
            namespace: "com.test.roundtrip".to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            version: "0.1.0".to_string(),
            boundary_path: Some(path.to_string()),
        },
    )
    .unwrap();
}

fn put(store: &FileStore, selector: &str, kind: DefinitionKind, rel: &str, v: Value) {
    store
        .ensure_instance_dir(&format!("{selector}/{}", rel.rsplit_once('/').unwrap().0))
        .unwrap();
    store
        .save_instance_json(&format!("{selector}/{rel}"), &v)
        .unwrap();
    store
        .add_definition_to_boundary(&Some(selector.to_string()), kind, rel)
        .unwrap();
}

const DEP: &str = "packages/dep";
const APP: &str = "packages/app";
const DEP_FIELD: &str = "d0000000-0000-4000-8000-0000000000f1";

fn dep_field() -> Value {
    json!({"id": DEP_FIELD, "namespace": "com.test.roundtrip", "name": "dep_field",
        "version": 1, "description": "d", "fieldType": {"datatype": "string"},
        "aiGuidance": {"purpose": "p"}, "createdAt": "2026-01-01T00:00:00Z"})
}

/// Repo with `packages/dep` (one Field) and `packages/app`.
fn dep_and_app() -> (TempDir, FileStore) {
    let (t, store) = fresh("17575e57-0000-4000-8000-0000000000c1");
    sub_package(&store, DEP, "5e000000-0000-4000-8000-0000000000d1");
    sub_package(&store, APP, "5e000000-0000-4000-8000-0000000000d2");
    put(
        &store,
        DEP,
        DefinitionKind::Field,
        "fields/dep-field.json",
        dep_field(),
    );
    (t, store)
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn bundle_roundtrip_dependency_refs_satisfy_package_invariants() {
    let (_t, a) = repo_a();
    let b = parse(&export(&a).text);
    assert!(!b["dependencyRefs"].as_array().unwrap().is_empty());
    assert_package_invariants(&b);
}

#[test]
fn bundle_roundtrip_cross_boundary_closure_installs_clean() {
    let (_t, a) = dep_and_app();
    let core: Value = serde_json::from_str(include_str!("../assets/core-bundle.srsj")).unwrap();
    let core_field = core["fields"][0]["id"].as_str().unwrap().to_string();
    // A View, not a Type, for the core field: srs-rust#1208 (the [R13] catalog does
    // not resolve a Type's core Field).
    put(
        &a,
        APP,
        DefinitionKind::View,
        "views/app-view.json",
        json!({"$schema": "https://srs.semanticops.com/schema/2.0/view.json",
            "id": "a0000000-0000-4000-8000-0000000000e1", "namespace": "com.test.roundtrip",
            "name": "app-view", "version": 1, "description": "d",
            "createdAt": "2026-01-01T00:00:00Z",
            "fieldViews": [
                {"fieldId": DEP_FIELD, "order": 0, "required": false, "visible": true},
                {"fieldId": core_field, "order": 1, "required": false, "visible": true}]}),
    );
    let text = export_mode(&a, APP, BundleMode::Bundled);
    let b = parse(&text);
    let listed: Vec<&str> = b["dependencyRefs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&DEP_FIELD) && listed.contains(&core_field.as_str()));
    assert_eq!(b["fields"].as_array().unwrap().len(), 1);
    assert_eq!(b["fields"][0]["id"], DEP_FIELD);
    assert_package_invariants(&b);

    let (_tb, dest) = fresh("17575e57-0000-4000-8000-0000000000c2");
    let r = install_package_bundle_bytes(&dest, text.as_bytes(), Default::default()).unwrap();
    assert_eq!(r.installed, 2);
    assert_zero_errors(&dest);
}

/// `packages/app` holds a Type using the `packages/dep` Field.
fn app_type(store: &FileStore) {
    put(
        store,
        APP,
        DefinitionKind::Type,
        "types/app-type.json",
        json!({"id": "a0000000-0000-4000-8000-0000000000e2", "namespace": "com.test.roundtrip",
            "name": "app_type", "version": 1, "description": "d",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [{"fieldId": DEP_FIELD, "order": 0, "required": false}]}),
    );
}

#[test]
fn bundle_roundtrip_standalone_into_repo_with_dependencies_has_zero_errors() {
    let (_t, a) = dep_and_app();
    app_type(&a);
    let dep = export_mode(&a, DEP, BundleMode::Bundled);
    let app = export_mode(&a, APP, BundleMode::Standalone);
    let b = parse(&app);
    assert_eq!(b["mode"], "standalone");
    assert_eq!(b["fields"], json!([]));
    assert_package_invariants(&b);

    let (_tb, dest) = fresh("17575e57-0000-4000-8000-0000000000c3");
    install_package_bundle_bytes(&dest, dep.as_bytes(), Default::default()).unwrap();
    let r = install_package_bundle_bytes(&dest, app.as_bytes(), Default::default()).unwrap();
    assert_eq!(r.installed, 1);
    assert_zero_errors(&dest);
}

/// D4: install does not check `dependencyRefs` (RFC-003 adds no reader rule), so a
/// standalone bundle installed without its dependencies succeeds and leaves a Type
/// `fieldId` dangling, which [R13] makes fatal to the destination's checked catalog.
#[test]
fn bundle_roundtrip_standalone_into_repo_missing_dependencies_leaves_dangling_refs() {
    let (_t, a) = dep_and_app();
    app_type(&a);
    let app = export_mode(&a, APP, BundleMode::Standalone);
    let (_tb, dest) = fresh("17575e57-0000-4000-8000-0000000000c4");
    let r = install_package_bundle_bytes(&dest, app.as_bytes(), Default::default()).unwrap();
    assert_eq!(r.installed, 1);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let report = validate_repository(&dest).unwrap();
    assert!(
        report.diagnostics.iter().any(|d| {
            let d = serde_json::to_string(d).unwrap();
            d.contains("SRS038-R13-DANGLING-REFERENCE") && d.contains(DEP_FIELD)
        }),
        "{:?}",
        report.diagnostics
    );
    assert!(dest.catalog().is_err());
}
