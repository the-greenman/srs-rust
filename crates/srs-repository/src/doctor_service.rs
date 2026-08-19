//! `srs repo doctor` — explicit, user-invoked detect-and-repair surface for
//! raw file adds and manual edits (srs-rust#857, generalizing srs-rust#844).
//!
//! Manual edits and copy-then-edit adds WILL happen; the core accommodates
//! them by **repair**, never by silent tolerance ([R21]). This module is the
//! one place that maps a diagnosable catalog class to either a deterministic
//! repair or a named manual step. It is **never** invoked on any load path
//! (ADR-047) — only from the `repo doctor` CLI command and its WASM/MCP
//! twins, all driven by an explicit user request.
//!
//! ## Read path
//!
//! Detection uses [`crate::store::RepositoryStore::catalog_unchecked`]
//! (`catalog::build`) — the diagnostics-carrying, non-[R24]-fatal variant
//! `repo validate` already uses. This is the reuse ADR-045 calls for: doctor
//! adds no second unchecked seam, it is simply another consumer of the one
//! that already exists.
//!
//! A retired manifest property or a too-low `dataModelRevision` makes
//! `catalog::build`'s own internal `store.load_manifest()` call fail; `build`
//! reports that as a single `MANIFEST_INVALID` diagnostic and returns an
//! otherwise-empty catalog rather than failing the call (see `catalog::build`
//! doc comment) — so those two classes are diagnosed from a **raw** read
//! (`load_manifest_raw_text`, never the checked `load_manifest`) independent
//! of the catalog. `load_manifest()`'s *typed* deserialize can fail for
//! other reasons a manual edit can produce (e.g. a malformed sub-object) that
//! the raw probe's two named classes do not cover — `doctor()` tracks
//! whether the raw probe actually reported something and only treats a
//! `MANIFEST_INVALID` catalog diagnostic as redundant when it did; otherwise
//! it is surfaced verbatim as its own manual-step finding (round-2 review:
//! doctor must never report a repository clean when the manifest failed to
//! load for any reason).
//!
//! `--fix` repairs one catalog-derived diagnostic at a time and **rebuilds
//! the catalog before picking the next one**, rather than acting on a single
//! snapshot taken up front. Two diagnostics can name the same locator (a
//! relation with both an [R11] filename mismatch and an [R13] dangling
//! endpoint) or the same underlying fault (both endpoints of one relation
//! dangling emits one diagnostic per endpoint); repairing one can move,
//! delete, or otherwise change what a second diagnostic's locator pointed
//! to, so acting on the pre-repair snapshot risks silently no-op'ing against
//! whatever used to be there. A dry run needs no such loop — nothing on disk
//! changes, so one snapshot is an accurate preview of everything visible in
//! it.
//!
//! ## The repair inventory
//!
//! | Class | Repair | Mechanism reused |
//! |---|---|---|
//! | retired manifest keys | deterministic (file-tree stores only) | `rfc038_storage_migration_service::migrate_storage` |
//! | unsupported generation | manual | `repo apply-migration` |
//! | duplicate instance id ("adopt") | deterministic *when the id has no incoming references*; ambiguous otherwise | fresh id via `writer::new_instance_id`, `save_instance_json` |
//! | dangling container membership | deterministic | `container_service::remove_member` / `remove_root` (ADR-045) |
//! | dangling relation endpoint | deterministic (relation deleted — an endpoint cannot be repaired without guessing) | `store.delete_relations_json` |
//! | relation filename↔id mismatch | deterministic | `store.save_relation` + delete the old file |
//! | everything else (malformed candidates, other duplicate-id classes, dangling field refs, ...) | manual | reported verbatim — never guessed |
//!
//! ### Adopt's identity semantics
//!
//! A duplicate instance id is the natural "copy this record, then edit it"
//! act. Adopt mints a fresh id for every copy but one, preserving content;
//! relations and container membership are deliberately **not** inherited by
//! the reminted copies — they still name the id that was kept. When the
//! duplicate id has *any* incoming relation or container reference, which
//! physical file those references were meant to resolve to is genuinely
//! undecidable from data alone (both files share the id today), so that
//! specific duplicate is left as a named manual step rather than guessed
//! (srs-rust#857 stop condition). The common case this issue's own
//! reproduction hits — a fresh verbatim copy, before anything else in the
//! repository could reference it — has no incoming references and is
//! therefore always auto-repairable.

use crate::catalog::{codes, CatalogDiagnostic, RepositoryCatalog};
use crate::container_service;
use crate::error::RepositoryError;
use crate::manifest::rfc038::RETIRED_PROPERTIES;
use crate::manifest::MIN_SUPPORTED_DATA_MODEL_REVISION;
use crate::rfc038_storage_migration_service::{self, StorageMigrationOptions};
use crate::store::{relation_object_from_value, relation_object_path, RepositoryStore};
use crate::writer::new_instance_id;
use serde::Serialize;

/// `repo doctor` input. Dry-run (`fix: false`) is the default at every call
/// site — `--fix` is what flips it.
#[derive(Debug, Clone, Copy, Default)]
pub struct DoctorInput {
    pub fix: bool,
}

/// The diagnosable class a finding belongs to, per the repair inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoctorClass {
    RetiredManifestKeys,
    UnsupportedGeneration,
    DuplicateInstanceId,
    DanglingContainerMembership,
    DanglingRelationEndpoint,
    RelationFilenameMismatch,
    /// Every diagnosable class with no automated repair: other duplicate-id
    /// sets (relation/container/source-document/definition/extension),
    /// dangling `FieldAssignment.fieldId` references, malformed candidates,
    /// and anything catalog.rs diagnoses that this table does not name.
    Unrepaired,
}

/// What doctor did (or would do) about one finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoctorOutcome {
    /// `--fix` applied the repair.
    Repaired,
    /// Dry run: this is what `--fix` would do.
    WouldRepair,
    /// A repair mechanism exists for this class, but this specific instance
    /// is undecidable from data alone — the adopt stop condition.
    Ambiguous,
    /// No automated repair exists for this class, or the attempted repair
    /// itself failed — named, never guessed.
    ManualStep,
}

/// One diagnosable problem, and what doctor did or would do about it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorFinding {
    pub class: DoctorClass,
    /// Locators from the underlying catalog diagnostic (or `manifest.json`
    /// for the raw manifest-level classes) — the same locator vocabulary
    /// `repo validate` reports.
    pub locators: Vec<String>,
    /// The diagnostic message, verbatim where it comes from the catalog.
    pub message: String,
    pub outcome: DoctorOutcome,
    /// Truthful diagnostics (srs-rust#857): what was repaired, would be
    /// repaired, or why it was not.
    pub detail: String,
}

/// The full doctor report. Dry-run by construction unless `input.fix` was
/// set — see [`DoctorReport::fix_applied`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub fix_applied: bool,
    pub findings: Vec<DoctorFinding>,
    pub repaired: usize,
    pub remaining: usize,
}

/// Safety bound on the fix loop below — never expected to bind in practice
/// (every repair either removes the diagnostic that triggered it or leaves
/// it unrepaired, and `seen` stops an unrepaired diagnostic from being
/// retried), but a tool that loops while writing to disk on untrusted
/// repository content gets a hard stop rather than a hang if some future
/// repair class ever violates that invariant.
const MAX_FIX_ITERATIONS: usize = 10_000;

/// Run doctor once. Never called from any load path — the caller (CLI/WASM
/// handler) is the only place this is invoked, always on explicit request.
pub fn doctor(
    store: &dyn RepositoryStore,
    input: DoctorInput,
) -> Result<DoctorReport, RepositoryError> {
    let mut report = DoctorReport {
        fix_applied: input.fix,
        ..Default::default()
    };

    // Whether the raw probe already reported a manifest-level finding — a
    // `codes::MANIFEST_INVALID` catalog diagnostic below is redundant with
    // it exactly when this is true, and otherwise names a manifest fault
    // the raw probe does not classify (e.g. a malformed sub-object the
    // typed `Manifest` shape rejects) and must be surfaced, not dropped.
    let manifest_fault_reported = check_manifest_raw(store, input.fix, &mut report);

    // The catalog-derived classes. Reuses the existing repair-only unchecked
    // seam (ADR-045) rather than adding a second one (srs-rust#844's ask).
    //
    // Dry run: one snapshot suffices — nothing on disk changes, so no
    // repair can invalidate a locator or a fault another finding's preview
    // describes; every finding is a preview only.
    //
    // `--fix`: repairs one diagnostic, then **rebuilds the catalog before
    // picking the next one**. Two diagnostics can name the same locator
    // (e.g. a relation with both an [R11] filename mismatch and an [R13]
    // dangling endpoint) or the same underlying fault (a relation with both
    // endpoints dangling emits one diagnostic per endpoint) — repairing one
    // can move, delete, or otherwise change what a second diagnostic's
    // locator pointed to. Acting on a diagnostic snapshot taken before an
    // earlier repair in the same pass would silently no-op against
    // whatever used to be there. `seen` (a diagnostic's own `(code,
    // locators, message)`) stops an unrepaired diagnostic — `ManualStep` or
    // `Ambiguous`, which does not change the disk state that produced it —
    // from being retried forever once nothing distinguishes rebuild N from
    // rebuild N+1 for that one finding.
    if input.fix {
        let mut seen: std::collections::HashSet<(&'static str, Vec<String>, String)> =
            std::collections::HashSet::new();
        for _ in 0..MAX_FIX_ITERATIONS {
            // Not `?`: by the second iteration onward this call follows
            // disk-mutating repairs already recorded in `report`. Propagating
            // the error here would discard that whole record — every repair
            // genuinely already applied to disk — leaving the caller with
            // nothing but an error and no idea anything happened. Truthful
            // diagnostics (the same principle `repair_duplicate_instance`'s
            // partial-failure path already follows) means returning what was
            // actually done, plus a finding naming the failure, instead.
            let cat = match store.catalog_unchecked() {
                Ok(c) => c,
                Err(e) => {
                    let repaired_so_far = report
                        .findings
                        .iter()
                        .filter(|f| f.outcome == DoctorOutcome::Repaired)
                        .count();
                    report.findings.push(DoctorFinding {
                        class: DoctorClass::Unrepaired,
                        locators: Vec::new(),
                        message: format!(
                            "failed to rebuild the catalog after {repaired_so_far} repair(s)"
                        ),
                        outcome: DoctorOutcome::ManualStep,
                        detail: format!(
                            "{e} — repairs already applied above are intact on disk; re-run \
                             `repo doctor --fix` once the underlying I/O error is resolved"
                        ),
                    });
                    break;
                }
            };
            let next = cat.diagnostics.iter().find(|d| {
                d.code != codes::MANIFEST_INVALID
                    && !seen.contains(&(d.code, d.locators.clone(), d.message.clone()))
            });
            let Some(diag) = next else {
                if manifest_fault_reported {
                    break;
                }
                // No repairable/seen diagnostic remains, but a MANIFEST_INVALID
                // may still be sitting there unclassified by the raw probe —
                // surface it once rather than loop on it (it is not in `seen`'s
                // domain, since the `find` above always excludes that code).
                if let Some(d) = cat
                    .diagnostics
                    .iter()
                    .find(|d| d.code == codes::MANIFEST_INVALID)
                {
                    report.findings.push(unclassified_manifest_finding(d));
                }
                break;
            };
            seen.insert((diag.code, diag.locators.clone(), diag.message.clone()));
            report
                .findings
                .push(classify_and_repair(store, &cat, diag, true));
        }
    } else {
        let cat = store.catalog_unchecked()?;
        for diag in &cat.diagnostics {
            if diag.code == codes::MANIFEST_INVALID {
                if !manifest_fault_reported {
                    report.findings.push(unclassified_manifest_finding(diag));
                }
                continue;
            }
            report
                .findings
                .push(classify_and_repair(store, &cat, diag, false));
        }
    }

    report.repaired = report
        .findings
        .iter()
        .filter(|f| f.outcome == DoctorOutcome::Repaired)
        .count();
    report.remaining = report.findings.len() - report.repaired;
    Ok(report)
}

/// A `codes::MANIFEST_INVALID` diagnostic the raw manifest probe did not
/// classify — surfaced verbatim rather than silently dropped (srs-rust#857
/// review round 2: doctor must never report a repository clean when
/// `catalog::build`'s own internal manifest load failed for a reason
/// outside the two classes `check_manifest_raw` names).
fn unclassified_manifest_finding(diag: &CatalogDiagnostic) -> DoctorFinding {
    DoctorFinding {
        class: DoctorClass::Unrepaired,
        locators: diag.locators.clone(),
        message: diag.message.clone(),
        outcome: DoctorOutcome::ManualStep,
        detail: "manifest.json failed to load through the typed path for a reason outside \
                 doctor's raw-probe classes (retired keys, unsupported generation) — inspect \
                 manifest.json by hand; doctor never guesses content"
            .to_string(),
    }
}

// ---------------------------------------------------------------------------
// Manifest-level classes (raw read, independent of the catalog)
// ---------------------------------------------------------------------------

/// Returns `true` when this raw probe reported at least one manifest-level
/// finding. `doctor()` uses that to decide whether a `codes::MANIFEST_INVALID`
/// catalog diagnostic (`catalog::build`'s own generic fallback for "the
/// internal `load_manifest()` call failed") is redundant with a finding
/// already pushed here, or whether it names a manifest fault this raw probe
/// does not classify (e.g. a malformed sub-object the typed `Manifest` shape
/// rejects but that is neither a retired key nor a low `dataModelRevision`)
/// — in which case it must be surfaced, not silently dropped.
fn check_manifest_raw(store: &dyn RepositoryStore, fix: bool, report: &mut DoctorReport) -> bool {
    let text = match store.load_manifest_raw_text() {
        Ok(t) => t,
        Err(e) => {
            report.findings.push(DoctorFinding {
                class: DoctorClass::Unrepaired,
                locators: vec!["manifest.json".to_string()],
                message: format!("manifest.json is missing or unreadable: {e}"),
                outcome: DoctorOutcome::ManualStep,
                detail: "restore manifest.json — doctor never guesses content".to_string(),
            });
            return true;
        }
    };
    let manifest: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            report.findings.push(DoctorFinding {
                class: DoctorClass::Unrepaired,
                locators: vec!["manifest.json".to_string()],
                message: format!("manifest.json does not parse as JSON: {e}"),
                outcome: DoctorOutcome::ManualStep,
                detail: "hand-repair manifest.json's JSON syntax — doctor never guesses content"
                    .to_string(),
            });
            return true;
        }
    };

    let mut reported = false;

    let retired: Vec<&'static str> = RETIRED_PROPERTIES
        .iter()
        .copied()
        .filter(|p| manifest.get(p).is_some())
        .collect();
    if !retired.is_empty() {
        let message = format!(
            "manifest.json declares retired propert{} {} (RFC-038 Change K)",
            if retired.len() == 1 { "y" } else { "ies" },
            retired.join(", ")
        );
        let (outcome, detail) = if !fix {
            (
                DoctorOutcome::WouldRepair,
                "would run the rfc038-storage migration (same transform `repo apply-migration \
                 --id rfc038-storage` runs). NOTE: this manifest fault makes catalog::build's \
                 own internal manifest read fail, so every catalog-derived finding (duplicate \
                 ids, dangling references, filename mismatches) is invisible until it is \
                 repaired — this dry run cannot show them. `--fix` strips the retired keys AND \
                 then repairs whatever it can newly see in the same pass; re-run `repo doctor` \
                 (dry run) afterward for a preview of what remains."
                    .to_string(),
            )
        } else if !store.is_file_tree_store() {
            (
                DoctorOutcome::ManualStep,
                "rfc038-storage is a file-placement transform and applies only to a file-tree \
                 store; run `repo apply-migration --id rfc038-storage` against a disk repository"
                    .to_string(),
            )
        } else {
            match rfc038_storage_migration_service::migrate_storage(
                store,
                &StorageMigrationOptions {
                    allow_non_atomic: true,
                },
            ) {
                Ok(result) => (
                    DoctorOutcome::Repaired,
                    format!(
                        "ran the rfc038-storage migration: stripped {:?}, exploded {} relation(s), \
                         removed {} collection file(s)",
                        result.manifest_properties_stripped,
                        result.relations_exploded,
                        result.collections_removed.len()
                    ),
                ),
                Err(e) => (
                    DoctorOutcome::ManualStep,
                    format!("rfc038-storage migration failed: {e}"),
                ),
            }
        };
        report.findings.push(DoctorFinding {
            class: DoctorClass::RetiredManifestKeys,
            locators: vec!["manifest.json".to_string()],
            message,
            outcome,
            detail,
        });
        reported = true;
    }

    let revision = manifest
        .get("dataModelRevision")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if revision < MIN_SUPPORTED_DATA_MODEL_REVISION {
        report.findings.push(DoctorFinding {
            class: DoctorClass::UnsupportedGeneration,
            locators: vec!["manifest.json".to_string()],
            message: format!(
                "manifest.json declares dataModelRevision {revision}; this build requires >= \
                 {MIN_SUPPORTED_DATA_MODEL_REVISION}"
            ),
            outcome: DoctorOutcome::ManualStep,
            detail: "run `repo apply-migration` (field-type, then rfc039-carrier) to reach a \
                      supported generation — doctor targets raw-add/manual-edit damage on an \
                      already-supported repository, not generation migration"
                .to_string(),
        });
        reported = true;
    }

    reported
}

// ---------------------------------------------------------------------------
// Catalog-derived classes
// ---------------------------------------------------------------------------

fn classify_and_repair(
    store: &dyn RepositoryStore,
    cat: &RepositoryCatalog,
    diag: &CatalogDiagnostic,
    fix: bool,
) -> DoctorFinding {
    let unrepaired = |detail: String| DoctorFinding {
        class: DoctorClass::Unrepaired,
        locators: diag.locators.clone(),
        message: diag.message.clone(),
        outcome: DoctorOutcome::ManualStep,
        detail,
    };

    if diag.code == codes::DUPLICATE_ID {
        return match duplicate_set_and_id(&diag.message) {
            Some(("instance", id)) => {
                let (outcome, detail) =
                    repair_duplicate_instance(store, cat, id, &diag.locators, fix);
                DoctorFinding {
                    class: DoctorClass::DuplicateInstanceId,
                    locators: diag.locators.clone(),
                    message: diag.message.clone(),
                    outcome,
                    detail,
                }
            }
            _ => unrepaired(
                "adopt is defined for duplicate instance ids only; resolve this duplicate by \
                 hand — doctor never guesses which object should keep the id"
                    .to_string(),
            ),
        };
    }

    if diag.code == codes::DANGLING_REFERENCE {
        if let Some((prop, id)) = container_dangling_prop_and_id(&diag.message) {
            let locator = diag.locators.first().cloned().unwrap_or_default();
            let (outcome, detail) = repair_dangling_container(store, cat, &locator, prop, id, fix);
            return DoctorFinding {
                class: DoctorClass::DanglingContainerMembership,
                locators: diag.locators.clone(),
                message: diag.message.clone(),
                outcome,
                detail,
            };
        }
        if let Some(relation_id) = relation_dangling_id(&diag.message) {
            let locator = diag.locators.first().cloned().unwrap_or_default();
            let (outcome, detail) = repair_dangling_relation(store, &locator, relation_id, fix);
            return DoctorFinding {
                class: DoctorClass::DanglingRelationEndpoint,
                locators: diag.locators.clone(),
                message: diag.message.clone(),
                outcome,
                detail,
            };
        }
        return unrepaired(
            "dangling-reference repair is defined for container membership and relation \
             endpoints only (not FieldAssignment.fieldId references) — resolve by hand"
                .to_string(),
        );
    }

    if diag.code == codes::RELATION_FILENAME_MISMATCH {
        let locator = diag.locators.first().cloned().unwrap_or_default();
        let (outcome, detail) = repair_relation_filename_mismatch(store, &locator, fix);
        return DoctorFinding {
            class: DoctorClass::RelationFilenameMismatch,
            locators: diag.locators.clone(),
            message: diag.message.clone(),
            outcome,
            detail,
        };
    }

    unrepaired(format!(
        "no automated repair for {} — report only, see `repo validate` for the same diagnostic; \
         doctor never guesses content",
        diag.code
    ))
}

/// Parse `"duplicate {set} identifier '{id}' declared by N objects"` (the
/// exact text `catalog::Builder::detect_duplicates` emits). Manual parsing,
/// not a regex dependency: the producer and consumer of this format live in
/// the same crate and change together; the `parses_*_message` unit tests
/// below (`tests` module) are the tripwire if they ever drift.
fn duplicate_set_and_id(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix("duplicate ")?;
    let (set_name, rest) = rest.split_once(" identifier '")?;
    let (id, _) = rest.split_once('\'')?;
    Some((set_name, id))
}

/// Parse `"container {prop} '{id}' resolves to nothing in the instance set"`.
fn container_dangling_prop_and_id(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix("container ")?;
    let (prop, rest) = rest.split_once(" '")?;
    let (id, _) = rest.split_once('\'')?;
    Some((prop, id))
}

/// Parse `"relation '{relationId}' {sourceInstanceId|targetInstanceId} '{id}' \
/// resolves to nothing in the instance set"` — only the relation id is
/// needed; the whole relation is removed (see [`repair_dangling_relation`]).
fn relation_dangling_id(message: &str) -> Option<&str> {
    let rest = message.strip_prefix("relation '")?;
    let (relation_id, _) = rest.split_once('\'')?;
    Some(relation_id)
}

/// Adopt: a duplicate instance id gets a freshly minted id for every copy
/// but one. Safe (and applied) only when the id has no incoming relation or
/// container reference — see the module doc's "Adopt's identity semantics".
fn repair_duplicate_instance(
    store: &dyn RepositoryStore,
    cat: &RepositoryCatalog,
    id: &str,
    locators: &[String],
    fix: bool,
) -> (DoctorOutcome, String) {
    let mut locators = locators.to_vec();
    locators.sort();

    if instance_id_referenced(store, cat, id) {
        return (
            DoctorOutcome::Ambiguous,
            format!(
                "'{id}' has an incoming relation or container reference; adopt cannot tell \
                 which copy those reference — both files declare the same id today. Decide \
                 which file is canonical, then remint the other's instanceId by hand (see \
                 srs-rust#857)."
            ),
        );
    }

    let Some((keeper, rest)) = locators.split_first() else {
        return (
            DoctorOutcome::ManualStep,
            "duplicate-id diagnostic named no locators".to_string(),
        );
    };

    if !fix {
        return (
            DoctorOutcome::WouldRepair,
            format!(
                "would keep '{id}' at {keeper}; would remint: {}",
                rest.join(", ")
            ),
        );
    }

    let mut reminted = Vec::new();
    for locator in rest {
        match adopt_one(store, locator) {
            Ok(new_id) => reminted.push(format!("{locator} -> {new_id}")),
            Err(e) => {
                // Truthful diagnostics even on a partial failure: any earlier
                // locator in this group has already been reminted on disk —
                // dropping it from the message would hide a real change.
                let already = if reminted.is_empty() {
                    "none yet".to_string()
                } else {
                    reminted.join(", ")
                };
                return (
                    DoctorOutcome::ManualStep,
                    format!(
                        "kept '{id}' at {keeper}; already adopted: {already}; failed to remint \
                         {locator}: {e} — repair the remaining duplicate(s) by hand"
                    ),
                );
            }
        }
    }
    (
        DoctorOutcome::Repaired,
        format!(
            "kept '{id}' at {keeper}; adopted duplicate(s): {}",
            reminted.join(", ")
        ),
    )
}

/// Rewrites the `instanceId` field in place at `locator` via the generic
/// `load_instance_json`/`save_instance_json` path shim (srs-rust#725/#726),
/// not the ADR-042 typed logical-id methods — deliberately: a duplicate id
/// cannot be addressed by logical id (that ambiguity is the fault being
/// repaired), so the path-keyed shim is the only correct entry point here.
fn adopt_one(store: &dyn RepositoryStore, locator: &str) -> Result<String, RepositoryError> {
    let mut value = store.load_instance_json(locator)?;
    let new_id = new_instance_id();
    match value.as_object_mut() {
        Some(obj) => {
            obj.insert(
                "instanceId".to_string(),
                serde_json::Value::String(new_id.clone()),
            );
        }
        None => {
            return Err(RepositoryError::InvalidInput {
                message: format!("{locator} is not a JSON object; cannot remint its instanceId"),
            })
        }
    }
    store.save_instance_json(locator, &value)?;
    Ok(new_id)
}

/// Best-effort: does any relation or container reference `id`? Skips any
/// individual relation/container it cannot read or parse — those are
/// diagnosed by their own catalog entries; a scan that aborted on the first
/// unrelated fault would make adopt strictly less safe, not more.
/// Parse a relation object the way `catalog.rs::classify_relations_file`
/// does: strip `$schema` if present, never require it. Deliberately more
/// lenient than [`relation_object_from_value`], which requires an exact
/// `$schema` match — that stricter contract is right for the checked
/// `store.load_relation` read path, but `instance_id_referenced` must
/// classify a relation the same way the catalog does, or a relation the
/// catalog accepts as valid (and resolvable) could be silently skipped here,
/// wrongly telling adopt an id has no incoming reference.
fn lenient_relation_from_value(
    mut value: serde_json::Value,
) -> Option<srs_core::types::relation::Relation> {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("$schema");
    }
    serde_json::from_value(value).ok()
}

fn instance_id_referenced(store: &dyn RepositoryStore, cat: &RepositoryCatalog, id: &str) -> bool {
    for entry in &cat.relations {
        let Some(locator) = &entry.locator else {
            continue;
        };
        let Ok(value) = store.load_relations_json(locator) else {
            continue;
        };
        // Lenient parse, matching `catalog.rs::classify_relations_file`'s own
        // acceptance criteria (`$schema` stripped if present, never required)
        // rather than `relation_object_from_value`'s stricter "$schema is
        // required and must match exactly" contract. Every relation the
        // catalog itself accepts as valid — `$schema`-bearing or not — must
        // be checked here; the strict parser silently skipping a
        // schema-less-but-catalog-valid relation would make adopt wrongly
        // proceed on an id that genuinely has an incoming reference.
        let Some(relation) = lenient_relation_from_value(value) else {
            continue;
        };
        if relation.source_instance_id == id || relation.target_instance_id == id {
            return true;
        }
    }
    for entry in &cat.containers {
        let Ok((container, _)) = container_service::load_container_for_repair(store, &entry.id)
        else {
            continue;
        };
        if container
            .member_instance_ids
            .as_ref()
            .is_some_and(|v| v.iter().any(|m| m == id))
            || container
                .root_instance_ids
                .as_ref()
                .is_some_and(|v| v.iter().any(|m| m == id))
            || container.identity_instance_id.as_deref() == Some(id)
        {
            return true;
        }
    }
    false
}

/// Dangling container membership: removal via the existing ADR-045 repair
/// seam (`container_service::remove_member`/`remove_root`) — never a
/// parallel implementation.
fn repair_dangling_container(
    store: &dyn RepositoryStore,
    cat: &RepositoryCatalog,
    locator: &str,
    prop: &str,
    dangling_id: &str,
    fix: bool,
) -> (DoctorOutcome, String) {
    let Some(container_id) = cat
        .containers
        .iter()
        .find(|e| e.locator.as_deref() == Some(locator))
        .map(|e| e.id.clone())
    else {
        return (
            DoctorOutcome::ManualStep,
            format!("could not resolve a container id for locator {locator}"),
        );
    };

    if !fix {
        return (
            DoctorOutcome::WouldRepair,
            format!("would remove '{dangling_id}' from {container_id}'s {prop}"),
        );
    }

    let result = if prop == "rootInstanceIds" {
        container_service::remove_root(store, &container_id, dangling_id)
    } else {
        container_service::remove_member(store, &container_id, dangling_id)
    };
    match result {
        Ok(remaining) => (
            DoctorOutcome::Repaired,
            format!(
                "removed '{dangling_id}' from {container_id}'s {prop} ({} entries remain)",
                remaining.len()
            ),
        ),
        Err(e) => (
            DoctorOutcome::ManualStep,
            format!("failed to remove '{dangling_id}' from {container_id}'s {prop}: {e}"),
        ),
    }
}

/// Dangling relation endpoint: the relation itself is removed. A binary
/// edge whose source or target does not resolve cannot be repaired without
/// guessing an intended replacement endpoint, which doctor never does.
fn repair_dangling_relation(
    store: &dyn RepositoryStore,
    locator: &str,
    relation_id: &str,
    fix: bool,
) -> (DoctorOutcome, String) {
    if !fix {
        return (
            DoctorOutcome::WouldRepair,
            format!(
                "would delete relation '{relation_id}' ({locator}) — its endpoint does not \
                 resolve and cannot be repaired without guessing the intended target"
            ),
        );
    }
    match store.delete_relations_json(locator) {
        Ok(()) => (
            DoctorOutcome::Repaired,
            format!("deleted relation '{relation_id}' ({locator}) — its endpoint did not resolve"),
        ),
        Err(e) => (
            DoctorOutcome::ManualStep,
            format!("failed to delete relation '{relation_id}' ({locator}): {e}"),
        ),
    }
}

/// Relation filename↔id mismatch ([R11]): rename to the in-file
/// `relationId`, which is authoritative. One mechanism: `store.save_relation`
/// (which derives the correct path from `relation.relationId`) followed by
/// deleting the old file — not a bespoke rename.
fn repair_relation_filename_mismatch(
    store: &dyn RepositoryStore,
    locator: &str,
    fix: bool,
) -> (DoctorOutcome, String) {
    let value = match store.load_relations_json(locator) {
        Ok(v) => v,
        Err(e) => {
            return (
                DoctorOutcome::ManualStep,
                format!("could not read {locator}: {e}"),
            )
        }
    };
    let relation = match relation_object_from_value(value, locator) {
        Ok(r) => r,
        Err(e) => {
            return (
                DoctorOutcome::ManualStep,
                format!("could not parse {locator} as a relation: {e}"),
            )
        }
    };
    let correct = relation_object_path(&relation.relation_id);

    // The rename target already has its own content: `store.save_relation`
    // overwrites unconditionally by design (its own doc comment: "Overwrites
    // an existing object with the same id"), so renaming here would silently
    // destroy whatever is at `correct`. This is really a relation-id
    // collision — effectively the duplicate-id case for relations, which
    // adopt is deliberately not defined for (`duplicate_set_and_id` only
    // auto-repairs the `"instance"` set; never guess which copy is
    // canonical). Checked before the dry-run branch too, so a preview never
    // claims "would rename" when renaming would actually destroy data.
    if correct != locator && store.load_relations_json(&correct).is_ok() {
        return (
            DoctorOutcome::Ambiguous,
            format!(
                "cannot rename {locator} -> {correct}: {correct} already has its own content — \
                 this is a relation-id collision, not a simple rename; resolve which copy is \
                 canonical by hand"
            ),
        );
    }

    if !fix {
        return (
            DoctorOutcome::WouldRepair,
            format!("would rename {locator} -> {correct}"),
        );
    }
    if let Err(e) = store.save_relation(&relation) {
        return (
            DoctorOutcome::ManualStep,
            format!("failed to write {correct}: {e}"),
        );
    }
    if correct != locator {
        if let Err(e) = store.delete_relations_json(locator) {
            return (
                DoctorOutcome::ManualStep,
                format!(
                    "wrote {correct} but failed to remove the old file {locator}: {e} — delete \
                     it by hand"
                ),
            );
        }
    }
    (
        DoctorOutcome::Repaired,
        format!("renamed {locator} -> {correct} (the in-file relationId is authoritative)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_instance_duplicate_message() {
        let msg = "duplicate instance identifier 'abc-123' declared by 2 objects";
        assert_eq!(duplicate_set_and_id(msg), Some(("instance", "abc-123")));
    }

    #[test]
    fn parses_relation_duplicate_message() {
        let msg = "duplicate relation identifier 'rel-1' declared by 2 objects";
        assert_eq!(duplicate_set_and_id(msg), Some(("relation", "rel-1")));
    }

    #[test]
    fn parses_container_dangling_message() {
        let msg = "container memberInstanceIds 'ghost-id' resolves to nothing in the instance set";
        assert_eq!(
            container_dangling_prop_and_id(msg),
            Some(("memberInstanceIds", "ghost-id"))
        );
    }

    #[test]
    fn parses_relation_dangling_message() {
        let msg =
            "relation 'rel-1' targetInstanceId 'ghost-id' resolves to nothing in the instance set";
        assert_eq!(relation_dangling_id(msg), Some("rel-1"));
        // container parser must not misfire on a relation message.
        assert_eq!(container_dangling_prop_and_id(msg), None);
    }
}
