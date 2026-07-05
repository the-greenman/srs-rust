# Plan: WASM bindings — replace ad-hoc json!({}) with typed result structs (#205)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`srs-bindings/src/lib.rs` contains five bindings that construct ad-hoc anonymous shapes via `serde_json::json!({})` instead of serialising the service's own result struct directly. This means no named type represents the returned shape, field names are string literals rather than compiler-checked identifiers, and the binding silently drifts if the service result gains new fields. This plan adds `serde::Serialize` to five service result structs in `srs-repository` (the minimal change) and replaces the five `json!({})` calls in `srs-bindings` with direct `to_js(&result)`, restoring the thin-wrapper contract of ADR-013.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (solo — single worker, no concurrency needed) |
| Repository Service Worker | owns `crates/srs-repository/**` |
| Bindings Worker | owns `crates/srs-bindings/**` |
| Verification Agent | read-only; runs final gates |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Each binding method is a thin wrapper: deserialise JS input → one service call → `to_js(&result)`. No field-mapping in the binding layer. | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions return typed result structs; no `json!()` construction at the service or binding layer. | accepted |

No new ADRs are needed: this plan restores conformance to two existing ADRs.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. No `payload.rs` edits, no `generate-schemas` run required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

Five binding functions in `crates/srs-bindings/src/lib.rs` currently use Pattern B (`json!({})`):

| Binding fn | Current json!() fields | Service result struct | File |
|---|---|---|---|
| `create_record_successor` | `record`, `relation` | `CreateRecordSuccessorResult` | `crates/srs-repository/src/record_store.rs:766` |
| `blueprint_schema` | `schema`, `diagnostics` | `BlueprintSchemaResult` | `crates/srs-repository/src/blueprint_schema_service.rs:33` |
| `render_document_view` | `rendered`, `diagnostics`, `projection` | `RenderResult` | `crates/srs-repository/src/render_service.rs:41` |
| `type_schema` | `schema`, `diagnostics` | `TypeSchemaResult` | `crates/srs-repository/src/type_schema_service.rs:32` |
| `list_blueprints` | `summaries`, `diagnostics` | `BlueprintListResult` | `crates/srs-repository/src/blueprint_service.rs:79` |

All five result struct field names are snake_case single-word identifiers that match the existing json!() key strings exactly — no `rename_all` attribute is needed.

**Out of scope:**

- Adding `Serialize` to other structs not involved in Pattern B sites.
- Any changes to `srs-cli`, `srs-core`, `srs-schema`, or schema golden files.
- Adding TypeScript type declarations or serde-generated schema for WASM consumers.
- Fixing the `create_record_successor` WASM binding's missing `mut` / interior-mutability issue if encountered (file separately).

---

## Phases

### Phase 1: Add `serde::Serialize` to five service result structs in `srs-repository`

**Goal:** All five Pattern B result structs derive `Serialize` so their instances can be passed directly to `serde_json::to_string` (and thus to `to_js`).

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/record_store.rs` line 765: add `serde::Serialize` to `CreateRecordSuccessorResult`'s derive.
  Before: `#[derive(Debug, Clone)]`
  After:  `#[derive(Debug, Clone, serde::Serialize)]`

- [ ] In `crates/srs-repository/src/blueprint_schema_service.rs` line 32: add `serde::Serialize` to `BlueprintSchemaResult`'s derive.
  Before: `#[derive(Debug, Clone)]`
  After:  `#[derive(Debug, Clone, serde::Serialize)]`

- [ ] In `crates/srs-repository/src/render_service.rs` line 41: add `#[derive(Debug, serde::Serialize)]` to `RenderResult`.
  Before: `pub struct RenderResult {`  (no derive)
  After:  `#[derive(Debug, serde::Serialize)]` on the line before `pub struct RenderResult {`
  Note: `Clone` is intentionally omitted — `DocumentViewProjection` does not implement `Clone`. `Debug` matches every other public struct in the file that derives `Serialize`.

- [ ] In `crates/srs-repository/src/type_schema_service.rs` line 31: add `serde::Serialize` to `TypeSchemaResult`'s derive.
  Before: `#[derive(Debug, Clone)]`
  After:  `#[derive(Debug, Clone, serde::Serialize)]`

- [ ] In `crates/srs-repository/src/blueprint_service.rs` line 78: add `serde::Serialize` to `BlueprintListResult`'s derive.
  Before: `#[derive(Debug, Clone)]`
  After:  `#[derive(Debug, Clone, serde::Serialize)]`

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles with no errors or warnings.
- [ ] All five structs can be serialised: `serde_json::to_string(&result).is_ok()` holds.
- [ ] No existing tests in `srs-repository` break.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- No new tests needed for Phase 1: `Serialize` is a compiler-checked derive with no runtime branching. If any struct is missing `Serialize`, the `to_js(&result)` call in Phase 2 will not compile — there is no silent drift. Correctness of output shape is proven by the bindings test in Phase 2.

#### Milestone gate

1. Check all acceptance criteria above.
2. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
3. Mark task checkboxes `[x]`, commit:
```bash
git commit -m "feat(srs-repository): derive Serialize on 5 WASM result structs (#205)"
```

---

### Phase 2: Replace `json!({})` with `to_js(&result)` in `srs-bindings`

**Goal:** All five Pattern B binding functions pass the typed service result directly to `to_js`, eliminating anonymous shapes and string-literal field names.

**Agent:** Bindings Worker

#### Tasks

Note: line numbers below are approximate (the exact code strings are the authoritative locators).

- [ ] `create_record_successor` (lib.rs ~line 258): replace
  ```rust
  to_js(&serde_json::json!({
      "record": result.record,
      "relation": result.relation,
  }))
  ```
  with `to_js(&result)`

- [ ] `blueprint_schema` (lib.rs ~line 276): replace
  ```rust
  to_js(&serde_json::json!({
      "schema": result.schema,
      "diagnostics": result.diagnostics,
  }))
  ```
  with `to_js(&result)`

- [ ] `render_document_view` (lib.rs ~line 300): replace
  ```rust
  to_js(&serde_json::json!({
      "rendered": result.rendered,
      "diagnostics": result.diagnostics,
      "projection": result.projection,
  }))
  ```
  with `to_js(&result)`

- [ ] `type_schema` (lib.rs ~line 403): replace
  ```rust
  to_js(&serde_json::json!({
      "schema": result.schema,
      "diagnostics": result.diagnostics,
  }))
  ```
  with `to_js(&result)`

- [ ] `list_blueprints` (lib.rs ~line 415): replace
  ```rust
  to_js(&serde_json::json!({
      "summaries": result.summaries,
      "diagnostics": result.diagnostics,
  }))
  ```
  with `to_js(&result)`

- [ ] Verify `serde_json::json` is no longer used in `srs-bindings/src/lib.rs`. If the import `use serde_json::json;` exists, remove it. If only `serde_json` is used (for `from_str`), leave the `serde_json` crate import but do not import `json` macro.

#### Acceptance Criteria

- [ ] All five bindings compile with `to_js(&result)` — no `json!({})` remains in `lib.rs`.
- [ ] The JSON output shape is identical to before: field names and types are unchanged (verified by the test below).
- [ ] `cargo test -p srs-bindings` passes.
- [ ] No unused import warnings from removing `json!`.

#### Testing

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests:
- `graduate_note_service_result_serialises` — existing; still passes as-is (that binding already used `to_js(&result)`).
- Add `blueprint_schema_result_serialises` in `srs-bindings/src/lib.rs` tests block: construct `BlueprintSchemaResult { schema: serde_json::Value::Null, diagnostics: vec![] }`, call `serde_json::to_value(&result)`, assert `json["schema"].is_null()` and `json["diagnostics"].is_array()`. This proves the pattern compiles and the field names are correct for the representative two-field case.
- One representative test suffices: if any of the five structs lacked `Serialize`, the `to_js(&result)` call added in Phase 2 would fail to compile — making silent drift impossible. The test proves shape correctness, not compilability.

#### Milestone gate

1. Check all acceptance criteria above.
2. Run:
```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
3. Mark checkboxes `[x]`, commit:
```bash
git commit -m "feat(srs-bindings): replace ad-hoc json!({}) with to_js(&result) (#205)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] No `serde_json::json!({...})` calls remain in `crates/srs-bindings/src/lib.rs` (verify with `grep "json!(" crates/srs-bindings/src/lib.rs`)
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` passes (no entity schemas were changed)
- [ ] Five affected binding functions return structurally identical JSON as before

## Coordination Rules

- Repository Service Worker keeps changes to the five struct derives only — no other srs-repository edits.
- Bindings Worker keeps changes to the five `to_js` call sites only — no refactoring of unrelated bindings.
- Verification Agent runs `cargo test && cargo clippy -- -D warnings` and confirms no `json!({` pattern remains in lib.rs.

## Assumptions

- `DocumentViewProjection` already derives `serde::Serialize` (confirmed: `render_service.rs:94`).
- `BlueprintSummary` already derives `serde::Serialize` (confirmed: `blueprint_service.rs:63`).
- `Record` already derives `Serialize` (confirmed: `crates/srs-core/src/types/record.rs` — `Record` is used in `record_store` list operations that serialize through `serde_json`). `Relation` already derives `Serialize` (confirmed: `crates/srs-core/src/types/relation.rs`).
- Field name casing: all five structs use single-word field names (no underscores) so snake_case and camelCase are identical — no `rename_all` attribute needed.
