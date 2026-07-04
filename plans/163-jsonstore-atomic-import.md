# Plan: JsonStore Atomic Write for import_repository_snapshot (#163)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`JsonStore::save_instance_json` calls `flush()` after every insert, writing the entire `.srsj` file to disk on each record write during `import_repository_snapshot`. If the import fails mid-stream (e.g. at record 22 of 23), the `.srsj` file on disk contains partial record data with an empty `instanceIndex` — logically inconsistent but syntactically valid JSON. This plan adds an opt-in batch write mode to `RepositoryStore` (via default-no-op trait methods), implements it in `JsonStore` using a `batching` flag that defers `flush()` until `commit_batch()`, and wraps `import_repository_snapshot` in begin/commit so the `.srsj` file is only written once — after all records, packages, containers, and relations are committed successfully. The WASM binding path (`from_srsj`/`to_srsj_string`) is unaffected because it uses the `<memory>` sentinel path and already no-ops in `flush()`.

## Agent Assignments

| Role | Agent |
|---|---|
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-store-write-ordering.md) | FileStore atomicity: per-file write-before-index ordering already provides partial protection; atomic batch for FileStore is out of scope | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Storage-agnostic portability — `import_repository_snapshot` must remain backend-neutral; fix belongs in the store layer, not the service | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding strategy establishes `from_srsj`/`to_srsj_string` interface and `<memory>` sentinel convention; batch mode must not affect this path | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | WASM write bindings: `flush()` no-ops for `<memory>` sentinel path; batch mode guard precedes the `<memory>` guard in `flush()`, so WASM callers are unaffected even in a hypothetical batch context | accepted |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | `commit_batch()` must call `to_srsj_string()` (same as `flush()`), preserving deterministic BTreeMap serialisation | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) | `RepositoryStore` gains optional `begin_batch`/`commit_batch`/`abort_batch` with default no-op impls; `JsonStore` defers disk writes when in batch mode; `import_repository_snapshot` uses this to make bulk imports atomic | proposed |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. No `payload.rs` edits required. No `cargo run --bin generate-schemas` run needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files modified. No action required.

---

## Scope

- Add three default-no-op methods to `RepositoryStore` trait: `begin_batch(&self)`, `commit_batch(&self) -> Result<(), RepositoryError>`, `abort_batch(&self)`.
- Add `batching: bool` field to `JsonStoreState`.
- Override `begin_batch`, `commit_batch`, `abort_batch` in `JsonStore`.
  - `begin_batch`: set `state.batching = true`.
  - `commit_batch`: set `state.batching = false`, perform atomic write via `flush()` (which internally calls `to_srsj_string()` + `std::fs::write`). The `<memory>` path guard in `flush()` already prevents any I/O in WASM context.
  - `abort_batch`: set `state.batching = false`, then reload the in-memory state from disk (for disk-backed stores). This is required because partial import data already accumulated in `JsonStoreState.data`; without a reload, a subsequent write on the same instance would flush dirty data to disk. For the WASM `<memory>` path, no reload is possible; callers must not reuse a memory-backed store after `abort_batch`.
- Guard `flush()` in `JsonStore`: return `Ok(())` immediately when `state.borrow().batching`. This guard is placed **before** the `<memory>` sentinel guard so the two checks are independent (ADR-021).
- Wrap `import_repository_snapshot` in `crates/srs-repository/src/repository_portability.rs` with `begin_batch` / `commit_batch` / `abort_batch`.
- Add four regression tests in `crates/srs-repository/src/json_store.rs`.
- Add one test in `crates/srs-repository/src/repository_portability.rs`.

**Out of scope:**
- Atomic writes for `FileStore` (separate concern; the per-file write-before-index ordering of ADR-007 already provides partial protection there).
- Batch mode for any service other than `import_repository_snapshot` (no current consumer).
- Changes to `srs-cli`, `srs-bindings`, `srs-core`, `srs-projection`, or any schema/payload files.
- Any change to `to_srsj_string` or `from_srsj` (WASM-safe path remains unchanged).

---

## Phases

### Phase 1: Trait extension and JsonStore implementation

**Goal:** `RepositoryStore` has batch methods with default no-op impls; `JsonStore` implements them and suppresses intermediate flushes in batch mode.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/store.rs`, add the following three methods to the `RepositoryStore` trait (after the `save_manifest` group is a good location), with default no-op implementations:
  ```rust
  fn begin_batch(&self) {}
  fn commit_batch(&self) -> Result<(), RepositoryError> { Ok(()) }
  fn abort_batch(&self) {}
  ```
- [x] In `crates/srs-repository/src/json_store.rs`, add `batching: bool` to `JsonStoreState` (default: `false`).
- [x] In `JsonStore::flush()`, add an early return at the top of the method body (before the `<memory>` guard) when `self.state.borrow().batching` is `true`:
  ```rust
  if self.state.borrow().batching {
      return Ok(());
  }
  ```
- [x] In `JsonStore`'s `impl RepositoryStore for JsonStore` block, add batch method implementations including `abort_batch` which reloads in-memory state from disk to prevent dirty-state writes after abort.
- [x] Add `batching: false` to every `JsonStoreState { ... }` struct literal in `crates/srs-repository/src/json_store.rs`. There are exactly 2 construction sites: one in `JsonStore::create` and one in `JsonStore::from_srsj`. Search for `JsonStoreState {` in that file to locate both.

#### Acceptance Criteria

- [x] `cargo build -p srs-repository` compiles with zero errors.
- [x] `JsonStore::save_instance_json` called in a loop while in batch mode does NOT write to disk until `commit_batch()` is called.
- [x] `JsonStore::save_instance_json` called outside batch mode continues to flush to disk immediately (existing `json_store_flush_on_save_instance` test still passes).
- [x] Calling `commit_batch()` on a `MemoryStore` or `FileStore` returns `Ok(())` via the default no-op. (Compiler-verified: the default impl is `fn commit_batch(&self) -> Result<(), RepositoryError> { Ok(()) }` — no dedicated test needed.)
- [x] `from_srsj` path: `flush()` still returns `Ok(())` without I/O when `file_path == "<memory>"`.

#### Testing

```bash
cargo test -p srs-repository json_store
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify (add to `#[cfg(test)]` in `crates/srs-repository/src/json_store.rs`):

- `json_store_batch_mode_suppresses_intermediate_flushes` — create a `JsonStore`, call `begin_batch()`, call `save_instance_json` with a record, verify the on-disk file does NOT yet contain the record, then call `commit_batch()`, verify the on-disk file now contains the record.
- `json_store_abort_batch_leaves_file_unchanged` — create a `JsonStore`, initialise a repo, flush baseline, call `begin_batch()`, call `save_instance_json` with a new record, call `abort_batch()`, reload from disk, confirm the new record is absent (disk file unchanged). Also confirm the in-memory state was restored: call `load_instance_json` on the same store instance and verify the new record is absent there too.
- `json_store_commit_batch_writes_all_accumulated_data` — create a `JsonStore`, call `begin_batch()`, call `save_instance_json` three times, call `commit_batch()`, reload from disk, confirm all three records are present.

The existing `json_store_flush_on_save_instance` test covers non-batch flush-on-write behaviour — no duplicate needed.

#### Milestone gate

1. Verify all three new tests above exist and pass.
2. Verify existing `json_store_flush_on_save_instance` test still passes.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes `[x]`.
5. Commit: `feat(#163): add batch write mode to RepositoryStore + JsonStore`

---

### Phase 2: Wrap import_repository_snapshot with begin/commit

**Goal:** `import_repository_snapshot` uses batch mode when the target supports it, making the full import atomic for `JsonStore` targets.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/repository_portability.rs`, refactor `import_repository_snapshot` to wrap its entire body in begin/commit/abort:
  1. Call `target.begin_batch()` before `ensure_target_empty(target)?`.
  2. Move the existing body (from `ensure_target_empty` through `save_relations_json`) into a **module-private** helper `fn do_import(target: &dyn RepositoryStore, snapshot: &RepositorySnapshot) -> Result<(), RepositoryError>` (no `pub` or `pub(crate)` — not callable outside `repository_portability`).
  3. In the public `import_repository_snapshot`, call `do_import`, and on error call `target.abort_batch()` then return the error; on success call `target.commit_batch()`.

  The refactored signature becomes:
  ```rust
  pub fn import_repository_snapshot(
      target: &dyn RepositoryStore,
      snapshot: &RepositorySnapshot,
  ) -> Result<(), RepositoryError> {
      target.begin_batch();
      let result = do_import(target, snapshot);
      match result {
          Ok(()) => target.commit_batch(),
          Err(e) => {
              target.abort_batch();
              Err(e)
          }
      }
  }
  ```

  `do_import` is the existing function body verbatim.

#### Acceptance Criteria

- [x] All existing `repository_portability` tests pass unchanged.
- [x] A partial import failure (injected error after 2 of 3 records) leaves the `.srsj` file unchanged on disk (see new test below).
- [x] A successful import into a `JsonStore` writes the file exactly once (commit_batch), not once-per-record.
- [x] `copy_repository` (which calls `import_repository_snapshot`) continues to work for all store combinations.

#### Testing

```bash
cargo test -p srs-repository repository_portability
cargo test -p srs-repository json_store
cargo clippy -p srs-repository -- -D warnings
```

Specific test to write (add to `#[cfg(test)]` in `crates/srs-repository/src/repository_portability.rs`):

- `json_store_partial_import_leaves_file_unchanged` — set up a `MemoryStore` with a repo and 3 instances; corrupt the 3rd instance by setting `instance_id` to a 4-character string (shorter than 8, which causes `id_prefix` to fail with `InvalidSnapshotData`); create a `JsonStore` (empty target); record the initial file contents via `std::fs::read_to_string`; call `import_repository_snapshot` targeting the `JsonStore`; confirm the call returns `Err(RepositoryError::InvalidSnapshotData { .. })`; reload the file from disk and confirm its contents match the initial file contents (no partial records written).
- `json_store_successful_import_writes_file_exactly_once` — create a `JsonStore` at a temp path; run `import_repository_snapshot` with a valid 3-instance snapshot; open a second `JsonStore::open(&path)`, call `load_instance_json` for all 3 expected canonical paths, and confirm all 3 records are present. mtime can be used as a soft assertion that the file changed, but record presence is the hard assertion.

#### Milestone gate

1. Verify the new test `json_store_partial_import_leaves_file_unchanged` exists and passes.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update plan checkboxes `[x]`.
4. Commit: `fix(#163): make import_repository_snapshot atomic for JsonStore`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `srs repo validate --repo ../srs/srs` exits 0 with 0 errors
- [ ] `json_store_batch_mode_suppresses_intermediate_flushes` test passes
- [ ] `json_store_abort_batch_leaves_file_unchanged` test passes
- [ ] `json_store_commit_batch_writes_all_accumulated_data` test passes
- [ ] `json_store_partial_import_leaves_file_unchanged` test passes
- [ ] Existing `json_store_flush_on_save_instance` test still passes (no regression)
- [ ] All existing `repository_portability` tests pass

## Coordination Rules

- Repository Service Worker owns `crates/srs-repository/**` exclusively.
- No other crates are touched.
- Verification Agent runs final acceptance after Phase 2 before PR.

## Assumptions

- `JsonStore::flush()` is the sole path that writes to disk; all other save methods call `flush()` internally.
- `std::fs::write` on the target platform is sufficiently atomic for the use cases (complete replacement, not partial write). A write-then-rename pattern is not required since `std::fs::write` on Linux uses `O_TRUNC` and the risk of partial content is limited to partial OS write, not the application-level partial-import risk this plan addresses.
- `MemoryStore` and `FileStore` default no-op implementations are correct and sufficient (neither requires batch-mode coordination).
- The WASM (`from_srsj`/`to_srsj_string`) path is unaffected because `file_path == PathBuf::from("<memory>")` already bypasses `flush()` entirely.
