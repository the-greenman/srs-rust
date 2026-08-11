//! RFC-038 [R17]: producing a snapshot enumerates and includes all six
//! authoritative sets, the manifest, and the marker; consuming one discovers by
//! the same rules as a live repository (srs-rust#783 Phase 4).
//!
//! This is RFC acceptance test 15 — a snapshot carrying declared changelog and
//! federation data round-trips the extension set without loss, alongside the
//! other five sets — proved for both snapshot carriers, `.srs` and `.srsj`.

use srs_repository::catalog::{self, RepositoryCatalog};
use srs_repository::srsj::{open_srsj, to_srsj_string};
use srs_repository::store::{FileStore, RepositoryStore};
use std::path::Path;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const REPO_ID: &str = "5e5e5e5e-0000-4000-8000-000000000001";
const TYPE_ID: &str = "5e5e5e5e-0000-4000-8000-0000000000t1";
const FIELD_ID: &str = "5e5e5e5e-0000-4000-8000-0000000000f1";
const RECORD_ID: &str = "5e5e5e5e-0000-4000-8000-00000000rec1";
const NOTE_ID: &str = "5e5e5e5e-0000-4000-8000-000000note1";
const RELATION_ID: &str = "5e5e5e5e-0000-4000-8000-0000000rel01";
const CONTAINER_ID: &str = "5e5e5e5e-0000-4000-8000-00000000ctr1";
const DOCUMENT_ID: &str = "5e5e5e5e-0000-4000-8000-00000000doc1";
const REGISTRY_ID: &str = "5e5e5e5e-0000-4000-8000-00000000reg1";

/// A repository populated across all six authoritative sets, including the
/// extension aggregates that only exist when their extension is declared.
fn six_set_repository(root: &Path) {
    write(
        root,
        "manifest.json",
        &format!(
            r#"{{
              "srsVersion": "2.0-draft",
              "repositoryId": "{REPO_ID}",
              "namespace": "com.example.snapshot",
              "dataModelRevision": 2,
              "instanceIndex": [],
              "declaredExtensions": ["ext:changelog", "ext:federation"],
              "changelogPath": "changelog.json",
              "federationPath": "federation/registry.json",
              "federationEventsPath": "federation/events.json"
            }}"#
        ),
    );
    write(root, ".srs/.gitkeep", "");

    // Definition set (and the package root that anchors it).
    write(
        root,
        "package/package.json",
        r#"{
              "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
              "id": "5e5e5e5e-0000-4000-8000-00000000pkg1",
              "namespace": "com.example.snapshot",
              "name": "snapshot-package",
              "version": "1.0.0",
              "title": "Snapshot Package",
              "description": "Six-set snapshot fixture",
              "status": "active",
              "createdAt": "2026-01-01T00:00:00Z",
              "fields": ["fields/title.json"],
              "types": ["types/thing.json"]
            }"#,
    );
    write(
        root,
        "package/fields/title.json",
        &format!(
            r#"{{
              "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
              "id": "{FIELD_ID}",
              "namespace": "com.example.snapshot",
              "name": "title",
              "version": 1,
              "description": "Title",
              "aiGuidance": {{"purpose": "Name the thing"}},
              "fieldType": {{"datatype": "string"}},
              "createdAt": "2026-01-01T00:00:00Z"
            }}"#
        ),
    );
    write(
        root,
        "package/types/thing.json",
        &format!(
            r#"{{
              "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
              "id": "{TYPE_ID}",
              "namespace": "com.example.snapshot",
              "name": "thing",
              "version": 1,
              "description": "A thing",
              "createdAt": "2026-01-01T00:00:00Z",
              "fields": [{{"fieldId": "{FIELD_ID}", "order": 0, "required": false}}]
            }}"#
        ),
    );

    // Instance set: one Tier-2 Record and one Tier-0 Note.
    write(
        root,
        "records/tier-2/thing.json",
        &format!(
            r#"{{
              "instanceId": "{RECORD_ID}",
              "typeId": "{TYPE_ID}",
              "typeVersion": 1,
              "typeNamespace": "com.example.snapshot",
              "typeName": "thing",
              "fieldValues": {{"title": "First"}}
            }}"#
        ),
    );
    write(
        root,
        "records/notes/intro.json",
        &format!(
            r#"{{"instanceId": "{NOTE_ID}", "sections": [{{"name": "body", "content": "hello"}}]}}"#
        ),
    );

    // Relation set: one standalone relation object (RFC-038 Change E).
    write(
        root,
        &format!("relations/{RELATION_ID}.json"),
        &format!(
            r#"{{
              "$schema": "https://srs.semanticops.com/schema/2.0/relation.json",
              "relationId": "{RELATION_ID}",
              "relationType": "precedes",
              "sourceInstanceId": "{NOTE_ID}",
              "targetInstanceId": "{RECORD_ID}",
              "createdAt": "2026-01-01T00:00:00Z"
            }}"#
        ),
    );

    // Container set.
    write(
        root,
        "containers/main.json",
        &format!(
            r#"{{
              "containerId": "{CONTAINER_ID}",
              "title": "Main",
              "createdAt": "2026-01-01T00:00:00Z",
              "rootInstanceIds": ["{RECORD_ID}"],
              "memberInstanceIds": ["{RECORD_ID}", "{NOTE_ID}"]
            }}"#
        ),
    );

    // Source-document set: a sidecar (identity) beside its opaque payload.
    write(
        root,
        "source-documents/brief.md.meta.json",
        &format!(
            r#"{{
              "documentId": "{DOCUMENT_ID}",
              "title": "Brief",
              "contentPath": "brief.md",
              "contentType": "text/markdown",
              "createdAt": "2026-01-01T00:00:00Z"
            }}"#
        ),
    );
    write(
        root,
        "source-documents/brief.md",
        "# Brief\n\nopaque payload\n",
    );

    // Extension set: only enumerable because the manifest declares the owning
    // extensions ([R5] sixth location class).
    write(root, "changelog.json", r#"{"entries": []}"#);
    write(
        root,
        "federation/registry.json",
        &format!(r#"{{"registryId": "{REGISTRY_ID}", "repositories": []}}"#),
    );
    write(root, "federation/events.json", r#"{"events": []}"#);
}

/// `set/kind/id` triples across all six sets — identity, never locators.
fn identity_triples(cat: &RepositoryCatalog) -> Vec<String> {
    let mut out = Vec::new();
    for (set, entries) in [
        ("instances", &cat.instances),
        ("relations", &cat.relations),
        ("containers", &cat.containers),
        ("source-documents", &cat.source_documents),
        ("definitions", &cat.definitions),
        ("extensions", &cat.extensions),
    ] {
        for e in entries {
            out.push(format!("{set}/{}/{}", e.kind.as_str(), e.id));
        }
    }
    out.sort();
    out
}

fn assert_all_six_sets_present(cat: &RepositoryCatalog, label: &str) {
    assert!(!cat.instances.is_empty(), "{label}: instance set empty");
    assert!(!cat.relations.is_empty(), "{label}: relation set empty");
    assert!(!cat.containers.is_empty(), "{label}: container set empty");
    assert!(
        !cat.source_documents.is_empty(),
        "{label}: source-document set empty"
    );
    assert!(!cat.definitions.is_empty(), "{label}: definition set empty");
    assert!(
        !cat.extensions.is_empty(),
        "{label}: extension set empty — a snapshot carrying declared changelog \
         and federation data must round-trip it ([R17], RFC acceptance test 15)"
    );
}

#[test]
fn srs_archive_round_trips_all_six_sets_and_the_marker() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    let source = FileStore::new(src_tmp.path());
    let before = catalog::build(&source).unwrap();
    assert!(
        before.diagnostics.is_empty(),
        "the fixture must be clean: {:?}",
        before.diagnostics
    );
    assert_all_six_sets_present(&before, "source");

    let bytes = srs_repository::archive_to_vec(&source).expect("pack");

    // [R17]: the manifest and the marker travel with the sets.
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    for required in [
        "manifest.json",
        ".srs/.gitkeep",
        "package/package.json",
        "changelog.json",
        "federation/registry.json",
        "federation/events.json",
        "source-documents/brief.md",
        "source-documents/brief.md.meta.json",
    ] {
        assert!(
            names.contains(&required.to_string()),
            "pack must carry {required}; got {names:?}"
        );
    }

    // Consuming discovers by the same rules as a live repository.
    let dst_tmp = tempfile::tempdir().unwrap();
    let target = FileStore::new(dst_tmp.path());
    srs_repository::archive_unpack(std::io::Cursor::new(bytes), &target).expect("unpack");
    let after = catalog::build(&target).unwrap();
    assert_all_six_sets_present(&after, "unpacked");
    assert_eq!(
        identity_triples(&before),
        identity_triples(&after),
        "a `.srs` round-trip must preserve every authoritative set"
    );
}

#[test]
fn srsj_document_round_trips_all_six_sets() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    let source = FileStore::new(src_tmp.path());
    let before = catalog::build(&source).unwrap();
    assert_all_six_sets_present(&before, "source");

    let document = to_srsj_string(&source).expect("project as .srsj");
    let reloaded = open_srsj(&document).expect("reopen the projection");
    let after = catalog::build(&reloaded).unwrap();

    assert_all_six_sets_present(&after, "reloaded");
    assert_eq!(
        identity_triples(&before),
        identity_triples(&after),
        "a `.srsj` round-trip must preserve every authoritative set"
    );
    // The opaque payload beside the sidecar is content, not an entity — it must
    // survive the carrier even though it has no catalog entry of its own.
    assert_eq!(
        reloaded
            .load_text_file("source-documents/brief.md")
            .unwrap(),
        "# Brief\n\nopaque payload\n"
    );
}

/// [R17]: a package root discovered by presence, with no `PackageRef` naming
/// it, is still included in the snapshot.
#[test]
fn pack_carries_an_undeclared_package_root() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    write(
        src_tmp.path(),
        "packages/extra/package.json",
        r#"{
          "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
          "id": "5e5e5e5e-0000-4000-8000-00000000pkg2",
          "namespace": "com.example.extra",
          "name": "extra-package",
          "version": "1.0.0",
          "title": "Extra Package",
          "description": "An undeclared local package root",
          "status": "active",
          "createdAt": "2026-01-01T00:00:00Z",
          "fields": [],
          "types": []
        }"#,
    );
    let source = FileStore::new(src_tmp.path());
    assert!(
        catalog::build(&source)
            .unwrap()
            .package_roots
            .iter()
            .any(|r| r == "packages/extra"),
        "presence discovery must find the undeclared root"
    );

    let bytes = srs_repository::archive_to_vec(&source).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.contains(&"packages/extra/package.json".to_string()),
        "an undeclared local package root must still be packed ([R17]): {names:?}"
    );
}

/// A package root the catalog cannot anchor — a `package.json` that fails
/// `package-manifest.json` ([R4]) — must still be packed in full. Enumerating
/// only *validated* roots would silently pack the repository to nothing.
#[test]
fn pack_carries_a_package_root_the_catalog_cannot_anchor() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    // Drop a required property: near-miss manifest, diagnosed but not anchored.
    let pkg_path = src_tmp.path().join("package/package.json");
    let mut pkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pkg_path).unwrap()).unwrap();
    pkg.as_object_mut().unwrap().remove("status");
    std::fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let source = FileStore::new(src_tmp.path());
    assert!(
        !catalog::build(&source)
            .unwrap()
            .package_roots
            .iter()
            .any(|r| r.is_empty() || r == "package"),
        "the near-miss manifest must not anchor — otherwise this proves nothing"
    );

    let bytes = srs_repository::archive_to_vec(&source).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    for required in [
        "package/package.json",
        "package/fields/title.json",
        "package/types/thing.json",
    ] {
        assert!(
            names.contains(&required.to_string()),
            "an unanchored package root must still be packed whole; {required} missing from {names:?}"
        );
    }
}

/// `.srs/` holds ordinary content (agent profiles). Non-UTF-8 content there
/// must ride through rather than turning packing into a hard error.
#[test]
fn pack_carries_non_utf8_marker_content() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    std::fs::write(
        src_tmp.path().join(".srs/blob.bin"),
        [0x89u8, 0x50, 0x4e, 0x47, 0xff, 0xfe],
    )
    .unwrap();

    let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path()))
        .expect("non-UTF-8 marker content must not fail the pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(names.contains(&".srs/blob.bin".to_string()), "{names:?}");
}

/// Ordinary filename characters that only look dangerous must stay legal —
/// a colon is not a path separator on the platforms this runs on.
#[test]
fn a_colon_in_a_filename_is_not_a_traversal() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    std::fs::write(
        src_tmp.path().join("source-documents/meeting 10:00.md"),
        "# Standup\n",
    )
    .unwrap();

    let source = FileStore::new(src_tmp.path());
    let document = to_srsj_string(&source).expect("a colon must not fail the projection");
    let reloaded = open_srsj(&document).expect("nor the reopen");
    assert_eq!(
        reloaded
            .load_text_file("source-documents/meeting 10:00.md")
            .unwrap(),
        "# Standup\n"
    );
    srs_repository::archive_to_vec(&source).expect("nor the pack");
}

/// Pack is a faithful copier, not a validator: an object the catalog cannot
/// classify produces a diagnostic and no entry, and dropping its file would
/// turn a diagnosable repository into a lossy snapshot with a zero exit code.
#[test]
fn pack_carries_objects_the_catalog_cannot_classify() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    // A record with no instanceId, a container that is not valid JSON, a
    // relations file that will not parse, and a declared federation registry
    // with no identity to project — each diagnosable, none classifiable.
    write(
        src_tmp.path(),
        "records/tier-2/nameless.json",
        r#"{"typeName":"x"}"#,
    );
    write(src_tmp.path(), "containers/broken.json", "{ not json");
    write(src_tmp.path(), "relations/garbage.json", "{ not json");
    write(
        src_tmp.path(),
        "federation/registry.json",
        r#"{"repositories": []}"#,
    );

    let source = FileStore::new(src_tmp.path());
    let bytes = srs_repository::archive_to_vec(&source).expect("pack must not refuse");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    for required in [
        "records/tier-2/nameless.json",
        "containers/broken.json",
        "relations/garbage.json",
        "federation/registry.json",
    ] {
        assert!(
            names.contains(&required.to_string()),
            "{required} must be packed even though the catalog cannot classify it; got {names:?}"
        );
    }
}

/// A `.json` file whose entire content is a JSON string must survive the
/// carrier with its quotes — decoding it as raw text is content corruption.
#[test]
fn a_json_string_document_survives_the_codec() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    write(src_tmp.path(), "records/tier-2/quoted.json", "\"hello\"");

    let source = FileStore::new(src_tmp.path());
    let reloaded = open_srsj(&to_srsj_string(&source).expect("project")).expect("reopen");
    assert_eq!(
        reloaded
            .load_text_file("records/tier-2/quoted.json")
            .unwrap(),
        "\"hello\"",
        "the file's bytes must survive, quotes included"
    );
}
