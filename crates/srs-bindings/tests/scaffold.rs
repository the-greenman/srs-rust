//! Integration test for the governance scaffold path (issue #381).
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Exercises service functions directly, not the
//! `SrsRepository` binding wrapper (which calls `js_sys::JSON::parse` and panics off-wasm).
//! Proves that `load_from_srsj` on the raw (pre-RFC-014) governance seed, followed by
//! `create_governance_repository` and `validate_repository`, produces a valid bundle.

use srs_repository::{
    governance_scaffold_service::{create_governance_repository, CreateGovernanceRepositoryInput},
    srsj_migration_service,
    validation,
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

    create_governance_repository(
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

    assert!(
        report.diagnostics.is_empty(),
        "expected no diagnostics after scaffold, got: {:?}",
        report.diagnostics
    );
}
