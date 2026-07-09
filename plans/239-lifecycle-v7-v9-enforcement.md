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
- Add V9 inline-lifecycle validation in `validate_vocabulary_invariants`: for each `RecordType` whose `lifecycle` is `Some(TypeLifecycle{…})`, call `validate_type_lifecycle_v9` and also check that `TypeLifecycle.initial_state` matches the `isInitial` state key. The V7 and V9 inline checks are implemented in a **single loop** over `pkg.record_types`.
- Add targeted unit tests (in `validation.rs` `#[cfg(test)]` module) for all three new / fixed paths.

**Out of scope:**
- WASM bindings or CLI command changes (validation surface is unchanged from the outside).
- Record-creation-time lifecycle validation in `record_store.rs` (already enforced there via `validate_type_lifecycle`; no gap to close).
- Inline lifecycle `initialState` field definition — it already exists on `TypeLifecycle`.
- Any spec changes to `srs/`.

---

## Notes on label numbering

The existing codebase uses "V8" for two distinct check categories:
- **Record-level** (`validation.rs:502`): `"V8: record '{}' lifecycleState '{}' is not a valid state key..."` — guards a record's `lifecycleState` field against its type's lifecycle states.
- **Package-level** (added in Phase 1): `"V8: type '{}' lifecycleRef '{}' does not resolve..."` — guards a RecordType's `lifecycleRef` UUID against installed lifecycles.

Both are legitimately "V8" per the spec invariant numbering (different check categories, different subjects). Test assertions must include a specific substring (`"lifecycleRef"` or `"lifecycleState"`) to be unambiguous.

---

## Phases

### Phase 1: Rename "V7" → "V8" for the lifecycleRef-resolution check

**Goal:** The existing lifecycleRef-resolution diagnostic message says "V7" but the spec numbers it V8; after this phase, it says "V8", and all test assertions match. The cross-reference comment at `validation.rs:483` is also updated.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `validation.rs` `validate_vocabulary_invariants` (~line 948), change `"V7: type '{}' lifecycleRef '{}' does not resolve..."` to `"V8: type '{}' lifecycleRef '{}' does not resolve..."`.
- [x] Update `validation.rs:483` comment from `// V7 will report it` to `// V8 will report it`.
- [x] Rename test `vocabulary_v7_missing_lifecycle_ref_produces_error` → `vocabulary_v8_missing_lifecycle_ref_produces_error`; update assertion to `d.message.contains("V8") && d.message.contains("lifecycleRef")`.
- [x] Rename test `vocabulary_v7_resolved_lifecycle_ref_no_error` → `vocabulary_v8_resolved_lifecycle_ref_no_error`; update filter to `d.message.contains("V8") && d.message.contains("lifecycleRef")`.
- [x] Rename test `dangling_lifecycle_ref_produces_clear_v7_diagnostic` → `dangling_lifecycle_ref_produces_clear_v8_diagnostic`; update assertion from `d.message.contains("V7")` to `d.message.contains("V8") && d.message.contains("lifecycleRef")`.

#### Acceptance Criteria

- [x] `cargo test -p srs-repository 2>&1 | grep -E "FAILED|error"` — zero failures.
- [x] `grep -n '"V7.*lifecycleRef\|V7.*does not resolve' crates/srs-repository/src/validation.rs` — zero hits (old label gone).
- [x] `grep -n '"V8.*lifecycleRef\|V8.*does not resolve' crates/srs-repository/src/validation.rs` — one hit (new label present).

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify:
- `vocabulary_v8_missing_lifecycle_ref_produces_error` — V8 error emitted for dangling lifecycleRef
- `vocabulary_v8_resolved_lifecycle_ref_no_error` — no V8/lifecycleRef error when ref resolves
- `dangling_lifecycle_ref_produces_clear_v8_diagnostic` — error message contains "V8" and "lifecycleRef" and the dangling UUID

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

### Phase 2 & 3: Add V7 mutual-exclusion + V9 inline TypeLifecycle (single loop)

**Goal:** `validate_repository` emits (a) a `DiagnosticSeverity::Error` when a `RecordType` declares both `lifecycle` and `lifecycle_ref` (V7), and (b) V9 structural + `initialState`/`isInitial` key-match errors for inline `TypeLifecycle` blocks. Both checks are implemented in a single loop over `pkg.record_types`.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `validation.rs`, confirm `validate_type_lifecycle_v9` is already in scope via the existing `use srs_core::validation::lifecycle::{validate_lifecycle, LifecycleDiagnosticSeverity};` import. Add `validate_type_lifecycle_v9` to that import if not already present.
- [x] In `validate_vocabulary_invariants`, replace the existing `// V7: every type.lifecycleRef` loop (lines 939–954) with a single expanded loop:

  ```rust
  // V7: mutual exclusion (lifecycle and lifecycleRef both set)
  // V8: every type.lifecycleRef must resolve to an installed Lifecycle UUID
  // V9: structural integrity for inline TypeLifecycle
  for rt in &pkg.record_types {
      // V7: mutual exclusion
      if rt.lifecycle.is_some() && rt.lifecycle_ref.is_some() {
          diagnostics.push(ValidationDiagnostic {
              severity: DiagnosticSeverity::Error,
              relative_path: "package/package.json".to_string(),
              schema_id: None,
              message: format!(
                  "V7: type '{}' declares both 'lifecycle' and 'lifecycleRef'; exactly one is allowed",
                  rt.name
              ),
          });
          // Skip V8 and V9 for this type — V7 already fired
          continue;
      }

      // V8: lifecycleRef must resolve
      if let Some(ref_id) = &rt.lifecycle_ref {
          if !pkg.lifecycles.iter().any(|lc| &lc.id == ref_id) {
              diagnostics.push(ValidationDiagnostic {
                  severity: DiagnosticSeverity::Error,
                  relative_path: "package/package.json".to_string(),
                  schema_id: None,
                  message: format!(
                      "V8: type '{}' lifecycleRef '{}' does not resolve to an installed Lifecycle",
                      rt.name, ref_id
                  ),
              });
          }
      }

      // V9: structural checks on inline TypeLifecycle
      if let Some(inline_lc) = &rt.lifecycle {
          for diag in validate_type_lifecycle_v9(&inline_lc.states, &inline_lc.transitions, &rt.name) {
              let severity = match diag.severity {
                  LifecycleDiagnosticSeverity::Error => DiagnosticSeverity::Error,
              };
              diagnostics.push(ValidationDiagnostic {
                  severity,
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

- [x] Add test `vocabulary_v7_both_lifecycle_and_ref_produces_error`:
  - Package-only repo; type with both inline lifecycle AND lifecycleRef set.
  - Assert `DiagnosticSeverity::Error` with `d.message.contains("V7")` and `d.message.contains("both")`.
- [x] Add test `vocabulary_v7_only_lifecycle_ref_no_v7_error` — type with only `lifecycleRef` → no V7 error.
- [x] Add test `vocabulary_v7_only_inline_lifecycle_no_v7_error` — type with only inline `lifecycle` → no V7 error.
- [x] Add test `vocabulary_v7_both_set_no_v9_error` — type with both set produces V7 but NOT V9 error (skip guard works).
- [x] Add test `vocabulary_v9_inline_no_initial_state_produces_error`:
  - Package-only repo; type with `lifecycle` inline that has no `isInitial: true` state.
  - Assert `DiagnosticSeverity::Error` with "no initial state" in message.
- [x] Add test `vocabulary_v9_inline_multiple_initial_states_produces_error`:
  - Inline lifecycle with two `isInitial: true` states.
  - Assert error with "initial states" in message.
- [x] Add test `vocabulary_v9_inline_unknown_transition_state_produces_error`:
  - Inline lifecycle where a transition references a state key not in `states[]`.
  - Assert error.
- [x] Add test `vocabulary_v9_inline_initial_state_mismatch_produces_error`:
  - Inline lifecycle where `initialState` field differs from the `isInitial` state's `key`.
  - Assert V9 error with "initialState" and "isInitial" in message.
- [x] Add test `vocabulary_v9_inline_valid_no_error`:
  - Valid inline lifecycle (one `isInitial`, `initialState` matches, valid transitions).
  - Assert no V7 or V9 lifecycle diagnostics.

#### Acceptance Criteria

- [x] Test `vocabulary_v7_both_lifecycle_and_ref_produces_error` passes.
- [x] Tests `vocabulary_v7_only_lifecycle_ref_no_v7_error` and `vocabulary_v7_only_inline_lifecycle_no_v7_error` pass (no false positives).
- [x] Test `vocabulary_v7_both_set_no_v9_error` passes (V7 fires, V9 skipped).
- [x] All five inline-lifecycle V9 tests pass.
- [x] Existing `record_v8_*` tests still pass (V8 record-level check unaffected).
- [x] Existing standalone-lifecycle V9 tests still pass.
- [x] `cargo test -p srs-repository` — zero failures.
- [x] `cargo test -p srs-core` — zero failures (no core changes expected).

#### Testing

```bash
cargo test -p srs-repository vocabulary_v7
cargo test -p srs-repository vocabulary_v9
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `vocabulary_v7_both_lifecycle_and_ref_produces_error` — mutual-exclusion error fires
- `vocabulary_v7_only_lifecycle_ref_no_v7_error` — no false positive when only ref set
- `vocabulary_v7_only_inline_lifecycle_no_v7_error` — no false positive when only inline set
- `vocabulary_v7_both_set_no_v9_error` — V7 fires but V9 is suppressed
- `vocabulary_v9_inline_no_initial_state_produces_error`
- `vocabulary_v9_inline_multiple_initial_states_produces_error`
- `vocabulary_v9_inline_unknown_transition_state_produces_error`
- `vocabulary_v9_inline_initial_state_mismatch_produces_error`
- `vocabulary_v9_inline_valid_no_error`

#### Milestone gate

1. All acceptance criteria checked.
2. Run:

```bash
cargo test -p srs-repository
cargo test -p srs-core
cargo clippy -p srs-repository -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `git commit -m "feat(validation): add V7 mutual-exclusion and V9 inline TypeLifecycle checks (#239)"`

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

- `pkg.record_types()` returns `&[RecordType]` — confirmed from `package.rs:171`. The existing loop at `validation.rs:940` uses `&pkg.record_types` (direct field access) — match this style.
- `validate_type_lifecycle_v9(states, transitions, lifecycle_name)` is available in `srs-core::validation::lifecycle` (used in `record_store.rs`). It creates a temporary `Lifecycle` with `initial_state: String::new()` and calls `validate_lifecycle`. It does NOT check the `initial_state` field against the `isInitial` state's key — that key-match check must be added explicitly in Phase 2/3 inline in `validate_vocabulary_invariants`.
- `TypeLifecycle.initial_state` is the `initialState` field (confirmed from `record_type.rs:53`).
- Inline lifecycle V9 is already enforced at **write time** in `record_store.rs`; this plan closes the **read/validate** path gap.
