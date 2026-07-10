# Plan: enforce CrossFieldRules at record write boundary (#437)

## Summary

`record_service::create` and `record_service::update` can persist records that violate a Type's `validationRules` — cross-field rules (CFRs) are only evaluated post-hoc by `repo validate`. This write/validate asymmetry means a `record create` call can succeed with an invalid record. This plan closes the gap by wiring `validate_cross_field_rules` (already implemented in `srs-core` for `repo validate`) into the two write paths in `srs-repository`, making CFR violations a hard error consistent with how required-field violations are handled today.

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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All validation lives in the service layer; CLI handlers do not validate | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI output is typed payload structs; no payload change needed here | accepted |

No new ADRs required. The decision to treat CFR violations as hard write errors is consistent with the existing required-field pattern (`validate_record` → `RepositoryError::RecordValidation`). This plan extends the same constraint to CFRs, not a new architectural choice.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `record create` and `record update` already surface `RepositoryError::RecordValidation` when required-field validation fails; CFR violations will use the same error variant and flow through the same handler output path. No payload structs change. `cargo test --test payload_contracts` will continue to pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. Schema sync is a no-op.

---

## Scope

- Wire `validate_cross_field_rules` into `create_record_at_dir` and `update_record` in `crates/srs-repository/src/record_store.rs`.
- Return the first CFR error as `RepositoryError::RecordValidation { path, source }` (hard reject, consistent with required-field failures).
- Extend `validate_record_input` (preflight) to also run CFR checks, so a passing preflight guarantees a passing write.
- Add integration tests in `record_store.rs` using `MemoryStore` covering create violation, update violation, create happy path, update happy path, and preflight.

**Out of scope:**
- Bulk-write paths (e.g. import / snapshot) — deferred to follow-up; the two write service functions cover the interactive write surface.
- Returning all CFR errors at once (only first error is returned, matching `validate_record` fail-fast semantics).
- WASM bindings — they call the same service functions and will pick up enforcement automatically; no WASM-specific changes needed.
- Lifecycle-transition write path (`transition_record_lifecycle`) — CFRs apply to field values, not lifecycle states; no CFR re-evaluation is needed on a lifecycle-only update.

---

## Phases

### Phase 1: Service wiring + tests

**Goal:** `create_record_at_dir`, `update_record`, and `validate_record_input` enforce CFRs; passing tests prove it.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add imports to `crates/srs-repository/src/record_store.rs`:
  - `use srs_core::types::field::ValueType;`
  - `use srs_core::validation::record_type::validate_cross_field_rules;`
- [x] In `create_record_at_dir` (after `validate_record` call, before `record.instance_id = new_instance_id()`):
  ```rust
  if let Some(rules) = &record_type.validation_rules {
      if !rules.is_empty() {
          let field_type_map: HashMap<String, ValueType> = package
              .fields
              .iter()
              .map(|f| (f.id.clone(), f.value_type))
              .collect();
          if let Some(err) = validate_cross_field_rules(&record, rules, &field_type_map)
              .into_iter()
              .next()
          {
              return Err(RepositoryError::RecordValidation {
                  path: std::path::PathBuf::from(relative_dir),
                  source: err,
              });
          }
      }
  }
  ```
- [x] In `update_record` (after `validate_record` call, before `store.load_manifest()`):
  - Same CFR check block, using `"records"` as the path string.
- [x] In `validate_record_input` (after `validate_record_all`, extend to also collect CFR errors):
  ```rust
  if let Some(rules) = &record_type.validation_rules {
      let field_type_map: HashMap<String, ValueType> = package
          .fields
          .iter()
          .map(|f| (f.id.clone(), f.value_type))
          .collect();
      let cfr_errors = validate_cross_field_rules(&record, rules, &field_type_map);
      errors.extend(cfr_errors.iter().map(|e| e.to_string()));
  }
  ```
- [x] Add test helper `make_store_with_cfr_package()` in `record_store.rs` `#[cfg(test)]` mod:
  - Package with two fields: `field-trigger-001` (String) and `field-target-001` (String)
  - Type `cfr-test-type` version 1 with a `ConditionalRequired` rule:
    predicate_field_id = `field-trigger-001`, predicate_value = `"active"`, target_field_id = `field-target-001`
- [x] Add tests:
  - `cfr_create_rejects_violating_record` — create with trigger=`"active"`, target absent → `RepositoryError::RecordValidation`
  - `cfr_create_accepts_satisfying_record` — create with trigger=`"active"`, target=`"x"` → `Ok`
  - `cfr_create_accepts_when_predicate_not_triggered` — create with trigger absent → `Ok` (rule not triggered)
  - `cfr_update_rejects_violating_record` — create valid, then update to violate → `RepositoryError::RecordValidation`
  - `cfr_validate_input_reports_violation` — `validate_record_input` with violating values → `report.ok == false`

#### Acceptance Criteria

- [x] `create_record_at_dir` returns `Err(RecordValidation)` when CFR is violated
- [x] `update_record` returns `Err(RecordValidation)` when CFR is violated
- [x] `validate_record_input` includes CFR errors in its report
- [x] Happy-path writes still succeed when CFR is satisfied or no rules are declared
- [x] No existing tests broken

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `cfr_create_rejects_violating_record` — proves write is blocked on CFR violation
- `cfr_create_accepts_satisfying_record` — proves happy path still works
- `cfr_create_accepts_when_predicate_not_triggered` — proves rules are only enforced when triggered
- `cfr_update_rejects_violating_record` — proves update path is also enforced
- `cfr_validate_input_reports_violation` — proves preflight consistency

#### Milestone gate

1. All five named tests exist and pass.
2. No regressions (`cargo test -p srs-repository` passes).
3. Clippy clean.
4. Plan checkboxes updated and committed.

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Commit with message: `feat(repository): enforce CrossFieldRules at record write boundary (#437)`

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] `cfr_create_rejects_violating_record` test passes
- [x] `cfr_update_rejects_violating_record` test passes
- [x] `cfr_validate_input_reports_violation` test passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- After the phase milestone gate passes, commit and do not proceed until committed.

## Assumptions

- `validate_cross_field_rules` in `srs-core` is correct and fully tested (it has dedicated tests in `record_type.rs`).
- The `Package::fields` vec contains all field definitions needed to build the `field_type_map`; this is the same approach used by `validate_repository` in `validation.rs`.
- `record_type.validation_rules` is `None` for types without CFRs; the check short-circuits on `None` and empty vecs.
