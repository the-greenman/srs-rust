# Plan: Best-effort rollback in create_note_in_context (#455)

## Summary

`create_note_in_context` in `crates/srs-repository/src/services.rs` performs two sequential writes: (1) `create_note` writes the note file and appends to `manifest.instanceIndex`; (2) `container_service::add_member` rewrites the container JSON. If step 2 fails, the manifest retains an orphaned note entry not belonging to any container. This is the same two-write gap fixed for records in #364. This plan applies the same best-effort rollback principle (ADR-024) to `create_note_in_context`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead |
| Repository Service Worker | Lead (single-file change, solo execution) |
| Verification | Lead |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan applies ADR-024 to `create_note_in_context`. The rollback is inlined as `let _ = delete_note(store, &id)` rather than behind a named helper (the `attempt_rollback_delete` helper in `record_store.rs` is an artifact of the record-store module's private scope; replicating a helper in `services.rs` for a single call site adds no clarity).

| ADR | Decision | Status |
|---|---|---|
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Best-effort rollback via compensating delete for two-write service operations | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. The `create_note_in_context` function is a service-layer change only — no handler changes, no payload struct changes. `cargo test --test payload_contracts` will pass unchanged.

### Entity schema sync (check-schema-sync.sh)

No schema files added or modified. No action required.

---

## Scope

- Modify `create_note_in_context` in `crates/srs-repository/src/services.rs` to wrap the `add_member` call in an error arm that calls `delete_note` as a best-effort rollback.
- Update the function's doc comment to document the rollback behaviour and reference ADR-024.
- Add `rollback_mechanism_delete_note_cleans_manifest` test in `services.rs` tests — verifies the building blocks: `create_note` followed by `delete_note` returns the manifest instance index to its original length.
- Add `create_note_in_context_container_branch_success` regression test — verifies the success path still works after the rollback error arm is added (note created, manifest grows by one, note is container member).

**Out of scope:**

- Fault-injection testing (deferred per ADR-024 — requires a `FailStore` test double not yet implemented).
- WAL/journal crash-safe atomicity (Option B in ADR-024, deferred).
- Any changes to `create_note` itself or the `delete_note` ordering (the known partial-failure mode of `delete_note`'s own two-step delete is documented in ADR-024 as a limitation).
- Cross-store roundtrip test for `create_note_in_context` container branch (this is a bug fix on an existing function, not a new service feature; the existing `create_note` tests already cover the `MemoryStore` path).

---

## Phases

### Phase 1: Apply rollback pattern and add tests

**Goal:** `create_note_in_context` has a best-effort rollback in the `add_member` error arm, documented in its doc comment, with two new tests covering the rollback mechanism building blocks and the success path.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Modify `create_note_in_context` in `crates/srs-repository/src/services.rs` (lines 240-256): wrap `container_service::add_member(store, cid, &result.note.instance_id)?` in an `if let Err(e) = ...` block; on error call `let _ = delete_note(store, &result.note.instance_id);` then `return Err(e);`
- [ ] Update the doc comment on `create_note_in_context`: add the line "If the `add_member` step fails, best-effort rollback via `delete_note`. See ADR-024 for the accepted limitations of this approach." after the existing description.
- [ ] Add test `rollback_mechanism_delete_note_cleans_manifest` in the `#[cfg(test)] mod tests` block of `services.rs`: create a `MemoryStore::default()`, call `create_note`, assert manifest length increased by 1, call `delete_note`, assert manifest returns to original length.
- [ ] Add test `create_note_in_context_container_branch_success` in the same test block: create a `MemoryStore::default()`, create a container via `container_service::create_container`, call `create_note_in_context` with that container ID, assert the note is created, manifest grew by 1, and `container_service::list_members` contains the note's `instance_id`.

#### Acceptance Criteria

- [ ] `create_note_in_context` wraps `add_member` in `if let Err(e) = ...` with `let _ = delete_note(...)` + `return Err(e)` in the error arm.
- [ ] The function's doc comment mentions best-effort rollback and references ADR-024.
- [ ] `rollback_mechanism_delete_note_cleans_manifest` exists and passes.
- [ ] `create_note_in_context_container_branch_success` exists and passes.
- [ ] No existing tests broken.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `rollback_mechanism_delete_note_cleans_manifest` — proves `create_note` + `delete_note` returns manifest to original state (the building blocks the rollback error arm executes)
- `create_note_in_context_container_branch_success` — proves the success path of `create_note_in_context` with a container is unaffected by adding the error arm

#### Milestone gate

1. All acceptance criteria above are met.
2. Both new tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark checkboxes `[x]` and commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [ ] `rollback_mechanism_delete_note_cleans_manifest` passes
- [ ] `create_note_in_context_container_branch_success` passes

## Coordination Rules

- Single-developer execution — no multi-agent coordination needed for this small change.
- Write scope: `crates/srs-repository/src/services.rs` only.

## Assumptions

- `delete_note` is the correct inverse of `create_note`: it removes the note file from disk and removes its entry from `manifest.instanceIndex`. Confirmed by reading `services.rs:579-603`.
- The existing `MemoryStore` test infrastructure in `services.rs` (line 610) is sufficient — no new test helpers required; container creation uses `container_service::create_container` directly.
- No `FailStore` test double exists in this repo yet (ADR-024 documents this gap as a deferred TODO).
