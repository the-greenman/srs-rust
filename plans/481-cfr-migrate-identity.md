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

- [x] In `crates/srs-repository/src/core_purpose.rs`: add `pub(crate) struct PurposeRecordSpec` and `pub(crate) fn purpose_record_spec(statement: &str, title: Option<&str>) -> PurposeRecordSpec` that returns type_id, type_version, and field_values by looking up the embedded core bundle (ADR-025). `build_purpose_record` now delegates to `purpose_record_spec`, eliminating the duplicate lookup.
- [x] In `crates/srs-repository/src/migrate_identity_service.rs`:
  - Remove imports: `write_new_record`, `upsert_record_index_entry` from `crate::record_store`
  - Add import: `crate::record_store::create_record`
  - Both None-branch and main-branch call `core_purpose::purpose_record_spec(...)` to obtain a `PurposeRecordSpec`, then pass `spec.type_id`, `spec.type_version`, `spec.field_values` to `create_record`
  - Batch closures return `Result<String, RepositoryError>` (the new instance_id)
  - After `create_record`, manifest is reloaded to capture the index entry it wrote, then written a second time with container changes (two manifest writes per ADR-021 batch; atomic for JsonStore, sequential-best-effort for FileStore/MemoryStore per ADR-024)
- [x] In `docs/adr/002-tier2-generic-record-operations.md`: update Known Deviations to note the `migrate_identity_service` deviation is resolved by this commit; `repository_lifecycle.rs#scaffold_purpose_record` remains as the only open deviation.

#### Acceptance Criteria

- [x] `migrate_identity_service.rs` has no `write_new_record` or `upsert_record_index_entry` imports or calls
- [x] `migrate_identity_service.rs` imports and calls `crate::record_store::create_record`
- [x] All existing tests in `migrate_identity_service.rs#tests` pass unchanged
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
