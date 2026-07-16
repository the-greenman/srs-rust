# Plan: Validate invariant-number uniqueness in `repo validate`

## Summary

Three merged RFCs ended up with duplicate invariant numbers on master, undetected for days (srs-rust#555). The core validator doesn't check cross-record uniqueness for the `invariant-number` field on `com.semanticops.spec/invariant` records. This plan adds that check to `validate_repository` in `srs-repository`, ensuring any SRS repository that contains spec-type invariant records will emit an Error diagnostic when the same `I-NN` number appears on more than one record.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | Claude (this session) |
| Verification | Claude (this session) |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation logic belongs in `srs-repository`, not the CLI | accepted |
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library-first: new rule added to the service layer only | accepted |

No new ADRs needed — this plan adds a diagnostic rule following the exact pattern of existing cross-record checks (RFC-013 I-80, vocabulary invariants). The field ID `1a000020-0000-4000-a000-000000000020` is a stable canonical ID from the spec; hardcoding it with a named constant matches the `STATEMENT_FIELD_ID`/`TITLE_FIELD_ID` pattern in `core_purpose.rs`.

## Contracts

### CLI output contract (ADR-011)

No new/changed CLI commands or payload structs. `repo validate` already returns diagnostics; this plan adds a new diagnostic message string only. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are touched.

---

## Scope

- Add a cross-record uniqueness check for field `1a000020-0000-4000-a000-000000000020` (invariant-number) on `com.semanticops.spec/invariant` tier-2 records in `validate_repository`.
- Emit a `DiagnosticSeverity::Error` for every record whose invariant-number duplicates an earlier record's number, citing both the current path and the first-seen path.
- Add tests covering: duplicate numbers → error; distinct numbers → clean; non-spec records → no false positive.

**Out of scope:**
- $schema stamping on record create (tracked separately in srs-rust#551).
- The `--dir` behavior for `record create` (CLI already has `--dir`; usage guidance is a doc concern, not a code change).
- WASM binding changes (no new public binding surface).

---

## Phases

### Phase 1: Add invariant-number uniqueness check

**Goal:** `validate_repository` emits an Error diagnostic for duplicate `invariant-number` values on `com.semanticops.spec/invariant` tier-2 records.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add module-level constant `SPEC_INVARIANT_NUMBER_FIELD_ID: &str = "1a000020-0000-4000-a000-000000000020"` in `crates/srs-repository/src/validation.rs` (near the top, after imports).
- [ ] Declare `let mut seen_invariant_numbers: HashMap<String, (String, String)> = HashMap::new();` before the `for entry in &manifest.instance_index` loop in `validate_repository`.
- [ ] Inside the tier-2 `Ok(record)` branch (after existing field-value validation), add: if `record.type_namespace == "com.semanticops.spec"` and `record.type_name == "invariant"`, extract the string value of field `SPEC_INVARIANT_NUMBER_FIELD_ID` via `record.find_field_value(...)`, and insert into `seen_invariant_numbers`. If the number is already present, push an Error diagnostic onto `diagnostics` citing both paths and instance IDs.
- [ ] After the main instance loop (before the Inv-43 cross-package check), add a comment block `// --- spec/invariant number uniqueness ---` and emit one Error diagnostic per path that holds a duplicate number, referencing the first-seen path and instance ID.
- [ ] Ensure `HashMap` is already imported (it is, via `std::collections::HashMap` at line 19).

#### Acceptance Criteria

- [ ] `srs repo validate --repo ../srs/srs` exits with 0 errors on the current (clean) spec repo.
- [ ] A test repo with two `com.semanticops.spec/invariant` records sharing the same `I-NN` value emits exactly one Error diagnostic (on the second record) citing the first.
- [ ] A test repo with two `com.semanticops.spec/invariant` records with distinct numbers emits no new diagnostics.
- [ ] A test repo with two tier-2 records of a non-spec type having the same value in field `1a000020-*` emits no false-positive diagnostics.

#### Testing

```bash
cargo test -p srs-repository validate_invariant
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write (in `crates/srs-repository/src/validation.rs`, in the existing test module):

- `validate_invariant_number_uniqueness_duplicate_emits_error` — two spec/invariant records with the same `I-NN` number → exactly one Error diagnostic with "duplicate invariant number" in the message.
- `validate_invariant_number_uniqueness_distinct_numbers_pass` — two spec/invariant records with distinct numbers → zero diagnostics with "duplicate invariant number".
- `validate_invariant_number_uniqueness_non_spec_type_no_false_positive` — two tier-2 records of a non-spec type with the same field value → zero diagnostics with "duplicate invariant number".

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists and passes.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark completed task checkboxes `[x]`.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs repo validate --repo ../srs/srs --pretty` shows 0 errors on the current spec repo
- [ ] Three new tests exist and pass (duplicate/distinct/non-spec-type)

## Coordination Rules

- Single-agent pipeline — no handoff needed.
- Stage 6 rebase + full `cargo test` before PR.

## Assumptions

- The spec repo at `../srs/srs` is accessible from the srs-rust worktree for dogfood validation.
- The field ID `1a000020-0000-4000-a000-000000000020` is stable (verified across all 89 invariant records in the spec repo).
- The existing tier-2 record loop in `validate_repository` can be extended without restructuring — the new collection happens inside the already-parsed `Ok(record)` arm.
