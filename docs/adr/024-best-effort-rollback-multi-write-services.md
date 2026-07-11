# ADR-024: Best-effort Rollback for Two-Write Service Operations

- **Status:** accepted
- **Date:** 2026-07-09
- **Supersedes:** —
- **Superseded by:** —

## Context

`create_record_in_container` and `create_record_in_context` each perform two sequential writes:

1. `create_record_at_dir` — writes the record file and appends to `manifest.instanceIndex`.
2. `container_service::add_member` — rewrites the container JSON to include the new record.

If step 2 fails after step 1 succeeds, the manifest retains an entry for a record that is not a member of any container. ADR-010 requires multi-step service operations to be atomic in the service layer. ADR-007 establishes the file-before-index create ordering used by step 1; no cross-file atomic transaction mechanism is available in the target environment (plain filesystem, no WAL, no SQLite).

Four options were evaluated:

- **Option A — best-effort rollback:** on step 2 failure, call `delete_record` in the error arm to undo step 1. Not crash-safe: if the cleanup call itself fails, the error is swallowed and the manifest may retain an orphaned entry. Transparent to callers — they see only the original `add_member` error.
- **Option B — write-ahead log / journal:** new infrastructure (a WAL or repair journal) that lets the repository detect and repair half-applied operations on load. Crash-safe but requires significant new machinery across `FileStore` and the load path.
- **Option C — explicit ADR waiver:** document the gap and close the issue as `won't fix`, acknowledging that ADR-010 is not satisfied for this class of operation.
- **Option D — `begin_batch`/`abort_batch` (ADR-021):** ADR-021 added `begin_batch`/`abort_batch`/`commit_batch` to `RepositoryStore` for bulk-write rollback. Calling `abort_batch` in the error arm would restore in-memory state for `JsonStore`, but `abort_batch` is a trait-default no-op for `FileStore` (and `MemoryStore`) — ADR-021 documents this explicitly. Because both functions operate on `store: &dyn RepositoryStore` and must work across all store implementations, `abort_batch` alone cannot replace an explicit compensating delete. Option D is therefore insufficient as a standalone mechanism.

## Decision

Implement **Option A (best-effort rollback)** for both `create_record_in_container` and `create_record_in_context`. On `add_member` failure, call `delete_record` in the error arm and swallow any secondary error from the cleanup.

Option B is deferred: the added complexity of a WAL is not justified by the current use cases (local filesystem access, in-process WASM). A crash between the two writes bypasses the error handler entirely, but such crashes are rare and recoverable via `srs repo repair` (future work, ADR-007).

Option C is rejected: it would leave ADR-010 unaddressed with no mitigation for the common case (transient I/O errors).

Option D is rejected: `abort_batch` is a no-op for `FileStore` and `MemoryStore`, so it cannot provide a store-agnostic rollback (see Context above).

## Consequences

**Positive:**
- The common failure mode (transient `save_container` error) is now handled: the record is removed from the manifest before the error surfaces to the caller.
- Satisfies the spirit of ADR-010 for the typical failure mode.
- No new infrastructure, no public API changes, and no payload contract changes.
- Option B can be layered on top at any time — this decision does not foreclose it.

**Negative / trade-offs:**
- Not crash-safe: a process kill between step 1 and step 2 bypasses the error handler. The repository is left with a manifest entry for a record not in any container.
- **Resolved (issue #475):** `delete_record` previously used file-before-index ordering for deletes, which was the inverse of ADR-007's prescribed index-before-file ordering. This has been fixed: `delete_record` now follows ADR-007 (removes the manifest entry, writes the manifest, then deletes the file as best-effort). The dangling-entry risk on the rollback path is eliminated. The only remaining crash-safety limitation is the not-crash-safe bullet above — a process kill between step 1 and step 2 bypasses the error handler entirely, but that path is unaffected by this ordering fix.
- If `find_relations_referencing_instance` (called inside `delete_record` before any mutation) fails during the rollback, the cleanup is suppressed silently — no file or manifest is touched, and the record remains intact in the manifest. No additional corruption occurs in this path.
- All secondary cleanup errors are swallowed via `let _ = …`. Operators relying on error logs will not see them.

**Neutral:**
- `srs repo repair` (identified in ADR-007 as future work) should handle residual orphaned entries from process kills or failed rollbacks.
- Both `create_record_in_container` and `create_record_in_context` are updated consistently.
- The limitation (best-effort, not crash-safe) is documented in each function's doc comment.
- The same pattern was subsequently extended to `create_note_in_context` (issue #455) using the inline form `let _ = delete_note(store, &id)` rather than a named helper (one call site, no clarity gain). The applicable caveats from the Negative section differ slightly from the record case. **The file-before-index ordering risk has been resolved for `delete_note` as well (issue #475):** `delete_note` now follows ADR-007 ordering (manifest-first). Only the `let _ = …` error-swallowing (third bullet) carries over to `delete_note`. The `find_relations_referencing_instance` pre-mutation call (second bullet) does **not** apply — `delete_note` has no such call and goes straight to `load_manifest` → manifest remove → `write_manifest` → best-effort `delete_instance_file`; a failure in `delete_note` during rollback therefore touches nothing, leaving the note intact in the manifest rather than producing a dangling entry.
