# Plan: WASM atomic create_record_in_container binding

> **Issue:** srs-rust#315

## Summary

`srs-web` currently creates a decision record by making two sequential WASM calls: `createRecord` then `addContainerMember`. If the second call fails (network drop, race condition, or caller omission), the record exists but is not registered in its container — a silent data-state divergence that violates the architectural invariant that this constraint belongs in the Rust layer (ADR-001, ADR-010). This plan adds a single-call WASM binding `create_record_in_container` that eliminates the two-step caller error by creating a Tier-2 record and adding it to a container in one service call, by adding a typed service function to `srs-repository` and a one-service-call binding in `srs-bindings`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | `crates/srs-repository/src/record_store.rs` |
| Bindings Worker | `crates/srs-bindings/src/lib.rs` |
| Verification Agent | read-only; test runs + audit |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Atomic business logic stays in `srs-repository`, not in bindings | accepted |
| [ADR-002](../docs/adr/002-tier-2-generic-operations.md) | Library has no knowledge of concrete record types; function is named `create_record_in_container` (general), not `create_decision` (type-specific) | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | New service function takes typed input struct, returns typed result struct; all validation inside | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Binding is a thin wrapper: parse input → one service call → serialize output | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | Each write binding calls exactly one `srs-repository` service function | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write.md) | Not applicable — two sequential writes are acceptable here; both occur in a `JsonStore` (WASM, in-memory), so no disk-flush ordering concern arises. Residual partial-write risk (record created, `add_member` fails) matches the existing risk in `create_record_in_context` (ADR-007). | n/a |

_No new ADRs required. This plan implements existing ADRs without introducing new architectural constraints._

**Design decision (resolved without human checkpoint):** The binding is named `create_record_in_container` (general, any type) rather than `create_decision` (type-specific). Rationale: the invariant "create record + add to container atomically" applies to any container-governed type, not only decisions. A general binding matches the existing pattern (`create_record`, `graduate_note`), and ADR-002 prohibits type-specific knowledge in the library. The CLI already covers this use case via `srs record create --container` (through `create_record_in_context` with namespace/name resolution); a new CLI command by type_id would be redundant — out of scope below.

---

## Contracts

### CLI output contract (ADR-011)

This plan adds no new CLI command and changes no existing CLI command output shape. No payload struct changes. No schema regeneration needed.

`cargo test --test payload_contracts` must still pass (no change expected).

### Entity schema sync (check-schema-sync.sh)

This plan makes no changes to `srs/docs/schema/2.0/`. No schema sync needed.

---

## Scope

- Add `CreateRecordInContainerInput` struct to `crates/srs-repository/src/record_store.rs`.
- Add `create_record_in_container` service function to `crates/srs-repository/src/record_store.rs` returning the existing `CreateRecordResult` struct (already defined at record_store.rs:~512 — do **not** add a new result type).
- Add `create_record_in_container` method to `SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- Tests: at least one unit test in `record_store.rs` (memory store, cross-store roundtrip), and one integration test in `srs-bindings/src/lib.rs` proving the method succeeds and the output serialises.

**Out of scope:**

- A new dedicated CLI command `srs record create-in-container` by type_id — the existing `srs record create --container` (via `create_record_in_context`) already covers the CLI surface. A future plan can wire the CLI to the new service if needed.
- Any changes to `srs-web` — this plan only adds the Rust/WASM surface; the srs-web migration is tracked in srs-web#103.
- Any changes to `srs-vscode`.
- WASM package rebuild (`wasm-pack build`) — not needed in this cloud session; CI builds the package.

---

## Phases

### Phase 1: Service function

**Goal:** `create_record_in_container` exists in `srs-repository` as a fully tested, typed service function that creates a Tier-2 record and adds it to a container in one call (caller-omission atomic).

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `CreateRecordInContainerInput` struct to `crates/srs-repository/src/record_store.rs`:

  ```rust
  #[derive(Debug, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct CreateRecordInContainerInput {
      pub container_id: String,
      pub type_id: String,
      pub type_version: u32,
      pub field_values: Vec<FieldValue>,
      #[serde(default)]
      pub group_values: Option<Vec<srs_core::types::record::FieldGroupValue>>,
      #[serde(default)]
      pub tags: Option<Vec<String>>,
  }
  ```

- [ ] Add `create_record_in_container` service function to `crates/srs-repository/src/record_store.rs`. The function returns the **existing** `CreateRecordResult` (already defined in this file — do **not** introduce a new result type):

  ```rust
  /// Create a Tier-2 record and add it to a container in one call (caller-omission atomic).
  ///
  /// Steps (in order):
  ///   1. Validate the container exists — returns `ContainerNotFound` if absent (pre-write).
  ///   2. Create the record via `create_record_at_dir` (uses existing `DEFAULT_RECORD_DIR` constant).
  ///   3. Add the new record to the container's `memberInstanceIds` via `container_service::add_member`.
  ///
  /// Residual risk: if step 3 fails after step 2 succeeds, the record exists but is not a member.
  /// This matches the existing partial-write risk in `create_record_in_context` (see ADR-007).
  pub fn create_record_in_container(
      store: &dyn RepositoryStore,
      input: CreateRecordInContainerInput,
  ) -> Result<CreateRecordResult, RepositoryError> {
      container_service::get_container(store, &input.container_id)?;

      let record = create_record_at_dir(
          store,
          &input.type_id,
          input.type_version,
          input.field_values,
          input.group_values,
          input.tags,
          DEFAULT_RECORD_DIR,
      )?;

      container_service::add_member(store, &input.container_id, &record.instance_id)?;

      Ok(CreateRecordResult { record })
  }
  ```

- [ ] Add unit tests for `create_record_in_container` inside the `#[cfg(test)]` block in `record_store.rs`:
  - `create_record_in_container_adds_to_membership` — happy path using `MemoryStore`, verifies record is created and is a member.
  - `create_record_in_container_missing_container_fails` — missing container returns `ContainerNotFound` before any record is written (manifest index length unchanged).
  - `create_record_in_container_invalid_type_fails` — unknown type_id returns `TypeNotFound` before any record is written.
  - `create_record_in_container_roundtrip_stores` — create via `MemoryStore`, copy to `FileStore` via `copy_repository`, re-load and verify membership and record data are identical (cross-store roundtrip required by CLAUDE.md Storage Boundary Rules).

#### Acceptance Criteria

- [ ] `create_record_in_container` is `pub` and callable from other crates.
- [ ] Function returns `CreateRecordResult` (the existing struct) — no new result type introduced.
- [ ] Missing container returns `RepositoryError::ContainerNotFound` and leaves the manifest index unchanged.
- [ ] Invalid type_id returns `RepositoryError::TypeNotFound` and leaves the manifest index unchanged.
- [ ] On success, `result.record.instance_id` appears in `container.member_instance_ids`.
- [ ] Cross-store roundtrip test passes.

#### Testing

```bash
cargo test -p srs-repository create_record_in_container
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `create_record_in_container_adds_to_membership` — proves happy path writes record and membership
- `create_record_in_container_missing_container_fails` — proves container validation is pre-write
- `create_record_in_container_invalid_type_fails` — proves type validation is pre-write
- `create_record_in_container_roundtrip_stores` — proves memory → file roundtrip consistency

#### Milestone gate

1. All acceptance criteria above checked.
2. All four tests listed above exist and pass.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Mark task checkboxes `[x]`.
5. Commit:

```bash
git commit -m "feat(record-store): add create_record_in_container service (#315)"
```

---

### Phase 2: WASM binding

**Goal:** `SrsRepository.create_record_in_container` is callable from JavaScript, calls exactly one service function, and returns the created `Record` as a JS value.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add `create_record_in_container` method to `SrsRepository` in `crates/srs-bindings/src/lib.rs`. The method reuses the existing `CreateRecordBindingInput` struct (already defined in this file at ~lib.rs:655 — it has `field_values`, `group_values`, `tags`). Use the qualified path `record_store::CreateRecordInContainerInput` in the method body; **do not** add `CreateRecordInContainerInput` to the `use` import block (avoids clippy unused-import warning if the type is only used once via qualified path).

  ```rust
  /// Create a Tier-2 record and add it to a container in one call.
  ///
  /// `container_id` is the UUID of the container to add the record to.
  /// `type_id` is the UUID of the type; `type_version` is the version number.
  /// `input_json` is a JSON object with `fieldValues` (required), `groupValues` (optional),
  /// and `tags` (optional) — the same shape as `create_record`.
  ///
  /// Returns the created `Record` as a JS value.
  /// Returns a JS error if the container does not exist, the type is not found,
  /// or field validation fails.
  pub fn create_record_in_container(
      &self,
      container_id: &str,
      type_id: &str,
      type_version: u32,
      input_json: &str,
  ) -> Result<JsValue, JsValue> {
      let input: CreateRecordBindingInput =
          serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
      let result = record_store::create_record_in_container(
          &self.store,
          record_store::CreateRecordInContainerInput {
              container_id: container_id.to_string(),
              type_id: type_id.to_string(),
              type_version,
              field_values: input.field_values,
              group_values: input.group_values,
              tags: input.tags,
          },
      )
      .map_err(js_err)?;
      to_js(&result.record)
  }
  ```

  Note: `CreateRecordBindingInput` (the deserialization struct for `input_json`) and `record_store::CreateRecordInContainerInput` (the service input struct) are two distinct types serving different roles; the binding maps between them.

- [ ] Add a unit test `create_record_in_container_result_serialises` in the `#[cfg(test)]` block at the bottom of `lib.rs`. Add a `srsj_with_container_and_type()` helper alongside existing helpers (e.g. `srsj_with_note_and_type`). The helper must embed a minimal valid `.srsj` string containing:
  - one type definition with a known `type_id` UUID and at least one required field
  - one container with a known `container_id` UUID and an empty `memberInstanceIds` array

  The test must:
  1. Load the repository from the helper string.
  2. Call `create_record_in_container(container_id, type_id, type_version, input_json)`.
  3. Deserialize the returned `JsValue` to a `serde_json::Value`.
  4. Assert `result["instanceId"]` is a non-empty string.
  5. Call `get_container(container_id)` and assert the `instanceId` appears in `memberInstanceIds`.

#### Acceptance Criteria

- [ ] `create_record_in_container` is `#[wasm_bindgen]` annotated (implicitly via the `#[wasm_bindgen] impl SrsRepository` block).
- [ ] The method calls exactly one service function (`record_store::create_record_in_container`) — no business logic in the binding.
- [ ] The binding returns `to_js(&result.record)` — a typed serialisation, no `json!({})` literal.
- [ ] Unit test passes and the returned JSON has `instanceId` as a non-empty string.
- [ ] `cargo clippy` passes with no warnings (no redundant imports, no unused variables).

#### Testing

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific test:
- `create_record_in_container_result_serialises` — proves output is parseable JSON with `instanceId` present and container membership updated

#### Milestone gate

1. All acceptance criteria above checked.
2. Test `create_record_in_container_result_serialises` exists and passes.
3. Run lint and tests:

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

4. Mark task checkboxes `[x]`.
5. Commit:

```bash
git commit -m "feat(bindings): expose create_record_in_container WASM binding (#315)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `create_record_in_container` is callable in the WASM binding surface
- [ ] Missing-container and invalid-type negative cases both return errors before any write

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `create_record_at_dir` is `pub(crate)` in `record_store.rs` and accessible from the new service function (same module).
- `DEFAULT_RECORD_DIR` is an existing constant in `record_store.rs` (at line ~34).
- `container_service::add_member` is accessible from `record_store.rs` (it is — `record_store` already calls `container_service::get_container` and `container_service::is_member`).
- `CreateRecordResult` is the existing result struct in `record_store.rs` (at line ~512) — the new function reuses it rather than introducing a parallel type.
- The `JsonStore`-based `MemoryStore` is the test double for cross-store roundtrip tests.
- No WASM package build step (`wasm-pack`) is required in CI for this PR — the package is rebuilt separately.
