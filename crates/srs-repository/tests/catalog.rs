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

const MINIMAL_MANIFEST: &str = r#"{"dataModelRevision": 2}"#;

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

    // The known instance identity set. Re-vendored from `srs` master at
    // origin/master tip (post srs#505 Tier-1 retirement + container-anchor,
    // srs-rust#887/#893): the previous 402 was the figure for the corpus
    // before that Tier-1 removal / container-anchor content shift.
    let catalog_ids: BTreeSet<String> = cat.instances.iter().map(|e| e.id.clone()).collect();
    assert_eq!(catalog_ids.len(), 403);
    assert_eq!(cat.instances.len(), 403);
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
    assert_eq!((notes, records), (27, 376));
    assert!(cat
        .instances
        .iter()
        .all(|e| e.tier == Some(if e.kind == CatalogKind::Note { 0 } else { 2 })));

    // Relations: one standalone object per relation ([R11]), no collection file.
    assert_eq!(cat.relations.len(), 229);
    assert!(cat
        .relations
        .iter()
        .all(|e| e.kind == CatalogKind::Relation && e.tier.is_none()));

    // Containers: twelve files plus the inline root container ([R1]).
    assert_eq!(cat.containers.len(), 13);
    assert!(cat
        .containers
        .iter()
        .any(|e| e.locator.as_deref() == Some("manifest.json#/container")));

    assert_eq!(cat.source_documents.len(), 5);

    // Extension set: nothing declared.
    assert!(cat.extensions.is_empty());

    assert_eq!(cat.definitions.len(), 271);

    // The corpus is clean. Every defect the previous vendored copy was
    // calibrated against — the srs#307 dangling `FieldAssignment.fieldId`, the
    // `protocol-tags.json` [R7] failure, the no-$schema document view ([R8]) —
    // is fixed upstream (the-greenman/srs#307 closed; protocol-tags.json
    // deleted; the document view carries its $schema). Those were fixture
    // staleness, not implementation behaviour, so the calibration is now
    // "zero diagnostics" rather than an enumeration of known damage. The
    // catalog's error-reporting paths are exercised by the purpose-built
    // fixtures in this file, not by hoping the vendored corpus stays broken.
    assert_eq!(code_counts(&cat), BTreeMap::new());
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);

    // [R24] has nothing to fire on, so the checked catalog loads.
    store
        .catalog()
        .expect("a clean corpus must load through the checked catalog");
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
fn catalog_denies_a_manifest_still_carrying_an_index() {
    // Post-flip a manifest with `instanceIndex` is not "ignored" — it is an
    // [R2] error, fatal to the load (no partial catalog).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "manifest.json",
        r#"{"dataModelRevision": 2, "instanceIndex": [{"instanceId": "00000000-0000-4000-8000-00000000dead", "tier": 0, "path": "records/phantom.json"}]}"#,
    );
    write(
        root,
        "records/real.json",
        &note_json("00000000-0000-4000-8000-000000000001"),
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    let counts = code_counts(&cat);
    assert_eq!(counts.get(codes::MANIFEST_INVALID), Some(&1), "{cat:?}");
    assert!(cat.instances.is_empty());
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

/// `.revisions.json` is no longer a recognised sidecar (rfc-decision-2a1e1590,
/// srs-rust#866): recognition-by-filename is retired along with the
/// mechanism it backed. A bare (no `$schema`) sidecar body — the shape
/// `revision_service` used to write — now falls through to ordinary
/// instance-candidate classification like any other object under [R8]/[R9],
/// matches no admissible shape, and errors — mirroring
/// `tier1_typed_record_shape_no_longer_classifies` below for the analogous
/// Tier-1 retirement.
#[test]
fn revisions_sidecar_shape_no_longer_classifies() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/a.json",
        &note_json("00000000-0000-4000-8000-000000000001"),
    );
    // The legacy sidecar shape: no `$schema`, `recordId` + `revisions[]`.
    write(
        root,
        "records/a.revisions.json",
        r#"{"recordId": "00000000-0000-4000-8000-000000000001", "revisions": []}"#,
    );
    let store = FileStore::new(root);
    let cat = catalog::build(&store).unwrap();
    assert_eq!(
        code_counts(&cat).get(codes::SHAPE_NO_MATCH),
        Some(&1),
        "{:?}",
        cat.diagnostics
    );

    // [R24]: the checked seam every ordinary command uses refuses the load
    // entirely — exactly why the cleanup migration (srs-rust#866,
    // `revisions-sidecar-cleanup`) exists rather than leaving these tolerated.
    assert!(
        store.catalog().is_err(),
        "store.catalog() must be fatal while a .revisions.json sidecar remains"
    );
}

/// A `.revisions.json` file declaring `$schema: .../revisions.json` is
/// likewise unresolvable now that the schema itself is retired (deleted from
/// the mirror, srs-rust#866) — mirrors
/// `declared_typed_record_schema_is_unresolvable` for the analogous Tier-1
/// retirement.
#[test]
fn declared_revisions_schema_is_unresolvable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/a.revisions.json",
        r#"{"$schema": "https://srs.semanticops.com/schema/2.0/revisions.json", "recordId": "00000000-0000-4000-8000-000000000001", "revisions": []}"#,
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert_eq!(
        code_counts(&cat).get(codes::SCHEMA_UNRESOLVABLE),
        Some(&1),
        "{:?}",
        cat.diagnostics
    );
}

/// [R8] shape-matching for a declared protocol definition: it carries no
/// `$schema`, so it can only classify once `protocol.json` is registered.
#[test]
fn declared_protocol_definition_classifies_by_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    let pkg: serde_json::Value = serde_json::from_str(&srs_package_json()).unwrap();
    let mut pkg = pkg.as_object().unwrap().clone();
    pkg.insert(
        "protocols".to_string(),
        serde_json::json!(["protocols/entry.json"]),
    );
    write(
        root,
        "pkg/package.json",
        &serde_json::to_string(&pkg).unwrap(),
    );
    let protocol = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/install-package/protocols/entry-9a1b0c90.json"),
    )
    .unwrap();
    write(root, "pkg/protocols/entry.json", &protocol);
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    assert_eq!(
        cat.definitions
            .iter()
            .filter(|e| e.kind == CatalogKind::Protocol)
            .count(),
        1
    );

    // And a protocol-shaped object that fails protocol.json is now an error
    // rather than a parse-and-hope pass ([R8]).
    write(
        root,
        "pkg/protocols/entry.json",
        r#"{"id": "9a1b0c90-0009-4aaa-8bbb-00000000a001"}"#,
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert_eq!(
        code_counts(&cat).get(codes::SHAPE_NO_MATCH),
        Some(&1),
        "{:?}",
        cat.diagnostics
    );
}

/// srs-rust#888: Tier 1 (TypedRecord) is retired (srs#448,
/// rfc-decision-53635966) — a shape-classified `typed-record.json`-style
/// instance (no `$schema`, `fields` array, no `typeId`/`sections`) is no
/// longer admissible at any revision. Classification fails loudly
/// ([R8] `SHAPE_NO_MATCH`) rather than silently accepting or dropping it,
/// and [R24] fatality means the checked `store.catalog()` seam refuses the
/// whole load — the clean cut the retirement requires, not a quiet carve-out.
#[test]
fn tier1_typed_record_shape_no_longer_classifies() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/typed-records/leftover.json",
        r#"{"instanceId": "00000000-0000-4000-8000-0000000000aa", "fields": [{"name": "owner", "fieldType": {"datatype": "string"}, "value": "x"}]}"#,
    );
    let store = FileStore::new(root);

    let cat = catalog::build(&store).unwrap();
    assert_eq!(
        code_counts(&cat).get(codes::SHAPE_NO_MATCH),
        Some(&1),
        "{:?}",
        cat.diagnostics
    );
    assert!(
        cat.instances.is_empty(),
        "a Tier-1-shaped file must never be classified into the instance set: {:?}",
        cat.instances
    );

    // [R24]: the checked seam every ordinary command uses refuses the load
    // entirely — a repository cannot silently tolerate leftover Tier-1
    // content elsewhere in the tree.
    assert!(
        store.catalog().is_err(),
        "store.catalog() must be fatal while Tier-1-shaped content remains"
    );
}

/// A `typed-record.json` `$schema` declaration is likewise unresolvable now
/// that the schema itself is retired (deleted from the mirror, srs-rust#888).
#[test]
fn declared_typed_record_schema_is_unresolvable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(
        root,
        "records/leftover.json",
        r#"{"$schema": "https://srs.semanticops.com/schema/2.0/typed-record.json", "instanceId": "00000000-0000-4000-8000-0000000000ab", "fields": []}"#,
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert_eq!(
        code_counts(&cat).get(codes::SCHEMA_UNRESOLVABLE),
        Some(&1),
        "{:?}",
        cat.diagnostics
    );
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

/// RFC-038 Revision 12 (srs#296, srs PR #538) retired [R3]'s package-root
/// instance-anchor branch (srs-rust#920): a conforming package manifest no
/// longer anchors a `records`/`notes`/`typed-records` directory beneath it as
/// an instance root — a record-shaped file placed there is not silently
/// classified as an instance any more. It is not an error either: an
/// undeclared file under a package root that is not a declared definition
/// path is ordinary, unvalidated application content ([R10]).
#[test]
fn nested_records_under_package_root_is_no_longer_anchored() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "manifest.json", MINIMAL_MANIFEST);
    write(root, "pkg/package.json", &srs_package_json());
    write(
        root,
        "pkg/records/notes/actually-a-record.json",
        &record_json("00000000-0000-4000-8000-000000000077"),
    );
    let cat = catalog::build(&FileStore::new(root)).unwrap();
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    assert!(
        cat.instances.is_empty(),
        "a package root must not anchor a nested records/ as an instance root any more: {:?}",
        cat.instances
    );
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
          "dataModelRevision": 2,
          "repositoryId": "0e0e0e0e-0000-4000-8000-000000000001",
          "declaredExtensions": ["ext:changelog"],
          "changelogPath": "changelog.json"
        }"#,
    );
    write(root, "changelog.json", r#"{"entries": []}"#);
    let store = FileStore::new(root);
    let cat = store.catalog().unwrap();
    let entries: Vec<(&str, &str)> = cat
        .extensions
        .iter()
        .map(|e| (e.kind.as_str(), e.id.as_str()))
        .collect();
    // [R14]: extension set orders by kind byte-wise, then id. Identity is the
    // {kind, id} pair: the aggregate projects the owning repositoryId.
    assert_eq!(
        entries,
        vec![("changelog", "0e0e0e0e-0000-4000-8000-000000000001")]
    );
    assert!(cat.extensions.iter().all(|e| e.tier.is_none()));
}

#[test]
fn extension_locations_require_the_owning_extension_declared() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Path named, file present — but no owning extension declared ([R5]).
    write(
        root,
        "manifest.json",
        r#"{
          "dataModelRevision": 2,
          "repositoryId": "0e0e0e0e-0000-4000-8000-000000000001",
          "changelogPath": "changelog.json"
        }"#,
    );
    write(root, "changelog.json", r#"{"entries": []}"#);
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
