# Plan: type_schema — field groups included in fieldOrder positional x-srs-order

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`type_schema_service.rs` assigns 1-based positional `x-srs-order` to regular fields (via their position in the `effective_fields()` list) but still writes the raw `group.order` integer for field groups (line 270 in `type_schema_service.rs`). This causes two classes of ordering bugs: (1) order collisions between groups with low `order` values and regular fields at positions 1+, and (2) no mechanism to interleave groups and fields because `fieldOrder` only lists field IDs. Issue #147 fixed fields; issue #148 extends the same fix to groups.

The fix is contained entirely within `srs-repository` (service logic). No CLI payload struct changes are required because `TypeSchemaResult.schema` is a `serde_json::Value` — the `x-srs-order` values within the JSON schema are implementation detail, not a named payload struct field. The golden schema files do not need regeneration.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All group-order logic stays in the service (`type_schema_service.rs`), not in the CLI handler | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct change needed — `TypeSchemaResult.schema` is a `serde_json::Value`; golden schemas unchanged | accepted |

No new ADRs required — this plan implements existing behaviour implied by ADR-010 and the field-groups extension.

---

## Contracts

### CLI output contract (ADR-011)

No CLI payload struct changes. `TypeSchemaResult.schema` is serialised as `serde_json::Value`. The `x-srs-order` extension field values inside the schema will change (correct positional values replacing wrong raw-order values), but this is a bugfix to the schema content, not a change to the payload struct shape. `cargo test --test payload_contracts` will continue to pass without regenerating golden files.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- Add `Package::effective_fields_and_groups` (in `crates/srs-repository/src/package.rs`) — a new function alongside the existing `effective_fields`. It computes a merged position sequence for both fields and groups, using `fieldOrder` when present.
- Update `type_schema_service::type_schema` (in `crates/srs-repository/src/type_schema_service.rs`) to call `effective_fields_and_groups` and use the resulting group positions when emitting `x-srs-order`.
- Add unit tests for both the new position-computation path and the validation path.

**Out of scope:**

- Modifying `effective_fields` — it is left unchanged; group completeness validation lives only in `effective_fields_and_groups`.
- Changing the `RecordType` JSON serialisation format — `field_order: Option<Vec<String>>` already supports group IDs as strings.
- Modifying the CLI handler in `srs-cli` (ADR-010: handlers call one service).
- Changing any payload struct in `payload.rs`.
- Regenerating JSON Schema golden files under `schemas/payload/`.
- Multi-group validation in `blueprint_schema_service` (separate concern).
- Changes to `srs-vscode`.

---

## Phases

### Phase 1: Add `Package::effective_fields_and_groups`

**Goal:** A new function in `package.rs` computes the merged field+group position sequence and validates `fieldOrder` completeness when groups are present.

**Agent:** Repository Service Worker

#### Background

`effective_fields` returns `Vec<FieldAssignment>` (fields only). Its callers (`type_schema` and record validation) need no change to group ordering. Rather than modifying `effective_fields` (which would change its return type and break existing callers), a new companion function `effective_fields_and_groups` is introduced. Callers that need group ordering call the new function; all existing callers of `effective_fields` are unchanged.

**Caller scope:** `effective_fields` has the following callers in `srs-repository` — run `grep -rn "effective_fields" crates/srs-repository/src/` to confirm. The new `effective_fields_and_groups` function will be called only from `type_schema_service.rs::type_schema`. Callers of the original `effective_fields` are not updated and do not gain group validation.

#### New types to add in `crates/srs-repository/src/package.rs` (above the `#[cfg(test)]` block)

```rust
/// Merged ordered sequence of fields and groups, returned by
/// [`Package::effective_fields_and_groups`].
pub struct EffectiveFieldsAndGroups {
    /// Fields in their final sorted/fieldOrder-reordered order (same as `effective_fields`).
    pub fields: Vec<FieldAssignment>,
    /// Groups with their 1-based position in the merged field+group sequence.
    pub groups: Vec<OrderedGroup>,
}

/// A field group with its computed 1-based position in the merged sequence.
pub struct OrderedGroup {
    /// The full FieldGroup struct (including group_id, order, fields, label, etc.).
    pub group: FieldGroup,
    /// 1-based position of this group in the merged (fields + groups) sequence.
    pub merged_position: usize,
}
```

#### Merged-sort algorithm (no `fieldOrder` present)

When `field_order` is `None`:
1. Collect fields from `effective_fields(record_type)?` — already sorted by effective order.
2. Sort groups by `group.order` ascending. Tie-breaking rule: when two groups have equal `order`, sort by `group_id` lexicographically (ascending). When a field and a group share an order value, **fields come before groups**.
3. Perform a stable merge of fields (using `assignment.order`) and groups (using `group.order`) into a single position sequence. Use the tie-breaking rule above. Each item gets a 1-based position counter starting at 1.
4. Fields' positions are their index+1 in the `effective_fields` output (unchanged). Only groups need `merged_position` recorded.
5. Build `Vec<OrderedGroup>` by mapping each group to its position in the merged sequence.

Concrete implementation: walk the `effective_fields` output and groups simultaneously with a two-pointer merge. When orders are equal, advance the field pointer first. After the merge loop, any remaining groups (order > all field orders) get sequential positions after the last field.

#### `fieldOrder`-present algorithm

When `field_order` is `Some(ids)`:
1. Call `effective_fields(record_type)?` to get fields (which also validates the field subset of `fieldOrder` for completeness and unknowns — **do not duplicate that validation here**).
2. Build a set of known IDs: field IDs from `effective_fields` output UNION group IDs from `record_type.field_groups` (if any).
3. Validate `fieldOrder` does not contain unknown IDs that are neither a field ID nor a group ID. For each ID in `field_order` that is not a field ID: check if it is a group ID. If not, return `Err(RepositoryError::FieldOrderMismatch { type_id, field_id: id.clone() })`.
4. Validate all group IDs are covered: for each group in `field_groups`, if its `group_id` is not in `field_order`, return `Err(RepositoryError::FieldOrderMismatch { type_id, field_id: group_id.clone() })`.
5. Walk `field_order` with a 1-based counter. For each ID: if it is a group ID, record `(group_id, counter)`. Counter increments for every entry regardless of whether it is a field or group.
6. Map recorded `(group_id, counter)` pairs back to `FieldGroup` entries from `field_groups` to build `Vec<OrderedGroup>`.

**Important:** `effective_fields` already validates that all field IDs are covered by `fieldOrder` and that no unknown field IDs appear. The new function only adds the group layer on top of that.

#### Tasks

- [ ] In `crates/srs-repository/src/package.rs`, add `use srs_core::types::record_type::FieldGroup;` to the imports at the top if not already present (check current imports).
- [ ] Add `pub struct EffectiveFieldsAndGroups` and `pub struct OrderedGroup` to `crates/srs-repository/src/package.rs` above the `#[cfg(test)]` block (after the `Package` impl block ends around line 348).
- [ ] Add `pub fn effective_fields_and_groups` to the `impl Package` block in `crates/srs-repository/src/package.rs`, after the `effective_fields` function (before `resolve_vocabulary`):
  - Signature: `pub fn effective_fields_and_groups(&self, record_type: &RecordType) -> Result<EffectiveFieldsAndGroups, RepositoryError>`
  - Fast path: if `record_type.field_groups.is_none()` (or empty), call `self.effective_fields(record_type)?` and return `EffectiveFieldsAndGroups { fields, groups: vec![] }`.
  - No-fieldOrder path: implement the merged-sort algorithm above.
  - fieldOrder-present path: implement the fieldOrder algorithm above.
- [ ] Run `grep -rn "effective_fields" crates/srs-repository/src/` to confirm no existing caller breaks (they call the original `effective_fields`, not the new function).

#### Acceptance Criteria

- [ ] `effective_fields_and_groups` with no `fieldOrder` and field(order=0), group(order=1), field(order=2): field positions are their `effective_fields` indices (1, 2); group gets `merged_position: 2`.
- [ ] `effective_fields_and_groups` with `fieldOrder: [field_a_id, group_id, field_b_id]`: group gets `merged_position: 2` (counter value at position 2 in the walk).
- [ ] `effective_fields_and_groups` with `fieldOrder` that omits a group ID: returns `Err(RepositoryError::FieldOrderMismatch)` with the group's ID.
- [ ] `effective_fields_and_groups` with `fieldOrder` containing an ID that is neither a field ID nor a group ID: returns `Err(RepositoryError::FieldOrderMismatch)` with that ID.
- [ ] `effective_fields_and_groups` with no `field_groups` (or empty): returns `EffectiveFieldsAndGroups { fields: <same as effective_fields>, groups: vec![] }`.
- [ ] All existing `effective_fields` tests still pass — `effective_fields` is unchanged.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Tests to write in `crates/srs-repository/src/package.rs` (in `#[cfg(test)]` block):

- `effective_fields_and_groups_no_field_order_interleaves_by_order` — field(order=0), group(order=1), field(order=2), no `fieldOrder`. Assert: `groups.len() == 1`, `groups[0].merged_position == 2`.
- `effective_fields_and_groups_field_order_assigns_group_positions` — `fieldOrder: [field_a_id, group_id, field_b_id]`. Assert: `groups[0].merged_position == 2`.
- `effective_fields_and_groups_field_order_missing_group_errors` — `fieldOrder` lists only field IDs when a group is present. Assert: `Err(RepositoryError::FieldOrderMismatch)` where the failing ID is the group_id.
- `effective_fields_and_groups_field_order_unknown_id_errors` — `fieldOrder` contains a string that is neither a field ID nor a group ID. Assert: `Err(RepositoryError::FieldOrderMismatch)` where the failing ID is the unknown string.
- `effective_fields_and_groups_no_groups_returns_empty` — type with no `field_groups`. Assert: `groups.is_empty()`.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `feat: effective_fields_and_groups supports group IDs in fieldOrder (#148)`

Do not start Phase 2 until this milestone gate passes.

---

### Phase 2: Update `type_schema_service` to use positional `x-srs-order` for groups

**Goal:** `type_schema` assigns 1-based positional `x-srs-order` to field groups using the merged position from `effective_fields_and_groups`, eliminating order collisions with regular fields.

**Agent:** Repository Service Worker

#### Caller verification (do this first)

Before changing `field_group_to_property`'s signature, verify it has no callers outside `type_schema`:

```bash
grep -rn "field_group_to_property" crates/
```

Expected: exactly one call, in `type_schema_service.rs::type_schema`. If any other callers exist, stop and report — they must be updated too.

#### Tasks

- [ ] In `crates/srs-repository/src/type_schema_service.rs`, add `use crate::package::EffectiveFieldsAndGroups;` and `use crate::package::OrderedGroup;` to imports.
- [ ] In `type_schema` (line 73), replace `let assignments = package.effective_fields(&record_type)?;` with:
  ```rust
  let EffectiveFieldsAndGroups { fields: assignments, groups: ordered_groups } =
      package.effective_fields_and_groups(&record_type)?;
  ```
- [ ] The field loop (lines 79–103) is unchanged — it iterates `assignments.iter().enumerate()` and uses `position + 1` for `x-srs-order`.
- [ ] Change `field_group_to_property` signature (line 217) to:
  ```rust
  fn field_group_to_property(
      group: &FieldGroup,
      package: &crate::package::Package,
      merged_position: usize,
      diagnostics: &mut Vec<String>,
  ) -> Value
  ```
  In the function body, replace `prop.insert("x-srs-order".into(), json!(group.order));` (line 270) with `prop.insert("x-srs-order".into(), json!(merged_position));`.
- [ ] Replace the `if let Some(groups) = &record_type.field_groups { for group in groups { ... } }` block (lines 108–116) with:
  ```rust
  for OrderedGroup { group, merged_position } in &ordered_groups {
      let property = field_group_to_property(group, &package, *merged_position, &mut diagnostics);
      if group.required {
          required.push(Value::String(group.group_id.clone()));
      }
      properties.insert(group.group_id.clone(), property);
  }
  ```

#### Acceptance Criteria

- [ ] A type with two fields (field_a order=0, field_b order=2) and one group (raw `order=1`), no `fieldOrder`: merged sort gives field_a position 1, group position 2, field_b position 3. Assert `schema["properties"]["group_id"]["x-srs-order"] == 2` (not raw 1).
- [ ] A type with `fieldOrder: [field_a_id, group_id, field_b_id]`: field_a gets `x-srs-order: 1`, group gets `x-srs-order: 2`, field_b gets `x-srs-order: 3`.
- [ ] Updated `type_schema_emits_field_groups_with_composite_renderer` test: the existing test has field(order=0) and group(order=1). Merged sort → field position 1, group position 2. Add assertion: `assert_eq!(tables["x-srs-order"], json!(2))`.
- [ ] `type_schema_order_recoverable` (fields-only) still passes with no regression.
- [ ] `type_schema_covers_all_value_types` still passes.
- [ ] A type with fields only and no `field_groups`: `x-srs-order` on fields is still 1-based positional (no regression).

#### Testing

```bash
cargo test -p srs-repository -- type_schema
cargo clippy -p srs-repository -- -D warnings
```

Tests to write/update in `crates/srs-repository/src/type_schema_service.rs`:

- `type_schema_group_order_is_positional_not_raw` — type with field_a(order=0), field_b(order=2), group(raw order=1), no `fieldOrder`. Merged sort: field_a→pos 1, group→pos 2, field_b→pos 3. Assert `schema["properties"][group_id]["x-srs-order"] == json!(2)` (not `json!(1)` or `json!(0)`).
- `type_schema_field_order_interleaves_groups` — type with `fieldOrder: [field_a_id, group_id, field_b_id]`. Assert: field_a `x-srs-order: 1`, group `x-srs-order: 2`, field_b `x-srs-order: 3`.
- Update `type_schema_emits_field_groups_with_composite_renderer`: add `assert_eq!(tables["x-srs-order"], json!(2))`.
- `type_schema_no_groups_retains_field_order` — type with three fields, no groups, no fieldOrder. Assert fields get `x-srs-order: 1, 2, 3` in order (regression guard).

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed exists and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `feat: type_schema assigns positional x-srs-order to field groups (#148)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] `effective_fields_and_groups` correctly handles: no groups, groups with no `fieldOrder` (merged sort), groups with `fieldOrder` covering both field IDs and group IDs, and validation errors
- [ ] `type_schema` assigns consistent 1-based positional `x-srs-order` to both regular fields and groups in the same schema — no raw `group.order` values emitted

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `RepositoryError::FieldOrderMismatch` is the correct error variant for the group-coverage check (reuse of existing variant; its `field_id` field carries the missing/unknown group ID).
- `EffectiveFieldsAndGroups` and `OrderedGroup` are defined in `package.rs` (not a new module) since `effective_fields` lives there.
- The no-`fieldOrder` fallback (merged sort by raw `order`) is backward-compatible: existing types with `field_groups` but no `fieldOrder` previously got `x-srs-order` equal to the raw `group.order` value. With this fix they get a 1-based positional value. This is a bugfix (matching the fix applied to fields in #147) and does not require regenerating golden schemas because `x-srs-order` is a schema extension field, not a payload struct field (ADR-011).
- `effective_fields` callers that do not need group ordering (record validation, etc.) are left unchanged and do not gain group validation.
