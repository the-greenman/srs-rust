//! `graduated-at-cleanup` migration (`srs-rust#896`) — strip the retired
//! legacy `graduatedAt` field from Notes, preserving graduation provenance.
//!
//! ## The bug this repairs
//!
//! `note.json`'s schema (srs PR #505, dataModelRevision 4) dropped
//! `graduatedAt` from its properties and set `additionalProperties: false`.
//! Schema validation (`SRS038-R7-SCHEMA-VALIDATION`) runs unconditionally at
//! catalog load, for every instance, **regardless of the repository's
//! stamped `dataModelRevision`** — there is only one, current, schema per
//! type. So a Note that still carries the legacy field (written by any
//! `note graduate` call before srs-rust#884) now fails schema validation, and
//! [R24] makes any `error` diagnostic fatal to the *entire* catalog build
//! (`RepositoryError::CatalogLoad`) — not just to that one Note.
//!
//! That fatality is the reason this migration cannot be written like an
//! ordinary service: `store.catalog()` (checked), and everything built on it
//! (`find_instance`, `load_note_by_id`, `save_note`, `relation_service`'s
//! `create_relation` validation data, ...) is unusable on exactly the
//! repositories this migration exists to repair. Detection and repair below
//! read the raw file tree directly and write raw JSON back — the same
//! ADR-045 "repair seam" reasoning `container_service::remove_member`/
//! `remove_root` already established (an operation that can only *reduce*
//! incoherence must not itself be blocked by the incoherence), applied here
//! because no typed `catalog_unchecked`-backed pair exists yet for notes
//! (ADR-042's remaining-entity-families migration, epic srs-rust#704, is out
//! of this issue's scope).
//!
//! ## Detection is schema/catalog-independent by construction
//!
//! A Note that still carries `graduatedAt` never becomes a `CatalogEntry`
//! at all — [R7]'s classifier only adds an entry on a **passing** validation,
//! and (when no `$schema` is declared) shape classification can fail
//! differently again (`SRS038-R8-SHAPE-NO-MATCH`) depending on whether the
//! object happens to also fail every other candidate shape. Rather than
//! chase every diagnostic code a broken Note can produce, detection here
//! looks at raw JSON directly: any file anywhere in the tree whose body has
//! both `sections` (the Note-shape discriminator) and `graduatedAt` is a
//! legacy-graduated Note candidate, independent of what — if anything — the
//! catalog classified it as.
//!
//! ## The repair: drop, assert-then-drop, or honestly refuse
//!
//! For each candidate, by srs-rust#896's design:
//! 1. A `derived-from` Relation already targets it (asserted by a prior
//!    `note graduate` call post-srs-rust#884, or by a manual repair) — the
//!    graduation is fully recorded there; the field is redundant and is
//!    dropped. Nothing is lost.
//! 2. No such relation exists. The graduated-to Record id must be
//!    **derivable** to preserve provenance before the field can go — but the
//!    pre-#884 write path recorded a graduation by stamping the timestamp
//!    and nothing else (srs-rust#884's own description: "left the relation
//!    graph silent... the only trace of graduation was `graduatedAt`... which
//!    records *that* it graduated but not *into what*"). No other field on
//!    this Note, on any Record, or on any `SourceReference` (whose
//!    `SourceType` enum has no note-referencing variant) names a target.
//!    **The target is therefore never derivable from repository data for
//!    this case** — inventing one (nearest-by-timestamp, matching
//!    type/title, ...) would be a guess dressed as provenance. Per the
//!    design brief: never invent it. The field is left in place, a named
//!    diagnostic is recorded, and the migration reports this Note as
//!    unresolved (see [`GraduatedAtMigrationResult`]).
//!
//! ## Revision-stamp semantics: none — structural, like `rfc038-storage`
//!
//! This is **not** a revision-keyed migration and stamps no
//! `dataModelRevision`. Two reasons, both already established by the one
//! other non-bumping repair in this registry (`rfc038-storage`):
//! - The defect it repairs is **not gated by revision** in the first place —
//!   schema validation rejects a legacy `graduatedAt` Note "regardless of the
//!   repository's stamped `dataModelRevision`" (this module's own opening
//!   paragraph, and srs-rust#896 verbatim). A revision requirement here would
//!   assert a relationship between "carries this legacy field" and "declares
//!   revision N" that does not exist in the data: a rev-3 *or* rev-4 manifest
//!   can carry the field, because the write path that produced it predates
//!   both the schema tightening and any revision-bump migration.
//! - Bumping a generation number implies "no unmigrated content of this
//!   shape remains, ever again" (compare `tier1-removal`'s own count-and-abort
//!   discipline). A partially-repaired repository — some Notes fixed, one
//!   left `unresolvable` because its target truly cannot be derived — is
//!   exactly the state a revision stamp must never describe as clean.
//!
//! ## Deliberate departure from "abort rather than skip"
//!
//! Every other content-dependent migration in this registry (`rfc039-carrier`,
//! `tier1-removal`, `rfc038-storage`) is all-or-nothing: on an unmet
//! precondition it writes nothing and returns `Err`. This migration instead
//! repairs every resolvable Note and *then* returns `Err` naming the
//! unresolved ones — a deliberate difference, not an oversight. Each Note here
//! is independent (unlike `rfc038-storage`'s single relations-collection
//! transform, or `tier1-removal`'s single revision stamp): withholding a
//! lossless, unconditionally-safe repair for every *resolvable* Note just
//! because one *unrelated* Note's provenance cannot be derived would trade a
//! real, always-available fix for an artificial one. Per the design brief:
//! "honest partial failure beats silent data loss" — the already-applied
//! repairs are honest progress; the `Err` for the remainder is the refusal to
//! guess. Re-running is idempotent: already-fixed Notes are not visited again
//! (they no longer match the `graduatedAt`-presence probe), and the same
//! unresolved Note is reported again until a human asserts its `derived-from`
//! relation by hand and reruns the migration.

use crate::error::RepositoryError;
use crate::services::note_has_graduation_relation;
use crate::store::RepositoryStore;
use serde::Serialize;

/// The field retired from `note.json` at data-model revision 4 (srs PR #505).
const LEGACY_GRADUATED_AT_KEY: &str = "graduatedAt";

/// A legacy Note whose `graduatedAt` field was dropped because a
/// `derived-from` relation already records the same graduation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigratedGraduatedNote {
    pub instance_id: String,
    pub path: String,
}

/// A legacy Note whose `graduatedAt` field could **not** be dropped: no
/// `derived-from` relation exists, and no other repository data can derive
/// what it graduated into. Left untouched on disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvableGraduatedNote {
    pub instance_id: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraduatedAtMigrationResult {
    /// Notes repaired: the legacy field was dropped, its graduation already
    /// recorded by an existing `derived-from` relation.
    pub migrated: Vec<MigratedGraduatedNote>,
    /// Notes left untouched: no `derived-from` relation exists and the
    /// graduation target cannot be derived from repository data.
    pub unresolvable: Vec<UnresolvableGraduatedNote>,
}

/// Raw-tree scan for legacy-graduated Note candidates (see module doc for
/// why this cannot go through `store.catalog()`). A candidate is any JSON
/// file, anywhere in the tree, whose body is an object carrying both
/// `sections` (the Note-shape discriminator — schema-validated in every
/// admissible case, so a genuine non-Note object essentially never collides)
/// and the retired `graduatedAt` key. Unparseable or vanished files are
/// silently skipped — not this migration's concern; `repo validate`/`repo
/// doctor` name those separately.
fn find_legacy_graduated_notes(store: &dyn RepositoryStore) -> Vec<(String, serde_json::Value)> {
    let mut out: Vec<(String, serde_json::Value)> = store
        .list_files_recursive("")
        .into_iter()
        .filter(|path| path.ends_with(".json") && !crate::catalog::is_foreign_tooling(path))
        .filter_map(|path| {
            let value = store.load_instance_json(&path).ok()?;
            let obj = value.as_object()?;
            if obj.contains_key("sections") && obj.contains_key(LEGACY_GRADUATED_AT_KEY) {
                Some((path, value))
            } else {
                None
            }
        })
        .collect();
    // Deterministic order — never expose filesystem iteration order (mirrors
    // the catalog walker's own [R14] discipline).
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Whether this repository still carries any legacy-graduated Note. `true`
/// regardless of whether every such Note is resolvable — "Needed" means
/// there is work here, not that all of it is automatable (same honesty
/// `tier1-removal` practices: it never silently reports a false-clean
/// state).
pub fn migration_needed(store: &dyn RepositoryStore) -> bool {
    !find_legacy_graduated_notes(store).is_empty()
}

/// Apply the `graduated-at-cleanup` migration: for every legacy-graduated
/// Note, drop the field when its graduation is already recorded by a
/// `derived-from` relation, else leave it and report why. See the module
/// doc for the full disposition and the deliberate non-revision, non-abort
/// design.
pub fn migrate_graduated_at(
    store: &dyn RepositoryStore,
) -> Result<GraduatedAtMigrationResult, RepositoryError> {
    let mut result = GraduatedAtMigrationResult::default();

    for (path, mut value) in find_legacy_graduated_notes(store) {
        let instance_id = value
            .get("instanceId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        if note_has_graduation_relation(store, &instance_id)? {
            value
                .as_object_mut()
                .expect("checked object shape in find_legacy_graduated_notes")
                .remove(LEGACY_GRADUATED_AT_KEY);
            store.save_instance_json(&path, &value)?;
            result
                .migrated
                .push(MigratedGraduatedNote { instance_id, path });
        } else {
            result.unresolvable.push(UnresolvableGraduatedNote {
                message: format!(
                    "Note '{instance_id}' ({path}) carries the legacy `graduatedAt` field but no \
                     `derived-from` relation records what it graduated into, and no other \
                     repository data names a target — the pre-srs-rust#884 write path never \
                     recorded one. Left untouched (never invented). Assert the missing relation \
                     by hand (`relation create`: relationType `derived-from`, source = the \
                     successor Record's id, target = '{instance_id}'), then re-run \
                     `srs repo apply-migration --id graduated-at-cleanup`."
                ),
                instance_id,
                path,
            });
        }
    }

    if !result.unresolvable.is_empty() {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "graduated-at-cleanup migrated {} legacy note(s) but cannot fully apply: \
                 {} note(s) have no derivable graduation target. First: {}",
                result.migrated.len(),
                result.unresolvable.len(),
                result.unresolvable[0].message
            ),
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relation_service::create_relation_auto;
    use crate::services::GRADUATION_RELATION_TYPE;
    use crate::store::memory::MemoryStore;
    use srs_core::types::relation::Relation;

    const NOTE_ID: &str = "00000000-0000-4000-8000-0000000000a1";
    const RECORD_ID: &str = "00000000-0000-4000-8000-0000000000b1";
    const NOTE_PATH: &str = "records/notes/legacy-a1.json";

    /// A valid Note (no `graduatedAt`) at `path`/`id` — schema-clean, so the
    /// checked catalog stays usable while a fixture still needs it (e.g. to
    /// assert the `derived-from` relation via the normal, catalog-backed
    /// write path, exactly as `note graduate` does post-srs-rust#884).
    fn write_clean_note(store: &MemoryStore, path: &str, id: &str, title: &str) {
        store
            .save_instance_json(
                path,
                &serde_json::json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
                    "instanceId": id,
                    "title": title,
                    "sections": [{"name": "body", "content": "hello"}]
                }),
            )
            .unwrap();
    }

    /// Inject the retired `graduatedAt` field directly onto an existing raw
    /// instance file — bypassing schema validation entirely, exactly the way
    /// a genuine pre-#505 repository ended up carrying it (written by a
    /// binary that predates the schema tightening, not by anything this
    /// crate's current write paths can still produce). This is deliberately
    /// the **only** way this fixture ever contains the field: doing it after
    /// any catalog-dependent setup (like asserting a relation, which needs
    /// both endpoints to already resolve) mirrors reality, where the field
    /// was already on disk before dataModelRevision 4 made it invalid.
    fn inject_legacy_graduated_at(store: &MemoryStore, path: &str, stamp: &str) {
        let mut raw = store.load_instance_json(path).unwrap();
        raw.as_object_mut().unwrap().insert(
            LEGACY_GRADUATED_AT_KEY.to_string(),
            serde_json::json!(stamp),
        );
        store.save_instance_json(path, &raw).unwrap();
    }

    fn assert_derived_from(store: &MemoryStore, source: &str, target: &str, created_at: &str) {
        create_relation_auto(
            store,
            Relation {
                relation_id: String::new(),
                relation_type: GRADUATION_RELATION_TYPE.to_string(),
                source_instance_id: source.to_string(),
                target_instance_id: target.to_string(),
                asserted_by: None,
                confidence: None,
                created_at: Some(created_at.to_string()),
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn migration_not_needed_on_a_clean_store() {
        let store = MemoryStore::default();
        assert!(!migration_needed(&store));
        let result = migrate_graduated_at(&store).unwrap();
        assert!(result.migrated.is_empty());
        assert!(result.unresolvable.is_empty());
    }

    /// Red-then-green path 1 (relation-exists): a `derived-from` relation
    /// already records the graduation — the field is dropped, nothing else
    /// changes, and the migration succeeds outright.
    ///
    /// Reverting the `note_has_graduation_relation` check to `false`
    /// unconditionally reproduces the pre-fix behaviour this test guards
    /// against: the field would be dropped here as it is skipped there.
    #[test]
    fn strips_field_when_derived_from_relation_already_exists() {
        let store = MemoryStore::default();
        // Both endpoints must already resolve for E2 before the relation can
        // be asserted — exactly what `note graduate` guarantees by creating
        // the Record and the relation atomically (srs-rust#884). The "source"
        // stands in for that Record; only its id matters to this migration.
        write_clean_note(&store, NOTE_PATH, NOTE_ID, "Legacy graduated note");
        write_clean_note(
            &store,
            "records/notes/source-b1.json",
            RECORD_ID,
            "Successor",
        );
        assert_derived_from(&store, RECORD_ID, NOTE_ID, "2026-01-01T00:00:00Z");

        // Only now does the repository become "legacy" — the field already
        // on disk before rev-4 tightened the schema.
        inject_legacy_graduated_at(&store, NOTE_PATH, "2026-01-01T00:00:00Z");

        assert!(migration_needed(&store));
        let result = migrate_graduated_at(&store).unwrap();
        assert_eq!(result.migrated.len(), 1);
        assert_eq!(result.migrated[0].instance_id, NOTE_ID);
        assert!(result.unresolvable.is_empty());

        let raw = store.load_instance_json(NOTE_PATH).unwrap();
        assert!(
            raw.get("graduatedAt").is_none(),
            "the legacy field must be gone: {raw}"
        );
        assert_eq!(raw["title"], "Legacy graduated note");
        assert!(
            !migration_needed(&store),
            "must be idempotent — nothing left to do"
        );
    }

    /// Red-then-green path 2 (underivable): no `derived-from` relation
    /// exists anywhere, and nothing else in the repository names a
    /// graduation target — the field must be left in place, and the
    /// migration must fail loudly for this Note rather than guess or drop
    /// silently.
    ///
    /// Reverting the `Err`/leave-in-place branch to silently drop the field
    /// instead reproduces the exact data loss this test guards against: the
    /// final assertion (`graduatedAt` still present) would fail.
    #[test]
    fn leaves_field_and_reports_diagnostic_when_target_is_not_derivable() {
        let store = MemoryStore::default();
        write_clean_note(&store, NOTE_PATH, NOTE_ID, "Legacy graduated note");
        inject_legacy_graduated_at(&store, NOTE_PATH, "2026-01-01T00:00:00Z");

        assert!(migration_needed(&store));
        let err = migrate_graduated_at(&store).unwrap_err();
        assert!(
            err.to_string().contains(NOTE_ID) && err.to_string().contains("cannot fully apply"),
            "error must name the unresolved note, got: {err}"
        );

        // The field must survive untouched — no silent data loss.
        let raw = store.load_instance_json(NOTE_PATH).unwrap();
        assert_eq!(raw["graduatedAt"], "2026-01-01T00:00:00Z");
        assert!(
            migration_needed(&store),
            "an unresolved legacy note must keep reporting Needed, never a false-clean state"
        );
    }

    /// A repeat run over a mix of resolvable and unresolvable notes must
    /// migrate the resolvable one exactly once and keep reporting the
    /// unresolvable one — idempotent, honest partial failure.
    #[test]
    fn partial_apply_is_idempotent_across_reruns() {
        const OTHER_NOTE_ID: &str = "00000000-0000-4000-8000-0000000000c2";
        const OTHER_NOTE_PATH: &str = "records/notes/legacy-b2.json";

        let store = MemoryStore::default();
        write_clean_note(&store, NOTE_PATH, NOTE_ID, "Legacy graduated note");
        write_clean_note(&store, OTHER_NOTE_PATH, OTHER_NOTE_ID, "Second legacy note");
        write_clean_note(
            &store,
            "records/notes/source-b1.json",
            RECORD_ID,
            "Successor",
        );
        assert_derived_from(&store, RECORD_ID, OTHER_NOTE_ID, "2026-02-02T00:00:00Z");

        inject_legacy_graduated_at(&store, NOTE_PATH, "2026-01-01T00:00:00Z");
        inject_legacy_graduated_at(&store, OTHER_NOTE_PATH, "2026-02-02T00:00:00Z");

        let err = migrate_graduated_at(&store).unwrap_err();
        assert!(err.to_string().contains(NOTE_ID));

        // The resolvable note (OTHER_NOTE_ID) must be fixed even though the
        // run as a whole reported Err for NOTE_ID.
        let raw_other = store.load_instance_json(OTHER_NOTE_PATH).unwrap();
        assert!(raw_other.get("graduatedAt").is_none());
        let raw_a1 = store.load_instance_json(NOTE_PATH).unwrap();
        assert_eq!(raw_a1["graduatedAt"], "2026-01-01T00:00:00Z");

        // Second run: idempotent — same outcome, no double work, no panic
        // from re-visiting the already-fixed note.
        let err2 = migrate_graduated_at(&store).unwrap_err();
        assert!(err2.to_string().contains(NOTE_ID));
        assert!(!err2.to_string().contains(OTHER_NOTE_ID));
    }
}
