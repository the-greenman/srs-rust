# Plan: Fix FileStore::save_container to use file-before-index ordering for new containers

## Summary

`FileStore::save_container` in `crates/srs-repository/src/store.rs` violates ADR-007 for new
container creates: it calls `file_store_upsert_container_index` (index write) before `write_json`
(file write). ADR-007 requires file-before-index for creates so that an interrupted write leaves
an orphaned file (invisible, recoverable) rather than a dangling index entry (causes read errors).
This plan reorders the operations and adds a fault-injection test to prove the ADR-007 invariant
holds.

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
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | File-before-index for creates; index-before-file for deletes | accepted |

No new ADRs are needed. This plan fixes an existing implementation to comply with an accepted ADR.
The `MemoryStore` `FailPoint` extension follows the pattern established in plan 516; adding a new
`SaveContainerIndex` variant to `FailPoint` is test infrastructure within `#[cfg(test)]` and
establishes no new architectural constraint.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No payload structs added or changed. No action required.

### Entity schema sync (check-schema-sync.sh)

No schema files added or modified. No action required.

---

## Scope

- Fix `FileStore::save_container` in `crates/srs-repository/src/store.rs` to use file-before-index
  ordering for new containers.
- Add `FailPoint::SaveContainerIndex` variant to `MemoryStore`'s fault-injection enum.
- Add a fault-injection test proving that a failed index update leaves no dangling entry.

**Out of scope:**

- `FileStore` OS-level fault injection (requires mocking outside scope).
- Existing-container update path — it only rewrites the file in place with no index change, so is
  not affected.
- `MemoryStore::save_container` ordering — it is already correct (data-then-index), but not
  testable without the new FailPoint.
- Any CLI, WASM, or payload changes.

---

## Phases

### Phase 1: Fix FileStore::save_container ordering

**Goal:** `FileStore::save_container` for a new container writes the file before updating the
index, complying with ADR-007.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/store.rs`, in `FileStore::save_container`, reorder the
  new-container path from index-first to file-first:

  **Current (buggy):**
  ```rust
  Err(RepositoryError::ContainerNotFound { .. }) => {
      let slug = ...;
      let prefix = ...;
      let filename = format!("containers/{slug}-{prefix}.json");
      // Register in index  ← index first (wrong)
      file_store_upsert_container_index(self, id, &container.title, &filename)?;
      filename
  }
  // ...
  self.ensure_dir(&self.repo_root.join("containers"))?;
  self.write_json(&self.abs(&path), &val)  // ← file second (wrong)
  ```

  **Fixed:**
  ```rust
  Err(RepositoryError::ContainerNotFound { .. }) => {
      let slug = ...;
      let prefix = ...;
      let filename = format!("containers/{slug}-{prefix}.json");
      // Write file first (ADR-007: file-before-index for creates)
      self.ensure_dir(&self.repo_root.join("containers"))?;
      self.write_json(&self.abs(&filename), &val)?;
      // Then register in index
      file_store_upsert_container_index(self, id, &container.title, &filename)?;
      filename
  }
  // Remove the trailing ensure_dir + write_json — already done above
  ```

  Concretely: pull `ensure_dir` and `write_json` into the `ContainerNotFound` branch,
  before `file_store_upsert_container_index`. The existing-container `Ok(p)` branch stays
  unchanged (it only rewrites the file, no index change).

  After the fix, the function body should end without a bare `self.write_json` at the bottom —
  the new-container write happens inside the branch, and the existing-container path only
  needs `self.write_json`.

  Rewrite `save_container` as:
  ```rust
  fn save_container(&self, container: &srs_core::types::container::Container) -> Result<(), RepositoryError> {
      let id = &container.container_id;
      let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
          path: std::path::PathBuf::from("containers"),
          source,
      })?;
      match file_store_find_container_path(self, id) {
          Ok(path) => {
              // Existing container — overwrite file in place; index unchanged
              self.write_json(&self.abs(&path), &val)
          }
          Err(RepositoryError::ContainerNotFound { .. }) => {
              // New container — file-before-index (ADR-007)
              let slug = container
                  .title
                  .to_lowercase()
                  .chars()
                  .map(|c| if c.is_alphanumeric() { c } else { '-' })
                  .collect::<String>();
              let prefix = &id[..id.len().min(8)];
              let filename = format!("containers/{slug}-{prefix}.json");
              self.ensure_dir(&self.repo_root.join("containers"))?;
              self.write_json(&self.abs(&filename), &val)?;
              file_store_upsert_container_index(self, id, &container.title, &filename)
          }
          Err(e) => Err(e),
      }
  }
  ```

#### Acceptance Criteria

- [ ] `FileStore::save_container` for a new container writes the file before calling
  `file_store_upsert_container_index`.
- [ ] `FileStore::save_container` for an existing container (found via `file_store_find_container_path`)
  only rewrites the file — no index update.
- [ ] `cargo test -p srs-repository` passes.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests that cover save_container:
- `memory_store_container_operations_are_keyed_by_id` — existing MemoryStore test (not affected by FileStore fix)
- `file_store_manifest_container_and_container_index_roundtrip` — existing FileStore test (roundtrip still passes)

#### Milestone gate

1. Verify all acceptance criteria above.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Mark completed task checkboxes `[x]`.
4. Commit: `fix(repository): file-before-index ordering in FileStore::save_container (#523)`

---

### Phase 2: Add FailPoint::SaveContainerIndex and fault-injection test

**Goal:** A `MemoryStore` test proves that the file-before-index ordering leaves no dangling index
entry when the index update fails after a successful data write.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/store.rs`, in the `#[cfg(test)] pub mod memory` block,
  add `SaveContainerIndex` to the `FailPoint` enum:

  ```rust
  pub enum FailPoint {
      /// Fail the next `save_manifest` call.
      SaveManifest,
      /// Fail the next `delete_instance_file` call.
      DeleteInstanceFile,
      /// Fail the next container-index update in `save_container` (after data write).
      SaveContainerIndex,
  }
  ```

- [ ] In `MemoryStore::save_container`, inject the FailPoint check **after** writing data to
  `self.data` but **before** updating the manifest's `container_index`. This simulates a crash
  between the file write (succeeded) and the index update (not yet started):

  ```rust
  fn save_container(&self, container: &srs_core::types::container::Container) -> Result<(), RepositoryError> {
      let id = &container.container_id;
      let key = format!("containers/{id}.json");
      let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
          path: std::path::PathBuf::from(&key),
          source,
      })?;
      self.data.borrow_mut().insert(key, val);
      // --- fault injection point: after data write, before index update ---
      let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::SaveContainerIndex));
      if should_fail {
          *self.fail_at.borrow_mut() = None;
          return Err(RepositoryError::Io {
              path: std::path::PathBuf::from("injected"),
              source: std::io::Error::new(
                  std::io::ErrorKind::Other,
                  "injected fault: save_container_index",
              ),
          });
      }
      // Update summary index in manifest
      let mut manifest = self.manifest.borrow_mut();
      let mut entries = manifest.container_index.take().unwrap_or_default();
      entries.retain(|e| &e.container_id != id);
      entries.push(srs_core::types::container::ContainerIndexEntry {
          container_id: id.clone(),
          title: Some(container.title.clone()),
          path: None,
          container_type: None,
          tags: None,
          extra: std::collections::HashMap::new(),
      });
      manifest.container_index = Some(entries);
      Ok(())
  }
  ```

- [ ] Add a test inside the `#[cfg(test)] pub mod memory` block in `store.rs`:

  **Test: `save_container_file_first_failed_index_leaves_orphaned_data_safe`**

  Purpose: proves that with file-before-index ordering, a failed index update after a successful
  data write leaves the container data on disk (orphaned, invisible to readers) but NO dangling
  index entry — satisfying ADR-007's safety invariant for creates.

  Steps:
  1. `let store = MemoryStore::empty();`
  2. Build a minimal container: `container_id = "c-test-adr007"`, `title = "ADR-007 Test"`,
     and the minimal fields required by `Container` (`srs_core::types::container::Container`).
  3. `store.arm_fail_at(FailPoint::SaveContainerIndex);`
  4. Call `store.save_container(&container)` — assert `Err(RepositoryError::Io { .. })`.
  5. Assert: `store.load_instance_json("containers/c-test-adr007.json")` succeeds (data was
     written before the injected failure — orphaned data present, safe).
  6. Assert: `store.list_container_summaries()` returns an empty list (no dangling index entry).

#### Acceptance Criteria

- [ ] `FailPoint::SaveContainerIndex` variant exists in the enum.
- [ ] `MemoryStore::save_container` injects the FailPoint between data write and index update.
- [ ] `save_container_file_first_failed_index_leaves_orphaned_data_safe` passes and proves no
  dangling entry when index update fails after data write.
- [ ] Existing `MemoryStore` container tests are unaffected.

#### Testing

```bash
cargo test -p srs-repository save_container_file_first
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `save_container_file_first_failed_index_leaves_orphaned_data_safe` — proves ADR-007 invariant

#### Milestone gate

1. All acceptance criteria above met.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Mark completed checkboxes `[x]`.
4. Commit: `test(repository): fault-injection test for save_container file-before-index ordering (#523)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — no payload structs changed
- [ ] `cargo test --test payload_contracts` passes — no payload structs changed
- [ ] `bash scripts/check-schema-sync.sh` exits 0 — no entity schemas changed
- [ ] `FileStore::save_container` for a new container writes the file before updating the index
- [ ] `save_container_file_first_failed_index_leaves_orphaned_data_safe` test exists and passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and
  pass, update the plan checkboxes, then commit. Do not proceed to the next phase without
  completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The existing-container update path in `FileStore::save_container` (where `file_store_find_container_path` returns `Ok(p)`) is correct — it only rewrites the file in place with no index change — and is not affected by this fix.
- `MemoryStore::save_container` is only used in `#[cfg(test)]` contexts; extending it with `FailPoint::SaveContainerIndex` introduces no production risk.
- `Container` requires `container_id`, `title`, and potentially other fields from `srs_core::types::container::Container`; check the struct definition if the test needs additional field initialisation.
