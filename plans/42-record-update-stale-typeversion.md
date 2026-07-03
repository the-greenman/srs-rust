# Plan: Fix `record update` Stale typeVersion Validation (#42)

## Summary

`srs record update` fails when the existing record's stored `typeVersion` no longer exists in the package (e.g. after a type version bump). The service resolves the type against the *stored* version rather than the version the caller is submitting. This plan fixes the service to use the incoming `type_version` (falling back to the stored one only when not specified), and refactors the function signature to a typed input struct per ADR-010.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | Phase 1 |
| CLI Worker | Phase 2 |
| Bindings Worker | Phase 2 |
| Verification Agent | Phase 3 |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan implements existing accepted ADRs.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions use typed input structs; all validation in service layer | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Named payload structs in `payload.rs`; golden schema files enforced by CI | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. `RecordPayload` is unchanged. No `payload.rs` modification required. No `cargo run --bin generate-schemas` needed. `cargo test --test payload_contracts` will pass without regeneration.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- Add `UpdateRecordInput` typed struct to `crates/srs-repository/src/record_store.rs`
- Fix `update_record` to use `input.type_version.unwrap_or(record.type_version)` as the effective type version
- Update `update_record` to write `effective_type_version` back to the stored record
- Update all three callers: CLI handler, WASM bindings, extension service
- Add regression tests for the stale-typeVersion bug and the fallback path

**Out of scope:**
- Changes to `srs-core` types
- Changes to payload structs or JSON schema files
- Changes to CLI command flags or output format
- Any other service function refactors

---

## Phases

### Phase 1: Fix `update_record` service in `srs-repository`

**Goal:** `update_record` accepts an `UpdateRecordInput` struct and resolves the type against the effective version (incoming `type_version` if provided, stored version otherwise).

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `UpdateRecordInput` struct to `crates/srs-repository/src/record_store.rs`:
  ```rust
  #[derive(Debug, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct UpdateRecordInput {
      pub field_values: Vec<FieldValue>,
      #[serde(default)]
      pub group_values: Option<Vec<srs_core::types::record::FieldGroupValue>>,
      #[serde(default)]
      pub tags: Option<Vec<String>>,
      #[serde(default)]
      pub type_version: Option<u32>,
  }
  ```
- [ ] Change `update_record` signature from:
  ```rust
  pub fn update_record(
      store: &dyn RepositoryStore,
      instance_id: &str,
      field_values: Vec<FieldValue>,
      group_values: Option<Option<Vec<srs_core::types::record::FieldGroupValue>>>,
      tags: Option<Vec<String>>,
  ) -> Result<Record, RepositoryError>
  ```
  to:
  ```rust
  pub fn update_record(
      store: &dyn RepositoryStore,
      instance_id: &str,
      input: UpdateRecordInput,
  ) -> Result<Record, RepositoryError>
  ```
- [ ] Inside `update_record`, compute `effective_type_version`:
  ```rust
  let effective_type_version = input.type_version.unwrap_or(record.type_version);
  ```
- [ ] Resolve `record_type` using `effective_type_version` (not `record.type_version`):
  ```rust
  let record_type = package
      .resolve_type(&record.type_id, effective_type_version)
      .ok_or_else(|| RepositoryError::TypeNotFound {
          type_id: record.type_id.clone(),
          version: effective_type_version,
      })?;
  ```
- [ ] In the constructed `updated_record`, write `type_version: effective_type_version` (not `record.type_version`). Also write `type_name` and `type_namespace` from `record_type` (these may change when upgrading to a new version).
- [ ] Handle `group_values` from `UpdateRecordInput` (None = preserve, Some([...]) = replace/clear):
  ```rust
  let new_group_values = match input.group_values {
      Some(gv) => Some(gv),
      None => record.group_values,
  };
  ```
  (The double-wrap `Option<Option<...>>` was an artifact of the old calling convention; `UpdateRecordInput` uses a single `Option<Vec<...>>` where `None` means preserve and `Some(vec![])` means clear.)
- [ ] Add test `record_update_allows_type_version_migration` in the test module of `record_store.rs`:
  - Create a type at version 1, create a record using that version
  - Remove version 1 from the package and add version 2 (simulate a bump)
  - Call `update_record` with `type_version: Some(2)` — must succeed
  - Confirm the saved record has `type_version: 2`
- [ ] Add test `record_update_preserves_version_when_not_specified` in the test module:
  - Create a type at version 1, create a record using version 1
  - Call `update_record` with `type_version: None`
  - Confirm the saved record still has `type_version: 1`
- [ ] Add test `record_update_fails_on_invalid_incoming_version`:
  - Create a type at version 1, create a record using version 1
  - Call `update_record` with `type_version: Some(99)` (nonexistent)
  - Confirm a `TypeNotFound` error is returned

#### Acceptance Criteria

- [ ] `update_record` compiles with the new `UpdateRecordInput` parameter
- [ ] `record_update_allows_type_version_migration` test passes
- [ ] `record_update_preserves_version_when_not_specified` test passes
- [ ] `record_update_fails_on_invalid_incoming_version` test passes
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository record_update
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `record_update_allows_type_version_migration` — proves the stale-typeVersion bug is fixed
- `record_update_preserves_version_when_not_specified` — proves fallback to stored version works
- `record_update_fails_on_invalid_incoming_version` — proves invalid incoming version is rejected

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit -m "fix(record-update): validate incoming typeVersion not stored (#42)"
```

Do not start Phase 2 until the milestone gate passes and the plan is updated.

---

### Phase 2: Update callers of `update_record`

**Goal:** All three callers of `update_record` construct and pass an `UpdateRecordInput` struct; the workspace compiles with no errors.

**Agent:** CLI Worker (srs-cli), Bindings Worker (srs-bindings), Repository Service Worker (extension_service.rs)

#### Tasks

**`crates/srs-cli/src/commands/record.rs`**

- [ ] Update the import in `record.rs` to include `UpdateRecordInput` (and remove the old individual-param import if separate):
  ```rust
  use srs_repository::record_store::{..., UpdateRecordInput, update_record, ...};
  ```
- [ ] In `cmd_record_update`, change the stdin deserialization target from `CreateRecordInput` to `UpdateRecordInput`:
  ```rust
  let input: UpdateRecordInput = match serde_json::from_str(&stdin) { ... };
  ```
- [ ] Call `update_record(store, &id, input)` — pass the struct directly, no deconstruction.
- [ ] Confirm the handler stays ≤15 lines (ADR-010 handler pattern).

**`crates/srs-bindings/src/lib.rs`**

- [ ] Add `type_version: Option<u32>` field to the existing `UpdateRecordBindingInput` struct in `lib.rs`.
- [ ] In the `update_record` WASM method, construct `UpdateRecordInput` from `UpdateRecordBindingInput`:
  ```rust
  let svc_input = record_store::UpdateRecordInput {
      field_values: input.field_values,
      group_values: input.group_values.and_then(|x| x),  // flatten double-wrap: Option<Option<Vec<...>>> → Option<Vec<...>>
      tags: input.tags,
      type_version: input.type_version,
  };
  let record = record_store::update_record(&self.store, instance_id, svc_input).map_err(js_err)?;
  ```
  (The double-wrap `Option<Option<Vec<...>>>` in `UpdateRecordBindingInput.group_values` is preserved for backward-compatible JSON deserialization via `deserialize_optional_optional`; `.and_then(|x| x)` flattens it to `Option<Vec<...>>` for the service.)

**`crates/srs-repository/src/extension_service.rs`**

- [ ] Find the call to `update_record(store, id, field_values, None, None)` (approximately line 144) and replace it with:
  ```rust
  update_record(store, id, UpdateRecordInput {
      field_values,
      group_values: None,
      tags: None,
      type_version: None,
  })?;
  ```
- [ ] Add `UpdateRecordInput` to the import in `extension_service.rs`:
  ```rust
  use crate::record_store::{..., UpdateRecordInput, update_record};
  ```

#### Acceptance Criteria

- [ ] `cargo build` succeeds with no errors across the workspace
- [ ] `cargo test -p srs-cli` passes
- [ ] `cargo test -p srs-bindings` passes
- [ ] `cargo test -p srs-repository` continues to pass
- [ ] `cmd_record_update` in `record.rs` is ≤15 lines (ADR-010 compliance)
- [ ] `cargo clippy -- -D warnings` passes for the full workspace

#### Testing

```bash
cargo build
cargo test -p srs-cli
cargo test -p srs-bindings
cargo clippy -- -D warnings
```

Specific tests:
- `cmd_record_update` handler integration test (existing) — proves CLI still routes correctly
- WASM binding smoke tests (existing in `srs-bindings`) — proves binding accepts `typeVersion` field

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run:

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

3. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:

```bash
git commit -m "fix(record-update): update all callers to UpdateRecordInput (#42)"
```

---

### Phase 3: Verification

**Goal:** All tests pass, no regressions, no lint errors, no architectural boundary violations.

**Agent:** Verification Agent

#### Tasks

- [ ] Run full test suite: `cargo test`
- [ ] Run clippy: `cargo clippy -- -D warnings`
- [ ] Run payload contract tests: `cargo test --test payload_contracts`
- [ ] Run schema sync check: `bash scripts/check-schema-sync.sh`
- [ ] Audit crate boundary: confirm no path strings in service logic, no business logic in CLI handler, `srs-core` unchanged
- [ ] Confirm `UpdateRecordInput` is exported from `srs-repository::record_store` (public, not hidden behind `pub(crate)`)

#### Acceptance Criteria

- [ ] `cargo test` passes with zero failures
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] No business logic outside `srs-repository`
- [ ] No path strings outside `FileStore`

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
bash scripts/check-schema-sync.sh
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (payload structs unchanged)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `record_update_allows_type_version_migration` test exists and passes
- [ ] `record_update_preserves_version_when_not_specified` test exists and passes
- [ ] All three callers updated: `record.rs`, `lib.rs`, `extension_service.rs`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phases 1 and 2 complete.

## Assumptions

- The `Package::resolve_type(type_id, version)` method exists and correctly returns `None` for unknown versions (confirmed from reading the code).
- `MemoryStore` supports multi-version types sufficient to write the regression tests without `FileStore`.
- The existing `record_update_validates_against_type` test in `record_store.rs` continues to pass unchanged (it uses an inline type that exists in the package at the stored version).
