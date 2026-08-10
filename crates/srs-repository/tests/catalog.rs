//! RFC-038 catalog conformance tests (srs-rust#783 Phase 1).
//!
//! The vendored fixture repo is rev-2-with-index and un-migrated (the Phase-6
//! enforcement flip owns fixture migration): the catalog must IGNORE the
//! manifest for membership and still reproduce the known identity set — and
//! it must fail loudly on the srs#307-shaped dangling `fieldId` the fixture
//! carries, which is RFC-038 Change H's own acceptance test.

use srs_repository::catalog::{self, codes, CatalogKind};
use srs_repository::error::RepositoryError;
use srs_repository::store::memory::MemoryStore;
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::DiagnosticSeverity;
use srs_repository::vfs::MemVfs;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/spec-repo")
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const MINIMAL_MANIFEST: &str = r#"{"instanceIndex": []}"#;

fn note_json(id: &str) -> String {
    format!(r#"{{"instanceId": "{id}", "sections": [{{"name": "body", "content": "x"}}]}}"#)
}

fn record_json(id: &str) -> String {
    format!(
        r#"{{"instanceId": "{id}", "typeId": "9c56b6ae-0000-4000-8000-000000000001", "typeVersion": 1, "typeNamespace": "com.test", "typeName": "thing", "fieldValues": {{"title": "x"}}}}"#
    )
}

fn srs_package_json() -> String {
    r#"{
      "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
      "id": "7d0a1c9e-0000-4000-8000-000000000001",
      "namespace": "com.test",
      "name": "pkg",
      "version": "1.0.0",
      "title": "pkg",
      "description": "",
      "status": "active",
      "createdAt": "2026-01-01T00:00:00Z",
      "fields": [],
      "types": []
    }"#
    .to_string()
}

fn code_counts(catalog: &catalog::RepositoryCatalog) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for d in &catalog.diagnostics {
        *counts.entry(d.code).or_insert(0) += 1;
    }
    counts
}

// ---------------------------------------------------------------------------
// Vendored fixture: identity set + corpus-calibrated diagnostics
// ---------------------------------------------------------------------------

#[test]
fn fixture_catalog_reproduces_identity_sets_ignoring_manifest() {
    let store = FileStore::new(fixture_repo());
    let cat = catalog::build(&store).unwrap();

    // The known instance identity set. The manifest's instanceIndex happens to
    // record it, so it doubles as the expected-value oracle — but the catalog
    // derived membership from the tree, not the index.
    let manifest_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture_repo().join("manifest.json")).unwrap(),
    )
    .unwrap();
    let indexed_ids: BTreeSet<String> = manifest_json["instanceIndex"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["instanceId"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(indexed_ids.len(), 266);

    let catalog_ids: BTreeSet<String> = cat.instances.iter().map(|e| e.id.clone()).collect();
    assert_eq!(catalog_ids, indexed_ids);
    assert_eq!(cat.instances.len(), 266);
    let notes = cat
        .instances
        .iter()
        .filter(|e| e.kind == CatalogKind::Note)
        .count();
    let records = cat
        .instances
        .iter()
        .filter(|e| e.kind == CatalogKind::Record)
        .count();
    assert_eq!((notes, records), (19, 247));
    assert!(cat
        .instances
        .iter()
        .all(|e| e.tier == Some(if e.kind == CatalogKind::Note { 0 } else { 2 })));

    // Relations: enumerated from the (transitional) collection file.
    assert_eq!(cat.relations.len(), 189);
    assert!(cat
        .relations
        .iter()
        .all(|e| e.kind == CatalogKind::Relation && e.tier.is_none()));

    // Containers: two files plus the inline root container ([R1]).
    assert_eq!(cat.containers.len(), 3);
    assert!(cat
        .containers
        .iter()
        .any(|e| e.locator.as_deref() == Some("manifest.json#/container")));

    // Source documents: the four sidecars; documentId read from the sidecar.
    assert_eq!(cat.source_documents.len(), 4);

    // Extension set: nothing declared.
    assert!(cat.extensions.is_empty());

    // Definitions: 156 unique declared paths (the primary package and
    // spec-rfc-process declare 4 relation-type files in common — one object
    // each, enumerated once; [R12] applies to distinct objects sharing an
    // id, not to one file declared twice). Two candidates fail
    // classification (below), so 154 classify.
    assert_eq!(cat.definitions.len(), 154);

    // Corpus-calibrated diagnostics, all errors:
    // - the srs#307-shaped dangling fieldId in 3 Type definitions ([R13])
    // - `protocol-tags.json` fails its declared field.json (valueDomain
    //   "closed" with neither allowedValues nor vocabularyRef) ([R7]) —
    //   which drops its id from the definition set, making meta.protocol's
    //   FieldAssignment a 4th dangling reference ([R13])
    // - 1 no-$schema document view failing shape classification ([R8])
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::DANGLING_REFERENCE), Some(&4));
    assert_eq!(counts.get(codes::SCHEMA_VALIDATION), Some(&1));
    assert_eq!(counts.get(codes::SHAPE_NO_MATCH), Some(&1));
    assert_eq!(cat.diagnostics.len(), 6);
    assert!(cat
        .diagnostics
        .iter()
        .all(|d| d.severity == DiagnosticSeverity::Error));
    let srs307: Vec<_> = cat
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("f1a2b3c4-d5e6-4a7b-8c9d-0e1f2a3b4c5c"))
        .collect();
    assert_eq!(srs307.len(), 3, "the srs#307 calibration case ([R13])");

    // [R24]: fatal diagnostics fail the load as a whole — RFC-038 Change H's
    // acceptance test ("a conforming implementation of [R13] fails loudly on
    // that repository").
    match store.catalog() {
        Err(RepositoryError::CatalogLoad {
            fatal, diagnostics, ..
        }) => {
            assert_eq!(fatal, 6);
            assert_eq!(diagnostics.len(), 6);
        }
        other => panic!("expected CatalogLoad error, got {other:?}"),
    }
}

#[test]
fn fixture_catalog_is_deterministic_and_r14_ordered() {
    let store = FileStore::new(fixture_repo());
    let a = catalog::build(&store).unwrap();
    let b = catalog::build(&store).unwrap();
    assert_eq!(a, b);
    for set in [
        &a.instances,
        &a.relations,
        &a.containers,
        &a.source_documents,
    ] {
        let ids: Vec<&String> = set.iter().map(|e| &e.id).collect();
        let mut sorted = ids.clone();
        sorted.sort(); // byte-wise ascending
        assert_eq!(ids, sorted);
    }
}

// ---------------------------------------------------------------------------
// Membership is the tree, not the manifest
// ---------------------------------------------------------------------------

#[test]
fn catalog_ignores_manifest_index_for_membership() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Index lists a phantom instance and omits the real one on disk.
    write(
        root,
        "manifest.json",
        r#"{"instanceIndex": [{"instanceId": "00000000-0000-4000-8000-00000000dead", "tier": 0, "path": "records/phantom.json"}]}"#,
    );
    write(
        root,
        "records/real.json",
        &note_json("00000000-0000-4000-8000-000000000001"),
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let ids: Vec<&str> = cat.instances.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["00000000-0000-4000-8000-000000000001"]);
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
}

// ---------------------------------------------------------------------------
// Diagnostic cases
// ---------------------------------------------------------------------------

#[test]
fn duplicate_instance_id_names_every_locator() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    let id = "00000000-0000-4000-8000-000000000042";
    write(root, "records/a.json", &note_json(id));
    write(root, "records/b.json", &record_json(id));
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let dup: Vec<_> = cat
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::DUPLICATE_ID)
        .collect();
    assert_eq!(dup.len(), 1);
    assert_eq!(
        dup[0].locators,
        vec!["records/a.json".to_string(), "records/b.json".to_string()]
    );
    assert_eq!(dup[0].severity, DiagnosticSeverity::Error);
}

#[test]
fn duplicate_definition_id_across_distinct_files_names_every_locator() {
    // The muSrs-style case [R12] exists for: two distinct files declaring the
    // same definition id (and version) — not silently coalesced.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    let field = |name: &str| {
        format!(
            r#"{{"$schema": "https://srs.semanticops.com/schema/2.0/field.json", "id": "0f0f0f0f-0000-4000-8000-000000000001", "namespace": "com.test", "name": "{name}", "version": 1, "description": "d", "aiGuidance": {{"purpose": "p"}}, "fieldType": {{"datatype": "string"}}, "createdAt": "2026-01-01T00:00:00Z"}}"#
        )
    };
    write(
        root,
        "package/package.json",
        &srs_package_json().replace(
            r#""fields": [],"#,
            r#""fields": ["fields/a.json", "fields/b.json"],"#,
        ),
    );
    write(root, "package/fields/a.json", &field("f1"));
    write(root, "package/fields/b.json", &field("f1"));
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let dup: Vec<_> = cat
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::DUPLICATE_ID)
        .collect();
    assert_eq!(dup.len(), 1, "{:?}", cat.diagnostics);
    assert_eq!(
        dup[0].locators,
        vec![
            "package/fields/a.json".to_string(),
            "package/fields/b.json".to_string()
        ]
    );
}

#[test]
fn malformed_candidate_fatal_inside_reserved_location_untouched_outside() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/good.json",
        &note_json("00000000-0000-4000-8000-000000000001"),
    );
    write(root, "docs/broken.json", "{ not json");
    let store = FileStore::new(root);
    // Outside every reserved location: application content, untouched ([R10]).
    assert!(store.catalog().is_ok());

    write(root, "records/broken.json", "{ not json");
    let cat = catalog::build(&store).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::CANDIDATE_MALFORMED), Some(&1));
    // [R24]: fatal — the load fails as a whole rather than dropping one file.
    assert!(matches!(
        store.catalog(),
        Err(RepositoryError::CatalogLoad { .. })
    ));
}

#[test]
fn revisions_sidecar_orphaned_vs_schemaless() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/a.json",
        &note_json("00000000-0000-4000-8000-000000000001"),
    );
    // Base resolves to a discovered instance, but the suffix has no declared
    // schema (owed by RFC-038) — recognition is not conferred by filename.
    write(root, "records/a.revisions.json", "{}");
    // Base resolves to nothing: orphaned.
    write(root, "records/b.revisions.json", "{}");
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::SIDECAR_SCHEMA), Some(&1));
    assert_eq!(counts.get(codes::SIDECAR_ORPHANED), Some(&1));
}

#[test]
fn near_miss_package_manifest_diagnosed_and_does_not_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    // Declares the SRS schema but misses required properties ([R4]).
    write(
        root,
        "pkg/package.json",
        r#"{"$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json", "id": "x", "namespace": "com.test", "name": "p", "version": "1.0.0", "fields": [], "types": []}"#,
    );
    // A records/ under the near-miss must NOT become an instance root.
    write(
        root,
        "pkg/records/orphan.json",
        &note_json("00000000-0000-4000-8000-000000000009"),
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::PACKAGE_MANIFEST_INVALID), Some(&1));
    assert!(cat.instances.is_empty());
}

#[test]
fn npm_package_json_is_ignored_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "web/package.json",
        r#"{"name": "web", "version": "1.0.0", "dependencies": {"react": "^18.0.0"}}"#,
    );
    let store = FileStore::new(root);
    let cat = catalog::build(&store).unwrap();
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    assert!(store.catalog().is_ok());
}

#[test]
fn nested_records_under_package_root_classified_as_instances_by_content() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(root, "pkg/package.json", &srs_package_json());
    // Record-shaped object under <package>/records/notes/: tier comes from
    // content ([R6]), never from the directory name — and the nested instance
    // root takes precedence over the package root ([R5]/[R8]).
    write(
        root,
        "pkg/records/notes/actually-a-record.json",
        &record_json("00000000-0000-4000-8000-000000000077"),
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    assert_eq!(cat.instances.len(), 1);
    assert_eq!(cat.instances[0].kind, CatalogKind::Record);
    assert_eq!(cat.instances[0].tier, Some(2));
}

#[test]
fn dangling_field_assignment_diagnosed_per_target_set() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "package/package.json",
        r#"{
          "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
          "id": "7d0a1c9e-0000-4000-8000-000000000001",
          "namespace": "com.test", "name": "pkg", "version": "1.0.0",
          "title": "pkg", "description": "", "status": "active",
          "createdAt": "2026-01-01T00:00:00Z",
          "fields": [], "types": ["types/t.json"]
        }"#,
    );
    write(
        root,
        "package/types/t.json",
        r#"{
          "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
          "id": "9c56b6ae-0000-4000-8000-000000000001",
          "namespace": "com.test", "name": "thing", "version": 1,
          "description": "t", "createdAt": "2026-01-01T00:00:00Z",
          "fields": [{"fieldId": "f1a2b3c4-0000-4000-8000-00000000beef", "order": 1, "required": true}]
        }"#,
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let dangling: Vec<_> = cat
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::DANGLING_REFERENCE)
        .collect();
    assert_eq!(dangling.len(), 1);
    assert!(dangling[0].message.contains("fieldId"));
    assert!(dangling[0]
        .message
        .contains("f1a2b3c4-0000-4000-8000-00000000beef"));
    assert_eq!(
        dangling[0].locators,
        vec!["package/types/t.json".to_string()]
    );
}

#[test]
fn sidecar_without_document_id_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "source-documents/x.md.meta.json",
        r#"{"contentPath": "x.md", "contentType": "text/markdown", "createdAt": "2026-01-01T00:00:00Z"}"#,
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::SIDECAR_NO_DOCUMENT_ID), Some(&1));
    assert!(cat.source_documents.is_empty());
}

#[test]
fn sidecar_wrong_schema_is_inadmissible_unknown_is_unresolvable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    // Registered but wrong entity for this location → [R7] inadmissible.
    write(
        root,
        "source-documents/a.md.meta.json",
        r#"{"$schema": "https://srs.semanticops.com/schema/2.0/note.json", "documentId": "aaaa0000-0000-4000-8000-000000000001", "contentPath": "a.md", "contentType": "text/markdown", "createdAt": "2026-01-01T00:00:00Z"}"#,
    );
    // Unknown URL → [R7] unresolvable.
    write(
        root,
        "source-documents/b.md.meta.json",
        r#"{"$schema": "https://example.com/nope.json", "documentId": "aaaa0000-0000-4000-8000-000000000002", "contentPath": "b.md", "contentType": "text/markdown", "createdAt": "2026-01-01T00:00:00Z"}"#,
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::SCHEMA_INADMISSIBLE), Some(&1));
    assert_eq!(counts.get(codes::SCHEMA_UNRESOLVABLE), Some(&1));
    assert!(cat.source_documents.is_empty());
}

#[test]
fn sidecar_with_absent_content_file_is_a_valid_tombstone() {
    // The vendored custom-source-path fixture: sourceDocumentsPath =
    // "attachments", sidecar present, content file absent ([R15] tombstone) —
    // and the custom path is honoured.
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/custom-source-path-repo");
    let store = FileStore::new(root);
    let cat = store.catalog().unwrap();
    assert_eq!(cat.source_documents.len(), 1);
    assert_eq!(
        cat.source_documents[0].locator.as_deref(),
        Some("attachments/doc.md.meta.json")
    );
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
}

#[test]
fn malformed_sidecar_fixture_is_fatal() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/malformed-sidecar-repo");
    let store = FileStore::new(root);
    let cat = catalog::build(&store).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::CANDIDATE_MALFORMED), Some(&1));
    assert!(matches!(
        store.catalog(),
        Err(RepositoryError::CatalogLoad { .. })
    ));
}

#[test]
fn opaque_source_payloads_are_never_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    // Unclaimed payloads, including broken JSON, under sourceDocumentsPath:
    // opaque, preserved, no diagnostics ([R9]).
    write(root, "source-documents/raw.md", "# not parsed");
    write(root, "source-documents/data.json", "{ not json");
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    assert!(cat.source_documents.is_empty());
}

// ---------------------------------------------------------------------------
// Extension set
// ---------------------------------------------------------------------------

#[test]
fn extension_set_enumerates_with_kind_id_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "manifest.json",
        r#"{
          "instanceIndex": [],
          "repositoryId": "0e0e0e0e-0000-4000-8000-000000000001",
          "declaredExtensions": ["ext:changelog", "ext:federation"],
          "changelogPath": "changelog.json",
          "federationPath": "federation/registry.json",
          "federationEventsPath": "federation/events.json"
        }"#,
    );
    write(root, "changelog.json", r#"{"entries": []}"#);
    write(
        root,
        "federation/registry.json",
        r#"{"registryId": "aaaa0000-0000-4000-8000-000000000002", "repositories": []}"#,
    );
    write(root, "federation/events.json", r#"{"events": []}"#);
    let store = FileStore::new(root);
    let cat = store.catalog().unwrap();
    let entries: Vec<(&str, &str)> = cat
        .extensions
        .iter()
        .map(|e| (e.kind.as_str(), e.id.as_str()))
        .collect();
    // [R14]: extension set orders by kind byte-wise, then id. Identity is the
    // {kind, id} pair: changelog and events both project the owning
    // repositoryId without colliding.
    assert_eq!(
        entries,
        vec![
            ("changelog", "0e0e0e0e-0000-4000-8000-000000000001"),
            ("federation-event", "0e0e0e0e-0000-4000-8000-000000000001"),
            (
                "federation-registry",
                "aaaa0000-0000-4000-8000-000000000002"
            ),
        ]
    );
    assert!(cat.extensions.iter().all(|e| e.tier.is_none()));
}

#[test]
fn extension_locations_require_the_owning_extension_declared() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Paths named, files present — but no owning extension declared ([R5]).
    write(
        root,
        "manifest.json",
        r#"{
          "instanceIndex": [],
          "repositoryId": "0e0e0e0e-0000-4000-8000-000000000001",
          "changelogPath": "changelog.json",
          "federationPath": "federation/registry.json"
        }"#,
    );
    write(root, "changelog.json", r#"{"entries": []}"#);
    write(root, "federation/registry.json", r#"{"registryId": "r"}"#);
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert!(cat.extensions.is_empty());
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
}

// ---------------------------------------------------------------------------
// [R14] platform independence + store equivalence, [R16] validity token
// ---------------------------------------------------------------------------

fn two_note_tree() -> Vec<(&'static str, String)> {
    // File order (a.json, b.json) deliberately reverses id order so any
    // implementation leaking directory iteration order fails the assertion.
    vec![
        ("manifest.json", MINIMAL_MANIFEST.to_string()),
        (
            "records/a.json",
            note_json("ffffffff-0000-4000-8000-000000000002"),
        ),
        (
            "records/b.json",
            note_json("00000000-0000-4000-8000-000000000001"),
        ),
    ]
}

#[test]
fn r14_ordering_is_identical_across_disk_and_memory_backends() {
    let tmp = tempfile::tempdir().unwrap();
    for (rel, content) in two_note_tree() {
        write(tmp.path(), rel, &content);
    }
    let disk = FileStore::new(tmp.path());

    let mem_vfs = MemVfs::new();
    for (rel, content) in two_note_tree() {
        srs_repository::vfs::Vfs::write(&mem_vfs, rel, content.as_bytes()).unwrap();
    }
    let mem = FileStore::from_vfs(Rc::new(mem_vfs));

    let disk_cat = disk.catalog().unwrap();
    let mem_cat = mem.catalog().unwrap();
    assert_eq!(disk_cat, mem_cat);
    let ids: Vec<&str> = disk_cat.instances.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "00000000-0000-4000-8000-000000000001",
            "ffffffff-0000-4000-8000-000000000002",
        ]
    );
    // [R16]: same logical content ⇒ same token, across backends.
    assert_eq!(
        disk.catalog_validity_token().unwrap(),
        mem.catalog_validity_token().unwrap()
    );
}

#[test]
fn validity_token_tracks_the_enumerated_id_set() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/a.json",
        &note_json("00000000-0000-4000-8000-000000000001"),
    );
    let store = FileStore::new(root);
    let t1 = store.catalog_validity_token().unwrap();
    assert_eq!(t1, store.catalog_validity_token().unwrap(), "stable");
    write(
        root,
        "records/b.json",
        &note_json("00000000-0000-4000-8000-000000000002"),
    );
    let t2 = store.catalog_validity_token().unwrap();
    assert_ne!(t1, t2, "token changes when the enumerable id set changes");
}

// ---------------------------------------------------------------------------
// MemoryStore: same walker over its object maps
// ---------------------------------------------------------------------------

#[test]
fn memory_store_catalog_enumerates_saved_instances() {
    let store = MemoryStore::empty();
    let note = srs_core::types::note::Note {
        instance_id: "00000000-0000-4000-8000-000000000abc".to_string(),
        title: Some("t".to_string()),
        tags: None,
        sections: vec![srs_core::types::note::NoteSection {
            name: "body".to_string(),
            label: None,
            content: "x".to_string(),
            content_hint: None,
            tags: None,
        }],
        graduated_at: None,
        source_refs: None,
        created_at: None,
        updated_at: None,
        meta: None,
    };
    store.save_note(&note).unwrap();
    let cat = store.catalog().unwrap();
    assert_eq!(cat.instances.len(), 1);
    assert_eq!(cat.instances[0].id, "00000000-0000-4000-8000-000000000abc");
    assert_eq!(cat.instances[0].kind, CatalogKind::Note);
    assert_eq!(cat.instances[0].tier, Some(0));
    // The minimal in-memory package manifest is a presence-keyed anchor with
    // no declared definitions.
    assert!(cat.definitions.is_empty());
    assert!(store.catalog_validity_token().is_ok());
}

// ---------------------------------------------------------------------------
// [R8] standing check
// ---------------------------------------------------------------------------

#[test]
fn instance_schema_discriminators_hold() {
    // Change C: each instance schema keeps a required property that is not a
    // declared property of the other two, and all are
    // additionalProperties: false. A schema edit that breaks this must fail
    // here rather than silently degrade classification.
    assert_eq!(catalog::instance_discriminator_error(), None);
}
