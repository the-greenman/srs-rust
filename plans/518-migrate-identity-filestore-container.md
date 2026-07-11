# Plan: Fix migrate_identity ContainerNotFound on fresh FileStore repos

## Summary

`srs repo migrate-identity` fails with `container not found: <id>` on any FileStore repository whose root container has never been persisted to a file. `create_repository_with_intent` calls `scaffold_purpose_record`, which updates `manifest.container` (the embedded root-container field) and saves the manifest, but never calls `store.save_container()`. Because `FileStore::load_container` looks up the container through `containerIndex` (not the embedded manifest field), any service that calls `load_container` on the root container — including `migrate_identity`'s `add_container_member` and `load_container` calls — returns `ContainerNotFound`. The fix is a one-line addition to `scaffold_purpose_record`: after `store.save_manifest(&manifest)`, also call `store.save_container()` with the updated container.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Main session |
| Repository Service Worker | Main session |
| Verification | Main session |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | File-before-index ordering; `save_container` handles new-container registration | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Lifecycle is adapter-owned; services must not write files directly | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service contract: repository mutations are expressed through `RepositoryStore` | accepted |

No new ADRs — this is a bug fix that fills a gap in the existing `scaffold_purpose_record` lifecycle path. The decision to call `store.save_container()` follows the same pattern already present in `migrate_identity_service.rs` (None-branch) and does not introduce new architectural constraints.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `repo create` payload is unchanged; `repo migrate-identity` payload is unchanged. No `payload.rs` or schema changes needed.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` entity schemas. `check-schema-sync.sh` will pass without changes.

---

## Scope

- Add `store.save_container()` call at the end of `scaffold_purpose_record` in `crates/srs-repository/src/repository_lifecycle.rs`.
- Add two FileStore-backed regression tests in the same file's `#[cfg(test)]` block:
  - `create_repository_with_intent_container_loadable_from_file_store` — verifies `store.load_container(repository_id)` succeeds after creation.
  - `create_repository_with_intent_container_in_container_index` — verifies `containerIndex` has an entry for the root container.

**Out of scope:**
- ADR-007 ordering issue in `FileStore::save_container` (index-before-file for new containers) — pre-existing pattern used by other callers; not part of this bug.
- MemoryStore container index (MemoryStore loads from its in-memory map, not `containerIndex`).
- Any migration/repair for existing repos already on disk that lack a `containerIndex` entry for their root container — those repos are functional (`validate` returns 0 errors; `migrate-identity` on a purpose-record repo already returns "already migrated"), so no data-migration is needed.
- Issue #516 (FailStore fault-injection tests for delete ordering) — separate issue.

---

## Phases

### Phase 1: Fix scaffold_purpose_record

**Goal:** `store.save_container()` is called at the end of `scaffold_purpose_record`, so any FileStore repo created via `create_repository_with_intent` has its root container registered in `containerIndex`.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/repository_lifecycle.rs`, at the end of `scaffold_purpose_record` (after `store.save_manifest(&manifest)?;`), add:
  ```rust
  let container = manifest
      .container
      .as_ref()
      .expect("container always set by get_or_insert_with above");
  if let Err(e) = store.save_container(container) {
      let _ = delete_record(store, &instance_id);
      return Err(e);
  }
  ```
  This persists the container to a file and registers it in `containerIndex` via `FileStore::save_container`'s existing new-container path. Best-effort rollback per ADR-024: if `save_container` fails, the written record is removed. For MemoryStore and JsonStore, `save_container` operates on their in-memory maps — no change in behaviour there.

#### Acceptance Criteria

- [x] `scaffold_purpose_record` calls `store.save_container()` with the manifest's embedded container after saving the manifest.
- [x] `cargo test -p srs-repository` passes with zero failures.

#### Testing

Existing tests (`create_repository_with_intent_roundtrips_via_file_store`) continue to pass.

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify acceptance criteria above.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update plan checkboxes `[x]`.
4. Commit: `fix(repository): scaffold_purpose_record persists root container to containerIndex (#518)`.

---

### Phase 2: Add FileStore regression tests

**Goal:** Two FileStore-specific tests prove that `load_container` succeeds on a freshly created repo and that `containerIndex` is populated, providing a regression guard against the bug.

**Agent:** Repository Service Worker

#### Tasks

- [x] In the `#[cfg(test)]` block of `crates/srs-repository/src/repository_lifecycle.rs`, add test `create_repository_with_intent_container_loadable_from_file_store`:
  1. Create a `TempDir` and `FileStore`.
  2. Call `create_repository_with_intent(&store, &input())`.
  3. Verify `store.load_container(&result.repository_id).is_ok()`.

- [x] Add test `create_repository_with_intent_container_in_container_index`:
  1. Create a `TempDir` and `FileStore`.
  2. Call `create_repository_with_intent(&store, &input())`.
  3. Load manifest; verify `manifest.container_index` has exactly one entry whose `container_id` matches `result.repository_id`.

#### Acceptance Criteria

- [x] `create_repository_with_intent_container_loadable_from_file_store` passes.
- [x] `create_repository_with_intent_container_in_container_index` passes.
- [x] `cargo test -p srs-repository` passes with zero failures.

#### Testing

```bash
cargo test -p srs-repository create_repository_with_intent_container_loadable_from_file_store
cargo test -p srs-repository create_repository_with_intent_container_in_container_index
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `create_repository_with_intent_container_loadable_from_file_store` — proves `load_container` succeeds on FileStore after creation (the bug scenario fails here without the Phase 1 fix).
- `create_repository_with_intent_container_in_container_index` — proves `containerIndex` has the root container entry, catching any future regression that skips `save_container`.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update plan checkboxes `[x]`.
4. Commit: `test(repository): add FileStore regression tests for root container index (#518)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (no payload changes)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-drift.sh` exits 0 for srs-rust (srs-vscode drift is pre-existing, unrelated)
- [x] `store.load_container(repository_id)` succeeds on a FileStore repo created by `create_repository_with_intent`
- [x] `containerIndex` has exactly one entry after `create_repository_with_intent` on FileStore

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `FileStore::save_container` for a new container writes the file AND registers in `containerIndex` via `file_store_upsert_container_index` — confirmed by reading `store.rs`.
- MemoryStore's `save_container` operates on its in-memory `data` map — calling it in `scaffold_purpose_record` is a no-op behavioural change for MemoryStore (it may write a container entry, but MemoryStore tests don't rely on its absence).
- No payload or schema changes required.
