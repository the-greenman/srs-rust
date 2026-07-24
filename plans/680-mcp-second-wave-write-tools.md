# Plan: MCP Second Wave — Validated Write Tools

## Summary

The first MCP cut (`srs-rust#676`) shipped six tools: `repo_validate`, `find`, `record_create`,
`relation_create`, `note_create`, and `type_schema`. Issue #680 extends the MCP server with
the remaining validated write workflows that already exist as service functions and CLI handlers:
`record_update`, `record_transition` (with RFC-022 fulfillment), `record_successor`,
`note_graduate`, `container_member_add`, and `container_member_remove`. A companion read tool
`record_allowed_transitions` ships alongside `record_transition` so agents know which transitions
are available before calling it.

All work is in `crates/srs-mcp/src/tools.rs`. Services are unchanged — this is a pure adapter
addition following ADR-037.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | MCP Adapter Worker |
| MCP Adapter Worker | — |
| Verification | Verification Agent — runs after Phase 1 milestone gate on the final diff |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | All new tools follow the shadow-input/`From`-conversion drift-guard pattern; one service call per handler; no business logic in `srs-mcp`. | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Handlers call the exact same service functions as the equivalent CLI handlers. No divergence permitted. | accepted |

No new ADRs are required — this plan implements established ADR-037 patterns.

---

## Contracts

### CLI output contract (ADR-011)

No CLI payload structs are added or changed. The new tools live entirely in `srs-mcp` and
follow the MCP envelope carve-out (ADR-037 §4) — they never emit the ADR-011 JSON envelope.

Verification: `cargo test --test payload_contracts` must still pass (no payload structs changed).

### Entity schema sync (check-schema-sync.sh)

No entity schemas under `srs/docs/schema/2.0/` are modified. No sync action required.

---

## Scope

- Add **seven new MCP tools** to `crates/srs-mcp/src/tools.rs`:
  1. `record_update` — mirrors `record_store::update_record(store, instance_id, UpdateRecordInput)`
  2. `record_transition` — mirrors `record_store::transition_record_lifecycle(store, instance_id, TransitionLifecycleInput)`, including RFC-022 fulfillment sub-structs
  3. `record_allowed_transitions` — mirrors `record_store::get_allowed_lifecycle_transitions(store, instance_id)` (read tool, companion to `record_transition`)
  4. `record_successor` — mirrors `record_store::create_record_successor(store, predecessor_id, CreateRecordSuccessorInput)`
  5. `note_graduate` — mirrors `services::graduate_note(store, GraduateNoteInput)`
  6. `container_member_add` — mirrors `container_service::add_container_member(store, container_id, instance_id)`
  7. `container_member_remove` — mirrors `container_service::remove_container_member(store, container_id, instance_id)`
- For each tool: add a `TOOL_*` name constant, a `DESC_*` description constant, a shadow input struct with `#[derive(Deserialize, JsonSchema)]`, a `From` conversion to the service type, and a `call_tool` match arm.
- Extend `list_tools()` to advertise all thirteen tools (six existing + seven new).
- Extend `tool_input_conversion_exercises_every_field` to cover every field of each new shadow struct.
- Add a `list_tools_advertises_all_thirteen_with_schemas` test (replacing the existing six-tool count test).

**Out of scope:**
- Any changes to `srs-repository` services (they're unchanged).
- Any CLI payload changes.
- WASM bindings for these operations (deferred).
- `container_member_list` as a new tool (already exposed via the `navigation` resource; a list tool is a separate issue if needed).
- `sourceRefs` on `NoteSection.contentHint` (deferred, tracked in the existing note in `NoteSectionInput`).

---

## Phases

### Phase 1: Shadow Input Structs + Tool Handlers

**Goal:** All seven tools are registered, dispatch correctly, and every shadow input field is
covered by the drift-guard test.

**Agent:** MCP Adapter Worker

#### Tasks

- [x] Add imports to `tools.rs`: `record_store::{update_record, transition_record_lifecycle, get_allowed_lifecycle_transitions, create_record_successor, UpdateRecordInput, TransitionLifecycleInput, TransitionFulfillmentInput, FulfillmentNewRecord, CreateRecordSuccessorInput}`, `container_service::{add_container_member, remove_container_member}`, `services::{graduate_note, GraduateNoteInput}`.
- [x] Add `TOOL_RECORD_UPDATE`, `TOOL_RECORD_TRANSITION`, `TOOL_RECORD_ALLOWED_TRANSITIONS`, `TOOL_RECORD_SUCCESSOR`, `TOOL_NOTE_GRADUATE`, `TOOL_CONTAINER_MEMBER_ADD`, `TOOL_CONTAINER_MEMBER_REMOVE` constants.
- [x] Add `DESC_*` constants for each new tool (see descriptions below).
- [x] Add shadow input structs with `From` conversions. All top-level shadow structs carry `#[serde(rename_all = "camelCase", deny_unknown_fields)]` (matching the existing pattern). Nested sub-structs (no direct service type conversion) carry `#[serde(rename_all = "camelCase")]` only.

  **Structs and their exact From bodies:**

  - `RecordUpdateToolInput` { `instance_id: String`, `field_values: Vec<FieldValueInput>`, `group_values: Option<Vec<FieldGroupValueInput>>`, `tags: Option<Vec<String>>`, `type_version: Option<u32>` } — `deny_unknown_fields`. Handler extracts `instance_id` before conversion. `From<RecordUpdateToolInput> for UpdateRecordInput` maps: `field_values`, `group_values`, `tags`, `type_version` (all directly).

  - `FulfillmentNewRecordInput` { `field_values: Vec<FieldValueInput>`, `type_version: Option<u32>` } → `From` → `FulfillmentNewRecord { field_values: input.field_values.into_iter().map(Into::into).collect(), type_version: input.type_version }`.

  - `TransitionFulfillmentToolInput` { `new_record: Option<FulfillmentNewRecordInput>`, `existing_instance_id: Option<String>`, `relation_type: Option<String>` } → `From` → `TransitionFulfillmentInput { new_record: input.new_record.map(Into::into), existing_instance_id: input.existing_instance_id, relation_type: input.relation_type }`.

  - `RecordTransitionToolInput` { `instance_id: String`, `to: Option<String>`, `by_transition: Option<String>`, `fulfillment: Option<TransitionFulfillmentToolInput>` } — `deny_unknown_fields`. Handler extracts `instance_id`. `From` → `TransitionLifecycleInput { to: input.to, by_transition: input.by_transition, fulfillment: input.fulfillment.map(Into::into) }`.

  - `RecordAllowedTransitionsToolInput` { `instance_id: String` } — `deny_unknown_fields`. No service struct conversion — `instance_id` is passed directly to the service.

  - `RecordSuccessorToolInput` { `predecessor_id: String`, `relation_type: String`, `field_values: Vec<FieldValueInput>`, `lifecycle_state: Option<String>`, `type_version: Option<u32>` } — `deny_unknown_fields`. Handler extracts `predecessor_id`. `From` → `CreateRecordSuccessorInput { relation_type: input.relation_type, field_values: input.field_values.into_iter().map(Into::into).collect(), lifecycle_state: input.lifecycle_state, type_version: input.type_version }`.

  - `NoteGraduateToolInput` { `note_id: String`, `#[serde(rename="type")] type_ref: String`, `type_version: Option<u32>`, `field_values: Vec<FieldValueInput>`, `group_values: Option<Vec<FieldGroupValueInput>>`, `tags: Option<Vec<String>>`, `container_id: Option<String>` } — `deny_unknown_fields`. `From` → `GraduateNoteInput { note_id: input.note_id, type_ref: input.type_ref, type_version: input.type_version, container_id: input.container_id, record_input: CreateRecordInput { field_values: input.field_values.into_iter().map(Into::into).collect(), group_values: input.group_values.map(|gs| gs.into_iter().map(Into::into).collect()), tags: input.tags } }`. Note: `field_values`, `group_values`, `tags` land in `result.record_input.*`, not directly on `GraduateNoteInput`.

  - `ContainerMemberToolInput` { `container_id: String`, `instance_id: String` } — `deny_unknown_fields`, shared by add and remove. Fields passed directly to the service; no service struct conversion needed.
- [x] Add `call_tool` match arms for all seven new tools.
- [x] Update `list_tools()` to include all seven new `Tool::new(...)` entries in this order (after the existing six): `TOOL_RECORD_UPDATE`, `TOOL_RECORD_TRANSITION`, `TOOL_RECORD_ALLOWED_TRANSITIONS`, `TOOL_RECORD_SUCCESSOR`, `TOOL_NOTE_GRADUATE`, `TOOL_CONTAINER_MEMBER_ADD`, `TOOL_CONTAINER_MEMBER_REMOVE`.
- [x] Update drift-guard test `tool_input_conversion_exercises_every_field` to include all new shadow structs. For `NoteGraduateToolInput` specifically, the assertions must verify that `field_values` maps to `result.record_input.field_values` (not a top-level field) — the nesting is the most likely failure mode.
- [x] Rename `list_tools_advertises_all_six_with_schemas` to `list_tools_advertises_all_thirteen_with_schemas` and update its expected `names` slice to all 13 tool constants in the same order they appear in `list_tools()`.
- [x] `record_successor` handler calls `tool_ok(&result)` on the whole `CreateRecordSuccessorResult` (which serializes `{record, relation}`) — not `tool_ok(&result.record)`. This is different from `record_create` which returns only the record; for successor, both artifacts are needed.

#### Tool Descriptions

```
RECORD_UPDATE: "Replace the fieldValues of an existing Tier-2 Record. Provide the full set of
field values you want stored (full replace, not a patch). Optional typeVersion migrates the
record to a different type version; omit to keep the stored version. Optional tags: null=preserve,
[]=clear, [...]=replace. Returns the updated Record. Run repo_validate after to confirm
consistency."

RECORD_TRANSITION: "Transition a record's lifecycle state as defined in its Type's lifecycle.
Use record_allowed_transitions first to see which transitions are available. Supply either 'to'
(target state key) or 'byTransition' (named transition, e.g. 'promote'). RFC-022: when the
target state has a requiresRelation obligation, supply 'fulfillment.newRecord' (spawn a successor)
or 'fulfillment.existingInstanceId' (adopt an existing instance). Returns the updated record,
any warnings (e.g. final-state notice), and the fulfillment artifacts if spawned."

RECORD_ALLOWED_TRANSITIONS: "Return the allowed next lifecycle transitions from a record's current
state. Returns currentState (empty string if never transitioned), a list of transitions each
with name/to/toIsFinal/requiresRelation, and isImmutable. Read this before calling
record_transition — an unknown transition is rejected."

RECORD_SUCCESSOR: "Create a successor Record and the linking relation in one atomic operation.
The successor inherits the predecessor's typeId (and optionally a pinned typeVersion). relationType
must be 'supersedes' or 'refines'. Validation is enforced before any write. Returns the new
Record and the linking Relation."

NOTE_GRADUATE: "Promote a Tier-0 Note to a typed Tier-2 Record in one atomic step. The Note's
graduated_at is stamped; a new Record is created from the supplied type and fieldValues. Optional
containerId adds the Record to a container. The Note is preserved with its graduated_at timestamp
— it is not deleted. Returns both the updated Note and the new Record."

CONTAINER_MEMBER_ADD: "Add an instance to a container's memberInstanceIds. Idempotent — adding
an already-present member is not an error. Returns the updated memberInstanceIds list."

CONTAINER_MEMBER_REMOVE: "Remove an instance from a container's memberInstanceIds. Returns the
updated memberInstanceIds list. No-op if the instance is not a member."
```

#### Acceptance Criteria

- [x] All seven new tool names appear in `list_tools()` output.
- [x] Each new tool has a non-empty description and a non-empty input schema.
- [x] `record_update` calls `update_record(store, instance_id, ...)` and returns the updated `Record`.
- [x] `record_transition` calls `transition_record_lifecycle(store, instance_id, ...)` and returns the `TransitionLifecycleResult` (record + warnings + optional successor/relation).
- [x] `record_allowed_transitions` calls `get_allowed_lifecycle_transitions(store, instance_id)` and returns the `AllowedLifecycleTransitionsResult`.
- [x] `record_successor` calls `create_record_successor(store, predecessor_id, ...)` and returns `{record, relation}`.
- [x] `note_graduate` calls `graduate_note(store, ...)` and returns `{note, record}`.
- [x] `container_member_add` calls `add_container_member(store, container_id, instance_id)` and returns the member list.
- [x] `container_member_remove` calls `remove_container_member(store, container_id, instance_id)` and returns the member list.
- [x] `tool_input_conversion_exercises_every_field` covers every field of every new shadow struct.
- [x] `cargo test -p srs-mcp` passes.
- [x] `cargo clippy -p srs-mcp -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-mcp
cargo clippy -p srs-mcp -- -D warnings
```

Tests to write or verify:

- `tool_input_conversion_exercises_every_field` — extended to cover all seven new shadow structs with every field populated, asserting the `From` conversion carries every field.
- `list_tools_advertises_all_thirteen_with_schemas` — asserts tool names, descriptions present, input schemas non-empty.

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm drift-guard test and list-tools test pass.
3. Run:

```bash
cargo test -p srs-mcp
cargo clippy -p srs-mcp -- -D warnings
```

4. Mark checkboxes `[x]`.
5. Commit:

```bash
git add crates/srs-mcp/src/tools.rs plans/680-mcp-second-wave-write-tools.md
git commit -m "feat(srs-mcp): add second wave write tools (#680)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] All thirteen tools listed and advertised in `list_tools()`
- [ ] Drift-guard test covers every field of every shadow input struct

## Coordination Rules

- MCP Adapter Worker writes `crates/srs-mcp/**` only.
- No service signatures are changed — all work is in the adapter layer.
- Lead Integrator owns final tool naming and description text.
- Verification Agent reviews the diff post-implementation.

## Assumptions

- The existing `FieldValueInput`, `FieldGroupValueInput`, `FieldGroupEntryInput`, and `FieldValueEntryInput` shadow structs (and their `From` conversions) are reused as-is for the new tools that need `Vec<FieldValue>` or `Option<Vec<FieldGroupValue>>`.
- `ContainerMembersResult` wrapping: `add_container_member` / `remove_container_member` return `Vec<String>`. For MCP, this is serialized as-is (a JSON array string). If a named struct is preferred, a wrapper `{memberInstanceIds: [...]}` can be added without affecting the service boundary.
- `record_allowed_transitions` is included as a read companion to `record_transition` even though it is not a write tool — it is essential for usable transition workflows.
- `container_member_list` is deliberately excluded: the `navigation` resource already exposes membership; a separate list tool would duplicate that surface.
