# Plan: `type schema` command — emit JSON Schema for a record's fieldValues

> Tracks issue #60. Save location: `plans/type-schema-command.md`.

## Summary

Add `srs type schema <typeId> [--type-version N]`, a CLI command that resolves a Type plus its referenced Fields and emits a **draft-07 JSON Schema describing a single record's `fieldValues`**. This lets a JSON-Schema-aware editor generate a form without re-implementing the field→widget mapping (replacing the hardcoded field-UUID constants in `srs-vscode/src/webview/guides/guideTypes.ts`). It is a **pure projection** over already-loaded `RecordType` + `Field` data — no new data model, no write path, no new file I/O primitives.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator (`agents.md#lead-integrator`) |
| Service logic | Repository Service Worker (`agents.md#repository-service-worker`) |
| CLI handler + payload | CLI Worker (`agents.md#cli-worker`) |
| Verification | Verification Agent (`agents.md#verification-agent`) |

See [agents.md](agents.md) for role definitions. No new agent role is required — the work falls entirely within existing crate-scoped roles.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service takes a typed input struct, does all logic, returns a typed result struct | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Output is a named payload struct in `payload.rs`; golden JSON Schema committed | accepted |
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Projection logic lives in `srs-repository`, not the CLI | accepted |

**No new architectural decisions.** This plan implements ADR-010/011/001 with an additive, read-only command. The one design choice worth recording inline (not ADR-worthy): the emitted schema describes the **`fieldValues` object keyed by field `name`** (not field UUID), because the consumer is a form generator that renders human-facing field names; UUID→name is recoverable from the same payload via `x-srs-field-id`. This is a local convention of one command's output, reversible by changing the struct, so it does not meet the ADR-TEMPLATE bar (new constraint / rejects a revisitable alternative / changes a prior decision).

---

## Contracts

### CLI output contract (ADR-011)

**New command added.** Actions required:
- Add `TypeSchemaPayload` to `crates/srs-cli/src/payload.rs`. The generated draft-07 schema is dynamic (`serde_json::Value`), so the payload wrapper uses `#[schemars(with = "serde_json::Value")]` on the schema field — matching the existing convention for embedded dynamic values (payload.rs lines using `#[schemars(with = "serde_json::Value")]`).
- Wire `cmd_type_schema` to call `output::serialize("type schema", ...)`.
- Run `cargo run --bin generate-schemas`; commit the new `crates/srs-cli/schemas/payload/type-schema.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

**No.** This plan adds no files under `srs/docs/schema/2.0/`. The emitted draft-07 schema is runtime output, not an SRS entity schema. No action required.

---

## Scope

In scope:

- New service function `type_schema(store, TypeSchemaInput) -> Result<TypeSchemaResult>` in a new module `crates/srs-repository/src/type_schema_service.rs`, exported from `lib.rs`.
- `TypeSchemaInput { type_id: String, type_version: Option<u32> }` and `TypeSchemaResult { schema: serde_json::Value, diagnostics: Vec<String> }` defined alongside the service. **Diagnostics contract:** non-fatal projection problems (dangling `field_id`, select/multiselect with no `allowed_values`) are collected into `diagnostics` and returned — matching the established `Vec<String>` diagnostics pattern in `blueprint_service.rs` / `protocol_service.rs` / `tree_service.rs`. The CLI surfaces them in the envelope's top-level `diagnostics[]` (per the CLI contract). A hard error (unresolvable Type) is an `Err`, not a diagnostic.
- Projection covering all 8 `ValueType` variants, `required[]`, `title`, `default`, `x-srs-order`, `x-srs-field-id`, and `aiGuidance`.
- `TypeCommand::Schema { id, type_version }` variant + `cmd_type_schema` handler in `crates/srs-cli/src/commands/record_type.rs`.
- `TypeSchemaPayload` in `payload.rs` + committed golden.
- Cross-store roundtrip test (memory store → schema) in `srs-repository`.

**Out of scope:**

- `blueprint schema` (whole-document composition) — issue #60 names it as a follow-up.
- The `srs-vscode` guide-editor refactor that consumes this — separate issue.
- `ext:repeatable-fields` array wrapping driven by `FieldAssignment.repeatable`. **Decision (reviewer-confirmed): deferred.** Baseline emits the scalar/select shapes from the mapping below. The consuming form editor's array-input needs are not yet specified; when they are, the projection extends inside the service without changing the handler or payload contract.
- `ext:type-inheritance` flattening of `extendsTypeId`. **Decision (reviewer-confirmed): deferred — hard boundary for this plan.** The service projects only the Type's own `fields[]`; inherited fields from an extended Type are not included. Flattening touches package-boundary semantics (ADR-009) and requires its own plan.

---

## Phases

### Phase 1: Projection service in `srs-repository`

**Goal:** `type_schema_service::type_schema` returns a correct draft-07 schema for any resolvable Type, fully tested against `MemoryStore`, with zero CLI involvement.

**Agent:** Repository Service Worker

#### Tasks

- [x] Create `crates/srs-repository/src/type_schema_service.rs` with `TypeSchemaInput`, `TypeSchemaResult`, and `pub fn type_schema`.
- [x] Resolve the Type: use `get_type_by_id` when `type_version` is `Some`, else `get_type_by_id_latest` (both in `package_service`). On `GetTypeResult::NotFound` return `RepositoryError` so the handler can surface a clean diagnostic.
- [x] For each `FieldAssignment` in `record_type.fields` (in `order`), resolve the Field via `get_field_by_id`; on `GetFieldResult::NotFound` push a diagnostic into `result.diagnostics` and skip that field (do not abort the whole schema).
- [x] Map `ValueType` → JSON Schema per the following mapping: string/text/number/boolean/date(`format:"date"`)/url(`format:"uri"`)/select(`enum`)/multiselect(`array` of `enum`). `text` carries `x-srs-widget: "textarea"`.
- [x] `select`/`multiselect` `enum` populated from `Field.allowed_values` (empty/None → omit `enum`, push a diagnostic into `result.diagnostics`).
- [x] Per-property annotations: `title` ← `FieldAssignment.display_label` else `Field.description`; `default` ← `Field.default_value`; `x-srs-order` ← `FieldAssignment.order`; `x-srs-field-id` ← `Field.id`; `description`/`x-srs-ai-guidance` ← `Field.ai_guidance` (string → `description`, object → `x-srs-ai-guidance`).
- [x] Top-level schema: `{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{<name>:…},"required":[<names of required assignments>],"additionalProperties":false}`.
- [x] Export the module and its public items from `crates/srs-repository/src/lib.rs`.

#### Acceptance Criteria

- [x] A Type exercising all 8 valueTypes produces a schema whose `properties` has one entry per resolvable field keyed by field `name`.
- [x] `select`/`multiselect` properties carry `enum` from `allowed_values`; multiselect is `{"type":"array","items":{"enum":[…]}}`.
- [x] Every `FieldAssignment` with `required: true` appears in top-level `required[]`; non-required do not.
- [x] Field order is recoverable from `x-srs-order` on each property.
- [x] `date`→`format:"date"`, `url`→`format:"uri"`, `text`→`x-srs-widget:"textarea"`.
- [x] Unknown `type_version` or unknown `type_id` returns an `Err` (not a partial schema).
- [x] A dangling `field_id` yields a diagnostic and is skipped, not a hard failure.

#### Testing

```bash
cargo test -p srs-repository type_schema
```

Specific tests to write (in `type_schema_service.rs` `#[cfg(test)]`, against `MemoryStore`):

- `type_schema_covers_all_value_types` — builds a Type + 8 Fields, asserts each property's shape.
- `type_schema_select_emits_enum` — asserts `enum` from `allowed_values` for select and multiselect.
- `type_schema_required_array` — required assignments land in `required[]`, others absent.
- `type_schema_order_recoverable` — `x-srs-order` matches `FieldAssignment.order`.
- `type_schema_title_prefers_display_label` — `display_label` wins when set; falls back to `Field.description` when absent.
- `type_schema_unknown_type_errors` — unknown id and unknown version both `Err`.
- `type_schema_dangling_field_skipped` — missing field_id → property absent, result still Ok, and the diagnostic appears in `result.diagnostics`.
- `type_schema_memory_roundtrip` — populate MemoryStore, call service, parse output as JSON (cross-store coverage per CLAUDE.md storage rules).

#### Milestone gate

1. Verify every acceptance criterion above.
2. Confirm all listed tests exist and pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Mark checkboxes `[x]` in this file.
5. `git commit` referencing #60.

---

### Phase 2: CLI command + payload contract

**Goal:** `srs type schema <typeId> [--type-version N]` returns the projection in the standard envelope, with a committed golden schema.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `Schema { id: String, #[arg(long)] type_version: Option<u32> }` to `TypeCommand` in `crates/srs-cli/src/commands/mod.rs` with a doc comment.
- [ ] Add `TypeSchemaPayload { #[schemars(with = "serde_json::Value")] schema: serde_json::Value }` to `crates/srs-cli/src/payload.rs`.
- [ ] Add `cmd_type_schema(ctx, id, type_version)` in `crates/srs-cli/src/commands/record_type.rs`: one `with_store` call to `type_schema_service::type_schema`, then serialize via `output::serialize("type schema", TypeSchemaPayload { schema })` while threading `result.diagnostics` into the envelope's top-level `diagnostics[]`; map a service `Err` to `output::err("type schema", …)`. Handler stays within the ADR-010 shape (parse args, one service call, wrap).
- [ ] Add the `TypeCommand::Schema { .. } =>` arm to the `dispatch` function in `crates/srs-cli/src/commands/record_type.rs`.
- [ ] `cargo run --bin generate-schemas`; stage `crates/srs-cli/schemas/payload/type-schema.json`.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- type schema <id> --repo <fixture>` emits `{ "ok": true, "command": "type schema", "payload": { "schema": { … draft-07 … } } }`.
- [ ] Unknown id → `{ "ok": false, … diagnostics … }`, exit code 0 (per ADR-011: exit 0 means "ran").
- [ ] `crates/srs-cli/schemas/payload/type-schema.json` exists and matches the generated output.
- [ ] The handler body is within the ADR-010 handler shape (no projection logic in the CLI).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

Specific tests:

- A CLI integration test (alongside the existing `type` command tests) asserting the envelope shape and a couple of property shapes against a fixture repo.
- `payload_contracts` golden test passes for `type-schema.json`.

#### Milestone gate

1. Verify acceptance criteria.
2. Confirm tests exist and pass.
3. Run:

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
```

4. Mark checkboxes `[x]`.
5. `git commit` referencing #60.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output envelope unchanged for existing commands (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (new `type-schema.json` golden committed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — should be untouched)
- [ ] All 8 valueTypes covered with correct widget hints
- [ ] `select`/`multiselect` emit `enum`; `required[]` correct; order recoverable
- [ ] Cross-store roundtrip test present in `srs-repository`

## Coordination Rules

- Repository Service Worker writes only `crates/srs-repository/**`; CLI Worker only `crates/srs-cli/**`.
- Phase 2 does not start until Phase 1's milestone gate passes (the handler depends on the service signature).
- No projection logic in the CLI handler — if `cmd_type_schema` exceeds the handler shape, the excess moves to the service.
- Workers return changed file paths + a one-line behaviour summary.
- Verification Agent runs after Phase 2 and before PR.

## Assumptions

- `repeatable` FieldAssignments are emitted as their scalar/select shape in this plan (no `array` wrapping); revisit if the consuming editor needs array forms. Flagged to the Plan/Architecture reviewers to confirm defer-vs-include.
- Inherited fields (`extendsTypeId`) are **not** flattened here; only `record_type.fields` is projected. Confirm with reviewer.
- `x-srs-*` is the chosen vendor-extension prefix for non-standard keywords, consistent with the issue's `x-srs-widget` example.
