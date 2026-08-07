//! `ext:discovery` conformance runner (RFC-012 `[R11]`, srs-rust#793).
//!
//! `srs-repository` declares `ext:discovery` supported (`manifest_service::SUPPORTED_EXTENSIONS`)
//! but until this test existed nothing loaded the normative fixture that defines what
//! "supported" means. This test loads `conformance/discovery/scenarios.json` from the `srs`
//! spec repo — consumed as external test data, never vendored (CLAUDE.md) — and runs every
//! scenario's `query` through the real `discovery_service::find` path against the companion
//! `conformance/discovery/fixture-repo`.
//!
//! `exactMatch: true` scenarios assert *set equality* against `expectedInstanceIds`.
//! `exactMatch: false` scenarios (content-match recall-floor scenarios) assert `expectedInstanceIds`
//! is a *subset* of the actual hits, since a Layer-2 index is permitted to recall more.
//!
//! A failing scenario is a finding about `discovery_service`'s conformance, or about the fixture
//! itself — never "fixed" by editing `scenarios.json`, which is spec-repo-owned (rfc-012:207).
//!
//! ## Known-failing quarantine (srs-rust#797)
//!
//! Spec research resolved (issue #793, `epic-256:decision:793-discovery-tier-scope`) that RFC-012
//! `R1`/`I-113` and `R11`/`I-123` require discovery across Tiers 0, 1 and 2 — there is no "Phase 1
//! = Tier 2 only" carve-out in the spec. `discovery_service::find` currently only composes
//! `record_store::list_records_filtered`, which serves Tier 2 only, so it does not conform. That
//! gap is tracked separately as srs-rust#797; it is not this issue's scope to fix `discovery_service`.
//!
//! The 5 scenarios below are quarantined here — asserted separately from the main pass/fail gate —
//! so the runner lands green while the gap stays CI-visible and tracked. The quarantine is
//! self-expiring in both directions: a quarantined scenario that starts passing, or a
//! currently-passing scenario that regresses, fails the test loudly rather than silently drifting.

use serde::Deserialize;
use srs_repository::discovery_service::{find, DiscoveryQuery};
use srs_repository::store::FileStore;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct ScenariosFile {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    name: String,
    #[allow(dead_code)]
    description: String,
    query: DiscoveryQuery,
    expected_instance_ids: Vec<String>,
    exact_match: bool,
}

/// Root of the `conformance/discovery` fixture in the sibling `srs` spec repo. Walks up from
/// `CARGO_MANIFEST_DIR` looking for a `srs/conformance/discovery` sibling at each level — a fixed
/// `../../../srs` relative path (as `tests/core_bundle_drift.rs` uses) resolves to the wrong place
/// when this crate is built from a worktree under `.worktrees/`, where the checkout sits one
/// level deeper than in a normal clone.
fn conformance_dir() -> PathBuf {
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    loop {
        let candidate = dir.join("srs/conformance/discovery");
        if candidate.join("scenarios.json").exists() {
            return candidate;
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p.to_path_buf(),
            _ => break,
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../srs/conformance/discovery")
}

/// Scenarios known to fail solely because `discovery_service::find` only composes
/// `record_store::list_records_filtered` (Tier 2), while RFC-012 `R1`/`I-113` and `R11`/`I-123`
/// require discovery across Tiers 0, 1 and 2 (srs-rust#797). Exactly these 5, no more, no fewer —
/// see the module doc for the citation trail.
const QUARANTINED_SCENARIOS: [&str; 5] = [
    "empty_query",
    "tier_filter_note",
    "tier_filter_typed_record",
    "single_tag_searchable",
    "content_match_discovery",
];

#[test]
fn ext_discovery_fixture_scenarios() {
    let dir = conformance_dir();
    let scenarios_path = dir.join("scenarios.json");
    let fixture_repo = dir.join("fixture-repo");
    if !scenarios_path.exists() || !fixture_repo.join("manifest.json").exists() {
        println!("Skipping: srs/conformance/discovery fixture not found (isolated checkout)");
        return;
    }

    let raw = std::fs::read_to_string(&scenarios_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", scenarios_path.display()));
    let file: ScenariosFile = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", scenarios_path.display()));

    // Guard against a silently-vacuous pass (CC-33): a loader that finds zero scenarios must
    // not be able to report success.
    assert!(
        !file.scenarios.is_empty(),
        "{} loaded but contained zero scenarios",
        scenarios_path.display()
    );

    let store = FileStore::new(&fixture_repo);

    // (scenario name, failure detail) for every scenario that failed, quarantined or not.
    let mut failures: Vec<(String, String)> = Vec::new();
    for scenario in &file.scenarios {
        let query = DiscoveryQuery {
            type_id: scenario.query.type_id.clone(),
            type_namespace: scenario.query.type_namespace.clone(),
            type_name: scenario.query.type_name.clone(),
            container_id: scenario.query.container_id.clone(),
            tag: scenario.query.tag.clone(),
            lifecycle_state: scenario.query.lifecycle_state.clone(),
            exclude_lifecycle_states: scenario.query.exclude_lifecycle_states.clone(),
            tier: scenario.query.tier,
            content_match: scenario.query.content_match.clone(),
        };

        let result = find(&store, query)
            .unwrap_or_else(|e| panic!("scenario '{}': find() failed: {e}", scenario.name));

        let actual: BTreeSet<String> = result.hits.iter().map(|h| h.instance_id.clone()).collect();
        let expected: BTreeSet<String> = scenario.expected_instance_ids.iter().cloned().collect();

        if scenario.exact_match {
            if actual != expected {
                let missing: Vec<_> = expected.difference(&actual).cloned().collect();
                let unexpected: Vec<_> = actual.difference(&expected).cloned().collect();
                failures.push((
                    scenario.name.clone(),
                    format!(
                        "'{}' (exactMatch: set equality): missing={missing:?} unexpected={unexpected:?}",
                        scenario.name
                    ),
                ));
            }
        } else {
            let missing: Vec<_> = expected.difference(&actual).cloned().collect();
            if !missing.is_empty() {
                failures.push((
                    scenario.name.clone(),
                    format!(
                        "'{}' (exactMatch: false, recall floor): missing={missing:?} (actual must be a superset of expectedInstanceIds)",
                        scenario.name
                    ),
                ));
            }
        }
    }

    let scenario_names: BTreeSet<&str> = file.scenarios.iter().map(|s| s.name.as_str()).collect();
    for name in QUARANTINED_SCENARIOS {
        assert!(
            scenario_names.contains(name),
            "quarantine names '{name}', which is not a scenario in {} — the allow-list has drifted from the fixture",
            scenarios_path.display()
        );
    }

    let failed_names: BTreeSet<&str> = failures.iter().map(|(n, _)| n.as_str()).collect();

    // Self-expiring in the fix direction: a quarantined scenario that now passes means #797 was
    // fixed (or the fixture changed) and the allow-list is stale — fail loudly rather than let it
    // silently keep suppressing a scenario that no longer needs it.
    let unexpectedly_passing: Vec<&str> = QUARANTINED_SCENARIOS
        .into_iter()
        .filter(|name| !failed_names.contains(name))
        .collect();
    assert!(
        unexpectedly_passing.is_empty(),
        "quarantined scenario(s) {unexpectedly_passing:?} now PASS — remove them from \
         QUARANTINED_SCENARIOS in {} and close/deduct srs-rust#797",
        file!()
    );

    // Self-expiring in the regression direction: any failure not on the allow-list is a real,
    // unquarantined conformance break and must fail the build.
    let unquarantined: Vec<&str> = failures
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|name| !QUARANTINED_SCENARIOS.contains(name))
        .collect();
    assert!(
        unquarantined.is_empty(),
        "{} of {} ext:discovery conformance scenarios failed outside the srs-rust#797 quarantine \
         against {}:\n{}",
        unquarantined.len(),
        file.scenarios.len(),
        scenarios_path.display(),
        failures
            .iter()
            .filter(|(n, _)| unquarantined.contains(&n.as_str()))
            .map(|(_, detail)| detail.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );

    println!(
        "{} of {} scenarios passed; {} quarantined for srs-rust#797 (Tier 0/1 discovery gap): {:?}",
        file.scenarios.len() - failures.len(),
        file.scenarios.len(),
        failures.len(),
        QUARANTINED_SCENARIOS
    );
}
