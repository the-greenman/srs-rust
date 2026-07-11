# Plan: Fix `delete_record` / `delete_note` to use ADR-007 index-first ordering

## Summary

`delete_record` in `crates/srs-repository/src/record_store.rs` and `delete_note` in
`crates/srs-repository/src/services.rs` both delete the entity file first and update the
manifest index second. ADR-007 prescribes the opposite for deletes (index-first, then file),
because file-first ordering means an interrupted delete leaves a **dangling index entry**
(manifest references a missing file) rather than an orphaned file (file exists but is not
indexed). Dangling entries cause every subsequent `list`/`get` to fail; orphaned files are
invisible to readers and recoverable by `srs repo repair`. The fix swaps the two steps in
both delete functions so they comply with ADR-007. ADR-024 will be updated to reflect that
the cited limitation is resolved.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. This plan implements the already-accepted ADR-007 delete ordering
rule for `delete_record` and `delete_note`, which previously violated it.

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Index-first ordering for delete; this plan brings both delete functions into compliance | accepted |
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Update to note the `delete_record` ordering limitation is now resolved | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. The fix is purely in the `srs-repository` service
layer; `delete_record` and `delete_note` return the same types. No payload struct changes.
`cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are added or modified. No sync action required.

---

## Scope

- Fix `delete_record` in `crates/srs-repository/src/record_store.rs` to use index-first
  ordering: remove from manifest in memory → persist manifest → delete file (best-effort) →
  delete sidecar (already best-effort).
- Fix `delete_note` in `crates/srs-repository/src/services.rs` to use the same index-first
  ordering: remove from manifest in memory → persist manifest → delete file (best-effort).
- Add tests for both functions that verify: after a successful delete, the manifest no longer
  contains the entry and the file is gone.
- Update ADR-024 to note that the `delete_record` ordering limitation cited in its Negative
  section has been resolved.

**Out of scope:**

- Implementing a `FailStore` test double for fault-injection tests (the ADR-024 TODO note).
  Deferred to a follow-up issue.
- `srs repo repair` implementation (already tracked separately as ADR-007 future work).
- Any changes to the WAL / journalling path (Option B from ADR-024).

---

## Phases

### Phase 1: Fix ordering and update ADR-024

**Goal:** Both `delete_record` and `delete_note` follow ADR-007 index-first ordering, tests
verify the happy-path post-delete invariants, and ADR-024 accurately reflects current state.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/record_store.rs`, function `delete_record` (line ~450):
  Reorder to:
  ```rust
  manifest.instance_index.remove(entry_index);
  write_manifest(store, &manifest)?;
  let _ = store.delete_instance_file(&path);
  let _ = revision_service::delete_sidecar(store, &path);
  ```
  The file deletion becomes best-effort (`let _ =`) because once the index write commits, the
  semantic delete is done; the file is an orphan. The sidecar was already best-effort.
  Update the doc comment on `delete_record` to reflect the corrected ordering.

- [x] In `crates/srs-repository/src/services.rs`, function `delete_note` (line ~586):
  Reorder to:
  ```rust
  manifest.instance_index.remove(entry_index);
  write_manifest(store, &manifest)?;
  let _ = store.delete_instance_file(&path);
  ```
  Same rationale. Update the doc comment on `delete_note`.

- [x] Update `docs/adr/024-best-effort-rollback-multi-write-services.md` in two places:
  1. **Negative / trade-offs bullet 2** — find the bullet beginning "`delete_record` itself uses
     file-before-index ordering for deletes (deletes the file, then removes the manifest entry),
     which is the inverse of ADR-007's prescribed index-before-file ordering." Replace with a
     note that the ordering has been fixed (index-first as of issue #475) and the dangling-entry
     risk on the rollback path is resolved. Preserve the crash-safety limitation note.
  2. **Final paragraph of the Negative section** — find the sentence "the file-before-index
     ordering risk (first bullet) and the `let _ = …` error-swallowing (third bullet) both carry
     over to `delete_note`." Update it to reflect that `delete_note` ordering is also now fixed,
     so only the `let _ = …` error-swallowing carries over.

- [x] Verify existing tests in `crates/srs-repository/src/record_store.rs` still pass after
  the ordering change. The tests `record_delete_removes_file_and_manifest_entry` (line 2793)
  and `rollback_mechanism_delete_record_cleans_manifest` (line 5269) already cover the
  happy-path post-delete invariants for `delete_record`. No new record-delete tests are
  needed; verify both pass.

- [x] Add one new test in `crates/srs-repository/src/services.rs`:
  - `delete_note_removes_file_and_manifest_entry` — builds on `store_with_note`, calls
    `delete_note`, asserts `store.load_instance_json("records/notes/test-note.json")` returns
    `Err`, and asserts `manifest.instance_index.is_empty()`. The existing test
    `note_delete_removes_and_updates_manifest` (line 1156) checks only the manifest; this
    new test additionally verifies the file is removed on the happy path.

- [x] File a GitHub issue for the FailStore fault-injection test double (deferred from
  ADR-024). Title: "Add FailStore test double for fault-injection testing of delete ordering
  (ADR-024 TODO)". Label: `enhancement`. Body: explain that ADR-024 documents the need,
  and that the test would verify: `delete_instance_file` succeeds but `write_manifest` fails
  → no dangling entry (the old bug) vs. manifest updated first then `write_manifest` fails
  → would never be reached with current ordering. Link this issue as a comment on #475.
  Filed as srs-rust #516. Linked under epic #475.

#### Acceptance Criteria

- [x] `delete_record` performs: manifest-remove → write_manifest → delete_instance_file (best-
  effort) → delete_sidecar (best-effort).
- [x] `delete_note` performs: manifest-remove → write_manifest → delete_instance_file (best-
  effort).
- [x] Both functions' doc comments describe the corrected ordering.
- [x] ADR-024 Negative section updated to note the ordering fix.
- [x] Existing `record_delete_removes_file_and_manifest_entry` and
  `rollback_mechanism_delete_record_cleans_manifest` tests still pass.
- [x] New `delete_note_removes_file_and_manifest_entry` test exists and passes.
- [x] Acceptance Criteria note (non-testable): the **ordering safety property** (no dangling
  index entry when file deletion fails mid-delete) is NOT covered by the happy-path tests.
  The FailStore issue filed in Phase 1 is the mechanism that will eventually close this gap.
  The fix is correct; the property is verified by code inspection of the ordering.
- [x] No existing tests regress.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `record_delete_removes_file_and_manifest_entry` (existing, `record_store.rs` line 2793) —
  proves both file and manifest entry are removed; must still pass after ordering change.
- `rollback_mechanism_delete_record_cleans_manifest` (existing, `record_store.rs` line 5269) —
  proves rollback path returns manifest to original length; must still pass.
- `delete_note_removes_file_and_manifest_entry` (**new**, `services.rs`) — proves both file
  and manifest entry are removed on the `delete_note` happy path.
- `create_then_delete_note_is_manifest_roundtrip` (existing, `services.rs` line 1462) —
  must still pass.
- `note_delete_removes_and_updates_manifest` (existing, `services.rs` line 1156) — must
  still pass.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm: all **existing** tests listed in the Testing section still pass, and the **new**
   test `delete_note_removes_file_and_manifest_entry` has been written and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
cargo test --test payload_contracts
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [x] `delete_record` ordering is index-first (manifest update before file deletion)
- [x] `delete_note` ordering is index-first (manifest update before file deletion)
- [x] ADR-024 updated to reflect resolved ordering limitation
- [x] Deferred FailStore fault-injection follow-up issue filed and linked (srs-rust #516, created during Phase 1)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist
  and pass, update the plan checkboxes, then commit. Do not proceed without completing the
  milestone gate.
- Verification Agent runs after the phase and before final sign-off.

## Assumptions

- `MemoryStore.delete_instance_file` succeeds even for paths that were never written
  (returns Ok silently), so making file deletion best-effort in the new ordering does not
  affect happy-path tests.
- The fault-injection test scenario (manifest write fails after file is deleted) requires a
  `FailStore` test double that does not currently exist. The fix can be shipped without that
  test; a follow-up issue tracks its addition.
