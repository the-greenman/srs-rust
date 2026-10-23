//! RFC-052 Change C / a8e833f1 (srs-rust#1417, review B1): published, immutable documents
//! below revision 10 that embed their root open in a revision-10 build through the pre-load
//! transformer, read as the revision-9 build reads them.

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
