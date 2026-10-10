//! Integration test for the governance scaffold path (issue #381).
//!
//! Proves that the raw governance seed, RFC-014-prepared and opened through the
//! `.srsj` codec, followed by `create_governance_repository` and
//! `validate_repository`, produces a valid bundle.

use srs_repository::{
    field_type_migration_service::CURRENT_DATA_MODEL_REVISION,
    governance_scaffold_service::{create_governance_repository, CreateGovernanceRepositoryInput},
    srsj::open_srsj,
    srsj_migration_service, validation,
};

#[test]
fn scaffold_from_raw_seed_produces_valid_repository() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/governance-seed.srsj"
    ))
    .expect("governance-seed.srsj fixture must exist");

    let prepared = srsj_migration_service::migrate_rfc014(&raw)
        .expect("RFC-014 seed preparation must succeed");
    let store = open_srsj(&prepared).expect("the prepared seed must open through the codec");

    // create_governance_repository mutates store in place via interior mutability.
    let _ = create_governance_repository(
        &store,
        CreateGovernanceRepositoryInput {
            namespace: Some("com.test.381".to_string()),
            title: "Test Org 381".to_string(),
            purpose: None,
            repository_id: None,
        },
    )
    .expect("scaffold must succeed on migrated seed");

    let report =
        validation::validate_repository(&store).expect("validate_repository must not error");

    // RFC-018 I-81: the scaffold now creates a com.semanticops.core/purpose identity record
    // (#568). A freshly scaffolded repository must have zero errors and zero I-81 warnings.
    assert!(
        report.is_ok(),
        "expected scaffold to produce a valid repository (no errors), got: {:?}",
        report.diagnostics
    );
    let i81_warnings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("RFC-018 I-81"))
        .collect();
    assert!(
        i81_warnings.is_empty(),
        "freshly scaffolded repo must have zero RFC-018 I-81 warnings: {i81_warnings:?}"
    );

    // srs#163: the scaffold re-binds the installed document views to the containers it
    // created, so a fresh repo must not ship dangling container references (which the
    // #509 validate check reports as warnings).
    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("references containerId"))
        .collect();
    assert!(
        dangling.is_empty(),
        "fresh scaffold must not ship dangling composition container refs: {dangling:?}"
    );
}

/// srs-rust#1180: the bundled seed must carry the binary's current data-model
/// revision at all times. A lagging seed is what made srs-bindings-web ship a
/// revision-8 seed after the revision-9 (RFC-046) bump landed — a document
/// created from it then prompted for a migration in srs-web immediately on
/// re-import (the-greenman/srs-web#360). When `CURRENT_DATA_MODEL_REVISION`
/// moves, this test goes red until the seed is re-stamped with
/// `srs repo apply-migration --id <new-migration-id> --repo
/// crates/srs-repository/tests/fixtures/governance-seed.srsj` (the single copy; also the release asset).
#[test]
fn seed_is_stamped_at_the_current_data_model_revision() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/governance-seed.srsj"
    ))
    .expect("governance-seed.srsj fixture must exist");
    let seed: serde_json::Value = serde_json::from_str(&raw).expect("seed must parse as JSON");

    let revision = seed["manifest"]["dataModelRevision"]
        .as_u64()
        .expect("manifest.dataModelRevision must be a stamped integer");

    assert_eq!(
        revision, CURRENT_DATA_MODEL_REVISION,
        "governance-seed.srsj is stamped at dataModelRevision {revision}, but the binary's \
         current revision is {CURRENT_DATA_MODEL_REVISION} — re-stamp the seed (both the \
         srs-gov asset and this fixture copy) via `srs repo apply-migration`"
    );
}
