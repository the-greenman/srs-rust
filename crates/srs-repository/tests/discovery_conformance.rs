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

    let mut failures = Vec::new();
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
                failures.push(format!(
                    "'{}' (exactMatch: set equality): missing={missing:?} unexpected={unexpected:?}",
                    scenario.name
                ));
            }
        } else {
            let missing: Vec<_> = expected.difference(&actual).cloned().collect();
            if !missing.is_empty() {
                failures.push(format!(
                    "'{}' (exactMatch: false, recall floor): missing={missing:?} (actual must be a superset of expectedInstanceIds)",
                    scenario.name
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} ext:discovery conformance scenarios failed against {}:\n{}",
        failures.len(),
        file.scenarios.len(),
        scenarios_path.display(),
        failures.join("\n")
    );
}
