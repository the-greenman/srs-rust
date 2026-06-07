# Plan: Fix `type schema` to include inherited fields via `effective_fields`

## Summary

`srs type schema <typeId>` currently iterates `record_type.fields` directly, which only includes the Type's own field assignments. When a Type carries `extendsTypeId`, all inherited fields are silently omitted from the emitted JSON Schema. `package.effective_fields(record_type)` already exists and walks the inheritance chain correctly — it is already used by `record_store.rs`, `validation.rs`, and `render_service.rs`. This plan wires `type_schema_service::type_schema` to use it, bringing schema projection in line with the rest of the system. No new architectural decisions are required.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements the existing pattern established in `record_store.rs` (using `store.load_package()` + `package.effective_fields()`) and is governed by ADR-009 (package boundary model) and ADR-010 (service boundary contract).

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | All package access goes through `RepositoryStore::load_package()`; field lookup uses `package.resolve_field()` | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions take typed input/output structs; no business logic in CLI handlers | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct change — output shape is unchanged | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. The `TypeSchemaPayload` struct and the `schemas/payload/type-schema.json` golden file are unchanged — the fix only changes which field assignments are projected, not the output shape. No schema regeneration needed.

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` entity schemas. No action required.

---

## Scope

- Modify `type_schema_service::type_schema` in `crates/srs-repository/src/type_schema_service.rs`:
  - Load the package once via `store.load_package()`.
  - Call `package.effective_fields(&record_type)` instead of iterating `record_type.fields` directly. `effective_fields` already returns assignments sorted by `order`, so the manual sort can be removed.
  - Replace per-field `get_field_by_id(store, ...)` calls with `package.resolve_field(&fa.field_id)` to avoid re-loading the package on every field lookup.
  - Propagate `effective_fields` errors (cycle, missing parent) as hard errors.
- Add one unit test in `type_schema_service.rs` covering a child type that inherits fields from a parent type, confirming both parent and child fields appear in the schema.

**Out of scope:**
- Changes to the CLI handler `cmd_type_schema` in `crates/srs-cli/src/commands/record_type.rs`.
- Changes to `TypeSchemaPayload` in `payload.rs` or the golden schema file.
- Changes to `get_field_by_id` in `package_service.rs` (it remains for other callers).
- Changes to `package.rs` or the `effective_fields` implementation.

---

## Phases

### Phase 1: Fix `type_schema_service::type_schema` and add test

**Goal:** `type schema` projects all effective fields (own + inherited) and a passing test proves it.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/type_schema_service.rs`, after resolving `record_type`, add:
  ```rust
  let package = store.load_package()?;
  let assignments = package.effective_fields(&record_type)?;
  ```
  Note: `store.load_package()` is called at most twice per `type_schema` invocation — once inside `get_type_by_id` / `get_type_by_id_latest` (which load the package to resolve the type), and once here for `effective_fields` + `resolve_field`. For `FileStore` this means two disk reads; for `MemoryStore` it is a cheap clone.
- [ ] Remove the existing `let mut assignments: Vec<&FieldAssignment> = record_type.fields.iter().collect(); assignments.sort_by_key(|fa| fa.order);` block (`effective_fields` already returns assignments sorted by `order`).
- [ ] Change the `for fa in assignments` loop to `for fa in &assignments`. Note: `assignments` is now `Vec<FieldAssignment>` (owned values returned by `effective_fields`); `fa` is `&FieldAssignment`. This is the same type as before via auto-deref — no other loop-body changes are needed for this reason alone.
- [ ] Inside the loop, replace:
  ```rust
  let field = match get_field_by_id(store, &fa.field_id)? {
      GetFieldResult::Found(field) => *field,
      GetFieldResult::NotFound => { ... continue; }
  };
  ```
  with:
  ```rust
  let field = match package.resolve_field(&fa.field_id) {
      Some(f) => f.clone(),
      None => {
          diagnostics.push(format!(
              "field assignment references unknown fieldId '{}'; skipped",
              fa.field_id
          ));
          continue;
      }
  };
  ```
  The replacement uses `f.clone()` (not `*field`) because `resolve_field` returns an owned `Field`. `type_schema_dangling_field_skipped` uses a literal `"missing-field-id"` string; `resolve_field` does a plain string comparison so that test continues to exercise the not-found path correctly without changes.
- [ ] Remove now-unused imports `get_field_by_id` and `GetFieldResult` from the `use` block at the top of `type_schema_service.rs` **only**. Do not touch `package_service.rs` — the CLI `field get` handler still imports and calls these symbols from there.
- [ ] Add `use crate::package::Package;` to the production `use` block at the top of `type_schema_service.rs` (outside `#[cfg(test)]`). The test block already imports it — this adds it for the production path.
- [ ] Add unit test `type_schema_includes_inherited_fields` in the `#[cfg(test)]` block of `type_schema_service.rs`:
  - Declare two UUID constants in the test (these are distinct from the existing `TID` const):
    ```rust
    const PARENT_TID: &str = "00000000-0000-4000-8000-000000000001";
    const CHILD_TID:  &str = "00000000-0000-4000-8000-000000000002";
    ```
  - Add a new test helper `store_with_types(fields: Vec<Field>, record_types: Vec<RecordType>) -> MemoryStore` alongside the existing `store_with`. Do not modify `store_with` — 8 existing tests use it. The new helper is identical to `store_with` but accepts `Vec<RecordType>` instead of a single `RecordType`.
  - Create a parent `RecordType` via `make_type(PARENT_TID, vec![assignment(&fid(1), 0, false)])`. Note: `make_type` sets `version: 1` by default; the child's `extends_type_version: Some(1)` must match this.
  - Create a child `RecordType` via `make_type(CHILD_TID, vec![assignment(&fid(2), 1, false)])`, then set `extends_type_id: Some(PARENT_TID.to_string())` and `extends_type_version: Some(1)` on it.
  - Build the store with `store_with_types(vec![field(&fid(1), "parent_field", ValueType::String), field(&fid(2), "child_field", ValueType::String)], vec![parent_type, child_type])`. Both types' fields must be in the single flat `fields` vec — `package.resolve_field` searches this list and will return `None` for any field not present here.
  - Call `type_schema(&store, TypeSchemaInput { type_id: CHILD_TID.to_string(), type_version: None })`.
  - Assert both `"parent_field"` and `"child_field"` appear as keys in `result.schema["properties"]`.
  - Assert `result.diagnostics` is empty.
  - Note on errors: `effective_fields` errors propagate via `?` as `Err(RepositoryError::TypeInheritanceCycle { type_id }` (inheritance cycle) or `Err(RepositoryError::TypeNotFound { type_id, version })` (missing parent — the type_id in this case names the parent, not the root type). No error mapping is needed; `?` handles propagation.

#### Acceptance Criteria

- [ ] `type schema` on a child type that extends a parent type returns all fields (own + inherited) in the schema.
- [ ] `type schema` on a non-inheriting type is unchanged (existing tests pass).
- [ ] `get_field_by_id` / `GetFieldResult` are no longer imported in `type_schema_service.rs`.
- [ ] `store.load_package()` is called at most twice per `type_schema` invocation (once inside the type-resolution helper, once for `effective_fields` + `resolve_field`).
- [ ] Hard errors from `effective_fields` (cycle, missing parent) propagate as `Err(RepositoryError::...)`.

#### Testing

```bash
cargo test -p srs-repository type_schema
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `type_schema_includes_inherited_fields` — proves child schema includes parent fields
- `type_schema_covers_all_value_types` — unchanged; proves non-inheriting path still works
- `type_schema_required_array` — unchanged
- `type_schema_dangling_field_skipped` — unchanged; still works via `resolve_field`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm `type_schema_includes_inherited_fields` exists and passes.
3. Run:
   ```bash
   cargo test -p srs-repository type_schema
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update this plan: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit with message referencing #68.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (payload structs unchanged)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas unchanged)
- [ ] `type schema` on a child type returns both own and inherited fields in its JSON Schema
- [ ] All existing `type_schema_*` tests continue to pass

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- Verification Agent runs after implementation and before PR creation.

## Assumptions

- `MemoryStore` already supports multiple `record_types` in its `Package` (it does — `Package.record_types` is `Vec<RecordType>`).
- A new `store_with_types` helper is added alongside the existing `store_with`. `store_with` is **not** modified — 8 existing tests depend on its current signature.
- No integration tests or golden files reference the inherited-fields behaviour (the existing golden file only tests the output shape, which is unchanged).
