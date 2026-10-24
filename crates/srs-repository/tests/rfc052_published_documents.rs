//! RFC-052 Change C / a8e833f1 (srs-rust#1417, review B1): published, immutable documents
//! below revision 10 that embed their root open in a revision-10 build through the pre-load
//! transformer, read as the revision-9 build reads them. Also RFC-052 [R7] by declared
//! revision and the half-migrated manifest (srs-rust#1453, owner ruling A2 on srs#963).

use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::srsj::open_srsj;
use srs_repository::store::RepositoryStore;
use srs_repository::validation::validate_repository;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/published")
            .join(name),
    )
    .unwrap()
}

fn opens_at_revision_10(name: &str) {
    let text = fixture(name);
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let embedded = &doc["manifest"]["container"];
    assert!(embedded.is_object(), "{name}: the fixture embeds its root");
    let root_id = embedded["containerId"].as_str().unwrap();

    let store = open_srsj(&text).unwrap_or_else(|e| panic!("{name} must open: {e}"));
    let manifest = store.load_manifest().unwrap();
    assert_eq!(manifest.root_container_id(), Some(root_id), "{name}");
    assert_eq!(
        manifest.extra["dataModelRevision"],
        srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION,
        "{name}"
    );
    let report = validate_repository(&store).unwrap();
    assert_eq!(report.summary.errors, 0, "{name}: {:?}", report.diagnostics);

    // Navigation is the embedded root read the revision-9 way: identity, then its entries.
    let nav = repository_navigation(&store).unwrap();
    assert_eq!(nav.root_container_id, root_id, "{name}");
    let identity = embedded["identityInstanceId"].as_str();
    assert_eq!(
        nav.identity.as_ref().map(|n| n.instance_id.as_str()),
        identity,
        "{name}"
    );
    let want: Vec<&str> = embedded["memberInstanceIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("instanceId").and_then(|i| i.as_str()).or(e.as_str()))
        .filter(|id| Some(*id) != identity)
        .collect();
    let got: Vec<&str> = nav
        .sections
        .iter()
        .map(|s| s.instance_id.as_str())
        .collect();
    assert_eq!(got, want, "{name}");
}

#[test]
fn published_governance_seed_1_0_0_opens() {
    opens_at_revision_10("governance-1.0.0-seed.srsj");
}

#[test]
fn published_governance_seed_1_2_2_opens() {
    opens_at_revision_10("governance-1.2.2-seed.srsj");
}

#[test]
fn spec_gallery_opens() {
    opens_at_revision_10("spec-gallery.srsj");
}

// ── RFC-052 [R7] by declared revision; the half-migrated manifest (owner ruling A2, srs#963) ──

use srs_repository::migration_registry_service::apply_migration;
use srs_repository::FileStore;
use std::path::Path;

/// A revision-9 corpus that already names its root by `rootContainerId`, with the root
/// stored in `containers/`: a `repo create` repository whose stamp is put back to 9.
/// Returns the root file's path.
fn half_migrated(dir: &Path) -> std::path::PathBuf {
    use srs_repository::repository_lifecycle::*;
    let store = FileStore::new(dir);
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "14530000-0000-4000-8000-000000000001".to_string(),
                namespace: "com.test.halfway".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "14530000-0000-4000-8000-000000000002".to_string(),
                namespace: "com.test.halfway".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    let path = dir.join("manifest.json");
    let mut m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    m["dataModelRevision"] = serde_json::json!(9);
    std::fs::write(&path, serde_json::to_string_pretty(&m).unwrap() + "\n").unwrap();
    let root = m["rootContainerId"].as_str().unwrap();
    dir.join(format!("containers/{root}.json"))
}

/// Every file of the tree, by relative path.
fn tree(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn manifest_without_stamp(dir: &Path) -> serde_json::Value {
    let mut m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    m.as_object_mut().unwrap().remove("dataModelRevision");
    m
}

#[test]
fn a_revision_9_corpus_naming_its_root_is_refused_then_restamped() {
    let tmp = tempfile::tempdir().unwrap();
    let root_file = half_migrated(tmp.path());

    // (a) Refused by its declared revision, whatever its manifest's shape.
    let err = FileStore::new(tmp.path()).load_manifest().unwrap_err();
    assert_eq!(err.code(), "container-migration-needed", "{err}");
    assert_eq!(
        err.report().details.unwrap()["pending"],
        serde_json::json!(["rfc052-root-container"])
    );

    // (b) The registry entry only restamps it: the root file is byte-identical and the
    // manifest differs only in its stamp.
    let root_before = std::fs::read(&root_file).unwrap();
    let manifest_before = manifest_without_stamp(tmp.path());
    let tooling = FileStore::new(tmp.path()).with_rfc038_exemption();
    apply_migration(&tooling, "rfc052-root-container").unwrap();
    assert_eq!(std::fs::read(&root_file).unwrap(), root_before);
    assert_eq!(manifest_without_stamp(tmp.path()), manifest_before);
    let store = FileStore::new(tmp.path());
    assert_eq!(
        store.load_manifest().unwrap().extra["dataModelRevision"],
        serde_json::json!(10)
    );
    assert_eq!(validate_repository(&store).unwrap().summary.errors, 0);
}

#[test]
fn a_half_migrated_manifest_whose_root_is_missing_or_duplicated_is_refused_writing_nothing() {
    for (case, code) in [
        ("missing", "manifest-root-container-missing"),
        ("duplicated", "migration-root-duplicate"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let root_file = half_migrated(tmp.path());
        if case == "missing" {
            std::fs::remove_file(&root_file).unwrap();
        } else {
            std::fs::copy(&root_file, tmp.path().join("containers/duplicate.json")).unwrap();
        }
        let before = tree(tmp.path());
        let tooling = FileStore::new(tmp.path()).with_rfc038_exemption();
        let err = apply_migration(&tooling, "rfc052-root-container").unwrap_err();
        assert_eq!(err.code(), code, "{case}: {err}");
        assert_eq!(tree(tmp.path()), before, "{case}: nothing written");
    }
}
