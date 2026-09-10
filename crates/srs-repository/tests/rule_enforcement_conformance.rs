//! RFC-032 `[R3]` / `V3` rule-enforcement conformance gate (srs-rust#1003).
//!
//! `repo validate` declared RFC-032 conformance checks (`validate_field_v3`,
//! `validate_field_type_conformance`) but until this test existed nothing proved they
//! actually fire through the **production** entry point: a `select`/closed-domain field
//! whose `vocabularyRef` resolved to a `mode:open` Vocabulary validated clean (#1003),
//! and the R3 family was wired at `Warning` severity, which `repo validate`'s exit code
//! never surfaces. This is the same disease `discovery_conformance.rs`'s docstring names
//! for `ext:discovery`: a capability "declared supported" with no test loading the
//! fixture that defines what "supported" means.
//!
//! Modeled directly on `discovery_conformance.rs`'s shape (load fixture with `FileStore`,
//! run the real service, assert on the result) — see that file first.
//!
//! ## Deliberate scope limits
//!
//! - This runner walks **fixtures only**, under `tests/fixtures/rule-enforcement/`,
//!   never a live corpus. Live-corpus cleanliness (the actual ecosystem blast-radius
//!   check before promoting a Warning to an Error) is `scripts/check-corpus-conformance.sh`'s
//!   job, run separately over the first-party corpora.
//! - The roster (`roster.json`) and this file's `RFC_032_R3_CLAUSES` const are
//!   **hand-transcribed** from `rfc-032-composite-field-range.md`'s `[R3]` clause and
//!   RFC-006's `V3` (vocabularyRef mode) text — never derived from the validator code.
//!   Deriving the clause list from what the code happens to check would be circular: a
//!   clause the implementation never enforced (exactly #1003's bug) could never appear
//!   as "missing" if the list came from the same code that has the gap.
//!
//! Three assertions, each below:
//! 1. `roster_entries_fire` — every roster fixture's `broken/` repo produces a
//!    diagnostic of the declared severity carrying the declared token, via
//!    `validation::validate_repository` (never `validate_field_v3` directly).
//! 2. `clean_twins_report_zero_errors` — every roster fixture's `clean/` twin (identical
//!    but for the one violation) reports zero errors, so the gate cannot pass by making
//!    everything red.
//! 3. `every_rfc_032_r3_clause_has_a_fixture` — the roster's rule-id set EQUALS (not
//!    merely a subset of) `RFC_032_R3_CLAUSES`. A clause added to the spec with no
//!    fixture fails the build; a fixture roster entry with no corresponding spec clause
//!    also fails the build.

use serde::Deserialize;
use srs_repository::store::FileStore;
use srs_repository::validation::{validate_repository, DiagnosticSeverity};
use std::path::{Path, PathBuf};

/// RFC-032 `rfc-032-composite-field-range.md` `[R3]`, hand-transcribed:
///
/// > **[R3]** `valueDomain` is meaningful only for `datatype == "string"` (extended to
/// > `integer` by srs#534). When `closed`, exactly one of `allowedValues` or
/// > `vocabularyRef` MUST be present; a `vocabularyRef` MUST resolve to a `mode:closed`
/// > Vocabulary (RFC-006).
///
/// Decomposed into the clauses this gate proves are each independently enforced:
///   - `r3.a` — `valueDomain` present on a datatype other than string/integer is rejected.
///   - `r3.b` — `closed` with BOTH `allowedValues` and `vocabularyRef` is rejected
///     (the "exactly one" clause, over-binding side).
///   - `r3.c` — a binding (`allowedValues`/`vocabularyRef`) present with `valueDomain`
///     absent is rejected (a binding implies `closed`).
///   - `r3.d` — a binding present with `valueDomain` explicitly `"open"` is rejected
///     (srs-rust#1003's `FieldType::validate` gap: only the absent case was checked).
///   - `v3` — a `vocabularyRef` on a `closed` field resolving to a `mode:open` Vocabulary
///     is rejected (srs-rust#1003's headline bug: `repo validate` accepted this).
const RFC_032_R3_CLAUSES: &[&str] = &["r3.a", "r3.b", "r3.c", "r3.d", "v3"];

#[derive(Debug, Deserialize)]
struct Roster {
    rules: Vec<RosterEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RosterEntry {
    id: String,
    #[allow(dead_code)]
    rfc_clause: String,
    expect_severity: String,
    rule_token: String,
    broken_path: String,
    clean_path: String,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rule-enforcement")
}

fn load_roster() -> Roster {
    let path = fixtures_dir().join("roster.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

fn severity_of(s: &str) -> DiagnosticSeverity {
    match s {
        "error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        other => panic!("unknown expectSeverity '{other}' in roster.json"),
    }
}

#[test]
fn roster_entries_fire() {
    let roster = load_roster();
    assert!(
        !roster.rules.is_empty(),
        "roster.json loaded but contained zero rules"
    );

    let mut failures: Vec<String> = Vec::new();
    for entry in &roster.rules {
        let repo_path = fixtures_dir().join(&entry.broken_path);
        let store = FileStore::new(&repo_path);
        let report = validate_repository(&store).unwrap_or_else(|e| {
            panic!(
                "rule '{}': validate_repository failed to load {}: {e}",
                entry.id,
                repo_path.display()
            )
        });

        let expected_severity = severity_of(&entry.expect_severity);
        let hit = report
            .diagnostics
            .iter()
            .any(|d| d.severity == expected_severity && d.message.contains(&entry.rule_token));

        if !hit {
            failures.push(format!(
                "rule '{}' ({}): expected a {} diagnostic containing '{}' from {}, got: {:?}",
                entry.id,
                entry.rfc_clause,
                entry.expect_severity,
                entry.rule_token,
                repo_path.display(),
                report.diagnostics
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} rule-enforcement fixtures did not fire:\n{}",
        failures.len(),
        roster.rules.len(),
        failures.join("\n")
    );
}

#[test]
fn clean_twins_report_zero_errors() {
    let roster = load_roster();
    let mut failures: Vec<String> = Vec::new();

    for entry in &roster.rules {
        let repo_path = fixtures_dir().join(&entry.clean_path);
        let store = FileStore::new(&repo_path);
        let report = validate_repository(&store).unwrap_or_else(|e| {
            panic!(
                "rule '{}': validate_repository failed to load {}: {e}",
                entry.id,
                repo_path.display()
            )
        });

        if report.summary.errors != 0 {
            failures.push(format!(
                "rule '{}': clean twin {} reported {} error(s), expected 0: {:?}",
                entry.id,
                repo_path.display(),
                report.summary.errors,
                report.diagnostics
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} clean-twin fixture(s) unexpectedly reported errors (gate would pass by making \
         everything red):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_rfc_032_r3_clause_has_a_fixture() {
    let roster = load_roster();
    let roster_ids: std::collections::BTreeSet<&str> =
        roster.rules.iter().map(|r| r.id.as_str()).collect();
    let spec_ids: std::collections::BTreeSet<&str> = RFC_032_R3_CLAUSES.iter().copied().collect();

    let missing: Vec<_> = spec_ids.difference(&roster_ids).collect();
    let extra: Vec<_> = roster_ids.difference(&spec_ids).collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "roster.json rule-id set must EQUAL RFC_032_R3_CLAUSES — missing (spec clause, no \
         fixture): {missing:?}; extra (fixture, no spec clause): {extra:?}"
    );
}
