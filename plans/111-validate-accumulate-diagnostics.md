# Plan: Accumulate all record-validation diagnostics

> Tracks srs-rust#111. Follow-up from #64.

## Summary

`srs_core::validation::record::validate_record` returns on the *first* `CoreError`, so `srs record validate` (and create/update) surface at most one problem per call. For the editor-preflight use case (srs-vscode#14) an author wants every problem in a record input at once, not one-fix-revalidate at a time. This plan adds a diagnostic-**accumulating** validator in `srs-core` and surfaces the full list through `record validate`, while leaving the fail-fast create/update/repo-validate paths behaviourally identical.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this session) |
| Core Worker | claude (this session) |
| Repository Worker | claude (this session) |
| Verification | Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation stays in `srs-core`; `srs-repository::validate_record_input` surfaces it. No CLI logic. | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload change — `RecordValidatePayload.errors` is already `Vec<String>`. | accepted |

No new ADRs.

**DRY decision (recorded, not open):** add `validate_record_all(record, record_type, effective_fields) -> Vec<CoreError>` as the single source of validation truth (pushes diagnostics in the existing check order). Reimplement `validate_record` as a thin fail-fast wrapper: `validate_record_all(...).into_iter().next().map_or(Ok(()), Err)`. Because the collector pushes in the **same order** the old function returned, the first collected error equals the old first-returned error — so every fail-fast caller (create, update, repo validate, package tests) is behaviourally unchanged. This avoids duplicating ~100 lines of validation logic.

---

## Contracts

### CLI output contract (ADR-011)

**No payload change.** `RecordValidatePayload { ok, errors: Vec<String> }` already carries a list; the golden `record-validate.json` is unchanged. `record validate` simply populates `errors` with all diagnostics instead of one. `payload_contracts` must still pass (no regeneration needed).

### Entity schema sync

**No** — no `srs/docs/schema/2.0/` files touched.

---

## Scope

- `crates/srs-core/src/validation/record.rs`: add `validate_record_all(...) -> Vec<CoreError>` (same checks as today, `push` instead of early `return Err`). Reimplement `validate_record` to delegate and return the first element. Order of checks unchanged.
- `crates/srs-repository/src/record_store.rs`: `validate_record_input` calls `validate_record_all`, mapping every `CoreError` to a string into `RecordValidateReport.errors` (`ok = errors.is_empty()`). Create/update keep calling `validate_record` (fail-fast) unchanged.
- Tests: srs-core unit test (input with ≥2 independent violations → ≥2 diagnostics, in stable order); srs-repository service test (`validate_record_input` with ≥2 violations → ≥2 `errors`); confirm existing fail-fast tests still pass.

**Out of scope:**

- Enum `allowedValues` / `valueType` conformance validation — still not validated anywhere (separate gap; `validate` keeps parity with the write path).
- Any change to create/update/repo-validate behaviour (they stay fail-fast).
- CLI/payload/schema changes.

---

## Phases

### Phase 1: srs-core accumulating validator

**Goal:** `validate_record_all` returns every diagnostic; `validate_record` delegates and is behaviourally identical.

**Agent:** Core Worker

#### Tasks

- [x] Add `pub fn validate_record_all(record, record_type, effective_fields) -> Vec<CoreError>` — the existing checks (unknown fields → missing required → non-repeatable entries → repeatable min/max → field groups → tags → lifecycle state) rewritten to `push` into a `Vec` and continue, preserving order.
- [x] Reimplement `validate_record` as `match validate_record_all(...).into_iter().next() { Some(e) => Err(e), None => Ok(()) }`.

#### Acceptance Criteria

- [x] All existing `validate_record` unit tests pass unchanged (first error identical).
- [x] `validate_record_all` on a record with a missing-required field **and** an unknown field returns **both** diagnostics, in check order.
- [x] `validate_record_all` on a clean record returns an empty `Vec`.

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Specific tests:
- `validate_record_all_collects_multiple` — missing-required + unknown-field → 2 errors, order = [UnknownField, MissingRequiredField] per check order.
- `validate_record_all_empty_when_valid` — clean record → empty vec.
- (existing `validate_record_*` fail-fast tests remain and pass.)

#### Milestone gate

`cargo test -p srs-core && cargo clippy -p srs-core -- -D warnings`; mark checkboxes; commit `(#111)`.

---

### Phase 2: surface all diagnostics through record validate

**Goal:** `validate_record_input` reports every diagnostic; create/update unchanged.

**Agent:** Repository Worker

#### Tasks

- [ ] In `validate_record_input` (`record_store.rs`), replace the single `validate_record` call with `validate_record_all`, mapping each `CoreError` via `to_string()` into `RecordValidateReport.errors`; `ok = errors.is_empty()`. Keep the type-not-found early return as-is.
- [ ] Leave `create_record`/`update_record` calling `validate_record` (fail-fast) untouched.

#### Acceptance Criteria

- [ ] `validate_record_input` with ≥2 independent violations returns a report with ≥2 `errors`.
- [ ] A clean input still returns `{ ok:true, errors:[] }`; a single violation still returns one error.
- [ ] Repo state unchanged (no writes) — existing `validate_record_input_does_not_write` still passes.

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs
cargo clippy -- -D warnings
cargo test --test payload_contracts
```

Specific tests:
- `validate_record_input_collects_multiple_diagnostics` — input missing a required field and carrying an unknown field → `report.errors.len() >= 2`.

#### Milestone gate

`cargo test -p srs-repository && cargo test -p srs && cargo clippy -- -D warnings`; mark checkboxes; commit `(#111)`.

---

## Final Acceptance

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] `cargo test --test payload_contracts` passes (no payload change).
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed).
- [ ] `record validate` with multiple problems reports them all; create/update/repo-validate behaviour unchanged.

## Coordination Rules

- Phase 1 (srs-core) before Phase 2 (srs-repository).
- No CLI changes; no payload regeneration.

## Assumptions

- The check order in `validate_record_all` matches today's `validate_record` return order, so fail-fast callers see the identical first error.
- One diagnostic per distinct violation is sufficient (no need to, e.g., report every unknown field *and* keep going within a single check — though the collector naturally can; tests assert ≥2 across different check categories).
