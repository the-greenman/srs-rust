//! Golden-fixture regression test for export_record_bundle determinism.
//!
//! The golden file at tests/fixtures/golden-export-bundle.zip is the expected
//! byte-for-byte output of export_record_bundle on the canonical_store() defined here.
//! If this test fails, the bundle format has changed (or the zip crate's default
//! output changed). Regenerate with:
//!   REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture
//!
//! The canonical store contains a single instance with no attachments so that the
//! rendered decision.md content is determined entirely by the view preamble — a
//! static string with no template variables. This guarantees a byte-stable fixture.
//!
//! ADR-035: same record + attachments → identical bytes across runs.

use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::{export_record_bundle, ExportBundleInput, FileStore, RepositoryStore};
use std::io::Cursor;
use std::path::Path;
use tempfile::tempdir;

/// Stable identifiers — these must not change without regenerating the golden file.
const INSTANCE_ID: &str = "golden-exp-0000-4000-8000-000000000001";
const VIEW_ID: &str = "golden-exp-view-0000-4000-8000-00000001";

/// Build a deterministic FileStore for golden-fixture comparison.
///
/// Non-deterministic sources that must be pinned:
/// - `manifest.json` `createdAt` — set by `initialize_repository` using `chrono::Utc::now()`.
///
/// The document view uses a static preamble (no `{{...}}` template variables) and a
/// TypeQuery section pointing to a non-existent semantic type with `emptyBehavior: hide`.
/// This ensures the rendered `decision.md` contains exactly the preamble text and nothing
/// else, regardless of the store's record contents.
fn canonical_store() -> (tempfile::TempDir, FileStore) {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path());

    store
        .initialize_repository(&InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "golden-bundle-repo-000000000000000000000000".to_string(),
                namespace: "com.example.golden".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Golden Export Bundle Test Repository".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "golden-bundle-pkg-00000000-0000-0000-0000-000000000001".to_string(),
                namespace: "com.example.golden".to_string(),
                name: "golden-bundle".to_string(),
                version: "1.0.0".to_string(),
            },
        })
        .expect("initialize canonical store");

    // Pin createdAt to a fixed value so manifest.json is byte-stable across runs.
    let manifest_path = dir.path().join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("read manifest"),
    )
    .expect("parse manifest");
    manifest["createdAt"] = serde_json::json!("2026-01-01T00:00:00Z");
    manifest["instanceIndex"] = serde_json::json!([{
        "instanceId": INSTANCE_ID,
        "tier": 2,
        "path": "records/tier-2/golden-exp.json"
    }]);
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write pinned manifest");

    // Write the document view JSON file.
    let dv_dir = dir.path().join("package/document-views");
    std::fs::create_dir_all(&dv_dir).expect("create document-views dir");
    let dv_json = serde_json::json!({
        "createdAt": "2026-01-01T00:00:00Z",
        "description": "Golden export test view",
        "format": "markdown",
        "id": VIEW_ID,
        "name": "golden-bundle-view",
        "namespace": "com.example.golden",
        // Static preamble — no {{template}} variables — so rendered output is byte-stable.
        "preamble": "# Golden Export Bundle",
        "sections": [{
            "emptyBehavior": "hide",
            "order": 0,
            "sectionId": "content",
            "source": {
                // Points to a type that does not exist in this package; the section
                // will always be empty and hidden, producing no output.
                "semanticObjectType": "com.example.golden/does-not-exist",
                "type": "type-query"
            }
        }],
        "version": 1
    });
    std::fs::write(
        dv_dir.join("golden-bundle-view.json"),
        serde_json::to_string_pretty(&dv_json).expect("serialize document view"),
    )
    .expect("write document view JSON");

    // Update package.json to reference the document view.
    let pkg_path = dir.path().join("package/package.json");
    let mut pkg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&pkg_path).expect("read package.json"),
    )
    .expect("parse package.json");
    pkg["documentViews"] = serde_json::json!(["document-views/golden-bundle-view.json"]);
    std::fs::write(
        &pkg_path,
        serde_json::to_string_pretty(&pkg).expect("serialize package"),
    )
    .expect("write updated package.json");

    // Write the instance record JSON (no sourceRefs — no attachments).
    std::fs::create_dir_all(dir.path().join("records/tier-2"))
        .expect("create records/tier-2 dir");
    let instance_json = serde_json::json!({
        "instanceId": INSTANCE_ID,
        "typeId": "type-placeholder-001",
        "typeVersion": 1,
        "typeNamespace": "com.example.golden",
        "typeName": "placeholder",
        "fieldValues": []
    });
    std::fs::write(
        dir.path().join("records/tier-2/golden-exp.json"),
        serde_json::to_string_pretty(&instance_json).expect("serialize instance"),
    )
    .expect("write instance JSON");

    (dir, store)
}

/// Run export_record_bundle on the canonical store and return the raw ZIP bytes.
fn pack_canonical() -> Vec<u8> {
    let (_dir, store) = canonical_store();
    let mut buf = Cursor::new(Vec::new());
    export_record_bundle(
        &store,
        ExportBundleInput {
            instance_id: INSTANCE_ID.to_string(),
            view_id: VIEW_ID.to_string(),
            format: None,
        },
        &mut buf,
    )
    .expect("export_record_bundle failed on canonical store");
    buf.into_inner()
}

fn golden_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden-export-bundle.zip")
}

/// Byte-stable golden-fixture test.
///
/// Regenerate the fixture after an intentional format change:
///   REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture
/// Then commit the updated golden-export-bundle.zip.
#[test]
fn test_export_bundle_golden_fixture() {
    let actual = pack_canonical();

    if std::env::var("REGENERATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(golden_path(), &actual).expect("write golden fixture");
        println!(
            "golden-export-bundle.zip regenerated ({} bytes)",
            actual.len()
        );
        return;
    }

    let expected = std::fs::read(golden_path()).expect(
        "golden fixture missing — run: \
        REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture",
    );

    assert_eq!(
        actual, expected,
        "export_record_bundle output differs from golden fixture.\n\
        If the bundle format changed intentionally, regenerate with:\n\
        REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture\n\
        Then commit the updated golden-export-bundle.zip."
    );
}

/// Determinism assertion — ADR-035: same record + attachments → identical bytes across runs.
///
/// Two independent calls to pack_canonical() (each with a fresh FileStore) must
/// produce bit-for-bit identical ZIP output.
#[test]
fn test_export_bundle_determinism() {
    let run1 = pack_canonical();
    let run2 = pack_canonical();
    assert_eq!(
        run1, run2,
        "export_record_bundle must produce byte-identical output across independent runs \
        (ADR-035 determinism invariant)"
    );
}

/// ZIP-content validation — verifies that the bundle contains exactly the expected entries
/// and that decision.md contains the rendered preamble.
///
/// This is a structural check independent of byte pinning: it proves the ZIP is well-formed
/// and the content layer is correct, not just that the bytes are stable.
#[test]
fn test_export_bundle_zip_contents() {
    use zip::ZipArchive;

    let bytes = pack_canonical();
    let mut zip = ZipArchive::new(Cursor::new(bytes)).expect("should be a valid ZIP");

    assert_eq!(
        zip.len(),
        1,
        "canonical bundle (no attachments) must contain exactly one entry"
    );

    let entry_name = zip.by_index(0).unwrap().name().to_string();
    assert_eq!(entry_name, "decision.md", "sole entry must be decision.md");

    // Re-open to read the content (ZipFile is consumed by the name check above).
    let bytes2 = pack_canonical();
    let mut zip2 = ZipArchive::new(Cursor::new(bytes2)).expect("reopen zip");
    let mut entry = zip2.by_name("decision.md").expect("decision.md must exist");
    let mut content = String::new();
    std::io::Read::read_to_string(&mut entry, &mut content).expect("read decision.md");

    assert!(
        content.starts_with("# Golden Export Bundle"),
        "decision.md must start with the static preamble, got: {:?}",
        &content[..content.len().min(80)]
    );
}
