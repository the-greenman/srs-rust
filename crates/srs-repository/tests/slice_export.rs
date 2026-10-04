//! RFC-026 container slice export (srs-rust#631, ADR-051).
//!
//! Fixture: `tests/fixtures/exploded-basic/` (root container holding the purpose
//! record; a `Decisions` container holding the two decisions linked by
//! `precedes`), extended in memory per test.

use serde_json::{json, Value};
use srs_repository::archive::archive_to_tree;
use srs_repository::error::RepositoryError;
use srs_repository::slice_service::{
    export_container_slice, ExportSliceInput, SliceExport, CODE_CHILD_CONTAINER_CYCLE,
    CODE_CHILD_CONTAINER_MISSING, CODE_ROOT_IDENTITY_INVALID,
};
use srs_repository::validation::{validate_repository, DiagnosticSeverity};
use srs_repository::{open_tree, RepositoryStore};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/exploded-basic");
const ROOT: &str = "11111111-1111-4111-8111-111111111111";
const DECISIONS: &str = "55555555-5555-4555-8555-555555555551";
const PURPOSE: &str = "b3b90185-00a6-435f-b322-62fe8ecbeb86";
const D1: &str = "0ce8cbdd-5a77-4740-a34a-83b3afa63a3e";
const D2: &str = "75e926a4-2355-4571-a2de-cdeeb2279224";
const PRECEDES: &str = "44444444-4444-4444-8444-444444444441";
const CUT: &str = "44444444-4444-4444-8444-444444444442";
const CITES: &str = "44444444-4444-4444-8444-444444444443";
const DOC: &str = "77777777-7777-4777-8777-777777777771";
const EMPTY: &str = "55555555-5555-4555-8555-555555555552";
const SUB: &str = "55555555-5555-4555-8555-555555555553";
const SLICE_ID: &str = "99999999-9999-4999-8999-999999999991";

type Tree = BTreeMap<String, Vec<u8>>;

fn fixture() -> Tree {
    fn walk(root: &Path, dir: &Path, map: &mut Tree) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, map);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                map.insert(rel.replace('\\', "/"), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut map = Tree::new();
    walk(Path::new(FIXTURE), Path::new(FIXTURE), &mut map);
    map
}

fn put(tree: &mut Tree, path: &str, v: Value) {
    tree.insert(path.to_string(), serde_json::to_vec_pretty(&v).unwrap());
}

fn get(tree: &Tree, path: &str) -> Value {
    serde_json::from_slice(&tree[path]).unwrap()
}

fn edit(tree: &mut Tree, path: &str, f: impl FnOnce(&mut Value)) {
    let mut v = get(tree, path);
    f(&mut v);
    put(tree, path, v);
}

fn relation(id: &str, ty: &str, s: &str, t: &str) -> Value {
    json!({"$schema": "https://srs.semanticops.com/schema/2.0/relation.json",
           "relationId": id, "relationType": ty, "sourceInstanceId": s,
           "targetInstanceId": t, "createdAt": "2026-07-22T00:00:00Z"})
}

fn sub_package(tree: &mut Tree, dir: &str, id: &str, defs: Value) {
    let mut pkg = json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
        "id": id, "namespace": "com.example.treefix", "name": dir.replace('/', "-"),
        "title": dir, "description": "", "status": "active", "createdAt": "2026-01-01T00:00:00Z",
        "version": "1.0.0", "fields": [], "types": [], "relationTypes": []});
    for (key, rel, def) in defs.as_array().unwrap().iter().map(|d| {
        (
            d[0].as_str().unwrap().to_string(),
            d[1].as_str().unwrap().to_string(),
            d[2].clone(),
        )
    }) {
        pkg[&key].as_array_mut().unwrap().push(json!(rel));
        put(tree, &format!("{dir}/{rel}"), def);
    }
    put(tree, &format!("{dir}/package.json"), pkg);
    edit(tree, "manifest.json", |m| {
        let refs = m
            .as_object_mut()
            .unwrap()
            .entry("packageRefs")
            .or_insert(json!([]));
        refs.as_array_mut()
            .unwrap()
            .push(json!({"mode": "local", "path": dir}));
    });
}

/// The fixture plus: a cut relation (purpose → D1), a `cites` relation type in
/// `packages/extra` used by D1 → D2, an unused `packages/unused`, a source
/// document cited by D1 and an uncited one, an empty container, and a
/// container holding only D1. Neither is a declared child of anything.
fn rich() -> Tree {
    let mut t = fixture();
    put(
        &mut t,
        &format!("relations/{CUT}.json"),
        relation(CUT, "derived-from", PURPOSE, D1),
    );
    put(
        &mut t,
        &format!("relations/{CITES}.json"),
        relation(CITES, "com.example.treefix/cites", D1, D2),
    );
    sub_package(
        &mut t,
        "packages/extra",
        "aaaaaaaa-0000-4000-8000-000000000001",
        json!([["relationTypes", "relation-types/cites.json", {
            "$schema": "https://srs.semanticops.com/schema/2.0/relation-type.json",
            "description": "Source cites target",
            "id": "66666666-6666-4666-8666-666666666662", "key": "com.example.treefix/cites",
            "label": "Cites", "namespace": "com.example.treefix", "version": 1,
            "category": "evidence", "createdAt": "2026-07-22T00:00:00Z"}]]),
    );
    sub_package(
        &mut t,
        "packages/unused",
        "aaaaaaaa-0000-4000-8000-000000000002",
        json!([["fields", "fields/note.json", {
            "id": "22222222-2222-4222-8222-222222222229", "namespace": "com.example.treefix",
            "name": "note", "version": 1, "description": "A note",
            "aiGuidance": {"purpose": "A note."}, "fieldType": {"datatype": "string"},
            "createdAt": "2026-07-22T00:00:00Z"}]]),
    );
    put(
        &mut t,
        "source-documents/cited.md.meta.json",
        json!({"documentId": DOC, "contentPath": "cited.md", "contentType": "text/markdown",
               "createdAt": "2026-01-01T00:00:00Z"}),
    );
    t.insert("source-documents/cited.md".into(), b"# cited".to_vec());
    put(
        &mut t,
        "source-documents/other.md.meta.json",
        json!({"documentId": "77777777-7777-4777-8777-777777777772", "contentPath": "other.md",
               "contentType": "text/markdown", "createdAt": "2026-01-01T00:00:00Z"}),
    );
    t.insert("source-documents/other.md".into(), b"# other".to_vec());
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["$schema"] = json!("https://srs.semanticops.com/schema/2.0/container.json");
    });
    edit(&mut t, "records/tier-2/decision-0ce8cbdd.json", |r| {
        r["sourceRefs"] = json!([{"sourceType": "repository-document", "sourceId": DOC}]);
    });
    put(
        &mut t,
        "containers/empty.json",
        json!({"containerId": "55555555-5555-4555-8555-555555555552", "title": "Empty",
               "memberInstanceIds": []}),
    );
    put(
        &mut t,
        "containers/sub.json",
        json!({"containerId": "55555555-5555-4555-8555-555555555553", "title": "Sub",
               "memberInstanceIds": [{"instanceId": D1}]}),
    );
    t
}

fn export(tree: Tree, container: &str) -> Result<SliceExport, RepositoryError> {
    let store = open_tree(tree).unwrap();
    export_container_slice(
        &store,
        ExportSliceInput {
            container_id: container.to_string(),
            exported_at: Some("2026-10-04T00:00:00Z".to_string()),
            repository_id: Some(SLICE_ID.to_string()),
        },
    )
}

fn files(e: &SliceExport) -> Tree {
    let store = archive_to_tree(Cursor::new(&e.bytes)).unwrap();
    store.as_tree_snapshot().unwrap()
}

fn errors(e: &SliceExport) -> Vec<String> {
    let store = archive_to_tree(Cursor::new(&e.bytes)).unwrap();
    validate_repository(&store)
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .map(|d| d.message)
        .collect()
}

#[test]
fn container_slice_carries_the_closure_and_validates() {
    let e = export(rich(), DECISIONS).unwrap();
    let s = &e.summary;
    assert_eq!(
        (
            s.instance_count,
            s.relation_count,
            s.external_relation_ref_count
        ),
        (2, 2, 1)
    );
    assert_eq!((s.container_count, s.source_document_count), (1, 1));
    assert_eq!(s.origin_repository_id, ROOT);
    let f = files(&e);
    assert!(f.contains_key("records/tier-2/decision-0ce8cbdd.json"));
    assert!(!f.contains_key("records/tier-2/purpose-b3b90185.json"));
    assert!(f.contains_key(&format!("relations/{PRECEDES}.json")));
    assert!(f.contains_key(&format!("relations/{CITES}.json")));
    assert!(!f.contains_key(&format!("relations/{CUT}.json")));
    // RFC-034 [R9] / I-151: undeclared containers are never carried, even
    // `sub`, whose entries are all included.
    assert!(!f.contains_key("containers/sub.json"));
    assert!(!f.contains_key("containers/empty.json"));
    // The boundary is inline in the manifest, not a file.
    assert!(!f.contains_key("containers/decisions-55555555.json"));
    // Step 5: cited sidecar and its content only.
    assert!(f.contains_key("source-documents/cited.md.meta.json"));
    assert!(f.contains_key("source-documents/cited.md"));
    assert!(!f.contains_key("source-documents/other.md.meta.json"));
    assert!(!f.contains_key("README.md"));
    assert_eq!(errors(&e), Vec::<String>::new());
}

#[test]
fn manifest_is_rewritten_per_rfc026() {
    let e = export(rich(), DECISIONS).unwrap();
    let m = get(&files(&e), "manifest.json");
    assert_eq!(m["repositoryId"], SLICE_ID);
    assert_eq!(m["container"]["containerId"], DECISIONS);
    assert_eq!(m["namespace"], "com.example.treefix", "source keys kept");
    assert!(m["declaredExtensions"]
        .as_array()
        .unwrap()
        .contains(&json!("ext:slices")));
    assert_eq!(
        m["slice"],
        json!({
            "origin": {"repositoryId": ROOT},
            "spec": {"type": "container", "id": DECISIONS},
            "exportedAt": "2026-10-04T00:00:00Z",
            "externalRelationRefs": [{"relationId": CUT, "sourceInstanceId": PURPOSE,
                                      "targetInstanceId": D1, "relationType": "derived-from"}]
        })
    );
}

#[test]
fn packages_are_carried_whole_when_used_and_dropped_otherwise() {
    let e = export(rich(), DECISIONS).unwrap();
    let f = files(&e);
    assert_eq!(e.summary.package_count, 2);
    for (path, bytes) in rich().iter().filter(|(p, _)| p.starts_with("package/")) {
        assert_eq!(f.get(path), Some(bytes), "{path} carried unchanged");
    }
    assert!(f.contains_key("packages/extra/relation-types/cites.json"));
    assert!(!f.contains_key("packages/unused/package.json"));
    let refs = get(&f, "manifest.json")["packageRefs"].clone();
    assert_eq!(refs, json!([{"mode": "local", "path": "packages/extra"}]));
}

#[test]
fn root_slice_omits_relations_with_both_endpoints_outside() {
    let e = export(rich(), ROOT).unwrap();
    let refs = get(&files(&e), "manifest.json")["slice"]["externalRelationRefs"].clone();
    assert_eq!(refs.as_array().unwrap().len(), 1, "only the purpose edge");
    assert_eq!(refs[0]["relationId"], CUT);
    assert_eq!(e.summary.relation_count, 0);
    assert_eq!(e.summary.package_count, 1, "cites is unused here");
    assert_eq!(errors(&e), Vec::<String>::new());
}

#[test]
fn nested_outline_is_kept_as_is() {
    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["memberInstanceIds"] = json!([{"instanceId": D1}, {"instanceId": D2, "depth": 1}]);
    });
    let e = export(t, DECISIONS).unwrap();
    let m = get(&files(&e), "manifest.json");
    assert_eq!(
        m["container"]["memberInstanceIds"],
        json!([{"instanceId": D1}, {"instanceId": D2, "depth": 1}])
    );
    assert_eq!(errors(&e), Vec::<String>::new());
}

#[test]
fn identity_entry_with_descendants_is_refused() {
    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["identityInstanceId"] = json!(D1);
        c["memberInstanceIds"] = json!([{"instanceId": D1}, {"instanceId": D2, "depth": 1}]);
    });
    match export(t, DECISIONS) {
        Err(RepositoryError::SliceRefused { code, .. }) => {
            assert_eq!(code, CODE_ROOT_IDENTITY_INVALID)
        }
        other => panic!("expected refusal, got {other:?}"),
    }
}

#[test]
fn unknown_container_is_not_found() {
    assert!(matches!(
        export(rich(), "55555555-5555-4555-8555-55555555555f"),
        Err(RepositoryError::ContainerNotFound { .. })
    ));
}

#[test]
fn export_is_byte_deterministic() {
    assert_eq!(
        export(rich(), DECISIONS).unwrap().bytes,
        export(rich(), DECISIONS).unwrap().bytes
    );
}

#[test]
fn validator_checks_the_slice_block() {
    let e = export(rich(), DECISIONS).unwrap();
    let mut f = files(&e);
    edit(&mut f, "manifest.json", |m| {
        m["repositoryId"] = json!(ROOT);
        m["slice"]["spec"]["id"] = json!(ROOT);
        m["declaredExtensions"] = json!([]);
    });
    let report = validate_repository(&open_tree(f).unwrap()).unwrap();
    let has = |sev: DiagnosticSeverity, needle: &str| {
        report
            .diagnostics
            .iter()
            .any(|d| d.severity == sev && d.message.contains(needle))
    };
    assert!(has(DiagnosticSeverity::Error, "RFC-026 [R3]"));
    assert!(has(DiagnosticSeverity::Error, "RFC-026 [R12]"));
    assert!(has(DiagnosticSeverity::Warning, "RFC-026 [R4]"));
    assert!(has(
        DiagnosticSeverity::Info,
        "1 cross-boundary relation(s) were cut"
    ));
}

/// ADR-045: an ordinary operation sees the checked catalog, so a dangling
/// entry refuses the export (repair first) rather than being cut silently.
#[test]
fn dangling_boundary_entry_refuses_the_export() {
    let ghost = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["memberInstanceIds"] = json!([{"instanceId": ghost}, {"instanceId": D1}]);
    });
    assert!(matches!(
        export(t, DECISIONS),
        Err(RepositoryError::CatalogLoad { .. })
    ));
}

#[test]
fn source_documents_cited_by_carried_relations_are_carried() {
    let mut t = rich();
    edit(&mut t, &format!("relations/{PRECEDES}.json"), |r| {
        r["sourceRefs"] = json!([{"sourceType": "repository-document",
                                  "sourceId": "77777777-7777-4777-8777-777777777772"}]);
    });
    let e = export(t, DECISIONS).unwrap();
    assert_eq!(e.summary.source_document_count, 2);
    assert!(files(&e).contains_key("source-documents/other.md"));
    assert_eq!(errors(&e), Vec::<String>::new());
}

/// RFC-034 [R9]: no subset test. The source root's only entry is included
/// here, but the root is not a declared child, so it is not carried.
#[test]
fn undeclared_container_is_not_carried_even_when_its_entries_are_included() {
    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["memberInstanceIds"] =
            json!([{"instanceId": PURPOSE}, {"instanceId": D1}, {"instanceId": D2}]);
    });
    let e = export(t, DECISIONS).unwrap();
    assert_eq!(e.summary.container_count, 1);
    assert!(!files(&e).contains_key(&format!("containers/{ROOT}.json")));
    assert_eq!(errors(&e), Vec::<String>::new());
}

/// RFC-034 [R9] / I-151: the declared descendant closure, transitively —
/// Decisions -> Empty -> Sub. Sub's entry (the purpose record, outside the
/// boundary's own entries) joins the slice, so the cut edge becomes internal;
/// the memberless Empty is still carried, with its edge kept.
#[test]
fn declared_descendants_are_carried_with_their_entries() {
    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["childContainerIds"] = json!([EMPTY]);
    });
    edit(&mut t, "containers/empty.json", |c| {
        c["childContainerIds"] = json!([SUB]);
    });
    edit(&mut t, "containers/sub.json", |c| {
        c["memberInstanceIds"] = json!([{"instanceId": PURPOSE}]);
    });
    let e = export(t, DECISIONS).unwrap();
    let s = &e.summary;
    assert_eq!(
        (
            s.instance_count,
            s.relation_count,
            s.external_relation_ref_count,
            s.container_count
        ),
        (3, 3, 0, 3)
    );
    let f = files(&e);
    assert!(f.contains_key("records/tier-2/purpose-b3b90185.json"));
    assert_eq!(
        get(&f, "containers/empty.json")["childContainerIds"],
        json!([SUB])
    );
    assert!(f.contains_key("containers/sub.json"));
    assert_eq!(
        get(&f, "manifest.json")["container"]["childContainerIds"],
        json!([EMPTY])
    );
    assert_eq!(errors(&e), Vec::<String>::new());
}

fn refusal(r: Result<SliceExport, RepositoryError>) -> &'static str {
    match r {
        Err(RepositoryError::SliceRefused { code, .. }) => code,
        Err(e) => panic!("expected a slice refusal, got {e}"),
        Ok(_) => panic!("expected a slice refusal, got an export"),
    }
}

/// RFC-034 [R7]: a missing child or a cycle leaves the closure undefined, so
/// the export is refused rather than copying a dangling id or cutting short.
#[test]
fn broken_child_graph_refuses_the_export() {
    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["childContainerIds"] = json!(["55555555-5555-4555-8555-55555555ffff"]);
    });
    assert_eq!(refusal(export(t, DECISIONS)), CODE_CHILD_CONTAINER_MISSING);

    let mut t = rich();
    edit(&mut t, "containers/decisions-55555555.json", |c| {
        c["childContainerIds"] = json!([SUB]);
    });
    edit(&mut t, "containers/sub.json", |c| {
        c["childContainerIds"] = json!([DECISIONS]);
    });
    assert_eq!(refusal(export(t, DECISIONS)), CODE_CHILD_CONTAINER_CYCLE);
}

#[test]
fn package_dependencies_of_a_carried_package_are_carried() {
    let mut t = rich();
    edit(&mut t, "packages/extra/package.json", |p| {
        p["packageDependencies"] = json!([{"packageId": "aaaaaaaa-0000-4000-8000-000000000002",
            "namespace": "com.example.treefix", "name": "packages-unused", "version": "1.0.0"}]);
    });
    let e = export(t, DECISIONS).unwrap();
    assert_eq!(e.summary.package_count, 3);
    assert!(files(&e).contains_key("packages/unused/fields/note.json"));
    assert_eq!(errors(&e), Vec::<String>::new());
}
