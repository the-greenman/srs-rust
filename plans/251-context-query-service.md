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

- `srs context field` → `ContextFieldPayload` in `crates/srs-cli/src/payload.rs`
- `srs context record` → `ContextRecordPayload` in `crates/srs-cli/src/payload.rs`
- `srs context revision` → `ContextRevisionTracePayload` in `crates/srs-cli/src/payload.rs`

All payload structs that embed `srs-core` types must use `#[schemars(with = "...")]` overrides because `srs-core` has no `schemars` dependency (ADR-011).

After adding structs: `cargo run --bin generate-schemas` → commit new `schemas/payload/context-*.json` files.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No entity schema files under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- `context_query_service.rs` in `crates/srs-repository/src/` with three service functions: `get_field_context`, `get_record_context`, `get_revision_trace`.
- `context.rs` command module in `crates/srs-cli/src/commands/` wiring `srs context field|record|revision` subcommands.
- `ContextCommand` enum definition and dispatch arm added to `crates/srs-cli/src/commands/mod.rs`.
- Three payload structs (`ContextFieldPayload`, `ContextRecordPayload`, `ContextRevisionTracePayload`) in `crates/srs-cli/src/payload.rs`.
- Three golden schema files generated under `crates/srs-cli/schemas/payload/`.
- Three bindings methods (`context_field`, `context_record`, `context_revision`) on `SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- `lib.rs` export of `context_query_service` in `crates/srs-repository/src/lib.rs`.
- MemoryStore unit tests for each service function; one cross-store roundtrip test.
- Deferred stage-context issue filed and parented under epic #236.

**Out of scope:**

- Stage context (`{runId}/{stageId}`) — depends on protocol run model (#252). Tracked by the deferred issue filed in Phase 1.
- Tagged chunks — depends on conversation/transcript chunk storage (#252 or later). Field context and record context return an empty `tagged_chunks: []` placeholder field to preserve forward-compatibility.
- Protocol run history in record context — same dependency. Return `protocol_run_history: []` placeholder.
- Note / Tier-1 record contexts — only Tier-2 records are indexed in the manifest with revision sidecars.

---

## Phases

### Phase 1: context_query_service in srs-repository

**Goal:** Three typed service functions exist in `crates/srs-repository/src/context_query_service.rs`, all covered by MemoryStore tests including a cross-store roundtrip, and a deferred stage-context tracking issue is filed under epic #236.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/context_query_service.rs` with:

  **Input types** (add `#[derive(Debug, Clone, Deserialize)]` and `#[serde(rename_all = "camelCase")]` per ADR-010 convention; `serde` is already in scope in srs-repository):

  ```rust
  use serde::{Deserialize, Serialize};
  use std::collections::HashSet;
  use crate::error::RepositoryError;
  use crate::store::RepositoryStore;
  use crate::{package_service, record_store, relation_service};
  use relation_service::ListRelationsFilter;

  #[derive(Debug, Clone, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct FieldContextQuery {
      pub record_id: String,
      pub field_id: String,
  }

  #[derive(Debug, Clone, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RecordContextQuery {
      pub record_id: String,
  }

  #[derive(Debug, Clone, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RevisionTraceQuery {
      pub record_id: String,
      pub field_id: String,
      pub revision_id: String,
  }
  ```

  **Result types** (all derive `Debug, Clone, Serialize, Deserialize` and use `#[serde(rename_all = "camelCase")]`):

  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct FieldContextResult {
      pub record_id: String,
      pub field_id: String,
      pub field_name: Option<String>,
      pub field_namespace: Option<String>,
      // None when field not in package, or when field.ai_guidance is Value::Null
      pub ai_guidance: Option<serde_json::Value>,
      pub current_value: Option<serde_json::Value>,
      pub revisions: Vec<srs_core::types::revision::Revision>,
      pub tagged_chunks: Vec<serde_json::Value>,  // always empty; placeholder for #252
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RecordContextResult {
      pub record_id: String,
      // type_id/type_name/type_namespace are String (not Option) — they are always
      // present on a found Tier-2 Record
      pub type_id: String,
      pub type_name: String,
      pub type_namespace: String,
      pub display_label: String,
      pub field_values: Vec<srs_core::types::record::FieldValue>,
      pub relations: Vec<crate::relation_service::RelationSummary>,
      pub tagged_chunks: Vec<serde_json::Value>,        // always empty; placeholder for #252
      pub protocol_run_history: Vec<serde_json::Value>, // always empty; placeholder for #252
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
  1. Call `record_store::get_record_by_id(store, &query.record_id)?` — if `None`, return:
     ```rust
     Err(RepositoryError::NotFound { path: std::path::PathBuf::from(&query.record_id) })
     ```
  2. Find the `FieldValue` in `record.field_values` where `fv.field_id == query.field_id`; extract `current_value` as `Option<serde_json::Value>` (call `serde_json::to_value(&fv.value).ok()`, or clone the `value` field if it is already `serde_json::Value`).
  3. Call `record_store::list_record_revisions(store, &query.record_id, Some(&query.field_id), None, None)?` to get `Vec<Revision>`.
  4. Call `package_service::get_field_by_id(store, &query.field_id)?`. If `Found(field)`: set `field_name = Some(field.name)`, `field_namespace = Some(field.namespace)`, and `ai_guidance = if field.ai_guidance.is_null() { None } else { Some(field.ai_guidance) }`. If `NotFound`: all three are `None`.
  5. Return `FieldContextResult { record_id: query.record_id, field_id: query.field_id, field_name, field_namespace, ai_guidance, current_value, revisions, tagged_chunks: vec![] }`.

  ```rust
  pub fn get_record_context(
      store: &dyn RepositoryStore,
      query: RecordContextQuery,
  ) -> Result<RecordContextResult, RepositoryError>
  ```
  Implementation:
  1. Call `record_store::get_record_summary_by_id(store, &query.record_id)?` — if `None`, return `RepositoryError::NotFound`.
  2. Extract: `type_id = summary.record.type_id.clone()`, `type_name = summary.record.type_name.clone()`, `type_namespace = summary.record.type_namespace.clone()`, `display_label = summary.display_label.clone()`, `field_values = summary.record.field_values.clone()`.
  3. Call `relation_service::list_relations(store, ListRelationsFilter { source: Some(query.record_id.clone()), ..Default::default() })?`.
  4. Return `RecordContextResult { record_id: query.record_id, type_id, type_name, type_namespace, display_label, field_values, relations, tagged_chunks: vec![], protocol_run_history: vec![] }`.

  ```rust
  pub fn get_revision_trace(
      store: &dyn RepositoryStore,
      query: RevisionTraceQuery,
  ) -> Result<RevisionTraceResult, RepositoryError>
  ```
  Implementation:
  1. Call `record_store::list_record_revisions(store, &query.record_id, Some(&query.field_id), None, None)?` to load all revisions for this field.
  2. Find the target revision by `revision_id` in the list. If absent, return `RepositoryError::NotFound`.
  3. Build `prior_chain`: Build an index `HashMap<&str, &Revision>` mapping `revision_id → Revision`. Starting from `target.prior_revision_id`, follow the chain; collect each ancestor and push to `chain`. Guard with a `HashSet<String>` (seen revision IDs) to break cycles. Stop when `prior_revision_id` is `None`. Reverse `chain` so it is oldest-first.
  4. Return `RevisionTraceResult { record_id: query.record_id, field_id: query.field_id, revision: target, prior_chain }`.

- [ ] Export `context_query_service` in `crates/srs-repository/src/lib.rs`:
  ```rust
  pub mod context_query_service;
  ```

- [ ] File a deferred GitHub issue for stage context (Lead Integrator task, done immediately after service tests pass):
  ```
  Title: "Service + CLI: stage context query pattern ({runId}/{stageId})"
  Body:  "Deferred from #251. Depends on the protocol run model (#252). Once runs
          are implemented, add get_stage_context(store, RunStageContextQuery { run_id, stage_id })
          returning chunks produced during that stage and fields active in that stage,
          following the same service→CLI→bindings pattern as the other context query patterns."
  Labels: enhancement
  ```
  Then link it under epic #236 using `gh-project link` (see `docs/project-management.md`).
  Post a comment on issue #251 with the new issue number.

#### Acceptance Criteria

- [ ] `get_field_context` returns current value and empty revisions for a record with no sidecar.
- [ ] `get_field_context` returns only revisions for the queried field (not other fields).
- [ ] `get_field_context` populates `ai_guidance` from the field definition when available and non-null.
- [ ] `get_field_context` sets `ai_guidance = None` when `field.ai_guidance` is `Value::Null`.
- [ ] `get_field_context` returns `RepositoryError::NotFound` for a nonexistent record_id.
- [ ] `get_record_context` returns all field values and associated relations (source-filtered).
- [ ] `get_revision_trace` returns the target revision and prior chain oldest-first.
- [ ] `get_revision_trace` returns `RepositoryError::NotFound` for a missing revision_id.
- [ ] Cross-store roundtrip: results identical between MemoryStore and JsonStore.
- [ ] No file paths, no business logic in srs-cli; no path strings in service logic.

#### Testing

```bash
cargo test -p srs-repository context_query
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `context_query_service.rs` (inside a `#[cfg(test)] mod tests { ... }` block):

- `field_context_no_revisions` — record exists, no sidecar; `revisions` is empty, `current_value` is the field's value
- `field_context_filters_by_field_id` — two fields with revisions; only the queried field's revisions appear
- `field_context_ai_guidance_from_package` — MemoryStore with a package loaded containing a field definition; `ai_guidance` matches `field.ai_guidance`
- `field_context_ai_guidance_null` — package field has `ai_guidance: Value::Null`; `result.ai_guidance` is `None`
- `field_context_not_found` — nonexistent record_id returns `Err(RepositoryError::NotFound { .. })`
- `record_context_field_values` — record with multiple fields; all appear in `field_values`
- `record_context_relations` — record with two relations in store; only the one sourced from the queried record appears
- `revision_trace_prior_chain` — three revisions chained (rev-1 → rev-2 → rev-3); querying rev-3 yields `prior_chain = [rev-1, rev-2]` (oldest first)
- `revision_trace_not_found` — nonexistent revision_id returns `Err(RepositoryError::NotFound { .. })`
- `field_context_cross_store_roundtrip` — write a record + revision sidecar into a MemoryStore, export to SRSJ via `JsonStore::from_srsj`/`export_srsj`, reload into a new JsonStore, call `get_field_context` on both stores; assert `revisions` length and `revision_id` values match

#### Milestone gate

1. All acceptance criteria checked.
2. All ten named tests exist and pass.
3. Run:
```bash
cargo test -p srs-repository context_query
cargo clippy -p srs-repository -- -D warnings
```
4. Deferred stage-context issue filed and parented under epic #236; comment posted on #251.
5. Mark checkboxes `[x]`, commit: `feat(srs-repository): context query service (field/record/revision trace) (#251)`.

---

### Phase 2: CLI commands and golden schemas

**Goal:** `srs context field|record|revision` subcommands are wired in `commands/mod.rs` and `commands/context.rs`, payload structs added with correct schemars annotations, golden schemas generated.

**Agent:** CLI Worker

#### Tasks

- [ ] Add three payload structs to `crates/srs-cli/src/payload.rs` (append after the existing Revision payloads around line 1430):

  ```rust
  // ── Context query payloads ────────────────────────────────────────────────────

  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ContextFieldPayload {
      pub record_id: String,
      pub field_id: String,
      pub field_name: Option<String>,
      pub field_namespace: Option<String>,
      pub ai_guidance: Option<serde_json::Value>,
      pub current_value: Option<serde_json::Value>,
      #[schemars(with = "Vec<serde_json::Value>")]
      pub revisions: Vec<srs_core::types::revision::Revision>,
      pub tagged_chunks: Vec<serde_json::Value>,
  }

  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ContextRecordPayload {
      pub record_id: String,
      pub type_id: String,
      pub type_name: String,
      pub type_namespace: String,
      pub display_label: String,
      #[schemars(with = "Vec<serde_json::Value>")]
      pub field_values: Vec<srs_core::types::record::FieldValue>,
      #[schemars(with = "Vec<serde_json::Value>")]
      pub relations: Vec<srs_repository::relation_service::RelationSummary>,
      pub tagged_chunks: Vec<serde_json::Value>,
      pub protocol_run_history: Vec<serde_json::Value>,
  }

  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ContextRevisionTracePayload {
      pub record_id: String,
      pub field_id: String,
      #[schemars(with = "serde_json::Value")]
      pub revision: srs_core::types::revision::Revision,
      #[schemars(with = "Vec<serde_json::Value>")]
      pub prior_chain: Vec<srs_core::types::revision::Revision>,
  }
  ```

  Note: `ai_guidance` and `current_value` are `Option<serde_json::Value>` — `serde_json::Value` already implements `JsonSchema` from the schemars re-export in srs-cli, so no `#[schemars(with = "...")]` is needed for those fields. Only `srs-core` types need the override.

- [ ] Create `crates/srs-cli/src/commands/context.rs`:

  ```rust
  use crate::commands::{with_store, CliContext};
  use crate::output;
  use crate::payload::{ContextFieldPayload, ContextRecordPayload, ContextRevisionTracePayload};
  use anyhow::Result;
  use clap::Subcommand;
  use srs_repository::context_query_service::{
      self, FieldContextQuery, RecordContextQuery, RevisionTraceQuery,
  };

  #[derive(Subcommand)]
  pub enum ContextCommand {
      /// Assemble context for a single field: current value, revision history, aiGuidance
      Field {
          /// Record instance ID
          record_id: String,
          /// Field ID
          field_id: String,
      },
      /// Assemble context for a record: all field values and relations
      Record {
          /// Record instance ID
          record_id: String,
      },
      /// Trace a revision: value, source refs, and prior revision chain
      Revision {
          /// Record instance ID
          record_id: String,
          /// Field ID
          field_id: String,
          /// Revision ID to trace
          revision_id: String,
      },
  }

  pub fn dispatch(ctx: CliContext, cmd: ContextCommand) -> Result<String> {
      match cmd {
          ContextCommand::Field { record_id, field_id } => {
              cmd_context_field(ctx, record_id, field_id)
          }
          ContextCommand::Record { record_id } => cmd_context_record(ctx, record_id),
          ContextCommand::Revision { record_id, field_id, revision_id } => {
              cmd_context_revision(ctx, record_id, field_id, revision_id)
          }
      }
  }

  fn cmd_context_field(ctx: CliContext, record_id: String, field_id: String) -> Result<String> {
      with_store(&ctx, |store| {
          match context_query_service::get_field_context(
              store,
              FieldContextQuery { record_id: record_id.clone(), field_id: field_id.clone() },
          ) {
              Ok(result) => output::serialize(
                  "context field",
                  ContextFieldPayload {
                      record_id: result.record_id,
                      field_id: result.field_id,
                      field_name: result.field_name,
                      field_namespace: result.field_namespace,
                      ai_guidance: result.ai_guidance,
                      current_value: result.current_value,
                      revisions: result.revisions,
                      tagged_chunks: result.tagged_chunks,
                  },
              ),
              Err(e) => Ok(output::err("context field", vec![e.to_string()])),
          }
      })
  }

  fn cmd_context_record(ctx: CliContext, record_id: String) -> Result<String> {
      with_store(&ctx, |store| {
          match context_query_service::get_record_context(
              store,
              RecordContextQuery { record_id: record_id.clone() },
          ) {
              Ok(result) => output::serialize(
                  "context record",
                  ContextRecordPayload {
                      record_id: result.record_id,
                      type_id: result.type_id,
                      type_name: result.type_name,
                      type_namespace: result.type_namespace,
                      display_label: result.display_label,
                      field_values: result.field_values,
                      relations: result.relations,  // Vec<RelationSummary> → directly assigned
                      tagged_chunks: result.tagged_chunks,
                      protocol_run_history: result.protocol_run_history,
                  },
              ),
              Err(e) => Ok(output::err("context record", vec![e.to_string()])),
          }
      })
  }

  fn cmd_context_revision(
      ctx: CliContext,
      record_id: String,
      field_id: String,
      revision_id: String,
  ) -> Result<String> {
      with_store(&ctx, |store| {
          match context_query_service::get_revision_trace(
              store,
              RevisionTraceQuery {
                  record_id: record_id.clone(),
                  field_id: field_id.clone(),
                  revision_id: revision_id.clone(),
              },
          ) {
              Ok(result) => output::serialize(
                  "context revision",
                  ContextRevisionTracePayload {
                      record_id: result.record_id,
                      field_id: result.field_id,
                      revision: result.revision,
                      prior_chain: result.prior_chain,
                  },
              ),
              Err(e) => Ok(output::err("context revision", vec![e.to_string()])),
          }
      })
  }
  ```

- [ ] In `crates/srs-cli/src/commands/mod.rs`:
  1. Add `pub mod context;` to the module list (alongside the other `pub mod` declarations at the top).
  2. Add `Context(context::ContextCommand)` variant to the `Commands` enum (inside the `#[derive(Subcommand)] pub enum Commands { ... }` block, after the `Federation` variant):
     ```rust
     /// Addressability context-query commands (ext:addressability)
     #[command(subcommand)]
     Context(context::ContextCommand),
     ```
  3. Add dispatch arm to the `match` inside `pub fn dispatch(cli: Cli) -> Result<String>` (at the end of the existing match arms):
     ```rust
     Commands::Context(ctx_cmd) => context::dispatch(ctx, ctx_cmd),
     ```

- [ ] Run `cargo run --bin generate-schemas` and verify three new files appear:
  - `crates/srs-cli/schemas/payload/context-field.json`
  - `crates/srs-cli/schemas/payload/context-record.json`
  - `crates/srs-cli/schemas/payload/context-revision.json`

  Stage all three files with `git add crates/srs-cli/schemas/payload/context-*.json`.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- context field <id> <fid>` returns a JSON envelope with `"command": "context field"`.
- [ ] `cargo run --bin srs -- context record <id>` returns a JSON envelope with `"command": "context record"`.
- [ ] `cargo run --bin srs -- context revision <id> <fid> <rev>` returns a JSON envelope with `"command": "context revision"`.
- [ ] `cargo test --test payload_contracts` passes with the three new schemas committed.
- [ ] Handlers are ≤15 lines each (no business logic).
- [ ] `ContextCommand` enum, `Commands::Context` variant, and dispatch arm all live in `commands/mod.rs` (plus the `context.rs` handler module).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests:
- Payload contract golden tests auto-generated by `generate-schemas` and enforced by `payload_contracts`.

#### Milestone gate

1. All acceptance criteria checked.
2. Tests pass.
3. Run:
```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```
4. Mark checkboxes `[x]`, commit: `feat(srs-cli): context field|record|revision commands + golden schemas (#251)`.

---

### Phase 3: WASM bindings

**Goal:** Three `context_*` methods on `SrsRepository` in `crates/srs-bindings/src/lib.rs`, covered by native service-function smoke tests.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add `use srs_repository::context_query_service;` import at the top of `crates/srs-bindings/src/lib.rs` (alongside the existing `use srs_repository::...` imports).

- [ ] Add three methods to the `#[wasm_bindgen] impl SrsRepository { ... }` block in `crates/srs-bindings/src/lib.rs`. Use `&self.store` (plain field access — `SrsRepository.store` is a `JsonStore`, not a `RefCell`) and `to_js` (the local helper at line ~45 in `lib.rs`, not `serde_wasm_bindgen::to_value`):

  ```rust
  /// Assemble field context: current value, revision history, aiGuidance.
  pub fn context_field(&self, record_id: &str, field_id: &str) -> Result<JsValue, JsValue> {
      let result = context_query_service::get_field_context(
          &self.store,
          context_query_service::FieldContextQuery {
              record_id: record_id.to_string(),
              field_id: field_id.to_string(),
          },
      )
      .map_err(js_err)?;
      to_js(&result)
  }

  /// Assemble record context: all field values and relations.
  pub fn context_record(&self, record_id: &str) -> Result<JsValue, JsValue> {
      let result = context_query_service::get_record_context(
          &self.store,
          context_query_service::RecordContextQuery {
              record_id: record_id.to_string(),
          },
      )
      .map_err(js_err)?;
      to_js(&result)
  }

  /// Trace a revision: value at that revision and prior revision chain.
  pub fn context_revision(
      &self,
      record_id: &str,
      field_id: &str,
      revision_id: &str,
  ) -> Result<JsValue, JsValue> {
      let result = context_query_service::get_revision_trace(
          &self.store,
          context_query_service::RevisionTraceQuery {
              record_id: record_id.to_string(),
              field_id: field_id.to_string(),
              revision_id: revision_id.to_string(),
          },
      )
      .map_err(js_err)?;
      to_js(&result)
  }
  ```

- [ ] Add smoke tests in `crates/srs-bindings/tests/context_query.rs`. **Do NOT call `#[wasm_bindgen]` methods in native tests** (they panic outside a wasm runtime). Instead call the underlying service functions directly on a `JsonStore`, following the pattern in `crates/srs-bindings/tests/containers.rs`:

  ```rust
  // crates/srs-bindings/tests/context_query.rs
  use srs_repository::context_query_service::{
      self, FieldContextQuery, RecordContextQuery, RevisionTraceQuery,
  };
  use srs_repository::JsonStore;

  fn make_srsj_with_record_and_revision() -> String {
      // Build a minimal SRSJ string with one Tier-2 record and a revision sidecar.
      // Use serde_json::json!({...}) to construct; follow the pattern in containers.rs.
      // The record must have instanceId, typeId, typeName, typeNamespace, fieldValues.
      // The revision sidecar key is "<instanceId>.revisions.json" in the JSON store.
      todo!("construct minimal SRSJ")
  }

  #[test]
  fn context_field_smoke() {
      let srsj = make_srsj_with_record_and_revision();
      let store = JsonStore::from_srsj(&srsj).expect("load");
      let result = context_query_service::get_field_context(
          &store,
          FieldContextQuery { record_id: "rec-1".to_string(), field_id: "fld-1".to_string() },
      )
      .expect("get_field_context");
      assert_eq!(result.record_id, "rec-1");
      assert_eq!(result.field_id, "fld-1");
      // revisions count depends on sidecar content
  }

  #[test]
  fn context_record_smoke() {
      let srsj = make_srsj_with_record_and_revision();
      let store = JsonStore::from_srsj(&srsj).expect("load");
      let result = context_query_service::get_record_context(
          &store,
          RecordContextQuery { record_id: "rec-1".to_string() },
      )
      .expect("get_record_context");
      assert_eq!(result.record_id, "rec-1");
      assert!(!result.field_values.is_empty());
  }

  #[test]
  fn context_revision_smoke() {
      let srsj = make_srsj_with_record_and_revision();
      let store = JsonStore::from_srsj(&srsj).expect("load");
      // query must use a revision_id that actually exists in the sidecar
      let all = context_query_service::get_field_context(
          &store,
          FieldContextQuery { record_id: "rec-1".to_string(), field_id: "fld-1".to_string() },
      )
      .expect("field context");
      if let Some(first_rev) = all.revisions.first() {
          let result = context_query_service::get_revision_trace(
              &store,
              RevisionTraceQuery {
                  record_id: "rec-1".to_string(),
                  field_id: "fld-1".to_string(),
                  revision_id: first_rev.revision_id.clone(),
              },
          )
          .expect("get_revision_trace");
          assert_eq!(result.revision.revision_id, first_rev.revision_id);
      }
      // If no revisions, test still passes (smoke only)
  }
  ```

  Fill in `make_srsj_with_record_and_revision()` using `serde_json::json!({...})` following the same SRSJ format as the containers.rs test helper. Look at `crates/srs-bindings/tests/containers.rs` for the exact SRSJ structure.

#### Acceptance Criteria

- [ ] `context_field`, `context_record`, `context_revision` compile without errors.
- [ ] `cargo test -p srs-bindings` passes (including the three smoke tests).
- [ ] No business logic in binding methods (each is: receive string args → call service → `to_js`).
- [ ] Smoke tests call underlying service functions directly (no `#[wasm_bindgen]` method calls in native tests).

#### Testing

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests (`crates/srs-bindings/tests/context_query.rs`):
- `context_field_smoke` — JsonStore with a record; `get_field_context` returns the correct record_id and field_id.
- `context_record_smoke` — JsonStore with a record; `get_record_context` returns non-empty field_values.
- `context_revision_smoke` — if a revision exists, `get_revision_trace` returns the matching revision_id.

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
- `field.ai_guidance` of `Value::Null` maps to `ai_guidance: None` in the result (caller-facing "no guidance" rather than a null JSON value).
- `ContextRecordPayload.relations` is typed as `Vec<srs_repository::relation_service::RelationSummary>` with `#[schemars(with = "Vec<serde_json::Value>")]`; the handler assigns `result.relations` directly with no conversion.
- `SrsRepository.store` in `srs-bindings` is a plain `JsonStore` (not a `RefCell`); binding methods use `&self.store` directly.
- Binding methods use the local `to_js` helper (not `serde_wasm_bindgen::to_value`) to honour `#[serde(rename_all = "camelCase")]` on nested structs.
