# Plan: `record validate` — no-write record validation for editor preflight

> Tracks srs-rust#64. Unblocks srs-vscode#14.

## Summary

Multi-record editors (the guide editor, srs-vscode#14) save records in a loop via `record update` per section. To close the partial-save hole they must validate every section up front and only write if all pass — but today there is **no no-write validation path**: `record create`/`record update` always persist, and while `blueprint`/`container`/`protocol`/`repo` have `*-validate` commands, an unsaved record input does not. This plan adds `srs record validate`, which reads a self-contained record input from stdin, validates its field values against the resolved `typeId@typeVersion` (unknown/extra fields, missing required, type/enum mismatches — exactly the rules `record update` runs before persist), and returns diagnostics **without writing anything**.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this session) |
| Repository Worker | claude (this session) |
| CLI Worker | claude (this session) |
| Verification | Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation logic lives in a `srs-repository` service fn (`validate_record_input`) reusing `srs_core::validation::record::validate_record`; the handler is arg/stdin-parse → one service call → output. | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New command output is a named payload struct + committed golden schema, mirroring `ContainerValidatePayload`. | accepted |

No new ADRs — this implements ADR-010/011 with existing infrastructure.

**Input-shape decision (recorded, not open):** `record validate` reads a **self-contained** `ValidateRecordInput { typeId, typeVersion, fieldValues, groupValues?, tags? }` from stdin (no positional/flag args). This mirrors `blueprint validate` / `container validate` (read the full thing from stdin) and directly serves the editor, which already holds each record's `typeId` (UUID) and `typeVersion`. It resolves via `package.resolve_type(type_id, version)` — the same call `create_record`/`update_record` use. `fieldValues`/`groupValues`/`tags` reuse the existing `srs-core` types and the `CreateRecordInput` field conventions.

**No-write guarantee:** the service performs only reads (`load_package`, `resolve_type`, `effective_fields`, `validate_record`). It never calls `write_record`/`write_manifest`. A roundtrip test asserts repo state is byte-identical before/after.

---

## Contracts

### CLI output contract (ADR-011)

**New command added** → add `RecordValidatePayload { ok: bool, errors: Vec<String> }` to `crates/srs-cli/src/payload.rs` (identical shape to `ContainerValidatePayload`), wire the handler with the container-validate branch pattern, register `record-validate` in `generate-schemas.rs`, run `cargo run --bin generate-schemas`, commit `crates/srs-cli/schemas/payload/record-validate.json`.

Verification: `cargo test --test payload_contracts` passes.

### Entity schema sync (check-schema-sync.sh)

**No** — no JSON Schema files under `srs/docs/schema/2.0/` are touched.

---

## Scope

- Add `validate_record_input(store, ValidateRecordInput) -> Result<RecordValidateReport, RepositoryError>` to `crates/srs-repository/src/record_store.rs`. `RecordValidateReport { ok: bool, errors: Vec<String> }`. Resolve the type (unresolved → `{ ok:false, errors:["type not found: <id>@<v>"] }`), build an in-memory `Record`, resolve `effective_fields`, call `validate_record`; `Ok(())` → `{ ok:true, errors:[] }`, `Err(e)` → `{ ok:false, errors:[e.to_string()] }`. No writes.
- Add `ValidateRecordInput` (Deserialize, camelCase) to `record_store.rs` next to `CreateRecordInput`.
- Add `Validate` variant to `RecordCommand` (`record validate`, reads stdin, no args).
- Add `cmd_record_validate` handler to `crates/srs-cli/src/commands/record.rs`: parse stdin → `validate_record_input` → branch like `cmd_validate` in container.rs (`report.ok` → serialize `RecordValidatePayload`; else `output::err("record validate", report.errors)`).
- Add `RecordValidatePayload` to `payload.rs`; register + commit golden.
- Tests in `record_store.rs` (service unit tests) and `crates/srs-cli/tests/integration_tests.rs` (CLI + roundtrip).
- Doc: `srs/srs-usage.md` (Stage 7.5, committed in the `srs` repo).

**Out of scope:**

- Collecting *all* diagnostics in one call. `validate_record` is fail-fast (returns the first `CoreError`), so `errors` carries one diagnostic per call. Refactoring `srs-core` to accumulate diagnostics is a separate enhancement (file as follow-up).
- A `--dry-run` flag on `record create`/`record update` (Option B). Option A (`record validate`) is the chosen surface.
- Re-running the Type's lifecycle-definition invariants (V4/V5/V9) — those validate the *type*, not the record input, and are covered by `repo validate` / type creation.
- Validating against an existing stored record id (update-in-place preflight). The self-contained input already covers the editor's need; an id-based variant can be added later if required.

---

## Phases

### Phase 1: Service fn + input/report types (srs-repository)

**Goal:** `validate_record_input` validates a record input and returns a report, writing nothing; unit-tested against `MemoryStore`.

**Agent:** Repository Worker

#### Tasks

- [x] Add `ValidateRecordInput { type_id, type_version, field_values, group_values?, tags? }` (Deserialize, `rename_all = "camelCase"`) to `record_store.rs`.
- [x] Add `RecordValidateReport { ok: bool, errors: Vec<String> }`.
- [x] Implement `validate_record_input` per Scope. Reuse `package.resolve_type`, `package.effective_fields`, `validate_record`. No `write_*` calls.

#### Acceptance Criteria

- [x] Valid input → `{ ok:true, errors:[] }`.
- [x] Missing required field → `{ ok:false, errors:[..] }` with a non-empty diagnostic.
- [x] Unknown/extra field id → `{ ok:false, .. }`.
- [x] Bad enum / type mismatch value → `{ ok:false, .. }`.
- [x] Unresolved `typeId@typeVersion` → `{ ok:false, errors:["type not found: .."] }` (no panic, no error propagation).
- [x] Repo state unchanged after the call (no files written).

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests (in `record_store.rs` `#[cfg(test)]`, against `MemoryStore`):

- `validate_record_input_accepts_valid` — clean input → ok.
- `validate_record_input_rejects_missing_required` — ok:false + diagnostic.
- `validate_record_input_rejects_unknown_type` — type-not-found report.
- `validate_record_input_does_not_write` — capture store state (record count / manifest) before and after; assert unchanged.

#### Milestone gate

`cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings`; mark checkboxes; commit `(#64)`.

---

### Phase 2: CLI command + payload + golden (srs-cli)

**Goal:** `srs record validate` reads stdin and returns the validate envelope; golden committed; contract tests green.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `Validate` variant to `RecordCommand` ("Validate a record input from stdin without persisting (preflight)").
- [ ] Add `RecordValidatePayload { ok: bool, errors: Vec<String> }` to `payload.rs`.
- [ ] Add `cmd_record_validate` to `commands/record.rs`: `serde_json::from_reader(io::stdin())` → `validate_record_input` → branch like container's `cmd_validate`.
- [ ] Wire the variant in `record` dispatch.
- [ ] Register `write_schema!("record-validate", RecordValidatePayload)`; regenerate; commit golden.

#### Acceptance Criteria

- [ ] `record validate` on valid stdin → `ok:true`, `command == "record validate"`, `payload.ok == true`, `payload.errors == []`.
- [ ] Invalid stdin → envelope `ok:false`, diagnostics carry the error.
- [ ] Golden `record-validate.json` committed; `payload_contracts` passes.

#### Testing

```bash
cargo test -p srs
cargo clippy -p srs -- -D warnings
cargo test --test payload_contracts
```

Specific tests (in `integration_tests.rs`):

- `record_validate_accepts_valid_input` — ok:true envelope.
- `record_validate_rejects_invalid_input` — ok:false + diagnostic.
- `record_validate_does_not_persist` — record count via `record list` identical before/after a validate call (CLI-level roundtrip proof).

#### Milestone gate

`cargo test -p srs && cargo clippy -p srs -- -D warnings && cargo test --test payload_contracts`; mark checkboxes; commit `(#64)`.

---

## Final Acceptance

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] CLI integration tests pass.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — no-op pass).
- [ ] Repo state provably unchanged by a validate call (roundtrip test).
- [ ] `srs record validate` works end-to-end against a real repo.
- [ ] `srs/srs-usage.md` documents the command (Stage 7.5).

## Coordination Rules

- Single agent (this session). Phase 1 (srs-repository) before Phase 2 (srs-cli) — the handler depends on the service fn.
- No business logic in the CLI handler beyond the container-validate branch shape.

## Assumptions

- `srs_core::validation::record::validate_record` is the authoritative field-value check `record create`/`update` already run; reusing it keeps `validate` consistent with persist-time validation (no second validation implementation).
- Fail-fast single-diagnostic output is acceptable for v1 (matches create/update behaviour). Accumulating all diagnostics is deferred.
