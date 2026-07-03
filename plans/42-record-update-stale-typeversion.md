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
- Update `update_record` to write `effective_type_version`, `type_name`, and `type_namespace` back to the stored record
- Update all three callers: CLI handler (`record.rs:213`), WASM bindings (`lib.rs:133`), extension service (`extension_service.rs:144`)
- Update all existing test call sites of `update_record` in `record_store.rs` (lines 1977, 1998, 2606, 2647, 2761, 2914, 2935, 2965) to pass `UpdateRecordInput`
- Add regression tests for the stale-typeVersion bug, fallback path, and invalid incoming version

**Out of scope:**
- Changes to `srs-core` types
- Changes to payload structs or JSON schema files
- Changes to CLI command flags or output format
- Harmonizing `create_record` to a typed input struct (it remains on its pre-ADR-010 raw-parameter signature; harmonizing it is a separate issue)

---

## Phases

### Phase 1: Fix `update_record` service in `srs-repository`

**Goal:** `update_record` accepts an `UpdateRecordInput` struct, resolves the type against the effective version, writes all version-derived fields back, and all existing tests pass with the new calling convention.

**Agent:** Repository Service Worker

#### Tasks

- [x] **Baseline check:** run `cargo test -p srs-repository` before any changes and confirm it passes. Record the passing state as the baseline.
- [x] Add `UpdateRecordInput` struct to `crates/srs-repository/src/record_store.rs` (near the existing `CreateRecordInput` struct):
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
- [x] Change `update_record` signature from the 5-parameter form to:
  ```rust
  pub fn update_record(
      store: &dyn RepositoryStore,
      instance_id: &str,
      input: UpdateRecordInput,
  ) -> Result<Record, RepositoryError>
  ```
- [x] Inside `update_record`, compute `effective_type_version`:
  ```rust
  let effective_type_version = input.type_version.unwrap_or(record.type_version);
  ```
- [x] Resolve `record_type` using `effective_type_version` and emit `RepositoryError::TypeVersionNotFound` (the variant used by `create_record_successor` for the identical error) when resolution fails:
  ```rust
  let record_type = package
      .resolve_type(&record.type_id, effective_type_version)
      .ok_or_else(|| RepositoryError::TypeVersionNotFound {
          type_id: record.type_id.clone(),
          version: effective_type_version,
      })?;
  ```
- [x] In the constructed `updated_record`, write `type_version: effective_type_version`, `type_name: record_type.name.clone()`, and `type_namespace: record_type.namespace.clone()` (these may change when the caller specifies a new version).
- [x] Handle `group_values` from `UpdateRecordInput` (None = preserve stored value, Some(vec) = replace/clear):
  ```rust
  let new_group_values = match input.group_values {
      Some(gv) => Some(gv),
      None => record.group_values,
  };
  ```
- [x] Update all existing test call sites of `update_record` in `record_store.rs` to construct and pass an `UpdateRecordInput`. Exact lines and their conversions:
  - **Line 1977:** `update_record(&store, &instance_id, updated_values, None, None)` → `update_record(&store, &instance_id, UpdateRecordInput { field_values: updated_values, group_values: None, tags: None, type_version: None })`
  - **Line 1998:** `update_record(&store, &instance_id, invalid_values, None, None)` → `update_record(&store, &instance_id, UpdateRecordInput { field_values: invalid_values, group_values: None, tags: None, type_version: None })`
  - **Line 2606:** `update_record(&store, &id, new_fv, Some(new_gv), None)` → `update_record(&store, &id, UpdateRecordInput { field_values: new_fv, group_values: Some(new_gv), tags: None, type_version: None })`
  - **Line 2647:** `update_record(&store, &id, new_fv, None, None)` → `update_record(&store, &id, UpdateRecordInput { field_values: new_fv, group_values: None, tags: None, type_version: None })`
  - **Line 2761:** `update_record(&store, &id, new_fv, None, None)` → `update_record(&store, &id, UpdateRecordInput { field_values: new_fv, group_values: None, tags: None, type_version: None })`
  - **Line 2914:** `update_record(&store, &id, fv, None, None)` → `update_record(&store, &id, UpdateRecordInput { field_values: fv, group_values: None, tags: None, type_version: None })`
  - **Line 2935:** `update_record(&store, &id, fv, None, Some(vec![]))` → `update_record(&store, &id, UpdateRecordInput { field_values: fv, group_values: None, tags: Some(vec![]), type_version: None })`
  - **Line 2965:** multi-line call `update_record(&store, &id, fv, None, Some(vec!["new-tag-1"..., "new-tag-2"...]))` → `update_record(&store, &id, UpdateRecordInput { field_values: fv, group_values: None, tags: Some(vec!["new-tag-1".to_string(), "new-tag-2".to_string()]), type_version: None })`
- [x] Add test `record_update_allows_type_version_migration` in the `#[cfg(test)]` module of `record_store.rs`:
  - Create a type at version 1, create a record using version 1
  - Simulate a package upgrade: remove version 1, add version 2 with the same type_id but a different `name`/`namespace` to prove those fields also update
  - Call `update_record` with `UpdateRecordInput { type_version: Some(2), field_values: ..., ..Default::default() }`
  - Assert `record.type_version == 2`, `record.type_name == <v2 name>`, `record.type_namespace == <v2 namespace>`
- [x] Add test `record_update_preserves_version_when_not_specified`:
  - Create a type at version 1, create a record using version 1
  - Call `update_record` with `UpdateRecordInput { type_version: None, ... }`
  - Assert saved record still has `type_version: 1`
- [x] Add test `record_update_fails_on_invalid_incoming_version`:
  - Create a type at version 1, create a record using version 1
  - Call `update_record` with `UpdateRecordInput { type_version: Some(99), ... }`
  - Assert a `RepositoryError::TypeVersionNotFound` error is returned
- [x] Add cross-store roundtrip test `record_update_type_version_migration_roundtrip_stores`:
  - Create a type at version 1, create a record in `MemoryStore`
  - Call `update_record` with `type_version: Some(2)` on `MemoryStore`
  - Serialize/write the repository to a `tempfile::TempDir`-backed `FileStore` (use the existing `copy_repository` helper or equivalent)
  - Reload the record from `FileStore`
  - Assert `type_version`, `type_name`, and `type_namespace` are identical across both stores

#### Acceptance Criteria

- [ ] `UpdateRecordInput` struct compiles and is exported from `crates/srs-repository/src/record_store.rs`
- [ ] `update_record` function signature uses `UpdateRecordInput` as its third parameter
- [ ] All 7 existing test call sites (lines 1977, 1998, 2606, 2647, 2761, 2914, 2935, 2965) updated to `UpdateRecordInput`
- [ ] `record_update_allows_type_version_migration` test passes and asserts `type_version`, `type_name`, and `type_namespace`
- [ ] `record_update_preserves_version_when_not_specified` test passes
- [ ] `record_update_fails_on_invalid_incoming_version` test passes with `TypeVersionNotFound` error
- [ ] `record_update_type_version_migration_roundtrip_stores` test passes
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository record_update
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `record_update_allows_type_version_migration` — proves the stale-typeVersion bug is fixed; also verifies `type_name`/`type_namespace` update
- `record_update_preserves_version_when_not_specified` — proves fallback to stored version works
- `record_update_fails_on_invalid_incoming_version` — proves invalid incoming version is rejected with `TypeVersionNotFound`
- `record_update_type_version_migration_roundtrip_stores` — proves updated fields survive JSON serialization round-trip (CLAUDE.md Storage Boundary Rule)

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

**Goal:** All three callers construct and pass an `UpdateRecordInput` struct; the workspace compiles with no errors.

**Agent:** CLI Worker (srs-cli), Bindings Worker (srs-bindings), Repository Service Worker (extension_service.rs)

#### Tasks

**`crates/srs-cli/src/commands/record.rs`**

- [ ] Update the import at line 14 in `record.rs` to replace `CreateRecordInput` (used for update) with `UpdateRecordInput`:
  ```rust
  use srs_repository::record_store::{..., UpdateRecordInput, update_record, ...};
  ```
- [ ] In `cmd_record_update` at line 213, change the stdin deserialization target from `CreateRecordInput` to `UpdateRecordInput`:
  ```rust
  let input: UpdateRecordInput = match serde_json::from_str(&stdin) { ... };
  ```
  (Note: the current handler at line 213 uses `CreateRecordInput` as a stand-in for the update input — `UpdateRecordInput` is a new struct with the same `field_values`, `group_values`, `tags` fields plus the new `type_version` field.)
- [ ] Call `update_record(store, &id, input)` — pass the struct directly, no deconstruction.
- [ ] Confirm the handler stays ≤15 lines (ADR-010 handler pattern).

**`crates/srs-bindings/src/lib.rs`**

- [ ] Locate `UpdateRecordBindingInput` at `crates/srs-bindings/src/lib.rs:514`. Add `type_version` field with both `#[serde(default)]` (matching the pattern of `tags` at line 521) to ensure backward compatibility for existing callers that omit `typeVersion`:
  ```rust
  #[serde(default)]
  pub type_version: Option<u32>,
  ```
- [ ] In the `update_record` WASM method at line 133, construct `UpdateRecordInput` from `UpdateRecordBindingInput`. The `deserialize_optional_optional` function at line 528 handles the double-wrap for `group_values`; flatten it when constructing the service input:
  ```rust
  let svc_input = record_store::UpdateRecordInput {
      field_values: input.field_values,
      group_values: input.group_values.and_then(|x| x),  // Option<Option<Vec<...>>> → Option<Vec<...>>
      tags: input.tags,
      type_version: input.type_version,
  };
  let record = record_store::update_record(&self.store, instance_id, svc_input).map_err(js_err)?;
  ```

**`crates/srs-repository/src/extension_service.rs`**

- [ ] At line 144 of `extension_service.rs`, replace:
  ```rust
  let record = update_record(store, id, field_values, None, None)?;
  ```
  with:
  ```rust
  let record = update_record(store, id, UpdateRecordInput {
      field_values,
      group_values: None,
      tags: None,
      type_version: None,
  })?;
  ```
- [ ] Add `UpdateRecordInput` to the `use crate::record_store::` import near line 26 of `extension_service.rs`.

#### Acceptance Criteria

- [ ] `cargo build` succeeds with no errors across the workspace
- [ ] `cargo test -p srs-cli` passes
- [ ] `cargo test -p srs-bindings` passes
- [ ] `cargo test -p srs-repository` continues to pass
- [ ] `cmd_record_update` handler in `record.rs` is ≤15 lines (ADR-010 compliance)
- [ ] `UpdateRecordBindingInput.type_version` carries `#[serde(default)]`
- [ ] `cargo clippy -- -D warnings` passes for the full workspace

#### Testing

```bash
cargo build
cargo test -p srs-cli
cargo test -p srs-bindings
cargo clippy -- -D warnings
```

Specific tests:
- CLI integration test for `cmd_record_update` (existing) — proves CLI still routes correctly
- WASM binding smoke tests (existing in `srs-bindings`) — proves binding accepts `typeVersion` field
- Existing WASM binding test that omits `typeVersion` from JSON — proves backward compatibility with `#[serde(default)]`

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
- [ ] Confirm `UpdateRecordInput` is `pub` and re-exported from `srs-repository::record_store`

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
- [ ] `record_update_allows_type_version_migration` test exists and passes (asserts `type_version`, `type_name`, `type_namespace`)
- [ ] `record_update_preserves_version_when_not_specified` test exists and passes
- [ ] `record_update_type_version_migration_roundtrip_stores` test exists and passes
- [ ] All three callers updated: `record.rs`, `lib.rs`, `extension_service.rs`
- [ ] All 7 existing test call sites in `record_store.rs` updated to `UpdateRecordInput`

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
- `deserialize_optional_optional` already exists at `crates/srs-bindings/src/lib.rs:528` — confirmed; no new deserializer needed.
