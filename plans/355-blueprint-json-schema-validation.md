# Plan: Blueprint JSON Schema Validation (#355)

## Summary

`validate_repository` currently skips JSON Schema validation for blueprint definition files because `blueprint.json` previously declared `$schema` as required — but the CLI never writes it. RFC-020 (srs#150, now merged) removed `$schema` from the `required` array, making the registered schema usable for real files. This plan wires up the already-registered `BLUEPRINT_SCHEMA_ID` schema in `validate_repository`'s blueprint loop, so blueprints now receive both JSON Schema and semantic validation. No new ADRs, no CLI changes, no payload changes, no schema file changes (the mirror is already in sync).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — (single-file change in `srs-repository`) |
| Verification | — |

## Architecture Decisions

No new architectural decisions — this plan implements ADR-004 (schemas are registered for validation) by removing a work-around that existed only because the schema itself was incorrect. RFC-020 fixed the schema; this plan removes the work-around.

| ADR | Decision | Status |
|---|---|---|
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Registered schemas are used for validation — `BLUEPRINT_SCHEMA_ID` must be applied | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI output changes. Blueprint validation diagnostics already flow through the existing `RepositoryValidationReport` payload shape. No `payload.rs` changes, no golden schema regeneration.

### Entity schema sync (check-schema-sync.sh)

No schema files under `srs/docs/schema/2.0/` change in this plan. The schema mirror is already in sync (RFC-020 already merged and mirrored). `bash scripts/check-schema-sync.sh` must exit 0 — verified during Final Acceptance.

---

## Scope

- Insert `validate_value_against_schema(&bp_value, &full_path, srs_schema::BLUEPRINT_SCHEMA_ID, reg)` before the `serde_json::from_value::<Blueprint>(bp_value)` call in the blueprint loop, using the fully-qualified `srs_schema::BLUEPRINT_SCHEMA_ID` (not added to the `use` import — consistent with how `MANIFEST_SCHEMA_ID`, `PACKAGE_MANIFEST_SCHEMA_ID`, and `RELATIONS_COLLECTION_SCHEMA_ID` are used in the same file).
- Update the comment on that block (remove the out-of-date note about `$schema` not being in blueprint files).
- Add one new unit test: `test_validate_blueprint_json_schema_applied_to_extra_property` — uses `MemoryStore` (canonical test double per CLAUDE.md Storage Boundary Rules); blueprint JSON with an unknown extra property fails JSON Schema (`additionalProperties: false`) and emits a diagnostic with `schema_id: Some(srs_schema::BLUEPRINT_SCHEMA_ID.to_string())`.

**Out of scope:**

- Protocol JSON Schema validation (separate concern; protocols use a different schema).
- CLI command changes.
- Any schema file edits.

---

## Phases

### Phase 1: Wire up JSON Schema validation for blueprints

**Goal:** `validate_repository` applies JSON Schema validation to blueprint files before semantic validation; a new unit test proves JSON Schema errors surface with the blueprint schema ID.

**Agent:** Repository Service Worker

#### Tasks

- [x] In the blueprint loop (around line 820), replace the comment `"Blueprint: semantic validation only (blueprint files do not include $schema)"` with `"Blueprint: JSON Schema validation + semantic validation"`.
- [x] Before `match serde_json::from_value::<Blueprint>(bp_value)` (line ~835), add (using qualified `srs_schema::BLUEPRINT_SCHEMA_ID` — no import change required):
  ```rust
  if let Some(schema_diags) = validate_value_against_schema(
      &bp_value,
      &full_path,
      srs_schema::BLUEPRINT_SCHEMA_ID,
      reg,
  ) {
      diagnostics.extend(schema_diags);
  }
  ```
- [x] Add test `test_validate_blueprint_json_schema_applied_to_extra_property` in the existing `#[cfg(test)]` block near the blueprint tests (~line 4111). This test uses `MemoryStore` (following the `test_validate_blueprint_memory_with_data` pattern at line 4169), creates a blueprint JSON with an extra unknown field `"unknownField": "bad"`, calls `validate_repository`, and asserts a diagnostic with `schema_id: Some(srs_schema::BLUEPRINT_SCHEMA_ID.to_string())` is emitted.

#### Acceptance Criteria

- [x] `validate_repository` on a valid blueprint (no `$schema`, all required fields present) emits zero blueprint diagnostics — existing test `test_validate_blueprint_valid_passes` still passes.
- [x] A blueprint with an extra unknown field produces a diagnostic with `schema_id: Some(srs_schema::BLUEPRINT_SCHEMA_ID.to_string())`.
- [x] A blueprint failing semantic validation (empty `rootTypes`) still produces an error — existing test `test_validate_blueprint_semantic_empty_root_types_reports_diagnostic` still passes.
- [x] `cargo test -p srs-repository` passes with zero failures.
- [x] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository validation::tests::test_validate_blueprint
cargo test -p srs-repository validation::tests::test_validate_blueprint_json_schema_applied_to_extra_property
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `test_validate_blueprint_valid_passes` — valid blueprint, no diagnostics (regression)
- `test_validate_blueprint_semantic_empty_root_types_reports_diagnostic` — semantic error still fires (regression)
- `test_validate_blueprint_json_schema_applied_to_extra_property` — JSON Schema error surfaces with schema_id (new)

#### Milestone gate

1. All acceptance criteria above are checked.
2. All named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes to `[x]`.
5. Commit: `fix(srs-repository): add JSON schema validation for blueprints (#355)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema files changed, mirrors already in sync)
- [ ] `test_validate_blueprint_valid_passes` passes (regression)
- [ ] `test_validate_blueprint_json_schema_applied_to_extra_property` passes (new behavior)

## Coordination Rules

- Single implementer; no cross-crate coordination required.
- Only `crates/srs-repository/src/validation.rs` is modified.
- Workers return changed file paths and a short behaviour summary when done.

## Assumptions

- RFC-020 (srs#150) is merged and the schema mirror at `crates/srs-schema/schemas/2.0/blueprint.json` is already in sync (confirmed: `$schema` not in `required` array in both repos).
- `validate_value_against_schema` is the correct helper to call (consistent with how `manifest.json`, `package.json`, and `relations.json` are validated).
- Blueprint files without `$schema` on disk must now pass JSON Schema validation (RFC-020 [R1]–[R3]).
