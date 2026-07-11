# Plan: Route migrate_identity_service writes through create_record (CFR enforcement)

## Summary

`migrate_identity_service` writes purpose records via `write_new_record` directly, bypassing `create_record_at_dir` and therefore bypassing the CFR (cross-field rule) enforcement added in #437. This is ADR-002's known deviation: "must be replaced with `create_record()`". ADR-025 (implicit core package merge) has since made the `com.semanticops.core/purpose` type resolvable via `store.load_package()` in all store implementations, eliminating the blocker. This plan closes that deviation.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Repository Service Worker | (self) |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | Tier 2 record operations are generic; `migrate_identity_service` must write through `create_record()` — this plan resolves the known deviation | accepted |
| [ADR-025](../docs/adr/025-implicit-core-package-merge.md) | Core bundle is merged into every `load_package()` result, making `com.semanticops.core/purpose` resolvable without an explicit packageRef | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) | Two manifest writes inside a batch are safe — `create_record` writes manifest entry, outer code reloads and writes again with container updates | accepted |

No new ADRs required — this plan resolves a known deviation under existing ADRs.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. `migrate_identity` payload (`MigrateIdentityResult`) is unchanged.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files change.

---

## Scope

- Replace `write_new_record` in `migrate_identity_service.rs` (both None-branch and main branch) with `crate::record_store::create_record`
- Add `pub(crate) fn purpose_record_spec(statement: &str, title: Option<&str>) -> (String, u32, Vec<FieldValue>)` to `core_purpose.rs` so callers that route through `create_record` can get the type_id, type_version, and field_values without duplicating core-bundle lookup logic
- Update ADR-002 Known Deviations: mark this deviation resolved
- Existing tests must continue to pass; no test additions are required (the purpose type has no CFR rules, so CFR is already validated by the existing validate path)

**Out of scope:**
- `scaffold_purpose_record` in `repository_lifecycle.rs` still uses `write_new_record` — that is a separate issue (#481 only covers `migrate_identity_service`)
- Adding CFR rules to the purpose type itself (spec concern, not this plan)
- WASM binding changes (existing `srs-bindings` migrate_identity binding calls the same service, inherits the fix automatically)

---

## Phases

### Phase 1: Add purpose_record_spec helper and route migrate_identity through create_record

**Goal:** `migrate_identity_service` no longer imports or calls `write_new_record` or `upsert_record_index_entry`; all writes go through `crate::record_store::create_record`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/core_purpose.rs`: add `pub(crate) fn purpose_record_spec(statement: &str, title: Option<&str>) -> (String, u32, Vec<FieldValue>)` that returns `(purpose_type.id.clone(), purpose_type.version, field_values)` by looking up the core bundle — same lookup logic as `build_purpose_record` but returns the components needed for `create_record`, not a full `Record`.
- [ ] In `crates/srs-repository/src/migrate_identity_service.rs`:
  - Remove imports: `write_new_record`, `upsert_record_index_entry` from `crate::record_store`
  - Add import: `crate::record_store::create_record`
  - Signature of `crate::record_store::create_record`: `create_record(store, type_id: &str, type_version: u32, field_values: Vec<FieldValue>, group_values: Option<Vec<FieldGroupValue>>, tags: Option<Vec<String>>) -> Result<Record, RepositoryError>` — pass `None` for both `group_values` and `tags` since purpose records have neither.
  - **None-branch** (identity_instance_id is None, ~lines 78-139):
    - Delete: `let now = ...; let new_id = ...; let record = core_purpose::build_purpose_record(...);`
    - Before `store.begin_batch()`, call `let (type_id, type_version, field_values) = core_purpose::purpose_record_spec(statement, record_title);`
    - Inside the batch closure, change return type to `Result<String, RepositoryError>` (returns the new instance_id)
    - Replace `write_new_record` + `upsert_record_index_entry` + `write_manifest` block with this sequence:
      1. `let record = create_record(store, &type_id, type_version, field_values, None, None)?;`
      2. `let new_id = record.instance_id.clone();`
      3. `let mut manifest = store.load_manifest()?;` — reload to capture the record index entry that `create_record` already wrote internally (per ADR-021, `create_record` updates the manifest as part of its own write; we reload to base our container update on that state, then write manifest a second time with container changes)
      4. `if let Some(ref mut container) = manifest.container { container.identity_instance_id = Some(new_id.clone()); container.member_instance_ids.get_or_insert_with(Vec::new).push(new_id.clone()); }`
      5. `writer::write_manifest(store, &manifest)?;`
      6. `if let Some(ref container) = manifest.container { store.save_container(container)?; }` (unchanged)
      7. `Ok(new_id)`
    - In the batch match arm, extract `new_id` from `Ok(new_id)` and use it in `MigrateIdentityResult`
  - **Main-branch** (~lines 141-226):
    - Delete: `let new_id = ...; let now = ...; let record = core_purpose::build_purpose_record(...);`
    - Before `store.begin_batch()`, call `let (type_id, type_version, field_values) = core_purpose::purpose_record_spec(&statement, title.as_deref());`
    - Batch closure returns `Result<String, RepositoryError>` (the new_id)
    - Replace `write_new_record` + `upsert_record_index_entry` block with this sequence inside the closure:
      1. `let record = create_record(store, &type_id, type_version, field_values, None, None)?;`
      2. `let new_id = record.instance_id.clone();`
      3. `let mut manifest = store.load_manifest()?;` (reload — same ADR-021 rationale as None-branch)
      4. `if let Some(ref mut mc) = manifest.container { mc.identity_instance_id = Some(new_id.clone()); }`
      5. `writer::write_manifest(store, &manifest)?;`
      6. `container_service::add_container_member(store, &root_container_id, &new_id)?;` (unchanged)
      7. `container_service::remove_container_member(store, &root_container_id, &old_id)?;` (unchanged)
      8. Load persisted container, update its `identity_instance_id`, save container — retain the WASM nesting caveat comment from current lines 195-200 (do NOT use `container_service::update_container`; use `store.load_container` + `store.save_container` directly)
      9. `Ok(new_id)`
    - In the batch match arm, extract `new_id` from `Ok(new_id)` and use it in `MigrateIdentityResult`
- [ ] In `crates/srs-repository/src/docs/adr/002-tier2-generic-record-operations.md`: update Known Deviations to note the `migrate_identity_service` deviation is resolved by this commit; `repository_lifecycle.rs#scaffold_purpose_record` remains as the only open deviation.

#### Acceptance Criteria

- [ ] `migrate_identity_service.rs` has no `write_new_record` or `upsert_record_index_entry` imports or calls
- [ ] `migrate_identity_service.rs` imports and calls `crate::record_store::create_record`
- [ ] All existing tests in `migrate_identity_service.rs#tests` pass unchanged
- [ ] `cargo test -p srs-repository` passes
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository -- migrate_identity
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify (all pre-existing):
- `migrate_creates_purpose_record` — proves record is written with correct type and fields
- `migrate_adds_new_and_removes_old_from_container_members` — proves container membership is correct
- `migrate_updates_manifest_identity_pointer` — proves manifest is updated
- `migrate_updates_persisted_container_identity_pointer` — proves persisted container is updated
- `cross_store_roundtrip` — proves memory→json→file roundtrip works
- `migrate_from_container_adds_to_members` — proves None-branch container membership
- All other `migrate_*` tests in the module

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed above passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `migrate_identity_service` no longer calls `write_new_record` directly
- [ ] Writes through `create_record()` to inherit CFR enforcement

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- `MemoryStore::default()` returns a store whose `load_package()` includes the core bundle types (per ADR-025 and `load_package_memory_store_includes_core_types` test). Tests in `migrate_identity_service` use `MemoryStore::default()` and therefore `create_record` can resolve the purpose type.
- The purpose type has no CFR rules currently, so all existing tests pass with or without CFR enforcement. The fix is correct-by-construction: if CFR rules were added to the purpose type in future, they would now be enforced at migration time.
