# Plan: Audit & close ext:lifecycle V7–V9 enforcement gaps (#239)

## Summary

`validate_repository` enforces some lifecycle invariants (the lifecycleRef UUID resolution check and V9 for standalone `Lifecycle` definitions) but has three documented gaps: (1) no check that a `RecordType` declares **at most one** of `lifecycle` / `lifecycleRef` (V7 mutual exclusion); (2) V9 structural integrity is not enforced for **inline** `TypeLifecycle` blocks at validation time (only for standalone Lifecycles and at write time); (3) the code labels the lifecycleRef-resolution check "V7" when the spec numbers it V8, so two different invariants carry the same label. This plan closes all three gaps with targeted implementation in `srs-repository::validation` and unit tests in the same module.

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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All validation logic stays in `srs-repository`; no business logic in CLI | accepted |
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library-first: validation runs without CLI context | accepted |

No new ADRs are needed. This plan implements invariant enforcement in an existing validation function, with no new public API shape, no new payload structs, and no cross-crate dependency changes.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** This plan adds validation diagnostics emitted inside the existing `repo validate` output. The `RepositoryValidationReport` struct is unchanged; only the set of diagnostics it may contain grows. No payload structs are added or changed; golden schemas stay as-is.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

**No schema changes.** This plan does not touch any JSON Schema files.

---

## Scope

- Add V7 mutual-exclusion check in `validate_vocabulary_invariants` (`validation.rs`): if a `RecordType` has both `lifecycle` and `lifecycle_ref` set, emit a `DiagnosticSeverity::Error`.
- Rename existing lifecycle-ref-resolution diagnostic label from "V7" to "V8" to match spec numbering (update code + test assertions — no test logic changes, only the string we assert against).
- Add V9 inline-lifecycle validation in `validate_vocabulary_invariants`: for each `RecordType` whose `lifecycle` is `Some(TypeLifecycle{…})`, call `validate_type_lifecycle_v9` and also check that `TypeLifecycle.initial_state` matches the `isInitial` state key.
- Add targeted unit tests (in `validation.rs` `#[cfg(test)]` module) for all three new / fixed paths.

**Out of scope:**
- WASM bindings or CLI command changes (validation surface is unchanged from the outside).
- Record-creation-time lifecycle validation in `record_store.rs` (already enforced there via `validate_type_lifecycle`; no gap to close).
- Inline lifecycle `initialState` field definition — it already exists on `TypeLifecycle`.
- Any spec changes to `srs/`.

---

## Phases

### Phase 1: Rename "V7" → "V8" for the lifecycleRef-resolution check

**Goal:** The existing lifecycleRef-resolution diagnostic message says "V7" but the spec numbers it V8; after this phase, it says "V8", and all test assertions match.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `validation.rs` `validate_vocabulary_invariants`, change the message string in the `lifecycleRef` resolution check from `"V7: type '{}' lifecycleRef '{}' does not resolve..."` to `"V8: type '{}' lifecycleRef '{}' does not resolve..."`.
- [ ] Update test `vocabulary_v7_missing_lifecycle_ref_produces_error` → rename test body's assertion to check `"V8"` instead of `"V7"`, and rename the test function to `lifecycle_v8_dangling_lifecycle_ref_produces_error`.
- [ ] Update test `vocabulary_v7_resolved_lifecycle_ref_no_error` → rename to `lifecycle_v8_resolved_lifecycle_ref_no_error`; change assertion to filter on `"V8"`.
- [ ] Update test `dangling_lifecycle_ref_produces_clear_v7_diagnostic` → rename to `dangling_lifecycle_ref_produces_clear_v8_diagnostic`; change assertion `d.message.contains("V7")` to `d.message.contains("V8")`.

#### Acceptance Criteria

- [ ] `cargo test -p srs-repository 2>&1 | grep -E "FAILED|error"` — zero failures.
- [ ] `grep -n '"V7.*lifecycleRef\|V7.*does not resolve' crates/srs-repository/src/validation.rs` — zero hits (old label gone).
- [ ] `grep -n '"V8.*lifecycleRef\|V8.*does not resolve' crates/srs-repository/src/validation.rs` — one hit (new label present).

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify:
- `lifecycle_v8_dangling_lifecycle_ref_produces_error` — V8 error emitted for dangling lifecycleRef
- `lifecycle_v8_resolved_lifecycle_ref_no_error` — no V8 error when ref resolves
- `dangling_lifecycle_ref_produces_clear_v8_diagnostic` — error message contains "V8" and the dangling UUID

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `git commit -m "refactor: relabel lifecycleRef resolution diagnostic V7→V8 (#239)"`

---

### Phase 2: Add V7 mutual-exclusion check

**Goal:** `validate_repository` emits a `DiagnosticSeverity::Error` when a `RecordType` declares both `lifecycle` and `lifecycle_ref`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `validation.rs` `validate_vocabulary_invariants`, add a loop over `pkg.record_types()` after the existing `lifecycleRef` resolution check.
- [ ] Inside that loop, for each `rt` where `rt.lifecycle.is_some() && rt.lifecycle_ref.is_some()`, push a `ValidationDiagnostic { severity: Error, relative_path: "package/package.json", message: format!("V7: type '{}' declares both 'lifecycle' and 'lifecycleRef'; exactly one is allowed", rt.name) }`.
- [ ] Add test `lifecycle_v7_both_lifecycle_and_ref_produces_error` in `#[cfg(test)]` in `validation.rs`:
  - Build a package-only repo with one type that has both an inline lifecycle AND a lifecycleRef set.
  - Call `validate_repository` and assert a `DiagnosticSeverity::Error` with "V7" in the message is present.
- [ ] Add test `lifecycle_v7_only_lifecycle_ref_no_v7_error` — type with only `lifecycleRef` set → no V7 error.
- [ ] Add test `lifecycle_v7_only_inline_lifecycle_no_v7_error` — type with only inline `lifecycle` set → no V7 error.

#### Acceptance Criteria

- [ ] Test `lifecycle_v7_both_lifecycle_and_ref_produces_error` passes.
- [ ] Tests `lifecycle_v7_only_lifecycle_ref_no_v7_error` and `lifecycle_v7_only_inline_lifecycle_no_v7_error` pass (no false positives).
- [ ] `cargo test -p srs-repository` — zero failures.

#### Testing

```bash
cargo test -p srs-repository lifecycle_v7
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `lifecycle_v7_both_lifecycle_and_ref_produces_error` — mutual-exclusion error fires
- `lifecycle_v7_only_lifecycle_ref_no_v7_error` — no false positive when only ref set
- `lifecycle_v7_only_inline_lifecycle_no_v7_error` — no false positive when only inline set

#### Milestone gate

1. All acceptance criteria checked.
2. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `git commit -m "feat(validation): add V7 mutual-exclusion check for lifecycle/lifecycleRef (#239)"`

---

### Phase 3: Add V9 for inline TypeLifecycle at validation time

**Goal:** `validate_repository` runs full V9 structural integrity and `initialState`/`isInitial` key-match checks on every inline `TypeLifecycle` block, the same checks already applied to standalone `Lifecycle` definitions.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `validation.rs`, add `use srs_core::validation::lifecycle::validate_type_lifecycle_v9;` to imports (check if already imported; add only if missing).
- [ ] In `validate_vocabulary_invariants`, after the existing standalone-lifecycle V9 loop, add a new loop over `pkg.record_types()`:
  ```rust
  for rt in pkg.record_types() {
      // Skip if lifecycle is None or if V7 already fired (both set)
      if rt.lifecycle_ref.is_some() { continue; }
      if let Some(inline_lc) = &rt.lifecycle {
          // V9 structural checks (initial count, active initial, final states, transitions, duplicate IDs)
          for diag in validate_type_lifecycle_v9(&inline_lc.states, &inline_lc.transitions, &rt.name) {
              diagnostics.push(ValidationDiagnostic {
                  severity: DiagnosticSeverity::Error,
                  relative_path: "package/package.json".to_string(),
                  schema_id: None,
                  message: diag.message,
              });
          }
          // V9: initialState field must match the isInitial state's key
          let initial_states: Vec<_> = inline_lc.states.iter()
              .filter(|s| s.is_initial == Some(true))
              .collect();
          if initial_states.len() == 1 {
              if initial_states[0].key != inline_lc.initial_state {
                  diagnostics.push(ValidationDiagnostic {
                      severity: DiagnosticSeverity::Error,
                      relative_path: "package/package.json".to_string(),
                      schema_id: None,
                      message: format!(
                          "V9: inline lifecycle on type '{}' initialState '{}' does not match isInitial state key '{}'",
                          rt.name, inline_lc.initial_state, initial_states[0].key
                      ),
                  });
              }
          }
      }
  }
  ```
  > Note: skip types where `lifecycle_ref.is_some()` to avoid double-reporting when both are set (V7 already fires in that case). The `pkg.record_types()` accessor is `&self.record_types` returning `&[RecordType]`.

- [ ] Add test `lifecycle_v9_inline_no_initial_state_produces_error`:
  - Package-only repo; type with `lifecycle` inline that has no `isInitial: true` state.
  - Assert `DiagnosticSeverity::Error` with "no initial state" in message.
- [ ] Add test `lifecycle_v9_inline_multiple_initial_states_produces_error`:
  - Inline lifecycle with two `isInitial: true` states.
  - Assert error with "initial states" in message.
- [ ] Add test `lifecycle_v9_inline_unknown_transition_state_produces_error`:
  - Inline lifecycle where a transition references a state key not in `states[]`.
  - Assert error.
- [ ] Add test `lifecycle_v9_inline_initial_state_mismatch_produces_error`:
  - Inline lifecycle where `initialState` field differs from the `isInitial` state's `key`.
  - Assert V9 error with "initialState" and "isInitial" in message.
- [ ] Add test `lifecycle_v9_inline_valid_no_error`:
  - Valid inline lifecycle (one `isInitial`, `initialState` matches, valid transitions).
  - Assert no lifecycle diagnostics.

#### Acceptance Criteria

- [ ] All five new inline-lifecycle tests pass.
- [ ] Existing `record_v8_*` tests still pass (V8 record-level check unaffected).
- [ ] Existing standalone-lifecycle V9 tests still pass.
- [ ] `cargo test -p srs-repository` — zero failures.
- [ ] `cargo test -p srs-core` — zero failures (no core changes expected).

#### Testing

```bash
cargo test -p srs-repository lifecycle_v9_inline
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `lifecycle_v9_inline_no_initial_state_produces_error`
- `lifecycle_v9_inline_multiple_initial_states_produces_error`
- `lifecycle_v9_inline_unknown_transition_state_produces_error`
- `lifecycle_v9_inline_initial_state_mismatch_produces_error`
- `lifecycle_v9_inline_valid_no_error`

#### Milestone gate

1. All acceptance criteria checked.
2. Run:

```bash
cargo test -p srs-repository
cargo test -p srs-core
cargo clippy -p srs-repository -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `git commit -m "feat(validation): add V9 structural enforcement for inline TypeLifecycle (#239)"`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] No diagnostic labeled "V7" in the codebase refers to lifecycleRef resolution (relabeled to V8)
- [ ] `validate_repository` produces a V7 error when a type has both `lifecycle` and `lifecycleRef`
- [ ] `validate_repository` produces V9 errors for invalid inline `TypeLifecycle` blocks

## Coordination Rules

- All writes are in `crates/srs-repository/src/validation.rs`. One file, one phase at a time.
- Workers return changed line numbers and a short behaviour summary when done.
- Lead Integrator confirms no regressions in srs-core after each phase.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests pass, update plan checkboxes, then commit.

## Assumptions

- `pkg.record_types()` returns `&[RecordType]` — confirmed from `package.rs:171`.
- `validate_type_lifecycle_v9` is already imported and available in `srs-core`; adding the use in `validation.rs` is sufficient.
- `TypeLifecycle.initial_state` is the `initialState` field (confirmed from `record_type.rs:53`).
- Inline lifecycle V9 is already enforced at **write time** in `record_store.rs`; this plan closes the **read/validate** path gap.
