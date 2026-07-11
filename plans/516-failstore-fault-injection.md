# Plan: FailStore Fault-Injection Tests for Delete Ordering (ADR-024 TODO)

## Summary

Issue #475 fixed `delete_record` and `delete_note` to follow the ADR-007 index-first ordering
for deletes, but the resulting happy-path tests (`record_delete_removes_file_and_manifest_entry`,
`delete_note_removes_file_and_manifest_entry`) verify only success paths and would pass
identically with the old (file-first) ordering. This plan adds a `FailPoint` fault-injection
mechanism to `MemoryStore` and uses it to write tests that prove the key ADR-007 safety
invariant: an interrupted delete leaves an orphaned file (safe) rather than a dangling index
entry (causes errors). These are the fault-injection tests that ADR-024 documents as a TODO.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (self) |
| Repository Service Worker | (self) — writes to `crates/srs-repository/**` only |
| Verification | (self) — read-only audit of test results |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Index-first for deletes; orphaned file is safe, dangling entry is an error | accepted |
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Best-effort rollback; cites fault-injection tests as a TODO | accepted |

No new ADRs are needed. Extending `MemoryStore` with `Option<FailPoint>` is test infrastructure
within the `#[cfg(test)]` boundary; it establishes no new architectural constraint and does not
change any prior decision.

**Design decision (recorded here — no human input required):** Fault injection is implemented by
adding `fail_at: RefCell<Option<FailPoint>>` to `MemoryStore` rather than creating a separate
`FailStore` wrapper. Rationale: `RepositoryStore` has ~50+ methods; a wrapper requires all of
them as pass-throughs. Extending `MemoryStore` directly keeps test infrastructure compact and
consistent with existing `MemoryStore` factory methods (`with_field`, `with_type`, etc.). The
extension is confined to `#[cfg(test)]` and defaults to no failure (no change to existing tests).

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No payload structs added or changed. No action required.

### Entity schema sync (check-schema-sync.sh)

No schema files added or modified. No action required.

---

## Scope

- Add `FailPoint` enum (`SaveManifest`, `DeleteInstanceFile`) to the `#[cfg(test)] pub mod memory` section of `crates/srs-repository/src/store.rs`.
- Add `fail_at: RefCell<Option<FailPoint>>` to `MemoryStore`; update `new()` and `uninitialized()` constructors (note: `empty()` delegates to `new()` so needs no separate change).
- Add `with_fail_at(point: FailPoint) -> Self` builder and `arm_fail_at(&self, point: FailPoint)` / `disarm_fail_at(&self)` methods to `MemoryStore`.
- Update `MemoryStore::save_manifest` and `MemoryStore::delete_instance_file` to check `fail_at` and return an `Io` error when armed.
- Add two smoke tests in `store.rs` `#[cfg(test)] pub mod memory` verifying the FailPoint mechanism in isolation.
- Add three fault-injection tests for `delete_record` in `crates/srs-repository/src/record_store.rs` tests.
- Add three fault-injection tests for `delete_note` in `crates/srs-repository/src/services.rs` tests.

**Out of scope:**
- Fault injection for create paths (covered by ADR-024's "best-effort rollback" — separate issue if needed).
- A `FailStore` wrapper struct (rejected in favour of `MemoryStore` extension; see Design decision above).
- `FileStore` fault injection (not needed; file-level fault injection requires OS-level mocking out of scope).
- Any CLI or WASM surface changes.

---

## Phases

### Phase 1: Add `FailPoint` fault-injection capability to `MemoryStore`

**Goal:** `MemoryStore` can be armed to fail on `save_manifest` or `delete_instance_file`, and
existing tests still pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/store.rs`, inside `#[cfg(test)] pub mod memory { ... }`,
  add:
  ```rust
  /// Fault-injection point for `MemoryStore`. When armed, the next call to the
  /// named operation returns an `Io` error; subsequent calls proceed normally.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum FailPoint {
      /// Fail the next `save_manifest` call.
      SaveManifest,
      /// Fail the next `delete_instance_file` call.
      DeleteInstanceFile,
  }
  ```
- [ ] Add `fail_at: RefCell<Option<FailPoint>>` field to `MemoryStore`.
- [ ] Update `MemoryStore::new(manifest, package)` and `MemoryStore::uninitialized()` to
  initialise `fail_at: RefCell::new(None)`. (`empty()` delegates to `new()` and needs no change.)
- [ ] Add builder method (consistent with other `MemoryStore` builders that take `self`):
  ```rust
  /// Arm a fail point; the next call to the named operation will return an error.
  pub fn with_fail_at(self, point: FailPoint) -> Self {
      *self.fail_at.borrow_mut() = Some(point);
      self
  }
  ```
- [ ] Add mutator methods:
  ```rust
  pub fn arm_fail_at(&self, point: FailPoint) {
      *self.fail_at.borrow_mut() = Some(point);
  }
  pub fn disarm_fail_at(&self) {
      *self.fail_at.borrow_mut() = None;
  }
  ```
- [ ] In `MemoryStore::save_manifest`, add at the top (peek-then-conditional-clear to avoid
  consuming a non-matching variant):
  ```rust
  let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::SaveManifest));
  if should_fail {
      *self.fail_at.borrow_mut() = None;
      return Err(RepositoryError::Io {
          path: std::path::PathBuf::from("injected"),
          source: std::io::Error::new(std::io::ErrorKind::Other, "injected fault: save_manifest"),
      });
  }
  ```
- [ ] In `MemoryStore::delete_instance_file`, add at the top (same peek-then-conditional-clear
  pattern):
  ```rust
  let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::DeleteInstanceFile));
  if should_fail {
      *self.fail_at.borrow_mut() = None;
      return Err(RepositoryError::Io {
          path: std::path::PathBuf::from(relative_path),
          source: std::io::Error::new(std::io::ErrorKind::Other, "injected fault: delete_instance_file"),
      });
  }
  ```

**Why peek-then-conditional-clear:** `Option::take()` removes the value unconditionally before
the `if let` pattern check runs. If `SaveManifest` is armed and `delete_instance_file` is called,
`.take()` would consume the fail point with no error fired, leaving the store permanently
disarmed. The peek pattern (`matches!` read + explicit clear only when matching) avoids this.

- [ ] Add two smoke tests inside `#[cfg(test)] pub mod memory { ... }` in `store.rs`:

  **Smoke test 1:** `memory_store_save_manifest_fail_point_triggers_error`
  - Create `MemoryStore::empty()`.
  - Call `store.arm_fail_at(FailPoint::SaveManifest)`.
  - Call `store.save_manifest(&Manifest::default())` — assert `Err(RepositoryError::Io)`.
  - Call `store.save_manifest(&Manifest::default())` again — assert `Ok(())` (one-shot disarmed).

  **Smoke test 2:** `memory_store_delete_instance_file_fail_point_triggers_error`
  - Create `MemoryStore::empty()`.
  - Pre-populate with a dummy data entry at path `"records/test.json"`.
  - Call `store.arm_fail_at(FailPoint::DeleteInstanceFile)`.
  - Call `store.delete_instance_file("records/test.json")` — assert `Err(RepositoryError::Io)`.
  - Call `store.delete_instance_file("records/test.json")` again — assert `Ok(())` (disarmed).

#### Acceptance Criteria

- [ ] `MemoryStore::empty()` constructs without `fail_at` affecting any operation.
- [ ] An armed `MemoryStore` returns an `Io` error on the targeted operation.
- [ ] After the error fires, the fail point is automatically disarmed (one-shot).
- [ ] Arming `SaveManifest` does NOT consume the fail point when `delete_instance_file` is called (and vice versa).
- [ ] All existing `MemoryStore` tests continue to pass.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

- `memory_store_save_manifest_fail_point_triggers_error` — confirm an armed store errors on `save_manifest` and clears the point
- `memory_store_delete_instance_file_fail_point_triggers_error` — confirm an armed store errors on `delete_instance_file` and clears the point

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm the two smoke tests above exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark completed task checkboxes `[x]`.
5. Commit: `test(repository): add FailPoint fault-injection capability to MemoryStore (#516)`

---

### Phase 2: Fault-injection tests for `delete_record` (ADR-007 ordering invariant)

**Goal:** Three tests in `record_store.rs` prove that `delete_record` satisfies the ADR-007
index-first delete invariant and document the old file-first bug as a regression baseline.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/record_store.rs` test module, add:

  **Test A — old ordering (file-first) baseline: documents the bug**
  ```
  test name: delete_record_old_file_first_ordering_leaves_dangling_index_entry
  ```
  Steps:
  1. Call `make_store_with_package()` to get a `MemoryStore`.
  2. Create a record via `create_record` and retrieve its `path` from the manifest
     (load the manifest, find the entry for the record's instance_id, read its path field).
  3. Call `store.arm_fail_at(FailPoint::SaveManifest)`.
  4. Directly call `store.delete_instance_file(&path)` — succeeds (file deleted).
  5. Build `manifest_without_entry`: load the current manifest, remove the record's entry
     from `instance_index`, then call `store.save_manifest(&manifest_without_entry)` —
     returns `Err(RepositoryError::Io)` (fault injected; manifest NOT updated).
  6. Assert: `store.load_instance_json(&path)` returns `Err` (file is gone).
  7. Assert: the manifest's `instance_index` still contains the entry for this record's
     instance_id (dangling entry — this is the bug that file-first ordering causes).

  **Test B — new ordering, manifest-write failure: no data loss, no dangling entry**
  ```
  test name: delete_record_index_first_manifest_fail_leaves_record_intact
  ```
  Steps:
  1. `make_store_with_package()`, create record, get path.
  2. Call `store.arm_fail_at(FailPoint::SaveManifest)`.
  3. Call `delete_record(&store, &instance_id)` — returns `Err(RepositoryError::Io)`.
  4. Assert: `store.load_instance_json(&path)` succeeds (file still present).
  5. Assert: manifest index still contains the entry (no data loss, no dangling entry).

  **Test C — new ordering, file-delete failure: orphaned file, index cleared**
  ```
  test name: delete_record_index_first_file_fail_leaves_orphaned_file_safe
  ```
  Steps:
  1. `make_store_with_package()`, create record, get path.
  2. Call `store.arm_fail_at(FailPoint::DeleteInstanceFile)`.
  3. Call `delete_record(&store, &instance_id)` — returns `Ok` (manifest write succeeded;
     file delete failed but is swallowed per ADR-007).
  4. Assert: `store.load_instance_json(&path)` succeeds (orphaned file still present).
  5. Assert: manifest index no longer contains the entry (safe — no dangling entry).

#### Acceptance Criteria

- [ ] `delete_record_old_file_first_ordering_leaves_dangling_index_entry` — passes and demonstrates dangling entry when file deleted before manifest committed.
- [ ] `delete_record_index_first_manifest_fail_leaves_record_intact` — passes and demonstrates no data loss when manifest write fails.
- [ ] `delete_record_index_first_file_fail_leaves_orphaned_file_safe` — passes and demonstrates orphaned-file safety when file delete fails.
- [ ] All three tests use `MemoryStore` (not `FileStore`).

#### Testing

```bash
cargo test -p srs-repository delete_record_old_file_first
cargo test -p srs-repository delete_record_index_first_manifest_fail
cargo test -p srs-repository delete_record_index_first_file_fail
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All three tests listed above pass.
2. `cargo test -p srs-repository` and `cargo clippy -p srs-repository -- -D warnings` pass.
3. Mark checkboxes `[x]`.
4. Commit: `test(repository): fault-injection tests for delete_record ADR-007 ordering (#516)`

---

### Phase 3: Fault-injection tests for `delete_note` (ADR-007 ordering invariant)

**Goal:** Three parallel tests in `services.rs` verify the same index-first delete invariant for
`delete_note`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/services.rs` test module, add (using `store_with_note`
  helper already defined there):

  **Test A — old ordering (file-first) baseline: documents the bug**
  ```
  test name: delete_note_old_file_first_ordering_leaves_dangling_index_entry
  ```
  Steps:
  1. Build a note and a path string (e.g. `"notes/test-note.json"`).
  2. Call `store_with_note(note.clone(), path.clone())` to get a `MemoryStore`.
  3. Call `store.arm_fail_at(FailPoint::SaveManifest)`.
  4. Directly call `store.delete_instance_file(&path)` — succeeds (file deleted).
  5. Build `manifest_without_entry`: load the current manifest, remove the note's entry
     from `instance_index`, then call `store.save_manifest(&manifest_without_entry)` —
     returns `Err(RepositoryError::Io)` (fault injected; manifest NOT updated).
  6. Assert: `store.load_instance_json(&path)` returns `Err` (file is gone).
  7. Assert: the manifest's `instance_index` still contains the entry for this note's
     instance_id (dangling entry).

  **Test B — new ordering, manifest-write failure: no data loss**
  ```
  test name: delete_note_index_first_manifest_fail_leaves_note_intact
  ```
  Steps:
  1. Build note + path; call `store_with_note(note.clone(), path.clone())`.
  2. Call `store.arm_fail_at(FailPoint::SaveManifest)`.
  3. Call `delete_note(&store, note_id)` — returns `Err(RepositoryError::Io)`.
  4. Assert: `store.load_instance_json(&path)` succeeds (file still present).
  5. Assert: manifest index still contains the note's entry.

  **Test C — new ordering, file-delete failure: orphaned file, index cleared**
  ```
  test name: delete_note_index_first_file_fail_leaves_orphaned_file_safe
  ```
  Steps:
  1. Build note + path; call `store_with_note(note.clone(), path.clone())`.
  2. Call `store.arm_fail_at(FailPoint::DeleteInstanceFile)`.
  3. Call `delete_note(&store, note_id)` — returns `Ok` (manifest write succeeded;
     file delete failed but is swallowed per ADR-007).
  4. Assert: `store.load_instance_json(&path)` succeeds (orphaned file still present).
  5. Assert: manifest index no longer contains the note's entry (safe — no dangling entry).

#### Acceptance Criteria

- [ ] All three `delete_note` fault-injection tests pass.
- [ ] `cargo test -p srs-repository` and `cargo clippy -p srs-repository -- -D warnings` pass.

#### Testing

```bash
cargo test -p srs-repository delete_note_old_file_first
cargo test -p srs-repository delete_note_index_first_manifest_fail
cargo test -p srs-repository delete_note_index_first_file_fail
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All three tests pass.
2. Full test suite and clippy pass.
3. Mark checkboxes `[x]`.
4. Commit: `test(repository): fault-injection tests for delete_note ADR-007 ordering (#516)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass) — no CLI changes made
- [ ] `cargo test --test payload_contracts` passes — no payload structs changed
- [ ] `bash scripts/check-schema-sync.sh` exits 0 — no entity schemas changed
- [ ] Six new fault-injection tests exist and pass (3 for `delete_record`, 3 for `delete_note`)
- [ ] `MemoryStore` fault injection is one-shot: the fail point disarms after triggering
- [ ] Arming one `FailPoint` variant does not consume or interfere with the other variant
- [ ] No existing tests regress

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `MemoryStore` is only used in `#[cfg(test)]` contexts; adding `fail_at` introduces no production risk.
- The `MemoryStore::save_manifest` and `MemoryStore::delete_instance_file` implementations are the ones used by `delete_record` and `delete_note` (verified: both functions call `store.save_manifest()` and `store.delete_instance_file()` via the trait).
- The `manifest_without_entry` in Test A for both phases is constructed by loading the manifest, removing the entry, then calling `save_manifest` directly — this is the store manipulation test pattern used elsewhere.
