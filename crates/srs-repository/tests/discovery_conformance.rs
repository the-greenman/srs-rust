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
//! ## Tier 0/2 discovery (srs-rust#797, srs-rust#888)
//!
//! Spec research resolved (issue #793, `epic-256:decision:793-discovery-tier-scope`) that RFC-012
//! `R1`/`I-113` and `R11`/`I-123` require discovery across every live tier — there is no "Phase 1
//! = Tier 2 only" carve-out in the spec. srs-rust#797 landed that composition:
//! `discovery_service::find` composes Tier 0 (Note) alongside Tier 2, so every scenario is
//! asserted directly — no quarantine remains. (Tier 1 / TypedRecord was retired,
//! srs#448/rfc-decision-53635966, srs-rust#888 — the spec's own conformance fixture already
//! carries zero Tier-1 scenarios/content as of that retirement.)

use serde::Deserialize;
use srs_repository::discovery_service::{find, DiscoveryQuery};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::text_projection::{project_note_text, project_text};
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
    /// srs#483 (RFC-012 `[R11]`): optional exact ordered TextSegment expectation
    /// for one field of one instance — closes the gap I-120 left untestable
    /// (segment count/order, which `expectedInstanceIds`/`exactMatch` alone
    /// cannot express).
    expected_segments: Option<ExpectedSegments>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedSegments {
    instance_id: String,
    field_name: String,
    segments: Vec<String>,
}

/// Project one instance's full ordered `TextSegment` stream, dispatching by tier
/// via the catalog. This is glue only — every branch calls the exact same
/// per-tier projection function `discovery_service::find` uses internally
/// (`project_text` / `project_note_text`), never a second implementation of
/// the algorithm itself. (Tier 1 / TypedRecord is retired — srs#448/
/// rfc-decision-53635966, srs-rust#888 — there is no third branch.)
fn project_instance_text(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Vec<srs_repository::text_projection::TextSegment> {
    let cat = store
        .catalog()
        .unwrap_or_else(|e| panic!("catalog() failed: {e}"));
    let entry = cat
        .instances
        .iter()
        .find(|e| e.id == instance_id)
        .unwrap_or_else(|| {
            panic!("expectedSegments.instanceId '{instance_id}' not found in catalog")
        });

    match entry.tier {
        Some(0) => {
            let note = store
                .load_note_by_id(instance_id)
                .unwrap_or_else(|e| panic!("load_note_by_id('{instance_id}') failed: {e}"));
            project_note_text(&note)
        }
        _ => {
            let record = store
                .load_record_by_id(instance_id)
                .unwrap_or_else(|e| panic!("load_record_by_id('{instance_id}') failed: {e}"));
            let index = srs_repository::text_projection::build_field_text_index(store)
                .unwrap_or_else(|e| panic!("build_field_text_index failed: {e}"));
            project_text(&record, &index)
        }
    }
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

    // RFC-039 gate: a revision-2 binary rejects a pre-cutover fixture with an
    // [R9] diagnostic (see srs-rust CLAUDE.md, "RFC-039 carrier note"). Skip
    // until the `srs` cutover PR (unit 2 of the #242 train) migrates the
    // spec-repo-owned fixture — this test must not edit it.
    let fixture_manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_repo.join("manifest.json")).unwrap())
            .unwrap();
    let fixture_revision = fixture_manifest
        .get("dataModelRevision")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if fixture_revision < 2 {
        println!(
            "Skipping: srs/conformance/discovery fixture-repo is at dataModelRevision \
             {fixture_revision} (< 2) — pre-cutover spec repo, awaiting the srs #242 migration"
        );
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

    // Named permanent [R21]/[R2]-independent reader (RFC-038 resolved
    // dispositions): the fixture-repo is conformance test data, never
    // migrated, and this loader is the one sanctioned non-migration exempt
    // call site.
    let store = FileStore::new(&fixture_repo).with_rfc038_exemption();

    let mut failures: Vec<(String, String)> = Vec::new();
    for scenario in &file.scenarios {
        let query = scenario.query.clone();

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

        // srs#483 / I-120: segment COUNT and ORDER, via the real Text Projection.
        if let Some(expected_segments) = &scenario.expected_segments {
            let all_segments = project_instance_text(&store, &expected_segments.instance_id);
            let actual_texts: Vec<&str> = all_segments
                .iter()
                .filter(|s| s.field_name == expected_segments.field_name)
                .map(|s| s.text.as_str())
                .collect();
            let expected_texts: Vec<&str> = expected_segments
                .segments
                .iter()
                .map(String::as_str)
                .collect();
            if actual_texts != expected_texts {
                failures.push((
                    scenario.name.clone(),
                    format!(
                        "'{}' (expectedSegments: {}#{}): expected {expected_texts:?}, got {actual_texts:?}",
                        scenario.name, expected_segments.instance_id, expected_segments.field_name
                    ),
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
        failures
            .iter()
            .map(|(_, detail)| detail.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );

    println!(
        "{} of {} scenarios passed",
        file.scenarios.len() - failures.len(),
        file.scenarios.len(),
    );
}
