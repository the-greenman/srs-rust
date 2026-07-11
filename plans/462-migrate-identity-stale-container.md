# Plan: Fix migrate-identity leaving persisted Container's identityInstanceId stale

## Summary

`srs repo migrate-identity` updates `manifest.container.identityInstanceId` to the new `com.semanticops.core/purpose` record, but the **persisted root Container file** (`containers/<id>.json`) is never rewritten with the new pointer. Its `identityInstanceId` still references the old Tier-0 note, so the two representations disagree. The None-branch (no prior identity) correctly calls `store.save_container()` with the updated embed; the old-identity branch omits this step. This plan adds the missing write inside the existing ADR-021 batch and a regression test to prevent recurrence.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. All choices are governed by existing ADRs.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Fix stays in `srs-repository`; no CLI or handler changes | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) | Container update added inside the existing batch closure so all four writes (record file, manifest, container members, container identity) remain atomic | accepted |
| [ADR-021 — nested-batch exclusion](../docs/adr/021-jsonstore-batch-write-mode.md) | `container_service::update_container` MUST NOT be called inside this closure — for root containers it calls `begin_batch`/`commit_batch` internally, which would prematurely flush on `JsonStore` before all batch writes are in memory. Raw `store.load_container`/`store.save_container` are used instead; a mandatory code comment at the call site guards this invariant. | accepted |
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Inapplicable — this write updates an existing container; no `containerIndex` entry is created or deleted, so file-before-index ordering does not apply | inapplicable |
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Existing abort path handles container save failure | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands. No payload struct changes. `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- In `crates/srs-repository/src/migrate_identity_service.rs`: inside the batch closure for the "old identity present" path, after `remove_container_member`, load the persisted Container, set `identity_instance_id = Some(new_id.clone())`, and call `store.save_container`.
- Add one new unit test: `migrate_updates_persisted_container_identity_pointer` — verifies the persisted Container's `identity_instance_id` equals `new_identity_id` after migration.

**Out of scope:**
- None-branch (no prior identity) — that path already calls `store.save_container()` correctly; verified by `migrate_from_container_adds_to_members`.
- Any changes to CLI, payload, or bindings — the service fix is sufficient.
- Reconciling `manifest.container.member_instance_ids` with the persisted container (a separate potential gap, not reported as a bug).

---

## Phases

### Phase 1: Fix persisted Container's identityInstanceId update

**Goal:** After migration the persisted Container file and `manifest.container` embed agree on `identityInstanceId`; a regression test confirms this.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/migrate_identity_service.rs`, locate the batch closure in the "old identity present" path (the `let batch_result = (|| -> Result<(), RepositoryError> { ... })()` block that calls `add_container_member` then `remove_container_member`).

  After the `remove_container_member` call, add:

  ```rust
  // Update the persisted Container record's identityInstanceId in lockstep with the
  // manifest embed. Without this the container file disagrees with manifest.container
  // (issue #462).
  //
  // IMPORTANT: Do NOT replace this with container_service::update_container.
  // For root containers, update_container calls begin_batch/commit_batch internally
  // (container_service.rs ~line 234). Nesting that inside this outer batch causes
  // JsonStore to commit_batch prematurely — before add/remove_member writes are in
  // memory — violating ADR-021 atomicity on the WASM/srsj path. MemoryStore tests
  // would not catch this regression.
  let mut persisted_container = store.load_container(&root_container_id)?;
  persisted_container.identity_instance_id = Some(new_id.clone());
  store.save_container(&persisted_container)?;
  ```

  The new save is inside the existing `begin_batch`/`commit_batch` block, so all four writes (record file, manifest, container members, container identity) remain atomic per ADR-021.

- [ ] In the same file's `#[cfg(test)]` block, add the new test immediately after `migrate_adds_new_and_removes_old_from_container_members`:

  ```rust
  #[test]
  fn migrate_updates_persisted_container_identity_pointer() {
      let (store, container_id) = make_store_with_identity(
          "11111111-1111-4111-8111-111111111118",
          Some("Repo"),
          one_section("Content."),
      );
      let result = migrate_identity(&store).unwrap();
      let container = get_container(&store, &container_id).unwrap();
      assert_eq!(
          container.identity_instance_id,
          Some(result.new_identity_id),
          "persisted Container record must have identityInstanceId updated after migration"
      );
  }
  ```

#### Acceptance Criteria

- [ ] After `migrate_identity` on a repo with a Tier-0 identity, `get_container(store, root_container_id).identity_instance_id == Some(new_identity_id)`
- [ ] `manifest.container.identity_instance_id` continues to equal `new_identity_id` (no regression)
- [ ] Container `member_instance_ids` still updated correctly (no regression on `migrate_adds_new_and_removes_old_from_container_members`)
- [ ] `migrate_updates_persisted_container_identity_pointer` test passes
- [ ] All existing `migrate_identity_service` tests pass

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:

- `migrate_updates_persisted_container_identity_pointer` — proves persisted Container's `identity_instance_id` equals `new_identity_id`
- `migrate_updates_manifest_identity_pointer` — regression guard for manifest embed (existing)
- `migrate_adds_new_and_removes_old_from_container_members` — regression guard for membership (existing)
- `cross_store_roundtrip` — confirms purpose record and members survive `copy_repository`; does **not** assert `container.identity_instance_id` — that gap is closed by `migrate_updates_persisted_container_identity_pointer` (existing, partial coverage)

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm `migrate_updates_persisted_container_identity_pointer` exists in the test block and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Mark task checkboxes `[x]`.
5. Commit:

```bash
git commit -m "fix(repository): update persisted Container identityInstanceId in migrate_identity (#462)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] `get_container(store, root_container_id).identity_instance_id == Some(new_identity_id)` after migration (verified by new test)
- [ ] `manifest.container.identity_instance_id == Some(new_identity_id)` after migration (verified by existing test)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- `store.load_container(&root_container_id)` returns the post-mutation state after `add_container_member`/`remove_container_member`. This is safe because all three store implementations (`JsonStore`, `MemoryStore`, `FileStore`) update in-memory state immediately on each `save_container` call; `JsonStore` defers only the disk flush to `commit_batch`, not the in-memory update. Therefore `load_container` inside the closure reads the state written by the prior member mutations, and the subsequent `save_container` preserves those member changes while adding `identity_instance_id` — no overwrite hazard.
- The MemoryStore batch mode commits all pending writes atomically, including the new `save_container` call for the `identity_instance_id` update.
- No changes to CLI, payload schema, or bindings are needed; this is a pure service-layer bug fix.
