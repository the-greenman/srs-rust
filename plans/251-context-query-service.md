# Plan: Context-Query Service (ext:addressability patterns)

> Issue #251. Implements the addressability context-query behavioural requirements
> from `ext:addressability` as a typed service in `srs-repository`, exposed via
> CLI commands and WASM bindings.

## Summary

`ext:addressability` requires four context-query patterns (field context, record context, stage context, revision trace) as first-class service functions so that AI assistants and tooling can assemble focused context given an address. The core types (`Address`, `AttentionState`, `Revision`) and the revision CRUD service landed earlier (#579). This plan adds the three query patterns that are implementable without protocol runs: **field context**, **record context**, and **revision trace**. Stage context (which needs the run model from #252) is deferred with a tracking issue.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator |
| Repository Service Worker | Repository Service Worker |
| CLI Worker | CLI Worker |
| Bindings Worker | Bindings Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions: this plan implements ADR-010 (service boundary contract), ADR-011 (CLI output contract), and ADR-013 (WASM binding strategy). No new ADR is needed because the patterns follow the established service → CLI → bindings layering.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions in srs-repository; typed input/output structs | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | All CLI output via payload structs in payload.rs; golden schemas | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Bindings call the same services as the CLI, no duplicated logic | accepted |

---

## Contracts

### CLI output contract (ADR-011)

Three new commands with new payload structs:

- `srs context field` → `ContextFieldPayload` in `payload.rs`
- `srs context record` → `ContextRecordPayload` in `payload.rs`
- `srs context revision` → `ContextRevisionTracePayload` in `payload.rs`

After adding structs: `cargo run --bin generate-schemas` → commit new `schemas/payload/context-*.json` files.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No entity schema files under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- `context_query_service.rs` in `crates/srs-repository/src/` with three service functions: `get_field_context`, `get_record_context`, `get_revision_trace`.
- `context.rs` command module in `crates/srs-cli/src/commands/` wiring `srs context field|record|revision` subcommands.
- Three payload structs (`ContextFieldPayload`, `ContextRecordPayload`, `ContextRevisionTracePayload`) in `crates/srs-cli/src/payload.rs`.
- Three golden schema files generated under `crates/srs-cli/schemas/payload/`.
- Three bindings methods (`context_field`, `context_record`, `context_revision`) on `SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- `lib.rs` export of `context_query_service` in `crates/srs-repository/src/lib.rs`.
- MemoryStore unit tests for each service function; cross-store roundtrip test.

**Out of scope:**

- Stage context (`{runId}/{stageId}`) — depends on protocol run model (#252). File a deferred issue.
- Tagged chunks — depends on conversation/transcript chunk storage (#252 or later). Field context and record context return an empty `tagged_chunks: []` placeholder field to preserve forward-compatibility of the payload shape.
- Protocol run history in record context — same dependency. Return `protocol_run_history: []` placeholder.
- Note / Tier-1 record contexts — only Tier-2 records are indexed in the manifest with revision sidecars. Tier-1 scoping is out of scope.

---

## Phases

### Phase 1: context_query_service in srs-repository

**Goal:** Three typed service functions exist in `crates/srs-repository/src/context_query_service.rs`, all covered by MemoryStore tests.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/context_query_service.rs` with:

  **Input types:**

  ```rust
  pub struct FieldContextQuery {
      pub record_id: String,
      pub field_id: String,
  }
  pub struct RecordContextQuery {
      pub record_id: String,
  }
  pub struct RevisionTraceQuery {
      pub record_id: String,
      pub field_id: String,
      pub revision_id: String,
  }
  ```

  **Result types:**

  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct FieldContextResult {
      pub record_id: String,
      pub field_id: String,
      pub field_name: Option<String>,
      pub field_namespace: Option<String>,
      pub ai_guidance: Option<serde_json::Value>,
      pub current_value: Option<serde_json::Value>,
      pub revisions: Vec<srs_core::types::revision::Revision>,
      pub tagged_chunks: Vec<serde_json::Value>,  // always empty (placeholder for #252)
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RecordContextResult {
      pub record_id: String,
      pub type_id: Option<String>,
      pub type_name: Option<String>,
      pub type_namespace: Option<String>,
      pub display_label: Option<String>,
      pub field_values: Vec<srs_core::types::record::FieldValue>,
      pub relations: Vec<crate::relation_service::RelationSummary>,
      pub tagged_chunks: Vec<serde_json::Value>,       // always empty (placeholder for #252)
      pub protocol_run_history: Vec<serde_json::Value>, // always empty (placeholder for #252)
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RevisionTraceResult {
      pub record_id: String,
      pub field_id: String,
      pub revision: srs_core::types::revision::Revision,
      pub prior_chain: Vec<srs_core::types::revision::Revision>,
  }
  ```

  **Service functions:**

  ```rust
  pub fn get_field_context(
      store: &dyn RepositoryStore,
      query: FieldContextQuery,
  ) -> Result<FieldContextResult, RepositoryError>
  ```
  Implementation:
  1. Call `record_store::get_record_by_id(store, &query.record_id)?` — return `NotFound` error if `None`.
  2. Find `FieldValue` where `field_id == query.field_id` in `record.field_values`; extract `current_value`.
  3. Call `record_store::list_record_revisions(store, &query.record_id, Some(&query.field_id), None, None)?` to get revisions.
  4. Call `package_service::get_field_by_id(store, &query.field_id)?` — if `Found`, extract `ai_guidance`, `name`, `namespace`; if `NotFound`, use `None`.
  5. Return `FieldContextResult` with `tagged_chunks: vec![]`.

  ```rust
  pub fn get_record_context(
      store: &dyn RepositoryStore,
      query: RecordContextQuery,
  ) -> Result<RecordContextResult, RepositoryError>
  ```
  Implementation:
  1. Call `record_store::get_record_summary_by_id(store, &query.record_id)?` — return `NotFound` error if `None`.
  2. Extract `field_values` from `summary.record.field_values`.
  3. Call `relation_service::list_relations(store, ListRelationsFilter { source: Some(query.record_id.clone()), ..Default::default() })?` for relations where record is the source.
  4. Return `RecordContextResult` with `tagged_chunks: vec![]`, `protocol_run_history: vec![]`.
     Set `type_id`, `type_name`, `type_namespace` from `summary.record`.

  ```rust
  pub fn get_revision_trace(
      store: &dyn RepositoryStore,
      query: RevisionTraceQuery,
  ) -> Result<RevisionTraceResult, RepositoryError>
  ```
  Implementation:
  1. Call `record_store::list_record_revisions(store, &query.record_id, Some(&query.field_id), None, None)?` to load all revisions for this field.
  2. Find the target revision by `revision_id`; return `NotFound` error if absent.
  3. Build `prior_chain`: starting from `target.prior_revision_id`, follow the `prior_revision_id` chain through the full revision list (create an index by `revision_id`). Collect ancestors in reverse-chained order (oldest first). Stop when `prior_revision_id` is `None` or a cycle is detected (guard with a `HashSet`).
  4. Return `RevisionTraceResult { revision: target, prior_chain }`.

- [ ] Export `context_query_service` in `crates/srs-repository/src/lib.rs`:
  ```rust
  pub mod context_query_service;
  ```

#### Acceptance Criteria

- [ ] `get_field_context` returns current value and empty revisions for a record with no sidecar.
- [ ] `get_field_context` returns all revisions for the requested field and none for other fields.
- [ ] `get_field_context` populates `ai_guidance` from the field definition when available.
- [ ] `get_record_context` returns all field values and associated relations.
- [ ] `get_revision_trace` returns the target revision and correct prior chain (oldest first).
- [ ] `get_revision_trace` returns `RepositoryError::NotFound` for a missing revision_id.
- [ ] No business logic in `srs-cli`; no file paths in service logic.

#### Testing

```bash
cargo test -p srs-repository context_query
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `context_query_service.rs`:

- `field_context_no_revisions` — record exists, no sidecar; `revisions` is empty, `current_value` is the field's value
- `field_context_filters_by_field_id` — two fields with revisions; only the queried field's revisions appear
- `field_context_ai_guidance_from_package` — package loaded with a field definition; `ai_guidance` matches
- `field_context_not_found` — nonexistent record_id returns `RepositoryError::NotFound`
- `record_context_field_values` — record with multiple fields; all appear in `field_values`
- `record_context_relations` — two relations; one sourced from the record appears, one with different source does not
- `revision_trace_prior_chain` — three revisions chained; `prior_chain` has two entries in order oldest→second
- `revision_trace_not_found` — nonexistent revision_id returns `RepositoryError::NotFound`

#### Milestone gate

1. All acceptance criteria checked.
2. All named tests exist and pass.
3. Run:
```bash
cargo test -p srs-repository context_query
cargo clippy -p srs-repository -- -D warnings
```
4. Mark checkboxes `[x]`, commit: `feat(srs-repository): context query service (field/record/revision trace) (#251)`.

---

### Phase 2: CLI commands and golden schemas

**Goal:** `srs context field|record|revision` subcommands are wired, payload structs added, golden schemas generated.

**Agent:** CLI Worker

#### Tasks

- [ ] Add three payload structs to `crates/srs-cli/src/payload.rs`:

  ```rust
  // Context query payloads
  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ContextFieldPayload {
      pub record_id: String,
      pub field_id: String,
      pub field_name: Option<String>,
      pub field_namespace: Option<String>,
      pub ai_guidance: Option<serde_json::Value>,
      pub current_value: Option<serde_json::Value>,
      pub revisions: Vec<srs_core::types::revision::Revision>,
      pub tagged_chunks: Vec<serde_json::Value>,
  }

  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ContextRecordPayload {
      pub record_id: String,
      pub type_id: Option<String>,
      pub type_name: Option<String>,
      pub type_namespace: Option<String>,
      pub display_label: Option<String>,
      pub field_values: Vec<srs_core::types::record::FieldValue>,
      pub relations: Vec<serde_json::Value>,
      pub tagged_chunks: Vec<serde_json::Value>,
      pub protocol_run_history: Vec<serde_json::Value>,
  }

  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ContextRevisionTracePayload {
      pub record_id: String,
      pub field_id: String,
      pub revision: srs_core::types::revision::Revision,
      pub prior_chain: Vec<srs_core::types::revision::Revision>,
  }
  ```

  Note on `relations` in `ContextRecordPayload`: use `serde_json::Value` with `#[schemars(with = "serde_json::Value")]` to avoid a schemars derivation issue on `RelationSummary` — the payload serializes the service result directly.

- [ ] Create `crates/srs-cli/src/commands/context.rs`:

  ```rust
  // Commands: srs context field <record-id> <field-id>
  //           srs context record <record-id>
  //           srs context revision <record-id> <field-id> <revision-id>
  ```

  - Parse subcommand variants in a `ContextCommand` enum.
  - Each handler calls the matching service function and wraps via `output::serialize(...)`.
  - Command names for the envelope: `"context field"`, `"context record"`, `"context revision"`.
  - No business logic in handlers. Use `with_store(&ctx, |store| ...)` helper.
  - Handler for `context field`:
    ```rust
    fn cmd_context_field(ctx: CliContext, record_id: String, field_id: String) -> Result<String> {
        with_store(&ctx, |store| {
            let result = context_query_service::get_field_context(
                store,
                context_query_service::FieldContextQuery { record_id: record_id.clone(), field_id: field_id.clone() },
            )?;
            output::serialize("context field", ContextFieldPayload {
                record_id: result.record_id,
                field_id: result.field_id,
                field_name: result.field_name,
                field_namespace: result.field_namespace,
                ai_guidance: result.ai_guidance,
                current_value: result.current_value,
                revisions: result.revisions,
                tagged_chunks: result.tagged_chunks,
            })
        })
    }
    ```
    (Similar patterns for `context record` and `context revision`.)

- [ ] Add `pub mod context;` to `crates/srs-cli/src/commands/mod.rs`.

- [ ] Wire `context` as a top-level subcommand in the CLI's main command enum (same file that lists `record`, `note`, etc. — locate by grepping for `SubCommand::Record` or the top-level `Cli` struct in `crates/srs-cli/src/main.rs`).

- [ ] Run `cargo run --bin generate-schemas` and verify three new files appear:
  - `crates/srs-cli/schemas/payload/context-field.json`
  - `crates/srs-cli/schemas/payload/context-record.json`
  - `crates/srs-cli/schemas/payload/context-revision.json`

  Stage all three files.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- context field <id> <fid>` returns a JSON envelope with `"command": "context field"`.
- [ ] `cargo run --bin srs -- context record <id>` returns a JSON envelope with `"command": "context record"`.
- [ ] `cargo run --bin srs -- context revision <id> <fid> <rev>` returns a JSON envelope with `"command": "context revision"`.
- [ ] `cargo test --test payload_contracts` passes with the three new schemas committed.
- [ ] Handlers contain no business logic (≤15 lines each).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests:
- Payload contract golden tests auto-generated by `generate-schemas`.

#### Milestone gate

1. All acceptance criteria checked.
2. Named tests pass.
3. Run:
```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```
4. Mark checkboxes `[x]`, commit: `feat(srs-cli): context field|record|revision commands + golden schemas (#251)`.

---

### Phase 3: WASM bindings

**Goal:** Three `context_*` methods on `SrsRepository` in `crates/srs-bindings/src/lib.rs`, covered by smoke tests.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add three methods to the `SrsRepository` `impl` block in `crates/srs-bindings/src/lib.rs`:

  ```rust
  #[wasm_bindgen]
  pub fn context_field(&self, record_id: &str, field_id: &str) -> Result<JsValue, JsValue> {
      let store = self.store.borrow();
      let result = context_query_service::get_field_context(
          &*store,
          context_query_service::FieldContextQuery {
              record_id: record_id.to_string(),
              field_id: field_id.to_string(),
          },
      ).map_err(|e| JsValue::from_str(&e.to_string()))?;
      serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
  }

  #[wasm_bindgen]
  pub fn context_record(&self, record_id: &str) -> Result<JsValue, JsValue> {
      let store = self.store.borrow();
      let result = context_query_service::get_record_context(
          &*store,
          context_query_service::RecordContextQuery { record_id: record_id.to_string() },
      ).map_err(|e| JsValue::from_str(&e.to_string()))?;
      serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
  }

  #[wasm_bindgen]
  pub fn context_revision(&self, record_id: &str, field_id: &str, revision_id: &str) -> Result<JsValue, JsValue> {
      let store = self.store.borrow();
      let result = context_query_service::get_revision_trace(
          &*store,
          context_query_service::RevisionTraceQuery {
              record_id: record_id.to_string(),
              field_id: field_id.to_string(),
              revision_id: revision_id.to_string(),
          },
      ).map_err(|e| JsValue::from_str(&e.to_string()))?;
      serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
  }
  ```

- [ ] Add `use srs_repository::context_query_service;` import in `lib.rs`.

- [ ] Add smoke tests in `crates/srs-bindings/tests/` (new file `context_query.rs`):
  - Build a minimal MemoryStore SRSJ with one record and one revision sidecar.
  - Call each binding method; assert the returned JSON is parseable and contains expected keys (`record_id`, `revisions`, `revision`).

#### Acceptance Criteria

- [ ] `context_field` returns a parseable JSON value with `record_id` and `revisions` keys.
- [ ] `context_record` returns a parseable JSON value with `record_id` and `field_values` keys.
- [ ] `context_revision` returns a parseable JSON value with `revision` key.
- [ ] Bindings smoke tests pass.
- [ ] No business logic in bindings (each method: accept strings → call service → serialize).

#### Testing

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests (`crates/srs-bindings/tests/context_query.rs`):
- `context_field_smoke` — record with two field values; returned JSON has `recordId`, `fieldId`, `revisions` (empty).
- `context_record_smoke` — record with field values and one relation; returned JSON has `fieldValues`, `relations`.
- `context_revision_smoke` — record with two chained revisions; `context_revision` returns `revision` and `priorChain` with one entry.

#### Milestone gate

1. All acceptance criteria checked.
2. Named tests pass.
3. Run:
```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
4. Mark checkboxes `[x]`, commit: `feat(srs-bindings): context_field/record/revision bindings (#251)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] Three new CLI commands (`context field`, `context record`, `context revision`) work against a real FileStore repository
- [ ] Three golden schema files committed under `crates/srs-cli/schemas/payload/`
- [ ] Deferred stage-context issue filed and parented under epic #236

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phase 1 and before final sign-off.

## Assumptions

- Only Tier-2 records (those indexed in the manifest with `tier == 2`) are supported; Tier-0 and Tier-1 are out of scope.
- `tagged_chunks` and `protocol_run_history` are returned as empty arrays in all payloads; this preserves the spec-required payload shape while deferring the data source to #252.
- `ai_guidance` from a field definition is a `serde_json::Value`; the payload passes it through without normalization.
- `ContextRecordPayload.relations` uses `serde_json::Value` via `#[schemars(with = "serde_json::Value")]` to avoid a schemars constraint on `RelationSummary`.
