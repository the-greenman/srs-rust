//! Integration test for the governance scaffold path (issue #381).
//!
//! Proves that `load_from_srsj` on the raw (pre-RFC-014) governance seed, followed by
//! `create_governance_repository` and `validate_repository`, produces a valid bundle.

use srs_repository::{
    governance_scaffold_service::{create_governance_repository, CreateGovernanceRepositoryInput},
    srsj_migration_service, validation,
};

#[test]
fn scaffold_from_raw_seed_produces_valid_repository() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/governance-seed.srsj"
    ))
    .expect("governance-seed.srsj fixture must exist");

    let store = srsj_migration_service::load_from_srsj(&raw)
        .expect("load_from_srsj must succeed on raw (pre-migration) seed");

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
        "fresh scaffold must not ship dangling document-view container refs: {dangling:?}"
    );
}
