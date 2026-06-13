# Plan: Blueprint Brief Command (#129)

## Summary

Add `srs blueprint brief <id> [--format markdown|json]` — a new read-only render command
that composes, for one Blueprint, the full layered guidance context in the spec's recommended
AI guidance composition order: Blueprint `aiGuidance`, each root Type's `aiGuidance` and its
Fields in `order`, `structure[]` RelationSpecs, and any Protocol whose `targetType` matches a
root Type. The brief serves dual purposes: human review of guidance quality, and literal Claude
Code context for extraction sessions. Closes #129.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All business logic in `srs-repository`; CLI handler is one service call ≤ ~15 lines | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Named payload struct in `payload.rs`; golden schema committed | accepted |
| [ADR-014](../docs/adr/014-composite-schema-property-naming.md) | camelCase property names in JSON output | accepted |

No new ADRs required. This plan implements ADR-010 and ADR-011 for a new command without
establishing new architectural constraints.

---

## Contracts

### CLI output contract (ADR-011)

New command added: `blueprint brief`.

- Add `BlueprintBriefPayload` + constituent structs (`BriefType`, `BriefField`,
  `BriefRelationSpec`, `BriefProtocol`, `BriefStage`) to
  `crates/srs-cli/src/payload.rs`. All derive `Debug, Serialize, JsonSchema` and
  carry `#[serde(rename_all = "camelCase")]`.
- Wire handler to `output::serialize("blueprint brief", BlueprintBriefPayload { ... })`.
- After adding structs: `cargo run --bin generate-schemas` → commit
  `crates/srs-cli/schemas/payload/blueprint-brief.json`.

Verification: `cargo test --test payload_contracts` must pass after Phase 2.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- `srs blueprint brief <id> [--format markdown|json]` — read-only; no writes
- `--format markdown` (default): `rendered` field in JSON envelope contains the markdown prose
- `--format json`: same JSON envelope; user reads structured `types`, `structure`, `protocol` fields
- Both formats always populate all payload fields (rendered + structured); format flag is a usage hint
- New service: `crates/srs-repository/src/blueprint_brief_service.rs`
- Extended: `crates/srs-repository/src/protocol_service.rs` — one new public function
- New payload structs + golden schema file

**Out of scope:**
- Writing or persisting the brief to disk (no `--output` flag in this iteration)
- Streaming or incremental output
- Extending `ProtocolStage` in `srs-core` — rich stage fields are extracted from raw JSON
- Gallery-project-v2 Protocol data (depends on issue #48; acceptance test uses any available blueprint)
- Any writes to the SRS spec repo (`srs/`)

---

## Phases

### Phase 1: Service layer

**Goal:** `blueprint_brief_service::blueprint_brief(store, input)` returns a fully typed
`BlueprintBriefResult` with all composited guidance data; `protocol_service::find_protocol_by_target_type`
is implemented and covered by tests.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `crates/srs-repository/src/protocol_service.rs`:
  ```rust
  pub struct FindProtocolByTargetTypeResult {
      pub protocol_id: String,
      pub protocol_name: String,
      pub stages_raw: Vec<serde_json::Value>,
  }

  pub fn find_protocol_by_target_type(
      store: &dyn RepositoryStore,
      target_type_id: &str,
  ) -> Result<Option<FindProtocolByTargetTypeResult>, RepositoryError>
  ```
  Implementation: call `list_records_by_type(store, "com.semanticops.srs", "meta.protocol")`;
  for each record, call `find_fv(&record.field_values, FIELD_PROTOCOL_TARGET_TYPE)` and compare
  to `target_type_id`; on match, extract `find_fv(&record.field_values, FIELD_PROTOCOL_STAGES)`
  as `Vec<serde_json::Value>` via `serde_json::from_value`; also extract `FIELD_PROTOCOL_ID` and
  `FIELD_PROTOCOL_NAME` via `get_string_fv`; return first match. The existing `FIELD_PROTOCOL_*`
  constants and `find_fv`/`get_string_fv` helpers are already in scope in that file.

- [ ] Create `crates/srs-repository/src/blueprint_brief_service.rs`.

  **Input/output structs** (all `pub`, no `Serialize`/`Deserialize` on service types — that lives
  in `payload.rs`):
  ```rust
  pub struct BlueprintBriefInput { pub blueprint_id: String }

  pub struct BriefFieldResult {
      pub field_id: String,
      pub name: String,
      pub order: u32,
      pub required: bool,
      pub value_type: String,          // serialized ValueType (e.g. "string")
      pub ai_guidance: Option<serde_json::Value>,
  }

  pub struct BriefTypeResult {
      pub type_id: String,
      pub namespace: String,
      pub name: String,
      pub ai_guidance: Option<serde_json::Value>,
      pub fields: Vec<BriefFieldResult>,  // sorted ascending by order
  }

  pub struct BriefRelationSpecResult {
      pub relation_type: String,
      pub source_type_id: String,
      pub target_type_id: String,
      pub cardinality: Option<String>,
  }

  pub struct BriefStageResult {
      pub stage_id: String,
      pub name: String,
      pub order: i32,
      pub depends_on: Vec<String>,
      pub question: Option<String>,
      pub completion_criteria: Option<String>,
      pub contributes_to: Option<Vec<String>>,
      pub ai_guidance: Option<serde_json::Value>,
  }

  pub struct BriefProtocolResult {
      pub protocol_id: String,
      pub protocol_name: String,
      pub stages: Vec<BriefStageResult>,  // sorted ascending by order
  }

  pub struct BlueprintBriefResult {
      pub blueprint_id: String,
      pub namespace: String,
      pub name: String,
      pub version: u32,
      pub ai_guidance: Option<serde_json::Value>,
      pub required_types: Vec<serde_json::Value>,   // pass through TypeRef as Value
      pub types: Vec<BriefTypeResult>,
      pub structure: Vec<BriefRelationSpecResult>,
      pub protocol: Option<BriefProtocolResult>,
      pub diagnostics: Vec<String>,
  }
  ```

  **`blueprint_brief` function**:
  1. Call `get_blueprint_by_id(store, &input.blueprint_id)` — return
     `RepositoryError::BlueprintNotFound` if missing (propagate from service; caller wraps to
     `output::err`).
  2. For each `TypeRef` in `blueprint.root_types`:
     - Resolve via `package_service::get_type_by_id_latest(store, &tr.type_id)` (or
       `get_type_by_id(store, &tr.type_id, v)` when `type_version` is `Some(v)`).
     - On `GetTypeResult::NotFound`: push diagnostic `"root type {type_id} not found in package"`
       and skip.
     - Sort the type's `fields: Vec<FieldAssignment>` ascending by `order`.
     - For each `FieldAssignment`:
       - Resolve field via `package_service::get_field_by_id(store, &fa.field_id)`.
       - On `GetFieldResult::NotFound`: push diagnostic `"field {field_id} not found"` and skip.
       - Extract `ai_guidance`: `field.ai_guidance` (already `serde_json::Value`; use `None` if
         it is `Value::Null`).
       - Collect `BriefFieldResult`.
     - Extract type `ai_guidance` from `record_type.extra.get("aiGuidance")`.
     - Collect `BriefTypeResult`.
  3. Map `blueprint.structure` → `Vec<BriefRelationSpecResult>`.
  4. Find protocol: iterate over `blueprint.root_types`; call
     `find_protocol_by_target_type(store, &tr.type_id)`; return the first `Some(...)` as
     `BriefProtocolResult` with stages deserialized (see below). If no match, `protocol = None`.
  5. Deserialize each raw stage `serde_json::Value` → `BriefStageResult` using a private helper:
     ```rust
     fn deserialize_stage(v: &serde_json::Value) -> Result<BriefStageResult, String> {
         let stage_id = v.get("stageId").and_then(|x| x.as_str())
             .map(|s| s.to_string())
             .ok_or_else(|| "missing stageId".to_string())?;
         let name = v.get("name").and_then(|x| x.as_str())
             .map(|s| s.to_string())
             .ok_or_else(|| format!("stage {stage_id}: missing name"))?;
         let order = v.get("order").and_then(|x| x.as_i64())
             .map(|n| n as i32)
             .ok_or_else(|| format!("stage {stage_id}: missing order"))?;
         let depends_on = v.get("dependsOn")
             .and_then(|x| x.as_array())
             .map(|arr| arr.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect())
             .unwrap_or_default();
         let question = v.get("question").and_then(|x| x.as_str()).map(|s| s.to_string());
         let completion_criteria = v.get("completionCriteria").and_then(|x| x.as_str()).map(|s| s.to_string());
         let contributes_to = v.get("contributesTo")
             .and_then(|x| x.as_array())
             .map(|arr| arr.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect());
         let ai_guidance = v.get("aiGuidance").cloned();
         Ok(BriefStageResult { stage_id, name, order, depends_on, question,
             completion_criteria, contributes_to, ai_guidance })
     }
     ```
     Call `deserialize_stage(&raw_val)` for each element. On `Err(msg)`, push
     `format!("protocol stage deserialization: {msg}")` as a diagnostic and skip that stage.
     Sort the collected `Vec<BriefStageResult>` ascending by `order`.
  6. Return `BlueprintBriefResult`.

  **`render_brief_markdown` function** (`pub fn render_brief_markdown(result: &BlueprintBriefResult) -> String`):
  Outputs sections in composition order:
  1. `# Blueprint: {namespace}/{name} v{version}` — then `aiGuidance` prose if present.
     If `required_types` is non-empty, list them.
  2. For each type: `## Type: {namespace}/{name}` — then type `aiGuidance` if present, then a
     field table with columns `Field | ValueType | Required | Purpose | Extraction | Negative | Examples`.
  3. If `structure` is non-empty: `## Structure` — bullet list of RelationSpecs.
  4. If `protocol` is `Some`: `## Protocol: {name}` — ordered list of stages with
     `**{name}** ({order})` heading, then question/criteria/contributesTo if present.
  Returns the assembled `String`.

- [ ] Declare `pub mod blueprint_brief_service;` in `crates/srs-repository/src/lib.rs`.

- [ ] Register `blueprint_brief_service` exports in `crates/srs-repository/src/services.rs` if
  that file re-exports service modules (check; add if pattern exists there).

#### Acceptance Criteria

- [ ] `find_protocol_by_target_type(store, id)` returns `Some(...)` for a record whose
  `FIELD_PROTOCOL_TARGET_TYPE` value equals `id`.
- [ ] `find_protocol_by_target_type(store, "unknown")` returns `None`.
- [ ] `blueprint_brief(store, input)` on a missing `blueprint_id` propagates
  `RepositoryError::BlueprintNotFound`.
- [ ] `blueprint_brief(store, input)` on a blueprint with one root type and two ordered fields
  returns both fields sorted by `order` ascending.
- [ ] `blueprint_brief(store, input)` with an unresolvable type ref adds a diagnostic and still
  returns `Ok(...)`.
- [ ] `render_brief_markdown(&result)` returns a string containing the blueprint name and at
  least one field name.

#### Testing

Specific tests to write in `blueprint_brief_service.rs` (inline `#[cfg(test)]` module using
`MemoryStore`):

- `test_brief_blueprint_not_found` — `blueprint_brief` with unknown id → `BlueprintNotFound`.
- `test_brief_basic_composition` — blueprint with one root type + two fields (order 2, order 1);
  result has both fields sorted `[order=1, order=2]`.
- `test_brief_protocol_match` — `find_protocol_by_target_type` returns the matching protocol.
- `test_brief_protocol_no_match` — `find_protocol_by_target_type` returns `None`.
- `test_brief_unresolvable_type_is_diagnostic` — unresolvable type_id pushes diagnostic, result
  is still `Ok`.
- `test_render_markdown_contains_blueprint_name` — rendered output contains `blueprint.name`.

```bash
cargo test -p srs-repository blueprint_brief
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All 6 tests pass.
2. `cargo test -p srs-repository` passes with zero failures.
3. `cargo clippy -p srs-repository -- -D warnings` passes.
4. Mark task checkboxes `[x]`.
5. Commit: `feat(srs-repository): add blueprint_brief_service and find_protocol_by_target_type (#129)`

---

### Phase 2: Payload contract and CLI wiring

**Goal:** `srs blueprint brief <id> [--format markdown|json]` is wired end-to-end; golden schema
committed; `cargo test --test payload_contracts` passes.

**Agent:** CLI Worker

#### Tasks

- [ ] Add to `crates/srs-cli/src/payload.rs` (all derive `Debug, Serialize, JsonSchema`,
  all `#[serde(rename_all = "camelCase")]`):

  ```rust
  pub struct BriefField {
      pub field_id: String,
      pub name: String,
      pub order: u32,
      pub required: bool,
      pub value_type: String,
      // Use Value (not Option<Value>) for schemars — the existing pattern in payload.rs.
      // Rust field is Option so it serializes as null when absent.
      #[schemars(with = "serde_json::Value")]
      pub ai_guidance: Option<serde_json::Value>,
  }

  pub struct BriefType {
      pub type_id: String,
      pub namespace: String,
      pub name: String,
      #[schemars(with = "serde_json::Value")]
      pub ai_guidance: Option<serde_json::Value>,
      pub fields: Vec<BriefField>,
  }

  pub struct BriefRelationSpec {
      pub relation_type: String,
      pub source_type_id: String,
      pub target_type_id: String,
      pub cardinality: Option<String>,
  }

  pub struct BriefStage {
      pub stage_id: String,
      pub name: String,
      pub order: i32,
      pub depends_on: Vec<String>,
      pub question: Option<String>,
      pub completion_criteria: Option<String>,
      pub contributes_to: Option<Vec<String>>,
      #[schemars(with = "serde_json::Value")]
      pub ai_guidance: Option<serde_json::Value>,
  }

  pub struct BriefProtocol {
      pub protocol_id: String,
      pub protocol_name: String,
      pub stages: Vec<BriefStage>,
  }

  pub struct BlueprintBriefPayload {
      /// Markdown prose suitable for pasting to an LLM. Always populated.
      pub rendered: String,
      pub blueprint_id: String,
      pub namespace: String,
      pub name: String,
      pub version: u32,
      #[schemars(with = "serde_json::Value")]
      pub ai_guidance: Option<serde_json::Value>,
      #[schemars(with = "Vec<serde_json::Value>")]
      pub required_types: Vec<serde_json::Value>,
      pub types: Vec<BriefType>,
      pub structure: Vec<BriefRelationSpec>,
      pub protocol: Option<BriefProtocol>,
      pub diagnostics: Vec<String>,
  }
  ```

- [ ] Add `BriefFormat` enum to `crates/srs-cli/src/commands/mod.rs` (after existing
  `OutputFormat` enum, before `Commands`):
  ```rust
  #[derive(Debug, Clone, Copy, Default, ValueEnum, PartialEq)]
  pub enum BriefFormat {
      #[default]
      Markdown,
      Json,
  }
  ```

- [ ] Add `Brief` variant to `BlueprintCommand` enum in
  `crates/srs-cli/src/commands/mod.rs` (after `Schema` variant):
  ```rust
  /// Compose full layered guidance context for a Blueprint
  Brief {
      /// Blueprint definition ID (UUID)
      id: String,
      /// Output format: markdown (default) or json
      #[arg(long, default_value = "markdown")]
      format: BriefFormat,
  },
  ```

- [ ] Add dispatch arm in `crates/srs-cli/src/commands/blueprint.rs`:
  ```rust
  BlueprintCommand::Brief { id, format } => cmd_blueprint_brief(ctx, id, format),
  ```
  Update the `use crate::commands::{..., BriefFormat, ...}` import line and the
  `use crate::payload::{..., BlueprintBriefPayload, BriefField, BriefProtocol,
  BriefRelationSpec, BriefStage, BriefType}` import line.

- [ ] Implement `fn cmd_blueprint_brief(ctx: CliContext, id: String, format: BriefFormat) -> Result<String>` in `crates/srs-cli/src/commands/blueprint.rs`:
  - One `with_store` call to `blueprint_brief_service::blueprint_brief(store, BlueprintBriefInput { blueprint_id: id.clone() })`.
  - On `BlueprintNotFound`: return `Ok(output::err("blueprint brief", vec![format!("Blueprint '{id}' not found")]))`.
  - Map `BlueprintBriefResult` to `BlueprintBriefPayload`: use
    `render_brief_markdown(&result)` for `rendered`; map each constituent struct directly.
  - `output::serialize("blueprint brief", payload)`.
  - The `format` parameter is accepted but does not branch the payload — both formats always
    return the full payload. Suppress the unused-variable lint by adding `let _ = format;`
    at the top of the function (do not use an `_format` prefix since the variable name appears
    in the public CLI doc string).
  - Handler must be ≤ 25 lines. ADR-010 states ~15 lines; ~25 lines is acceptable here due
    to the explicit `BlueprintNotFound` error branch, following the precedent of
    `cmd_blueprint_schema` (lines 191–219 of `blueprint.rs`).

- [ ] Add `use srs_repository::blueprint_brief_service::{self, BlueprintBriefInput}` and
  `use srs_repository::blueprint_brief_service::render_brief_markdown` to
  `crates/srs-cli/src/commands/blueprint.rs`.

- [ ] Run `cargo run --bin generate-schemas` and stage
  `crates/srs-cli/schemas/payload/blueprint-brief.json`.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- blueprint brief --repo <any-valid-repo> <blueprint-id> --pretty`
  returns `{ "ok": true, "command": "blueprint brief", "payload": { "rendered": "<non-empty>",
  "blueprintId": "...", "types": [...], ... } }`.
- [ ] `cargo run --bin srs -- blueprint brief --repo <repo> <unknown-id>` returns `"ok": false`
  with `"Blueprint 'X' not found"` in `diagnostics`.
- [ ] `cargo run --bin srs -- blueprint brief --repo <repo> <id> --format json --pretty` returns
  populated `types` and `structure` arrays.
- [ ] `cargo run --bin srs -- blueprint brief --repo <repo> <id> --format markdown --pretty`
  returns non-empty `rendered` string.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] `blueprint-brief.json` exists in `crates/srs-cli/schemas/payload/`.
- [ ] Handler is one service call (ADR-010 check: ≤ 25 lines, no direct filesystem access).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

End-to-end smoke test using any existing blueprint in the spec repo:
```bash
cargo run --bin srs -- blueprint list --repo ../srs/srs --pretty
# pick a blueprint ID from output, then:
cargo run --bin srs -- blueprint brief --repo ../srs/srs <id> --pretty
```

#### Milestone gate

1. All acceptance criteria above are checked.
2. `cargo test --test payload_contracts` passes.
3. `cargo clippy -p srs-cli -- -D warnings` passes.
4. `blueprint-brief.json` is committed.
5. Mark task checkboxes `[x]`.
6. Commit: `feat(srs-cli): wire blueprint brief command and payload contract (#129)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `srs blueprint brief <id> --repo <test-repo> --pretty` returns non-empty `rendered` and
  populated `types` array with zero diagnostics for a valid blueprint
- [ ] `srs blueprint brief <unknown-id> --repo <test-repo>` returns `"ok": false`
- [ ] `srs blueprint brief --help` shows the `--format` flag

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and
  pass, update the plan checkboxes, then commit. Do not proceed to the next phase without
  completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Gallery-project-v2 Blueprint + Protocol data (issue #48 dependency) may not yet exist; Phase 2
  end-to-end tests use any blueprint available in `../srs/srs`.
- `ProtocolStage` in `srs-core` is NOT extended; rich stage fields (`question`,
  `completionCriteria`, `contributesTo`, `aiGuidance`) are extracted from the raw
  `serde_json::Value` stored in the protocol record's stages field value.
- `required_types` TypeRefs are passed through as `Vec<serde_json::Value>` (raw serialization)
  to avoid a `schemars` dependency in `srs-core`.
- Both `--format markdown` and `--format json` always return the same full JSON envelope with all
  payload fields populated. The format flag is accepted for user-ergonomic reasons but does not
  change the payload shape in this iteration.
- `services.rs` re-export: check at implementation time whether `blueprint_brief_service` needs
  a line there; follow the pattern of other services in that file.
