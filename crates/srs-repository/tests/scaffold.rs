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
/// moves, this test goes red until the seed is re-vendored: srs republishes
/// `com.mudemocracy.governance` with a seed built at the new revision, and
/// `packages/com.mudemocracy.governance/<version>/seed/empty-governance-document.srsj`
/// is copied byte-for-byte over this fixture (the single copy; also the release
/// asset). `seed_is_a_byte_copy_of_the_published_seed` holds the copy exact.
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
         current revision is {CURRENT_DATA_MODEL_REVISION} — re-vendor the seed srs publishes \
         at that revision"
    );
}

/// srs-rust#1452 (ADR-046, convergence): the vendored seed is the published
/// `com.mudemocracy.governance` seed byte-for-byte — for the version its own
/// `manifest.upstreamPackage` names — never a hand-edited fork. Resolves the spec
/// repo as `core_bundle_drift` does: `SRS_SPEC_DIR`, else a sibling `srs`
/// checkout on master; skips when neither is present.
#[test]
fn seed_is_a_byte_copy_of_the_published_seed() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/governance-seed.srsj");
    let raw = std::fs::read_to_string(&fixture).expect("governance-seed.srsj fixture must exist");
    let seed: serde_json::Value = serde_json::from_str(&raw).expect("seed must parse as JSON");
    let version = seed["manifest"]["upstreamPackage"]["version"]
        .as_str()
        .expect("manifest.upstreamPackage.version names the published version");
    let rel = format!(
        "packages/com.mudemocracy.governance/{version}/seed/empty-governance-document.srsj"
    );

    let spec = match std::env::var("SRS_SPEC_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => {
            let sibling = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../srs");
            if !sibling.join(&rel).exists() {
                return;
            }
            let out = std::process::Command::new("git")
                .args([
                    "-C",
                    sibling.to_str().unwrap_or("."),
                    "rev-parse",
                    "--abbrev-ref",
                    "HEAD",
                ])
                .output();
            if let Ok(out) = out {
                let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
                assert!(
                    !out.status.success() || branch == "master",
                    "srs sibling checkout is on '{branch}', not master — set SRS_SPEC_DIR to a \
                     fresh `origin/master` clone (srs-rust#874)"
                );
            }
            sibling
        }
    };
    let Ok(published) = std::fs::read_to_string(spec.join(&rel)) else {
        return; // explicit but unusable SRS_SPEC_DIR: skip, as core_bundle_drift does
    };
    // Between a revision bump and srs republishing the seed at the new revision (the
    // revision-bump choreography), the vendored seed is the published one brought to the
    // current revision through the registry ladder (`srs repo apply-migration` on the .srsj),
    // byte for byte — still never a hand edit.
    let published_revision = serde_json::from_str::<serde_json::Value>(&published)
        .ok()
        .and_then(|v| v["manifest"]["dataModelRevision"].as_u64())
        .unwrap_or(0);
    let expected = if published_revision < CURRENT_DATA_MODEL_REVISION {
        srs_repository::rfc052_root_container_migration_service::ladder_srsj_fixture(&published)
            .expect("the published seed climbs the ladder")
    } else {
        published
    };
    assert!(
        raw == expected,
        "governance-seed.srsj is not a byte-copy of srs {rel} (laddered to the current \
         revision while srs still publishes an older one) — re-vendor it with `cp` (or `srs \
         repo apply-migration` on the copy), never hand-edit (ADR-046)"
    );
}
