# Plan: Repeatable Fields and Field Groups

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`ext:repeatable-fields` and `ext:field-groups` are defined in the SRS spec and in the JSON schemas. This plan filled the gap end-to-end: core types → validation → rendering. **All phases complete.**

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan implements existing spec behaviour for two independently adoptable extensions. No ADRs required.

---

## Scope

- Add `repeatable`, `min_items`, `max_items` to `FieldAssignment` in `srs-core`
- Add `FieldValueEntry` struct to `srs-core`
- Add `entries`, `source`, `edited_at` to `FieldValue` in `srs-core`
- Add `FieldGroup`, `FieldGroupEntry`, `FieldGroupValue` structs to `srs-core`
- Add `field_groups` to `RecordType` and `group_values` to `Record` in `srs-core`
- Add validation rules for repeatability constraints and field group presence/entry counts to `srs-core`
- Add new `CoreError` variants for the new validation rules
- Replace the stub in `render_service.rs` with real rendering for repeatable fields and field groups
- Unit tests for all new types and validation rules (inline `#[cfg(test)]`)
- Fixture repositories for integration-level verification

**Out of scope:**

- `FieldValue.sourceRefs` — belongs to a separate source-references pass
- `ext:type-inheritance` effective field list — separate concern; field-group fields are not inherited fields
- CLI command surface changes — no new commands; existing `record create`/`inspect` benefit automatically
- `srs/` spec package changes — no new meta-types; these are runtime extension types, not spec authoring types

---

## Phases

### Phase 1: Core Types — `ext:repeatable-fields`

**Status:** `complete`

**Goal:** `FieldAssignment` carries repeatability metadata and `FieldValue` supports `entries`; existing JSON round-trips are unaffected.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/`

#### Files modified

| File | Action |
|---|---|
| `crates/srs-core/src/types/record_type.rs` | Edit — extend `FieldAssignment` |
| `crates/srs-core/src/types/record.rs` | Edit — add `FieldValueEntry`; extend `FieldValue` |

#### Tasks

- [x] Add `repeatable`, `min_items`, `max_items` fields to `FieldAssignment` in `record_type.rs`
- [x] Add `FieldValueEntry` struct to `record.rs`
- [x] Add `entries`, `source`, `edited_at` fields to `FieldValue` in `record.rs`
- [x] Update all existing `FieldValue` construction in tests to compile (add missing fields or `Default`)
- [x] Update all existing `FieldAssignment` construction in tests to compile

#### Tests

**`record_type.rs`:**
- `field_assignment_repeatable_defaults_to_false` ✓
- `field_assignment_repeatable_roundtrips` ✓
- `field_assignment_repeatable_false_omits_min_max_in_json` ✓

**`record.rs`:**
- `field_value_entry_roundtrips_json` ✓
- `field_value_entries_roundtrips` ✓
- `field_value_without_entries_omits_entries_key` ✓

---

### Phase 2: Core Types — `ext:field-groups`

**Status:** `complete`

**Goal:** `RecordType` carries `field_groups` and `Record` carries `group_values`; existing JSON round-trips are unaffected.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/`

#### Files modified

| File | Action |
|---|---|
| `crates/srs-core/src/types/record_type.rs` | Edit — add `FieldGroup`; add `field_groups` to `RecordType` |
| `crates/srs-core/src/types/record.rs` | Edit — add `FieldGroupEntry`, `FieldGroupValue`; add `group_values` to `Record` |

#### Tasks

- [x] Add `FieldGroup` struct to `record_type.rs`
- [x] Add `field_groups: Option<Vec<FieldGroup>>` to `RecordType`
- [x] Add `find_field_group` method to `RecordType`
- [x] Add `FieldGroupEntry` and `FieldGroupValue` structs to `record.rs`
- [x] Add `group_values: Option<Vec<FieldGroupValue>>` to `Record`
- [x] Add `find_group_value` method to `Record`
- [x] Update any `Record` construction in existing tests that now fail to compile

#### Tests

**`record_type.rs`:**
- `field_group_roundtrips_json` ✓
- `field_group_optional_fields_absent_when_none` ✓
- `record_type_with_field_groups_roundtrips` ✓
- `record_type_without_field_groups_omits_key` ✓
- `find_field_group_returns_correct_group` ✓
- `find_field_group_returns_none_for_unknown` ✓
- `find_field_group_returns_none_when_no_groups` ✓

**`record.rs`:**
- `field_group_value_roundtrips_json` ✓
- `record_with_group_values_roundtrips` ✓
- `record_without_group_values_omits_key` ✓
- `find_group_value_returns_correct_group` ✓
- `find_group_value_returns_none_for_unknown` ✓

---

### Phase 3: Validation — Repeatability and Field Group Constraints

**Status:** `complete`

**Goal:** `validate_record` enforces repeatable field entry counts and required field group presence; all failures go into `CoreError` variants (no hard panics, no silent drops).

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/`

#### Tasks

- [x] Add `TooFewEntries`, `TooManyEntries`, `EntriesOnNonRepeatableField`, `MissingRequiredFieldGroup`, `TooFewGroupEntries`, `TooManyGroupEntries` to `error.rs`
- [x] Add all six variants to the `PartialEq` impl in `error.rs`
- [x] Add repeatable-field entry count validation to `validate_record` in `validation/record.rs`
- [x] Add field group presence and entry count validation to `validate_record`

#### Tests

- `validate_repeatable_field_entry_count_ok` ✓
- `validate_repeatable_field_too_few_entries` ✓
- `validate_repeatable_field_too_many_entries` ✓
- `validate_entries_on_non_repeatable_field_fails` ✓
- `validate_repeatable_no_min_max_any_count_ok` ✓
- `validate_required_field_group_present_ok` ✓
- `validate_required_field_group_missing_fails` ✓
- `validate_optional_field_group_absent_ok` ✓
- `validate_field_group_entry_count_too_few` ✓
- `validate_field_group_entry_count_too_many` ✓
- `validate_field_group_no_min_max_any_count_ok` ✓

---

### Phase 4: Rendering — `srs-repository`

**Status:** `complete`

**Goal:** The stub in `render_service.rs` is replaced with real rendering; repeatable fields render all entries; field groups render each entry as an ordered mini-record.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-repository/src/render_service.rs`

#### Tasks

- [x] Remove the stub diagnostic (`[partial] repeatable field... ext:repeatable-fields not fully supported`)
- [x] Add repeatable field rendering using `FieldValue.entries` with `valueType`-aware join strategy
- [x] Add field group rendering using `Record.group_values` and `RecordType.field_groups` ordering

---

### Phase 5: Fixtures and Integration Tests

**Status:** `complete`

**Goal:** Two fixture repositories exist and the CLI validates and renders them correctly; no regressions.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-cli/tests/fixtures/`, `crates/srs-cli/tests/`

#### Tasks

- [x] Create `crates/srs-cli/tests/fixtures/repeatable-fields/` with `.srs/`, `manifest.json`, `package/`, and record files
- [x] Create `crates/srs-cli/tests/fixtures/field-groups/` with `.srs/`, `manifest.json`, `package/`, and record files
- [x] Add the four integration tests to `integration_tests.rs`

#### Integration tests

- `repeatable_fields_fixture_validates_ok` ✓
- `repeatable_fields_fixture_too_many_entries_in_diagnostics` ✓
- `field_groups_fixture_validates_ok` ✓
- `field_groups_fixture_missing_required_group_in_diagnostics` ✓

---

### Phase 6: README Roadmap Update

**Status:** `complete`

**Goal:** `README.md` accurately reflects implemented status of both extensions.

**Agent:** Lead Integrator

**Write scope:** `README.md`

#### Tasks

- [x] Mark `ext:repeatable-fields` row as implemented in the roadmap table
- [x] Add `ext:field-groups` row and mark it as implemented
- [x] Remove or update the note about `entries not modeled in srs-core::Record`

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `FieldAssignment` has `repeatable`, `min_items`, `max_items` fields; existing JSON without these keys still deserialises correctly
- [x] `FieldValue` has `entries`, `source`, `edited_at` fields; existing JSON without these keys still deserialises correctly
- [x] `FieldGroup`, `FieldGroupEntry`, `FieldGroupValue` exist in `srs-core`
- [x] `RecordType.field_groups` and `Record.group_values` exist and round-trip correctly
- [x] `validate_record` rejects records that violate `minItems`/`maxItems` on repeatable fields
- [x] `validate_record` rejects records missing a required field group
- [x] The `[partial] repeatable field` diagnostic stub is gone from `render_service.rs`
- [x] Both fixture repositories pass `srs repo validate` with `ok: true` for valid records
- [x] Integration tests for both extensions pass
- [x] `README.md` roadmap updated for both extensions

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phase 3 (validation complete) and before final sign-off.

## Assumptions

- `FieldValue` does not currently derive `Default` — check before adding `..Default::default()` in tests; add `#[derive(Default)]` where needed.
- Validation failures in `validate_record` return `Err` (early exit). This is consistent with the existing implementation. If the calling layer in `srs-repository` collects multiple diagnostics, it should call `validate_record` per record and map errors to diagnostic entries rather than short-circuiting the whole batch.
- The render service currently resolves `RecordType` from the loaded package. Phase 4 assumes this resolution is already in place (it is, via `package_service`). The typed `FieldGroup` and `FieldValue.entries` fields are accessible through the resolved model after Phases 1–2.
- Fixtures follow the same layout as existing fixtures inferred from the integration test helpers (`create_temp_repo`, `run_srs_in_dir`). If those helpers require specific manifest shapes, the new fixtures must match.
- `ext:repeatable-fields` and `ext:field-groups` are structurally independent per the spec. This plan implements both in sequence but they do not depend on each other at the Rust type level.
