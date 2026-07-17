//! Golden-fixture regression test for archive_pack determinism.
//!
//! The golden file at tests/fixtures/golden-archive.srs is the expected
//! byte-for-byte output of archive_pack on the canonical_store() defined here.
//! If this test fails, the archive format has changed (or the zip crate's
//! default output changed). Regenerate with:
//!   REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_archive_golden_fixture

use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::{archive_pack, archive_unpack, FileStore, RepositoryStore};
use std::io::Cursor;
use std::path::Path;
use tempfile::tempdir;

fn canonical_store() -> (tempfile::TempDir, FileStore) {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path());
    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "golden-archive-repo-00000000000000000000000000".to_string(),
                namespace: "com.example.golden".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Golden Archive Test Repository".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "golden-pkg-00000000-0000-0000-0000-000000000001".to_string(),
                namespace: "com.example.golden".to_string(),
                name: "golden-package".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .expect("initialize canonical store");

    // Pin createdAt to a fixed value. FileStore::initialize_repository writes
    // chrono::Utc::now() into manifest.json, which varies between process runs
    // and breaks the golden-fixture byte comparison.
    let mut manifest = store.load_manifest().expect("load manifest for pinning");
    manifest.extra.insert(
        "createdAt".to_string(),
        serde_json::json!("2026-01-01T00:00:00Z"),
    );
    store
        .save_manifest(&manifest)
        .expect("save pinned manifest");

    (dir, store)
}

fn pack_canonical() -> Vec<u8> {
    let (_dir, store) = canonical_store();
    let mut buf = Vec::new();
    archive_pack(&store, Cursor::new(&mut buf)).expect("archive_pack failed");
    buf
}

fn golden_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden-archive.srs")
}

#[test]
fn test_archive_golden_fixture() {
    let actual = pack_canonical();

    if std::env::var("REGENERATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(golden_path(), &actual).expect("write golden fixture");
        println!("golden-archive.srs regenerated ({} bytes)", actual.len());
        return;
    }

    let expected = std::fs::read(golden_path()).expect(
        "golden fixture missing — run: \
        REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_archive_golden_fixture",
    );

    assert_eq!(
        actual, expected,
        "archive_pack output differs from golden fixture.\n\
        If the archive format changed intentionally, regenerate with:\n\
        REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_archive_golden_fixture\n\
        Then commit the updated golden-archive.srs."
    );
}

#[test]
fn test_archive_golden_roundtrip() {
    let path = golden_path();
    let bytes = std::fs::read(&path).expect(
        "golden fixture missing — run: \
        REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_archive_golden_fixture",
    );
    let target_dir = tempdir().unwrap();
    let target = FileStore::new(target_dir.path());
    archive_unpack(Cursor::new(bytes), &target).expect("golden fixture failed to unpack");
    let manifest = target.load_manifest().expect("load manifest");
    assert_eq!(
        manifest.extra.get("namespace").and_then(|v| v.as_str()),
        Some("com.example.golden"),
        "unpacked namespace should match the canonical store's namespace"
    );
}
