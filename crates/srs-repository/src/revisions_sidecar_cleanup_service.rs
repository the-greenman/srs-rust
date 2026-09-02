//! `revisions-sidecar-cleanup` migration (`srs-rust#866`) — delete orphaned
//! `.revisions.json` sidecar files left behind by the retired Revision
//! mechanism.
//!
//! ## Why this exists
//!
//! `rfc-decision-2a1e1590` retired the per-field Revision sidecar mechanism
//! from the spec (zero corpus consumers at the time, a `RevisionAgent`
//! PascalCase wire-format leak, no implementation exercising the return-
//! trigger chain — see the decision record for the full rationale). The spec
//! schema for `revisions.json` no longer exists. srs-rust's own write path
//! (one production call site, in `transition_record_lifecycle`) is removed in
//! this same change (srs-rust#866) — but it had already run: srs PR #529's
//! generation-ledger walk found **75 live `.revisions.json` files** in the
//! `srs` spec corpus alone, most created by the #447 migration's 70 lifecycle
//! transitions on 2026-08-28, five days after the decision that retired the
//! mechanism.
//!
//! Once `catalog.rs` stops recognising `.revisions.json` as a sidecar (this
//! same change), any such file left on disk is no longer tolerated: it falls
//! through to ordinary instance-candidate classification, fails every shape
//! (`SRS038-R8-SHAPE-NO-MATCH`), and — like any other `error`-severity
//! diagnostic under a reserved instance root — makes the *entire* checked
//! catalog fail to load ([R24]). Exactly the `graduated-at-cleanup`
//! (srs-rust#896) situation: a repair that must not itself depend on
//! `store.catalog()`, because the defect it repairs is what breaks it.
//!
//! ## The repair: delete, unconditionally
//!
//! A `.revisions.json` sidecar carries no spec-recognised meaning any more —
//! there is no schema left to validate it against, and nothing in the
//! current data model reads it. Deleting it loses nothing the spec
//! recognizes: the sidecar was never itself a Record, Note, or Relation: it
//! was field-value history for a mechanism that no longer exists. Unlike
//! `graduated-at-cleanup`'s legacy `graduatedAt` field, there is no
//! "underivable target" case here to refuse honestly — a sidecar is either
//! present (delete it) or absent (nothing to do). So this migration is
//! all-or-nothing like the majority of this registry (`rfc039-carrier`,
//! `tier1-removal`, `rfc038-storage`), not a `graduated-at-cleanup`-style
//! partial apply: there is no partial case to depart the default for.
//!
//! ## Revision-stamp semantics: none — structural, like `rfc038-storage` and
//! `graduated-at-cleanup`
//!
//! This migration stamps no `dataModelRevision`. The same two reasons apply:
//! - The defect is **not gated by revision** — a `.revisions.json` sidecar
//!   could be written by any binary between the #297 cutover (when the
//!   schema first landed) and this change, regardless of the repository's
//!   own stamped `dataModelRevision`. A revision requirement would assert a
//!   relationship between "carries a stray sidecar" and "declares revision
//!   N" that the data does not support.
//! - A revision bump asserts "no unmigrated content of this shape remains,
//!   ever again" — appropriate for a structural rewrite, not for deleting a
//!   file type that no longer has a schema to be unmigrated *against*.
//!
//! Detection and repair read/write the raw file tree directly (never
//! `store.catalog()`), for the same ADR-045 "repair seam" reason
//! `graduated_at_migration_service` and `container_service::remove_member`/
//! `remove_root` already establish: an operation that can only *reduce*
//! incoherence must not itself be blocked by the incoherence.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;

/// The retired sidecar suffix (formerly `catalog.rs`'s `SIDECAR_SUFFIX_REVISIONS`).
const REVISIONS_SIDECAR_SUFFIX: &str = ".revisions.json";

/// One `.revisions.json` sidecar deleted by this migration.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedRevisionsSidecar {
    pub path: String,
    /// Best-effort `recordId` read from the sidecar body, for operator
    /// visibility only — never used to decide whether to delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionsSidecarCleanupResult {
    pub deleted: Vec<DeletedRevisionsSidecar>,
}

/// Raw-tree scan for `.revisions.json` files, deterministic order ([R14] —
/// never expose filesystem iteration order). Foreign-tooling directories
/// (`.git`, `node_modules`, `.srs`) are skipped, mirroring the catalog
/// walker's own discipline.
fn find_revisions_sidecars(store: &dyn RepositoryStore) -> Vec<String> {
    let mut out: Vec<String> = store
        .list_files_recursive("")
        .into_iter()
        .filter(|path| {
            path.ends_with(REVISIONS_SIDECAR_SUFFIX) && !crate::catalog::is_foreign_tooling(path)
        })
        .collect();
    out.sort();
    out
}

/// Whether this repository still carries any `.revisions.json` sidecar.
pub fn migration_needed(store: &dyn RepositoryStore) -> bool {
    !find_revisions_sidecars(store).is_empty()
}

/// Apply the `revisions-sidecar-cleanup` migration: delete every
/// `.revisions.json` sidecar found in the tree. All-or-nothing — see the
/// module doc for why there is no partial case to honour here.
pub fn migrate_revisions_sidecars(
    store: &dyn RepositoryStore,
) -> Result<RevisionsSidecarCleanupResult, RepositoryError> {
    let mut result = RevisionsSidecarCleanupResult::default();

    for path in find_revisions_sidecars(store) {
        let record_id = store.load_instance_json(&path).ok().and_then(|v| {
            v.get("recordId")
                .and_then(|r| r.as_str())
                .map(str::to_string)
        });
        store.delete_instance_file(&path)?;
        result
            .deleted
            .push(DeletedRevisionsSidecar { path, record_id });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;

    fn write_sidecar(store: &MemoryStore, path: &str, record_id: &str) {
        store
            .save_instance_json(
                path,
                &serde_json::json!({
                    "recordId": record_id,
                    "revisions": []
                }),
            )
            .unwrap();
    }

    #[test]
    fn migration_not_needed_on_a_clean_store() {
        let store = MemoryStore::default();
        assert!(!migration_needed(&store));
        let result = migrate_revisions_sidecars(&store).unwrap();
        assert!(result.deleted.is_empty());
    }

    /// Red-then-green: a planted sidecar is detected and deleted, and the
    /// migration is idempotent — nothing left to do on a second run.
    #[test]
    fn deletes_a_planted_sidecar() {
        let store = MemoryStore::default();
        write_sidecar(&store, "records/a.revisions.json", "rec-1");

        assert!(migration_needed(&store));
        let result = migrate_revisions_sidecars(&store).unwrap();
        assert_eq!(result.deleted.len(), 1);
        assert_eq!(result.deleted[0].path, "records/a.revisions.json");
        assert_eq!(result.deleted[0].record_id.as_deref(), Some("rec-1"));

        assert!(
            store
                .load_instance_json("records/a.revisions.json")
                .is_err(),
            "the sidecar must actually be gone from the store"
        );
        assert!(
            !migration_needed(&store),
            "must be idempotent — nothing left to do"
        );
    }

    #[test]
    fn deletes_multiple_sidecars_across_directories() {
        let store = MemoryStore::default();
        write_sidecar(&store, "records/tier-2/a.revisions.json", "rec-1");
        write_sidecar(&store, "records/rfcs/b.revisions.json", "rec-2");
        write_sidecar(&store, "notes/c.revisions.json", "rec-3");

        let result = migrate_revisions_sidecars(&store).unwrap();
        assert_eq!(result.deleted.len(), 3);
        assert!(!migration_needed(&store));
    }

    /// A record file sitting alongside a sidecar must survive untouched —
    /// this migration only ever deletes `.revisions.json` paths.
    #[test]
    fn leaves_the_record_itself_untouched() {
        let store = MemoryStore::default();
        store
            .save_instance_json(
                "records/a.json",
                &serde_json::json!({"instanceId": "rec-1"}),
            )
            .unwrap();
        write_sidecar(&store, "records/a.revisions.json", "rec-1");

        migrate_revisions_sidecars(&store).unwrap();

        assert!(store.load_instance_json("records/a.json").is_ok());
        assert!(store
            .load_instance_json("records/a.revisions.json")
            .is_err());
    }
}
