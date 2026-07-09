# ADR-024: Best-effort Rollback for Two-Write Service Operations

- **Status:** proposed
- **Date:** 2026-07-09
- **Supersedes:** —
- **Superseded by:** —

## Context

`create_record_in_container` and `create_record_in_context` each perform two sequential writes:

1. `create_record_at_dir` — writes the record file and appends to `manifest.instanceIndex`.
2. `container_service::add_member` — rewrites the container JSON to include the new record.

If step 2 fails after step 1 succeeds, the manifest retains an entry for a record that is not a member of any container. ADR-010 requires multi-step service operations to be atomic in the service layer. ADR-007 establishes the file-before-index create ordering used by step 1; no cross-file atomic transaction mechanism is available in the target environment (plain filesystem, no WAL, no SQLite).

Three options were evaluated:

- **Option A — best-effort rollback:** on step 2 failure, call `delete_record` in the error arm to undo step 1. Not crash-safe: if the cleanup call itself fails, the error is swallowed and the manifest may retain an orphaned entry. Transparent to callers — they see only the original `add_member` error.
- **Option B — write-ahead log / journal:** new infrastructure (a WAL or repair journal) that lets the repository detect and repair half-applied operations on load. Crash-safe but requires significant new machinery across `FileStore` and the load path.
- **Option C — explicit ADR waiver:** document the gap and close the issue as `won't fix`, acknowledging that ADR-010 is not satisfied for this class of operation.

## Decision

Implement **Option A (best-effort rollback)** for both `create_record_in_container` and `create_record_in_context`. On `add_member` failure, call `delete_record` in the error arm and swallow any secondary error from the cleanup.

Option B is deferred: the added complexity of a WAL is not justified by the current use cases (local filesystem access, in-process WASM). A crash between the two writes bypasses the error handler entirely, but such crashes are rare and recoverable via `srs repo repair` (future work, ADR-007).

Option C is rejected: it would leave ADR-010 unaddressed with no mitigation for the common case (transient I/O errors).

## Consequences

**Positive:**
- The common failure mode (transient `save_container` error) is now handled: the record is removed from the manifest before the error surfaces to the caller.
- Satisfies the spirit of ADR-010 for the typical failure mode.
- No new infrastructure, no public API changes, and no payload contract changes.
- Option B can be layered on top at any time — this decision does not foreclose it.

**Negative / trade-offs:**
- Not crash-safe: a process kill between step 1 and step 2 bypasses the error handler. The repository is left with a manifest entry for a record not in any container.
- `delete_record` itself uses file-before-index ordering for deletes (deletes the file, then removes the manifest entry), which is the inverse of ADR-007's prescribed index-before-file ordering. If the rollback's file deletion succeeds but the subsequent `write_manifest` fails, the file is gone from disk while the manifest still holds an entry for it — a **dangling index entry** that causes every subsequent `list`/`get` for that entity to fail. This is a narrowly-scoped failure mode (partial I/O failure affecting only the manifest write, not the file deletion) but is strictly worse than the pre-rollback state (orphaned non-container-member record, which remains accessible). A correct long-term fix requires either (a) fixing `delete_record` to follow ADR-007 for its own internal ordering, or (b) implementing Option B (WAL).
- If `find_relations_referencing_instance` (called inside `delete_record` before any mutation) fails during the rollback, the cleanup is suppressed silently — no file or manifest is touched, and the record remains intact in the manifest. No additional corruption occurs in this path.
- All secondary cleanup errors are swallowed via `let _ = …`. Operators relying on error logs will not see them.

**Neutral:**
- `srs repo repair` (identified in ADR-007 as future work) should handle residual orphaned entries from process kills or failed rollbacks.
- Both `create_record_in_container` and `create_record_in_context` are updated consistently.
- The limitation (best-effort, not crash-safe) is documented in each function's doc comment.
