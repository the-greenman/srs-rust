//! RFC-038 [R17]: producing a snapshot enumerates and includes all six
//! authoritative sets, the manifest, and the marker; consuming one discovers by
//! the same rules as a live repository (srs-rust#783 Phase 4).
//!
//! This is RFC acceptance test 15 — a snapshot carrying declared changelog
//! data round-trips the extension set without loss, alongside the other five
//! sets — proved for both snapshot carriers, `.srs` and `.srsj`.

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
              "declaredExtensions": ["ext:changelog"],
              "changelogPath": "changelog.json"
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
         data must round-trip it ([R17], RFC acceptance test 15)"
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
/// it, is still included in the snapshot — including the worst case, a root
/// that is *both* undeclared and unanchorable, which belongs to none of the
/// three sources the enumeration unions.
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
    // Undeclared *and* invalid: SRS-shaped, so [R4] diagnoses it rather than
    // ignoring it as an npm manifest, but it anchors nothing.
    write(
        src_tmp.path(),
        "packages/broken/package.json",
        r#"{
          "namespace": "com.example.broken",
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
    for required in [
        "packages/extra/package.json",
        "packages/broken/package.json",
    ] {
        assert!(
            names.contains(&required.to_string()),
            "an undeclared local package root must still be packed ([R17]); {required} missing from {names:?}"
        );
    }
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
    let cat = catalog::build(&source).unwrap();
    assert!(
        cat.diagnostics
            .iter()
            .any(|d| d.code == catalog::codes::PACKAGE_MANIFEST_INVALID),
        "the near-miss manifest must be diagnosed — otherwise this proves nothing"
    );
    assert!(
        cat.definitions.is_empty(),
        "a near-miss manifest must not anchor its definitions"
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
    // A record with no instanceId, a container that is not valid JSON, and a
    // relations file that will not parse — each diagnosable, none classifiable.
    write(
        src_tmp.path(),
        "records/tier-2/nameless.json",
        r#"{"typeName":"x"}"#,
    );
    write(src_tmp.path(), "containers/broken.json", "{ not json");
    write(src_tmp.path(), "relations/garbage.json", "{ not json");

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
    ] {
        assert!(
            names.contains(&required.to_string()),
            "{required} must be packed even though the catalog cannot classify it; got {names:?}"
        );
    }
}

/// The codec's two halves must be inverses whatever a `.json` file happens to
/// contain: a JSON string document keeps its quotes, and a payload that is not
/// JSON at all does not acquire any.
#[test]
fn json_payloads_survive_the_codec_byte_for_byte() {
    let cases = [
        ("records/tier-2/quoted.json", "\"hello\""),
        ("records/tier-2/broken.json", "{ this is not json"),
        ("records/tier-2/scalar.json", "42"),
        ("source-documents/notes.md", "# Heading\n\nbody\n"),
    ];
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    for (path, body) in cases {
        write(src_tmp.path(), path, body);
    }

    let source = FileStore::new(src_tmp.path());
    let reloaded = open_srsj(&to_srsj_string(&source).expect("project")).expect("reopen");
    for (path, body) in cases {
        assert_eq!(
            reloaded.load_text_file(path).unwrap(),
            body,
            "{path} must survive the carrier byte for byte"
        );
    }
}

/// RFC-038 Revision 12 (srs#296, srs PR #538) retired [R3]'s package-root
/// instance-anchor branch (srs-rust#920): a sub-package's `records/` is no
/// longer an instance root at all — a local package root stays reserved for
/// definitions only ([R5]). This is no longer a distinct "instance root
/// under a sub-package" case; it is the same generic-content-under-a-package-
/// root sweep exercised by `pack_carries_an_undeclared_package_root` and
/// `pack_carries_objects_the_catalog_cannot_classify` — kept as its own test
/// because a nested (non-root) package's content must be swept too, not only
/// the top-level package's.
#[test]
fn pack_sweeps_undeclared_content_under_a_sub_package() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    write(
        src_tmp.path(),
        "pkgs/sub/package.json",
        r#"{
          "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
          "id": "5e5e5e5e-0000-4000-8000-00000000pkg3",
          "namespace": "com.example.sub",
          "name": "sub-package",
          "version": "1.0.0",
          "title": "Sub Package",
          "description": "A nested local package root",
          "status": "active",
          "createdAt": "2026-01-01T00:00:00Z",
          "fields": [],
          "types": []
        }"#,
    );
    write(
        src_tmp.path(),
        "pkgs/sub/records/tier-2/broken.json",
        r#"{"typeName":"x"}"#,
    );

    let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path())).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.contains(&"pkgs/sub/records/tier-2/broken.json".to_string()),
        "undeclared content under a sub-package must be swept too: {names:?}"
    );
}

/// A manifest-declared location is data, and data can be wrong. A path that
/// resolves to the repository root would turn the reserved-location sweep into
/// the blind directory walk the enumeration exists to avoid — packing `.git/`
/// and whatever credentials it holds into the snapshot.
#[test]
fn a_declared_path_resolving_to_the_root_does_not_pack_the_whole_tree() {
    // relationsPath is retired ([R2]) — a manifest declaring it no longer
    // loads at all, so only the live declared-location properties remain.
    for (property, value) in [("sourceDocumentsPath", ""), ("changelogPath", "./")] {
        let src_tmp = tempfile::tempdir().unwrap();
        six_set_repository(src_tmp.path());
        write(
            src_tmp.path(),
            ".git/config",
            "[remote]\n  url = https://token@host/x",
        );
        let manifest_path = src_tmp.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest[property] = serde_json::json!(value);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path())).expect("pack");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.starts_with(".git/")),
            "{property}: {value:?} must not pull the working tree into the snapshot: {names:?}"
        );
    }
}

/// A repository and its `.srsj` projection must pack the same tree, marker
/// included — cross-store archives are compared byte for byte by ADR-039.
#[test]
fn a_srsj_session_packs_the_same_tree_as_the_repository_on_disk() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    std::fs::remove_file(src_tmp.path().join(".srs/.gitkeep")).unwrap();
    let source = FileStore::new(src_tmp.path());

    let from_disk = srs_repository::archive_to_vec(&source).expect("pack from disk");
    let session = open_srsj(&to_srsj_string(&source).expect("project")).expect("reopen");
    let from_session = srs_repository::archive_to_vec(&session).expect("pack from session");

    let names = |bytes: Vec<u8>| -> Vec<String> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect()
    };
    let disk_names = names(from_disk);
    assert!(disk_names.contains(&".srs/.gitkeep".to_string()));
    assert_eq!(disk_names, names(from_session));
}

/// `#` is a legal filename character. A locator's fragment is stripped only
/// when the locator does not already name a file — `notes/issue#42.json` is a
/// path, not a path plus a fragment.
#[test]
fn a_hash_in_a_filename_is_not_a_locator_fragment() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    let hashed = "records/notes/issue#42.json";
    write(
        src_tmp.path(),
        hashed,
        r#"{"instanceId": "5e5e5e5e-0000-4000-8000-00000000has1",
            "sections": [{"name": "body", "content": "hashed"}]}"#,
    );

    let source = FileStore::new(src_tmp.path());
    let bytes = srs_repository::archive_to_vec(&source).expect("pack must not choke on `#`");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(names.contains(&hashed.to_string()), "{names:?}");
}

/// A stray package root the repository merely *contains* — a vendored copy, a
/// half-deleted directory — must not stop the whole repository being archived.
/// The catalog reports the dangling definition; pack is not a validator.
#[test]
fn a_stray_package_root_with_missing_definitions_does_not_abort_the_pack() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    // The manifest names it nowhere and its declared definitions are absent.
    std::fs::create_dir_all(src_tmp.path().join("vendor")).unwrap();
    std::fs::copy(
        src_tmp.path().join("package/package.json"),
        src_tmp.path().join("vendor/package.json"),
    )
    .unwrap();

    let source = FileStore::new(src_tmp.path());
    assert!(
        catalog::build(&source)
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == catalog::codes::DEFINITION_PATH_MISSING),
        "the dangling definition must still be reported"
    );

    let bytes =
        srs_repository::archive_to_vec(&source).expect("a stray root must not abort the archive");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.contains(&"vendor/package.json".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"package/fields/title.json".to_string()),
        "{names:?}"
    );
}

/// The same absence inside a package the repository *declares* stays a hard
/// error naming the file (ADR-039).
#[test]
fn a_declared_package_with_missing_definitions_still_fails_loudly() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    std::fs::remove_file(src_tmp.path().join("package/fields/title.json")).unwrap();

    let err = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path()))
        .expect_err("a declared package's missing definition must fail the pack");
    assert!(
        err.to_string().contains("package/fields/title.json"),
        "{err}"
    );
}

/// Classification and snapshot production must agree about where a declared
/// location is. They used two different normalisations, so a manifest saying
/// `"/source-documents"` made pack drop every attachment *and* sidecar with a
/// zero exit code while the catalog enumerated them.
#[test]
fn a_declared_location_resolves_the_same_way_for_catalog_and_pack() {
    for spelling in [
        "/source-documents",
        "./source-documents",
        "source-documents/",
    ] {
        let src_tmp = tempfile::tempdir().unwrap();
        six_set_repository(src_tmp.path());
        let manifest_path = src_tmp.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["sourceDocumentsPath"] = serde_json::json!(spelling);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let source = FileStore::new(src_tmp.path());
        assert!(
            !catalog::build(&source).unwrap().source_documents.is_empty(),
            "{spelling}: the catalog must still find the sidecar"
        );
        let bytes = srs_repository::archive_to_vec(&source).expect("pack");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            names.contains(&"source-documents/brief.md".to_string()),
            "{spelling}: the opaque payload must still be packed: {names:?}"
        );
    }
}

/// A package root is a reserved location, so everything under it travels —
/// including a definition its manifest never declared.
#[test]
fn pack_carries_an_undeclared_file_inside_a_package_root() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    write(
        src_tmp.path(),
        "package/fields/orphan.json",
        r#"{"id": "5e5e5e5e-0000-4000-8000-0000000000f9"}"#,
    );

    let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path())).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.contains(&"package/fields/orphan.json".to_string()),
        "{names:?}"
    );
}

/// A package root is a reserved location, but the repository root is not a
/// sweep directory even when it holds a `package.json` — walking it is the
/// blind directory sweep the enumeration exists to avoid. Nor does a sweep
/// follow another tool's directories nested inside a reserved location.
#[test]
fn the_sweep_never_reaches_foreign_tooling() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    // A root-level SRS package manifest: a legitimate package root of `""`.
    std::fs::copy(
        src_tmp.path().join("package/package.json"),
        src_tmp.path().join("package.json"),
    )
    .unwrap();
    write(
        src_tmp.path(),
        ".git/config",
        "[remote]\n  url = https://token@host/x",
    );
    write(
        src_tmp.path(),
        "package/node_modules/left-pad/index.js",
        "module.exports=1",
    );

    let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path())).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        !names.iter().any(|n| n.starts_with(".git/")),
        "version-control metadata must never be packed: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("node_modules/")),
        "a dependency tree must never be packed: {names:?}"
    );
}

/// The skip list is the catalog's, so the two agree on what is foreign — an
/// opaque payload under an ordinarily-named directory is content, not tooling.
#[test]
fn an_opaque_payload_under_an_ordinary_directory_is_still_packed() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    write(
        src_tmp.path(),
        "source-documents/target/report.txt",
        "payload",
    );

    let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path())).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.contains(&"source-documents/target/report.txt".to_string()),
        "{names:?}"
    );
}

/// The codec, the catalog and the archive must agree which payloads are
/// opaque, whatever spelling the manifest uses — canonicalising an attachment
/// changes its checksum ([R9]).
#[test]
fn an_oddly_spelled_source_documents_path_still_marks_payloads_opaque() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    let manifest_path = src_tmp.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["sourceDocumentsPath"] = serde_json::json!("/source-documents");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let raw = "{\"z\":1,\"a\":2}";
    write(src_tmp.path(), "source-documents/payload.json", raw);

    let source = FileStore::new(src_tmp.path());
    let reloaded = open_srsj(&to_srsj_string(&source).expect("project")).expect("reopen");
    assert_eq!(
        reloaded
            .load_text_file("source-documents/payload.json")
            .unwrap(),
        raw,
        "an opaque payload must survive byte for byte"
    );
}

/// A definition declared as `./fields/x.json` names a file already carried;
/// emitting both spellings would break ADR-039 determinism.
#[test]
fn an_untidy_declared_definition_path_does_not_duplicate_its_entry() {
    let src_tmp = tempfile::tempdir().unwrap();
    six_set_repository(src_tmp.path());
    let pkg_path = src_tmp.path().join("package/package.json");
    let mut pkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pkg_path).unwrap()).unwrap();
    pkg["fields"] = serde_json::json!(["./fields/title.json"]);
    std::fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let bytes = srs_repository::archive_to_vec(&FileStore::new(src_tmp.path())).expect("pack");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert_eq!(
        names
            .iter()
            .filter(|n| n.ends_with("fields/title.json"))
            .count(),
        1,
        "one file, one entry: {names:?}"
    );
}
