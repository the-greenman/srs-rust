# Plan: Structured errors through the CLI envelope, WASM and MCP (#1338)

## Summary

Every `RepositoryError` reaches clients as Display text only, so clients that must act on an error
kind match on prose (srs-web#512 `LifecycleNotDefined`, srs-vscode#131 `CannotDeleteInUse`). That
violates ADR-048 rule 6 (identifier over label in outputs). This plan gives every `RepositoryError`
variant a stable kebab-case `code`, builds one typed `ErrorReport {code, message, details}` in
`srs-repository`, and carries it unchanged through all three adapters: a parallel `errors[]` in the
CLI `ok:false` envelope, a JS `Error` with `.code`/`.details` from WASM, and `structuredContent`
from MCP `tool_err`. Display text loses its code prefixes, and tests assert on `code`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | orchestrating session |
| Repository Worker | Phase 1 (sonnet) |
| CLI Worker | Phase 2 (sonnet) |
| Bindings Worker + MCP Adapter Worker | Phase 3 (sonnet) |
| Repository Worker | Phase 4 tests (sonnet) |
| Verification | Verification Agent (haiku) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

Owner decisions taken at the Stage 2 pause (2026-10-08). All four recommended options were chosen.

| ADR | Decision | Status |
|---|---|---|
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) rule 6 | Identifier over label: the core emits `code` as a field; adapters carry it unchanged; tests assert on it; kebab case; RFC names verbatim | accepted (governs) |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) / [ADR-011](../docs/adr/011-cli-output-contract.md) payload contract | The envelope keeps `ok/command/version/payload/diagnostics`; the new `errors[]` entry type gets a golden schema | accepted (governs) |
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) / [ADR-013](../docs/adr/013-wasm-binding-strategy.md) adapter layering + [capability-layering.md](../docs/architecture/capability-layering.md) | `tool_err` lives in `srs-mcp-core`; `srs-mcp` only translates | accepted (governs) |
| [ADR-053](../docs/adr/053-structured-error-reports.md) | One `ErrorReport` in srs-repository. Codes come from an explicit `code()` match, never from variant names. Details are serde-derived on `RepositoryError` (untagged, camelCase, sources skipped). Non-repository failures map to an existing variant where one fits, else `unclassified`. The CLI uses a parallel `errors[]`, WASM throws an `Error` with `.code`, MCP returns `structuredContent` | proposed (this plan) |

Owner rulings recorded:
1. **Envelope:** a top-level `errors: [{code, message, details?}]` beside `diagnostics: string[]`, aligned 1:1 with the strings. `diagnostics` is unchanged.
2. **Details:** an open `details` object from `#[derive(Serialize)]` on `RepositoryError` (`#[serde(untagged, rename_all_fields = "camelCase")]`, with `#[serde(skip)]` on non-serializable sources). The schema types it as `object`. A per-code typed union is deferred to #1342.
3. **Non-repository failures:** route through an existing variant where one fits (`InvalidInput`, `FieldNotFound`, `RelationNotFound`, `CompositionNotFound`, …). Everything else carries code `unclassified`. A follow-up drives `unclassified` to zero.
4. **Sub-coded variants:** `ActorProvenance`, `InvalidPackageBundle` and `SliceRefused` keep their inner `code: &'static str`. `code()` returns it verbatim, Display drops the `{code}: ` prefix, and the inner field is `#[serde(skip)]` in details.

No owner ruling was needed on the rename: the SCREAMING_SNAKE forms are renamed once with no aliases (owner ruling on #1264). No RFC names them (`git grep` over srs origin/master is empty).

## Contracts

### CLI output contract (ADR-011)

The envelope gains one optional top-level field on `ok:false`:

```json
{ "ok": false, "command": "srs", "version": "…",
  "diagnostics": ["cannot delete type 'x': still referenced by [a, b]"],
  "errors": [{ "code": "cannot-delete-in-use", "message": "cannot delete type 'x': still referenced by [a, b]",
               "details": { "entityType": "type", "id": "x", "usedBy": ["a", "b"] } }] }
```

- `errors` is present whenever `ok:false` and absent when `ok:true`. `errors.len() == diagnostics.len()` and `errors[i].message == diagnostics[i]`.
- `details` is omitted when the variant has no serializable fields.
- `errors[]` is **not** added to `ok:true` envelopes (`serialize_with_diagnostics`). Those strings are non-fatal notes, owned by #1339.
- `err_with_payload` (vocabulary) also gets `errors`.
- Golden schema: a new `crates/srs-cli/schemas/payload/error-report.json` from a payload struct `ErrorReportPayload` (schemars 0.8 mirror of `srs_repository::ErrorReport`, with `details: Option<serde_json::Value>`). Run `cargo run --bin generate-schemas`.

### WASM and MCP wire change

- WASM: functions that threw a **string** now throw a JS `Error` with `.message` (same text), `.code`, and `.details?`. Clients using `typeof e === 'string'` or `String(e)` see an `Error` (`String(e)` gains an `Error: ` prefix). Coordination: srs-web#513 (srs-vscode consumes the CLI, unaffected).
- MCP: error results add `structuredContent: {code, message, details?}`; `content[0].text` is unchanged.

### Entity schema sync

No. No `srs/docs/schema/2.0/` change. The CLI envelope and adapter error shapes are srs-rust contracts (ADR-011), not spec. RFC-046 / ADR-050 / ADR-051 codes pass through verbatim.

---

## Scope

- `RepositoryError::code() -> &'static str`: an explicit match over every variant (stable contract, not derived from the Rust name).
- `#[derive(Serialize)]` on `RepositoryError` for `details`.
- `pub struct ErrorReport { code: String, message: String, details: Option<Value> }`, plus `RepositoryError::report()` and `ErrorReport::unclassified(msg)`.
- Display text loses the `SCREAMING_SNAKE: ` prefixes and the `{code}: ` prefix of the three sub-coded variants.
- CLI: `OutputDTO.errors`; `main.rs` downcasts the anyhow chain to `RepositoryError`; `output::repo_err(cmd, &e)`; the 109 `output::err` sites move to `repo_err` where `e: RepositoryError`, and handler-invented NotFound sites route through the variant.
- WASM: `js_err` throws a `js_sys::Error` with `.code` and `.details` (Reflect-set) for both RepositoryError and other inputs; the "invalid input" sites use `invalid-input`.
- MCP: `tool_err(ErrorReport)` returns `structuredContent: {code, message, details?}`; the write guard uses code `write-guard-rejected`.
- Tests: convert the substring assertions on `RepositoryError` text to assert on `code()` / envelope `errors[0].code`. Add one adapter test per adapter.

**Out of scope:**
- A per-code typed `details` schema (discriminated union). Follow-up.
- Driving `unclassified` to zero (anyhow-only CLI failures, CLI arg checks). Follow-up.
- `ValidationDiagnostic` / binding `string[]` diagnostics: #1264. Remaining `Vec<String>` payloads and `ok:true` diagnostics: #1339.
- Client changes in srs-web#512 and srs-vscode#131. They consume `code` after this lands.
- `CoreError` codes nested inside `RecordValidation` etc. The outer variant's code only; follow-up if a client needs the inner kind.

---

## Phases

### Phase 1: Core — codes, details, ErrorReport (srs-repository)

**Goal:** every `RepositoryError` yields a stable `code()` and a `report()`; Display carries no code prefix.

**Agent:** Repository Worker

#### Tasks

- [ ] `crates/srs-repository/src/error.rs`: add `#[derive(serde::Serialize)]` and `#[serde(untagged, rename_all_fields = "camelCase")]` to `RepositoryError`. Mark every non-`Serialize` field `#[serde(skip)]` (`serde_json::Error`, `std::io::Error`, `Box<dyn Error>`, and `CoreError` if not Serialize), and also the inner `code` of `ActorProvenance` / `InvalidPackageBundle` / `SliceRefused`.
- [ ] Add `pub fn code(&self) -> &'static str`: an explicit match, one arm per variant, kebab-case. Use the variant name in kebab case unless an RFC/ADR names it (the three sub-coded variants return their inner `code`). Notable codes: `record-has-inbound-relations`, `lifecycle-relation-required`, `successor-relation-type-undetermined`, `lifecycle-fulfillment-not-applicable`, `lifecycle-fulfillment-relation-type-mismatch`, `lifecycle-state-unreachable`, `lifecycle-not-defined`, `cannot-delete-in-use`, `invalid-input`, `instance-not-found`, `rfc043-migration-needed`. Rule for acronyms/digits: kebab-case the variant name treating a digit run as part of the preceding word (`Rfc043MigrationNeeded` → `rfc043-migration-needed`).
- [ ] Remove the `SCREAMING_SNAKE: ` prefixes from the `#[error]` strings (variants `RecordHasInboundRelations`, `LifecycleRelationRequired`, `SuccessorRelationTypeUndetermined`, `LifecycleFulfillmentNotApplicable`, `LifecycleFulfillmentRelationTypeMismatch`, `LifecycleStateUnreachable` — find them with `grep -n '"[A-Z_]\{6,\}: ' error.rs`) and change `"{code}: {message}"` to `"{message}"` for the three sub-coded variants.
- [ ] Add `pub struct ErrorReport { pub code: String, pub message: String, #[serde(skip_serializing_if = "Option::is_none")] pub details: Option<serde_json::Value> }` (Serialize + Deserialize, Debug, Clone, PartialEq) and `pub const UNCLASSIFIED: &str = "unclassified"`.
- [ ] Add `impl RepositoryError { pub fn report(&self) -> ErrorReport }`. `details` = `serde_json::to_value(self)` when it is a non-empty object, else `None`. Add `ErrorReport::unclassified(message: impl Into<String>)`. Re-export `ErrorReport` from the crate root.
- [ ] Fix any in-crate code that parsed the removed prefixes (grep `"LIFECYCLE_`, `"SUCCESSOR_`, `"RECORD_HAS_`, `"actor-`, `"bundle-`, `"slice-` in `starts_with`/`contains` outside tests).

#### Acceptance Criteria

- [ ] `code()` covers every variant (the compiler enforces an exhaustive match with no `_` arm).
- [ ] No `#[error(...)]` string starts with a code or SCREAMING_SNAKE token (`error::tests::display_has_no_code_prefix`).
- [ ] `report()` of `CannotDeleteInUse` yields `details.usedBy`; of `SuccessorRelationTypeUndetermined` yields `details.candidates` (#1247).

#### Testing

- `error::tests::codes_are_kebab_case`: constructs one value of **every** variant (a list in the test; a comment ties it to the exhaustive `code()` match) and asserts codes are unique and `code()` matches `^[a-z0-9]+(-[a-z0-9]+)*$`.
- `error::tests::report_carries_details`: CannotDeleteInUse → `usedBy`; SuccessorRelationTypeUndetermined → `candidates`; ActorProvenance → `code == "actor-supplied"`, message has no prefix, no `code` in details.
- `error::tests::display_has_no_code_prefix`: the six former SCREAMING_SNAKE variants.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository --all-targets -- -D warnings
```
Then update the checkboxes and commit `feat(repository): RepositoryError codes and ErrorReport (#1338)`.

### Phase 2: CLI envelope

**Goal:** every `ok:false` envelope carries `errors[]` aligned with `diagnostics`.

**Agent:** CLI Worker

#### Tasks

- [ ] `crates/srs-cli/src/output.rs`: add `#[serde(skip_serializing_if = "Option::is_none")] pub errors: Option<Vec<ErrorReport>>` to `OutputDTO`. `OutputDTO::err(cmd, diagnostics)` fills `errors` with `ErrorReport::unclassified` per string. Add `OutputDTO::from_reports(cmd, Vec<ErrorReport>)` (diagnostics = messages) and `pub fn repo_err(command, &RepositoryError) -> String`. `err_with_payload` fills `errors` too. `ok`/`serialize_with_diagnostics` leave `errors: None`.
- [ ] `crates/srs-cli/src/main.rs`: for `Err(e)`, find the first `RepositoryError` in `e.chain()` (`downcast_ref`). If found, render `OutputDTO::from_reports("srs", vec![ErrorReport { message: format!("{e:#}"), ..re.report() }])` (message keeps the anyhow context); else `OutputDTO::err("srs", vec![format!("{e:#}")])` (→ `unclassified`). `OutputDTO` derives Deserialize, so `errors` set by a handler's `repo_err` survives the `Ok(result)` reparse in main.rs; the reparse-failure fallback yields `unclassified`.
- [ ] Convert every `output::err(cmd, vec![e.to_string()])` where `e: RepositoryError` to `output::repo_err(cmd, &e)`. Convert handler-invented not-found / invalid-input sites in `crates/srs-cli/src/commands/*.rs` (find with `grep -n 'NotFound =>\|None =>' crates/srs-cli/src/commands`; e.g. `GetFieldResult::NotFound`, `GetRelationResult::NotFound`, `GetCompositionResult::NotFound`, relation-type `None`, `find` arg check, …) to construct the matching `RepositoryError` variant and call `repo_err`. Leave the remainder on `output::err` (→ `unclassified`, tracked by #1343) and list them in the commit message. Acceptance grep: `grep -rn 'output::err(.*e.to_string' crates/srs-cli/src` returns only sites whose `e` is not a `RepositoryError`.
- [ ] `payload.rs`: add `ErrorReportPayload` (JsonSchema mirror: `code: String`, `message: String`, `details: Option<serde_json::Value>`), register `write_schema!("error-report", ErrorReportPayload)` in `bin/generate-schemas.rs`, and run `cargo run --bin generate-schemas`. The mirror is deliberate (schemars is forbidden in srs-repository's default build, ADR-011); add `payload_contracts::error_report_mirror_roundtrip` that serializes an `ErrorReport` and deserializes it as `ErrorReportPayload` and back, unchanged.

#### Acceptance Criteria

- [ ] `srs record transition` on a record with no lifecycle → `errors[0].code == "lifecycle-not-defined"`.
- [ ] `srs type delete` of an in-use type → `errors[0].code == "cannot-delete-in-use"`, `details.usedBy` non-empty.
- [ ] `errors.len() == diagnostics.len()` on every `ok:false` envelope.

#### Testing

- `crates/srs-cli/tests/`: `error_envelope_lifecycle_not_defined`, `error_envelope_cannot_delete_in_use` (the two criteria above, asserting `errors[0].code` and `details`), and `error_envelope_handler_path_survives_reparse` (a handler `repo_err` path, e.g. `relation get` of a missing id → `relation-not-found`).
- `cargo test --test payload_contracts`

#### Milestone gate

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli --all-targets -- -D warnings
```
Commit `feat(cli): structured errors[] in the ok:false envelope (#1338)`.

### Phase 3: WASM and MCP adapters

**Goal:** WASM throws `Error` with `.code`/`.details`; MCP `tool_err` carries `structuredContent`.

**Agent:** Bindings Worker + MCP Adapter Worker

#### Tasks

- [ ] `crates/srs-bindings/src/lib.rs`: one private `fn report_to_js(report: ErrorReport) -> JsValue` builds `js_sys::Error::new(&report.message)` and sets `code` (string) and `details` (when present) with `js_sys::Reflect::set`. `js_err(e: &RepositoryError)` → `report_to_js(e.report())` — sites pass the error, **never `e.to_string()`**. `js_invalid_input(msg)` → `RepositoryError::InvalidInput { message }` (variant exists at error.rs `InvalidInput { message: String }`) for the input/query/options parse sites. `js_unclassified(msg)` for everything else (e.g. output serialization in `to_js`). **No** blanket `From<String>`/`From<serde_json::Error>` — each site chooses. Keep every message string unchanged. Acceptance grep: `grep -n 'js_err(.*to_string\|js_err(format' crates/srs-bindings/src` is empty.
- [ ] `crates/srs-mcp-core/src/tools.rs`: change `tool_err` to `tool_err(report: ErrorReport) -> Value`, returning `{content:[{type:"text",text:report.message}], structuredContent: report, isError:true}`. Convert callers: `tool_err(e.report())` for RepositoryError, `ErrorReport::unclassified(...)` otherwise, `GetRunResult::NotFound` → a matching variant if one exists, else unclassified.
- [ ] Write guard: `WriteGuard::check` (`crates/srs-mcp-core/src/guard.rs`) keeps returning `Err(String)`; at its single call site in `SrsMcpApplication` `tools/call` (`crates/srs-mcp-core/src/lib.rs`, `guard.check(store, &name, …)`) wrap it as `ErrorReport { code: "write-guard-rejected".into(), message, details: None }` before `tool_err`.
- [ ] `crates/srs-mcp`: rmcp 2.2 `CallToolResult.structured_content` round-trips through JSON (verified at plan review), so no change; the test below asserts it end to end.

#### Acceptance Criteria

- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds.
- [ ] An MCP `record_transition` on a no-lifecycle record → `structuredContent.code == "lifecycle-not-defined"`.
- [ ] A guard rejection → `structuredContent.code == "write-guard-rejected"`.

#### Testing

- `crates/srs-mcp/tests/tools.rs`: `tool_error_carries_structured_code`.
- `crates/srs-mcp-core` guard test: `guard_rejection_is_coded` (asserts `structuredContent.code`, not the "Rejected by…" text).
- Bindings: a native unit test of `ErrorReport` conversion (`From<serde_json::Error>` → `invalid-input`). The JS `Error` construction is covered by the wasm32 build only (no wasm test runner in CI).

#### Milestone gate

```bash
cargo test -p srs-mcp-core -p srs-mcp -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo clippy --workspace --all-targets -- -D warnings
```
Commit `feat(bindings,mcp): structured error code through WASM and MCP (#1338)`.

### Phase 4: Tests assert on code

**Goal:** no test asserts on `RepositoryError` message substrings where a code exists (ADR-048 6(d)).

**Agent:** Repository Worker

#### Tasks

- [ ] Grep `to_string().contains`, `msg.contains`, `err.contains`, and `diagnostics"][0]` in `crates/**`. For each assertion whose subject is a `RepositoryError` (or a CLI/MCP error envelope), assert on `code()` / `errors[0].code` / `structuredContent.code`, plus a structured field where the test cared about one. Leave assertions on non-RepositoryError text (migration reports, CoreError, validation diagnostics: #1264) untouched, and list them in the commit message.
- [ ] Update tests that asserted on the removed `SCREAMING_SNAKE` / `{code}:` prefixes.

#### Acceptance Criteria

- [ ] `cargo test --workspace` has zero failures.
- [ ] No remaining test asserts a removed prefix string.

#### Milestone gate

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Commit `test: assert on error code, not message text (#1338)`.

---

## Final Acceptance

- [ ] `cargo build --workspace`, `cargo test --workspace` (zero failures), `cargo clippy --workspace --all-targets -- -D warnings`, all judged by exit code
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings`
- [ ] `cargo test --test payload_contracts` passes with `error-report.json` committed
- [ ] Entity schemas unchanged (no `check-schema-sync` action)
- [ ] ADR-053 committed; Status flipped proposed → accepted at Stage 7.5
- [ ] srs-vscode payload-contract sync: none needed in this PR (new golden only; srs-vscode consumes via its own sync) — noted in the PR body
- [ ] Client coordination issue filed in srs-web (the only WASM consumer; srs-vscode uses the CLI, where `errors[]` is additive): WASM now throws an `Error` (not a string); catch sites doing `typeof e === 'string'` / `String(e)` must read `.message` / `.code`
- [ ] The three adapters each carry `code` for `LifecycleNotDefined` and `CannotDeleteInUse` (the srs-web#512 / srs-vscode#131 root causes)

## Coordination Rules

- Agents keep to their write scopes unless the Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- The Lead Integrator owns final API naming and dependency boundaries.
- Each phase ends at its milestone gate (criteria verified, tests pass, checkboxes updated, commit).

## Assumptions

- `CoreError` is not `Serialize` (srs-core/src/error.rs); it is skipped in details (its text stays in `message`).
- rmcp 2.2 forwards `structuredContent` on error results (verified at plan review).
- Changing the CLI envelope's top-level shape is a contract change, so the PR is `gate:owner-merge` (Door: none, non-normative to the spec; Mode: complicated).
