//! Untrusted carriers name their own paths, so every ingestion boundary rejects
//! a path that resolves outside the repository root (srs-rust#783 Phase 4).
//!
//! Regression: before the `.srsj` codec collapse, a hostile `data` key or
//! archive entry (`../evil.json`) survived load, was packed verbatim, and was
//! written outside the target directory on unpack — the "zip slip" class, one
//! `srs archive unpack` away from arbitrary file write.

use srs_repository::srsj::open_srsj;
use srs_repository::store::FileStore;
use std::collections::BTreeMap;

fn envelope(key: &str) -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "srsVersion": "2.0-draft",
            "repositoryId": "7a7a7a7a-0000-4000-8000-000000000001",
            "namespace": "com.example.hostile",
            "dataModelRevision": 2,
        },
        "data": {
            key: {"pwned": true},
            "package/package.json": {
                "id": "7a7a7a7a-0000-4000-8000-0000000000p1",
                "namespace": "com.example.hostile",
                "name": "hostile",
                "version": "1.0.0",
                "fields": [],
                "types": []
            }
        }
    })
    .to_string()
}

#[test]
fn srsj_refuses_a_data_key_outside_the_root() {
    for key in [
        "../evil.json",
        "../../evil.json",
        "package/../../evil.json",
        "/etc/passwd",
    ] {
        let err = open_srsj(&envelope(key))
            .expect_err("a data key outside the root must be refused: {key}");
        assert!(
            err.to_string().contains("does not resolve inside"),
            "{key}: unexpected error {err}"
        );
    }
}

#[test]
fn open_tree_refuses_a_path_outside_the_root() {
    let mut files = BTreeMap::new();
    files.insert("manifest.json".to_string(), b"{}".to_vec());
    files.insert("../evil.json".to_string(), b"{}".to_vec());
    let err = srs_repository::open_tree(files).expect_err("must be refused");
    assert!(err.to_string().contains("does not resolve inside"), "{err}");
}

#[test]
fn archive_unpack_refuses_an_entry_outside_the_root() {
    // Build the archive by hand: pack itself refuses to emit such an entry, so
    // the only way one reaches a consumer is a hand-crafted or hostile file.
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for (name, body) in [
            (
                "manifest.json",
                r#"{"repositoryId":"x","instanceIndex":[]}"#,
            ),
            ("../evil.json", r#"{"pwned":true}"#),
        ] {
            zip.start_file(name, options).unwrap();
            std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    let bytes = buf.into_inner();

    let outer = tempfile::tempdir().unwrap();
    let inner = outer.path().join("repo");
    std::fs::create_dir_all(&inner).unwrap();
    let target = FileStore::new(&inner);

    let err = srs_repository::archive_unpack(std::io::Cursor::new(bytes), &target)
        .expect_err("an entry outside the root must be refused");
    assert!(err.to_string().contains("does not resolve inside"), "{err}");
    assert!(
        !outer.path().join("evil.json").exists(),
        "nothing may be written outside the target root"
    );
}

/// Containment normalises rather than nit-picks: an archive another tool wrote
/// with `./manifest.json` or `a//b.json` entries resolves inside the root and
/// must open, not fail.
#[test]
fn untidy_but_contained_paths_are_normalised_not_refused() {
    let mut files = BTreeMap::new();
    files.insert("manifest.json".to_string(), b"{}".to_vec());
    files.insert("./package/./package.json".to_string(), b"{}".to_vec());
    files.insert("records//tier-2//x.json".to_string(), b"{}".to_vec());
    let store = srs_repository::open_tree(files).expect("contained paths must open");
    let snapshot = srs_repository::export_tree(&store).unwrap();
    assert!(
        snapshot.contains_key("package/package.json"),
        "{snapshot:?}"
    );
    assert!(
        snapshot.contains_key("records/tier-2/x.json"),
        "{snapshot:?}"
    );
}

/// Two spellings of one path with different content is ambiguous, not untidy.
#[test]
fn conflicting_aliases_are_refused() {
    let mut files = BTreeMap::new();
    files.insert("manifest.json".to_string(), b"{}".to_vec());
    files.insert("a/b.json".to_string(), b"{\"v\":1}".to_vec());
    files.insert("./a/b.json".to_string(), b"{\"v\":2}".to_vec());
    let err = srs_repository::open_tree(files).expect_err("must be refused");
    assert!(err.to_string().contains("resolve to 'a/b.json'"), "{err}");
}

/// The `[R19]` shadow-manifest refusal must survive re-spelling: `./manifest.json`
/// is the same key and must hit the same error, not be silently overwritten.
#[test]
fn a_respelled_shadow_manifest_is_still_refused() {
    let mut doc: serde_json::Value = serde_json::from_str(&envelope("unused.json")).unwrap();
    doc["data"] = serde_json::json!({ "./manifest.json": {"repositoryId": "other"} });
    let err = open_srsj(&doc.to_string()).expect_err("must be refused");
    assert!(
        err.to_string().contains("shadowing the envelope manifest"),
        "{err}"
    );
}

/// A tree keyed `./manifest.json` still has a manifest.
#[test]
fn a_respelled_manifest_still_opens_the_tree() {
    let mut files = BTreeMap::new();
    files.insert("./manifest.json".to_string(), b"{}".to_vec());
    srs_repository::open_tree(files).expect("`./manifest.json` is the manifest");
}
