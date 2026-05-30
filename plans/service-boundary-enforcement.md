# Plan: Service Boundary Enforcement

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

The CLI currently contains 26 instances of business logic that should live in `srs-repository` services: container membership filtering duplicated across 4+ list handlers, multi-step create/delete orchestration wired in handlers, input parsing/normalization, validation rules, and branching service-selection logic. This blocks any future consumer (HTTP API, Python bindings, WASM) from sharing the same semantics without duplicating or rewriting the logic. This plan enforces ADR-001 (library-first) and ADR-010 (service boundary contract) by migrating all leaked logic to the service layer and establishing a consistent, enforceable handler pattern.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Service Layer Worker | — |
| CLI Cleanup Worker | — |
| Schema Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | CLI is a thin consumer of library crates; no business logic in handlers | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Every service function takes a typed input struct and returns a typed result struct; the CLI calls one function per logical operation | proposed → accept on plan approval |

---

## Enforcement Strategy

Rust visibility modifiers are the primary enforcement mechanism. Compile-time enforcement is preferred over documentation alone.

### What is enforced at compile time

**Membership functions are `pub(crate)` in `srs-repository`.**

`list_members`, `add_member`, `remove_member`, `is_member` in `container_service.rs` are changed from `pub` to `pub(crate)`. Because `srs-cli` is a separate crate, any attempt to import these functions produces a compile error:

```
error[E0603]: function `list_members` is private
  --> crates/srs-cli/src/commands/note.rs:8:5
```

This is unbreakable — no comment or convention can be bypassed; the compiler refuses.

**Internal helpers that should not cross the service boundary are `pub(crate)` or `pub(super)`.**

Any function in `srs-repository` that is implementation detail (not a service API) is `pub(crate)`. Only service-level functions that are the intended public API are `pub`.

### What is enforced by code structure

**Typed input/output structs.** Every public service function takes a named struct and returns a named struct. `serde_json::Value` parameters are not permitted on public service functions. This is enforced by inspection during code review — the pattern is unambiguous. A function with a `Value` parameter is immediately visible as a violation.

**Single `with_store()` per handler.** Because membership operations are `pub(crate)`, a handler physically cannot call `with_store` to do membership work. The only thing it can do with `with_store` is call the service function that already handles everything.

### The enforced handler pattern

```rust
// CORRECT — the entire handler
fn cmd_note_create(ctx: CliContext) -> Result<OutputDTO> {
    let input: CreateNoteInput = serde_json::from_reader(io::stdin())?;
    let result = with_store(&ctx, |store| Ok(note_service::create(store, input)?))?;
    Ok(output::ok("note create", result))
}

// CORRECT — flag-based list
fn cmd_note_list(ctx: CliContext, tag: Option<String>) -> Result<OutputDTO> {
    let filter = NoteListFilter { container_id: ctx.container_id, tag };
    let result = with_store(&ctx, |store| Ok(note_service::list(store, filter)?))?;
    Ok(output::ok("note list", result))
}
```

The following cannot appear in a handler — compile errors enforce most of these:
- Import or call to `list_members`, `add_member`, `remove_member`, `is_member` → compile error (`pub(crate)`)
- `.retain()` or `.filter()` on a list result → no members list to filter against
- More than one `with_store()` call → structurally unnecessary once services are atomic
- `serde_json::Value` field access (`.get()`, `.as_object_mut()`) → reviewed at PR time

### The enforced service function structure

```rust
// ── Input struct ─────────────────────────────────────────────────
// Defined alongside the service function in the same module.
// All fields that the CLI or any consumer must supply go here.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteInput {
    #[serde(flatten)]
    pub note: Note,
    // container_id is always passed explicitly — never read from global context
    pub container_id: Option<String>,
}

// ── Output struct ─────────────────────────────────────────────────
// Defined alongside the service function in the same module.
// All consumers receive the same typed result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteResult {
    pub note: Note,
}

// ── Service function ──────────────────────────────────────────────
// One function per logical operation. All validation and orchestration here.
pub fn create(store: &dyn RepositoryStore, input: CreateNoteInput) -> Result<NoteResult, RepositoryError> {
    // 1. Schema validate raw input (where applicable — see Phase 4)
    // 2. Semantic validation (validate_note, etc.)
    // 3. Container validation — may call pub(crate) is_member / get_container internally
    // 4. Write instance
    // 5. Add to container — may call pub(crate) add_member internally
    // 6. Return typed result
}
```

### Comment header for service modules

Every service module gets this header to orient future agents and contributors:

```rust
//! # Note Service
//!
//! Public API for note operations. This module is the sole entry point for
//! all note logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, container orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust
//! // CLI or API handler — this is the entire function body
//! let input: CreateNoteInput = serde_json::from_reader(io::stdin())?;
//! let result = note_service::create(store, input)?;
//! output::ok("note create", result)
//! ```
```

---

## Scope

- Migrate all 26 identified leaked logic items from CLI handlers to service functions
- Define typed input/output structs for every service operation
- Enforce one-with_store-call-per-handler rule in all CLI command handlers
- Register three currently-unregistered schemas (document-view, view, theme)
- Author two missing schemas (container, protocol)
- Add schema validation at service boundaries for note, field, type, relation, protocol create/update
- Add schema alignment tests for Field, RecordType, Container

**Out of scope:**
- New CLI commands or features
- HTTP API layer (no implementation, no OpenAPI spec)
- Python/WASM bindings implementation
- `ext:lifecycle` state machine enforcement
- Federation, themes, type inheritance implementation
- Changes to CLI output JSON shapes (output contract is frozen)

---

## Phases

### Phase 0: Compile-Time Enforcement Infrastructure

**Goal:** Membership functions are `pub(crate)`, making it physically impossible for CLI handlers to call them directly. This creates the compile-time fence before any migration work begins.

**Agent:** Service Layer Worker

This phase does not move any logic — it only changes visibility. The CLI will fail to compile after this phase until Phase 3 removes the imports. That is expected and correct: the compiler is now pointing at every violation.

#### Tasks

- [ ] In `crates/srs-repository/src/container_service.rs`: change `pub fn list_members`, `pub fn add_member`, `pub fn remove_member`, `pub fn is_member` to `pub(crate)`. Keep all other container service functions (`get_container`, `create_container`, `update_container`, `delete_container`, `list_containers`, `list_roots`, `add_root`, `remove_root`, `validate_container`, `list_members_full`) as `pub`.
- [ ] Add the service module doc comment header (see Enforcement Strategy section above) to `container_service.rs`, `services.rs`, `record_store.rs`, `tag_service.rs`, `relation_service.rs`, `package_service.rs`, `extension_service.rs`, `protocol_service.rs`
- [ ] Confirm `cargo build -p srs-repository` still succeeds (the repository crate itself uses these functions internally — `pub(crate)` is sufficient)
- [ ] Confirm `cargo build -p srs-cli` now **fails** with `error[E0603]` on `list_members`, `add_member`, `remove_member`, `is_member` imports — this failure is the proof the enforcement works
- [ ] Record the list of compile errors (file + line) in a comment at the bottom of this plan — these are the exact locations Phase 3 must fix

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` succeeds
- [ ] `cargo build -p srs-cli` fails with E0603 errors on membership function imports
- [ ] No other compile errors introduced (only the expected E0603s)
- [ ] All 8 service modules have the doc comment header

#### Testing

```bash
cargo build -p srs-repository          # must succeed
cargo build -p srs-cli 2>&1 | grep E0603  # must show membership import errors
```

#### Milestone gate

1. Verify `srs-repository` builds cleanly.
2. Verify `srs-cli` fails on exactly the membership imports (no other errors).
3. Record the E0603 error locations in the Assumptions section below.
4. Add doc headers to all 8 service modules.
5. Commit.

---

### Phase 1: Schema Housekeeping

**Goal:** All entity types that have CLI commands have a registered JSON schema; schema drift is detectable by CI.

**Agent:** Schema Worker

#### Tasks

- [ ] Copy `document-view.json`, `view.json`, `theme.json` from `/srs/docs/schema/2.0/` to `crates/srs-schema/schemas/2.0/`
- [ ] Author `container.json` in `/srs/docs/schema/2.0/` from the `Container` struct in `crates/srs-core/src/types/container.rs`. Required fields: `containerId` (UUID), `title` (string). All other fields optional. Use `additionalProperties: false`.
- [ ] Author `protocol.json` in `/srs/docs/schema/2.0/` from the `Protocol` struct fields. Required fields: `instanceId` (UUID), `typeId` (string), `typeVersion` (integer), `fieldValues` (array). Use `additionalProperties: false`.
- [ ] Copy `container.json` and `protocol.json` to `crates/srs-schema/schemas/2.0/`
- [ ] In `crates/srs-schema/src/lib.rs`: add 5 new `pub const` schema ID strings; add to `SCHEMA_SOURCES` array; update `registry_builds_and_has_all_schema_ids` assertion count from 13 to 18
- [ ] Add `minimal_field_passes_schema_contract` test to `crates/srs-core/src/types/field.rs` following the pattern in `note.rs`
- [ ] Add `minimal_record_type_passes_schema_contract` test to `crates/srs-core/src/types/record_type.rs`
- [ ] Add `minimal_container_passes_schema_contract` test to `crates/srs-core/src/types/container.rs`
- [ ] Write `scripts/check-schema-sync.sh`: for each file in `srs/docs/schema/2.0/`, verify a file with the same name and identical SHA256 exists in `crates/srs-schema/schemas/2.0/`; exit non-zero on mismatch

#### Acceptance Criteria

- [ ] `cargo test -p srs-schema` passes with schema count assertion at 18
- [ ] `cargo test -p srs-core` passes including 3 new schema alignment tests
- [ ] `scripts/check-schema-sync.sh` exits 0 with current files
- [ ] `scripts/check-schema-sync.sh` exits non-zero when any schema file in `srs/docs/schema/2.0/` has no matching copy in `crates/srs-schema/schemas/2.0/`

#### Testing

```bash
cargo test -p srs-schema
cargo test -p srs-core
bash scripts/check-schema-sync.sh
```

Specific tests to write or verify:

- `registry_builds_and_has_all_schema_ids` — proves 18 schemas are registered
- `minimal_field_passes_schema_contract` — proves Field Rust type is compatible with field.json schema
- `minimal_record_type_passes_schema_contract` — proves RecordType Rust type is compatible with type.json schema
- `minimal_container_passes_schema_contract` — proves Container Rust type is compatible with container.json schema

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-schema
cargo test -p srs-core
cargo clippy -p srs-schema -- -D warnings
cargo clippy -p srs-core -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

Do not start Phase 2 until the milestone gate passes.

---

### Phase 2: Typed Input/Output Structs for All Service Operations

**Goal:** Every service function has a typed input struct and a typed result struct; no `serde_json::Value` parameters on public service functions.

**Agent:** Service Layer Worker

Define all structs alongside their service function in the same module. Use `#[derive(Debug, Deserialize)]` with `#[serde(rename_all = "camelCase")]` for inputs; `#[derive(Debug, Serialize)]` with `#[serde(rename_all = "camelCase")]` for outputs.

#### Tasks

**`crates/srs-repository/src/services.rs` (note service):**
- [ ] Define `NoteListFilter { container_id: Option<String> }`
- [ ] Define `CreateNoteInput` that wraps `Note` + `container_id: Option<String>`
- [ ] Define `NoteResult { note: Note }`
- [ ] Define `NoteSummary { instance_id: String, title: Option<String> }`
- [ ] Rewrite `list_notes(store, filter: NoteListFilter) -> Result<Vec<NoteSummary>, RepositoryError>` — performs container filtering internally
- [ ] Rewrite `create_note(store, input: CreateNoteInput) -> Result<NoteResult, RepositoryError>` — validates container, creates note, adds member, all in one function
- [ ] Define `DeleteNoteInput { id: String, container_id: Option<String> }`
- [ ] Add `delete_note(store, input: DeleteNoteInput) -> Result<(), RepositoryError>` — checks membership, removes from container, deletes; all in one function

**`crates/srs-repository/src/record_store.rs`:**
- [ ] Define `RecordListFilter { type_namespace: Option<String>, type_name: Option<String>, container_id: Option<String> }`
- [ ] Define `CreateRecordInput { type_namespace: String, type_name: String, type_version: Option<u32>, field_values: Vec<FieldValue>, container_id: Option<String> }`
- [ ] Define `RecordResult { record: Record }`
- [ ] Define `RecordSummary { instance_id: String, type_namespace: String, type_name: String, type_version: u32 }`
- [ ] Rewrite `list_records(store, filter: RecordListFilter) -> Result<Vec<RecordSummary>, RepositoryError>` — performs container filtering internally
- [ ] Rewrite `create_record(store, input: CreateRecordInput) -> Result<RecordResult, RepositoryError>` — resolves type, creates record, adds to container, all in one function; type resolution logic moved from `record.rs` `resolve_type()` and `parse_type_filter()` helper functions
- [ ] Add `delete_record(store, id: String, container_id: Option<String>) -> Result<(), RepositoryError>` — checks membership, removes from container, deletes

**`crates/srs-repository/src/tag_service.rs`:**
- [ ] Define `TagListFilter { container_id: Option<String> }`
- [ ] Define `CreateTagInput` that wraps `TagDefinition` + `container_id: Option<String>`
- [ ] Define `TagResult { tag: TagDefinition }`
- [ ] Rewrite `list_tag_definitions(store, filter: TagListFilter) -> Result<Vec<TagSummary>, RepositoryError>` — performs container filtering internally
- [ ] Rewrite `create_tag_definition(store, input: CreateTagInput) -> Result<TagResult, RepositoryError>` — validates container, creates, adds member
- [ ] Add `delete_tag_definition(store, id: String, container_id: Option<String>) -> Result<(), RepositoryError>` — checks membership, removes, deletes

**`crates/srs-repository/src/relation_service.rs`:**
- [ ] Define `RelationListFilter { container_id: Option<String> }`
- [ ] Define `RelationResult { relation: Relation }`
- [ ] Rewrite `list_relations(store, filter: RelationListFilter) -> Result<Vec<RelationSummary>, RepositoryError>` — performs container filtering internally (source AND target both members)
- [ ] Rewrite `create_relation(store, relation: Relation) -> Result<RelationResult, RepositoryError>` — loads package internally (removes `defs` parameter from current signature)

**`crates/srs-repository/src/package_service.rs`:**
- [ ] Define `FieldListFilter { namespace: Option<String>, package: Option<String> }`
- [ ] Define `TypeListFilter { namespace: Option<String>, package: Option<String> }`
- [ ] Add `list_fields_filtered(store, filter: FieldListFilter) -> Result<Vec<FieldSummary>, RepositoryError>` — unified replacement for `list_fields`, `list_fields_by_namespace`, `list_fields_by_package`
- [ ] Add `list_types_filtered(store, filter: TypeListFilter) -> Result<Vec<TypeSummary>, RepositoryError>` — unified replacement for `list_types`, `list_types_by_namespace`, `list_types_by_package`
- [ ] Add `list_relation_types_filtered(store, status: Option<String>) -> Result<Vec<RelationTypeDefinition>, RepositoryError>` — moves status filtering from `relation_type.rs` handler
- [ ] Move `normalize_field_input` normalization logic from `crates/srs-cli/src/commands/field.rs` into `create_field_in_package()` — function accepts `serde_json::Value` and normalizes internally before deserializing

**`crates/srs-repository/src/extension_service.rs`:**
- [ ] Define `ExtensionSummary { instance_id: String, namespace: Option<String>, name: Option<String>, extension_type: String }`
- [ ] Define `ExtensionResult { extension: serde_json::Value }` (extensions are stored as generic Records; typed projection is the summary)
- [ ] Define `CreateExtensionInput { raw: serde_json::Value }` — service extracts fieldValues, infers type_id/version internally (moves logic from `cmd_extension_create`)
- [ ] Rewrite `list_extensions(store) -> Result<Vec<ExtensionSummary>, RepositoryError>` — returns typed summaries instead of generic `Vec<Record>`
- [ ] Add `create_extension(store, input: CreateExtensionInput) -> Result<ExtensionResult, RepositoryError>`
- [ ] Add `update_extension(store, id: String, input: CreateExtensionInput) -> Result<ExtensionResult, RepositoryError>`

**`crates/srs-repository/src/protocol_service.rs`:**
- [ ] Define `ProtocolImportInput { raw: serde_json::Value }` — service performs all field-ID mapping and FieldValue construction (moves the 153-line `cmd_protocol_import` body from `crates/srs-cli/src/commands/protocol.rs`)
- [ ] Define `ProtocolResult { protocol: serde_json::Value, instance_id: String }` — service injects `instanceId` into result (moves `obj.insert("instanceId", ...)` from `cmd_protocol_get`)
- [ ] Add `import_protocol(store, input: ProtocolImportInput) -> Result<ProtocolResult, RepositoryError>`
- [ ] Rewrite `get_protocol(store, id: String) -> Result<ProtocolResult, RepositoryError>` — returns complete struct with `instance_id` field populated

#### Acceptance Criteria

- [ ] No public service function in `srs-repository` has a `serde_json::Value` parameter (except `CreateExtensionInput.raw` and `ProtocolImportInput.raw` which are intentionally opaque at that boundary)
- [ ] Every entity's list function accepts a filter struct — no multiple list functions for the same entity type
- [ ] `cargo test -p srs-repository` passes
- [ ] `cargo build -p srs-cli` still succeeds (CLI may not compile yet — Phase 3 fixes that)

#### Testing

```bash
cargo test -p srs-repository
cargo build -p srs-cli  # may fail until Phase 3; that is acceptable
```

Specific tests to write or verify:

- `create_note_adds_to_container_when_container_id_provided` — proves container membership is set by service, not CLI
- `create_note_errors_when_container_not_found` — proves container validation is in service
- `delete_note_checks_membership_before_delete` — proves membership check is in service
- `list_notes_filters_by_container` — proves filtering is in service
- `list_records_filters_by_container` — proves filtering is in service
- `list_relations_filters_by_container` — proves both source and target must be members

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in the Testing section exists and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

### Phase 3: CLI Handler Cleanup

**Goal:** Every CLI handler conforms to the enforced pattern: one `from_reader` or flag-to-struct map, one `with_store` call, one `output::ok/err`.

**Agent:** CLI Cleanup Worker

#### Tasks

**`crates/srs-cli/src/commands/note.rs`:**
- [ ] `cmd_note_list`: replace `list_members` + `retain` with `list_notes(store, NoteListFilter { container_id: ctx.container_id })`
- [ ] `cmd_note_create`: remove container validation block and `add_member` call; pass `CreateNoteInput { note, container_id: ctx.container_id }` to `note_service::create`
- [ ] `cmd_note_update`: remove ID mismatch check (move validation into service's `update_note`)
- [ ] `cmd_note_delete`: remove membership check and `remove_member` call; call `delete_note(store, DeleteNoteInput { id, container_id: ctx.container_id })`

**`crates/srs-cli/src/commands/record.rs`:**
- [ ] Remove `parse_field_values_payload()` function entirely
- [ ] Remove `resolve_type()` function entirely
- [ ] Remove `parse_type_filter()` function entirely
- [ ] `cmd_record_list`: replace member fetch + retain with `list_records(store, RecordListFilter { ... })`
- [ ] `cmd_record_create`: deserialize to `CreateRecordInput`; single service call
- [ ] `cmd_record_delete`: remove membership check and `remove_member` call; call `delete_record(store, id, ctx.container_id)`

**`crates/srs-cli/src/commands/tag.rs`:**
- [ ] `cmd_tag_list`: replace member fetch + retain with `list_tag_definitions(store, TagListFilter { container_id: ctx.container_id })`
- [ ] `cmd_tag_create`: remove container validation and `add_member` call; pass `CreateTagInput` to service
- [ ] `cmd_tag_update`: remove ID mismatch check (validation moves to service)
- [ ] `cmd_tag_delete`: remove membership check and `remove_member` call; call `delete_tag_definition(store, id, ctx.container_id)`

**`crates/srs-cli/src/commands/relation.rs`:**
- [ ] `cmd_relation_list`: replace member fetch + filter with `list_relations(store, RelationListFilter { container_id: ctx.container_id })`
- [ ] `cmd_relation_create`: remove `store.load_package()` + `defs` construction; call `create_relation(store, relation)` (service loads package internally)

**`crates/srs-cli/src/commands/field.rs`:**
- [ ] Remove `normalize_field_input()` function entirely
- [ ] `cmd_field_list`: replace 4-branch match with `list_fields_filtered(store, FieldListFilter { namespace, package })`
- [ ] `cmd_field_create`: pass raw `serde_json::Value` as `CreateFieldInput`; remove normalization call

**`crates/srs-cli/src/commands/record_type.rs`:**
- [ ] `cmd_type_list`: replace 4-branch match with `list_types_filtered(store, TypeListFilter { namespace, package })`

**`crates/srs-cli/src/commands/relation_type.rs`:**
- [ ] `cmd_relation_type_list`: replace inline filter closure with `list_relation_types_filtered(store, status_filter)`

**`crates/srs-cli/src/commands/extension.rs`:**
- [ ] `cmd_extension_list`: replace `Record`-to-JSON transformation with `list_extensions(store)` returning `Vec<ExtensionSummary>`
- [ ] `cmd_extension_create`: remove field extraction and type inference; call `create_extension(store, CreateExtensionInput { raw })`
- [ ] `cmd_extension_update`: remove field extraction; call `update_extension(store, id, CreateExtensionInput { raw })`

**`crates/srs-cli/src/commands/protocol.rs`:**
- [ ] `cmd_protocol_import`: replace 153-line body with `import_protocol(store, ProtocolImportInput { raw })`
- [ ] `cmd_protocol_get`: remove `obj.insert("instanceId", ...)` mutation; call `get_protocol(store, id)` returning `ProtocolResult` with `instance_id` already set

#### Acceptance Criteria

- [ ] `cargo build -p srs-cli` succeeds
- [ ] `cargo test` passes across all crates
- [ ] No `with_store` called more than once in any single handler function
- [ ] No `list_members`, `is_member`, `add_member`, `remove_member` calls in any handler function
- [ ] No `.retain()` or `.filter()` applied to a list result in any handler function
- [ ] No `serde_json::Value` field access (`.get()`, `.as_object_mut()`, `.as_str()`) in any handler function except the initial `from_reader` deserialization
- [ ] `srs repo validate --repo ../../srs/srs` shows 0 errors

#### Testing

```bash
cargo test
cargo build -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo run --bin srs -- note list --repo ../../srs/srs
cargo run --bin srs -- repo validate --repo ../../srs/srs
```

Specific tests to write or verify:

- `cmd_note_create_handler_calls_single_service` — integration test verifying note create works end-to-end
- `cmd_record_list_handler_has_no_container_filtering_logic` — structural: verify `list_members` is not called in `record.rs`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run the full test suite and CLI smoke tests.
3. Run lint:

```bash
cargo test
cargo clippy -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

### Phase 4: Schema Validation at Service Boundaries

**Goal:** Every service create/update function validates the input against the registered JSON schema before deserializing to a Rust type.

**Agent:** Service Layer Worker

The validation pattern:

```rust
use srs_schema::{SchemaRegistry, NOTE_SCHEMA_ID};

pub fn create(store: &dyn RepositoryStore, input: CreateNoteInput) -> Result<NoteResult, RepositoryError> {
    // Validate raw JSON against schema before serde deserialization
    // (catches type violations that serde would silently coerce)
    SchemaRegistry::global()
        .validate_by_id(NOTE_SCHEMA_ID, &serde_json::to_value(&input.note).unwrap())
        .map_err(|e| RepositoryError::SchemaValidation {
            path: "<stdin>".into(),
            message: e.to_string(),
        })?;
    // existing semantic validation continues unchanged
    validate_note(&input.note)?;
    // ...
}
```

#### Tasks

- [ ] `services.rs`: Add schema validation in `create_note` and `update_note` against `NOTE_SCHEMA_ID`
- [ ] `package_service.rs`: Add schema validation in `create_field_in_package` against `FIELD_SCHEMA_ID`
- [ ] `package_service.rs`: Add schema validation in `create_type_in_package` against `TYPE_SCHEMA_ID`
- [ ] `relation_service.rs`: Add schema validation in `create_relation` against `RELATIONS_COLLECTION_SCHEMA_ID` (validate the single relation object, not the full collection)
- [ ] `protocol_service.rs`: Add schema validation in `import_protocol` against `PROTOCOL_SCHEMA_ID` (after the field-mapping step, validate the assembled Record against the schema)
- [ ] `container_service.rs`: Add schema validation in container create/update against `CONTAINER_SCHEMA_ID`

#### Acceptance Criteria

- [ ] Passing malformed JSON (wrong field type, missing required field) to `srs note create` returns a diagnostic error, not a panic or silent coercion
- [ ] Passing malformed JSON to `srs field create` returns a diagnostic error
- [ ] All existing create/update integration tests still pass
- [ ] `cargo test` passes

#### Testing

```bash
cargo test
# Manual smoke test:
echo '{"title": 123}' | cargo run --bin srs -- note create --repo /tmp/test-repo
# Should return: {"ok": false, "diagnostics": ["schema validation failed: ..."]}
```

Specific tests to write or verify:

- `create_note_rejects_wrong_title_type` — passes `{"title": 123}` and expects a `SchemaValidation` error
- `create_field_rejects_missing_required_fields` — passes `{}` and expects error

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run full test suite:

```bash
cargo test
cargo clippy -- -D warnings
```

3. Update plan checkboxes.
4. Commit.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `srs repo validate --repo ../../srs/srs` shows 0 errors, same record/relation counts as before
- [ ] `srs note list`, `srs record list`, `srs relation list` with and without `--container` produce correct results
- [ ] `srs note create`, `srs record create`, `srs field create`, `srs protocol import` produce identical JSON output to pre-plan behavior
- [ ] `cargo build -p srs-cli` contains zero imports of `list_members`, `add_member`, `remove_member`, `is_member` — enforced by `pub(crate)` visibility (compile-time proof)
- [ ] No CLI handler function performs `.retain()` or `.filter()` on a list result (verified by search: `grep -r "\.retain\|\.filter" crates/srs-cli/src/commands/` returns nothing)
- [ ] No public service function in `srs-repository` has a `serde_json::Value` parameter (verified by search)
- [ ] `scripts/check-schema-sync.sh` exits 0
- [ ] ADR-010 status updated to `accepted`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phase 3 and before final sign-off.

## Assumptions

- The CLI JSON output format (envelope shape, field names) is frozen — no payload keys may change.
- `ctx.container_id` is the mechanism by which container scoping is passed from CLI flags to service input structs; this is not changing.
- Extensions and protocols are stored as generic Tier 2 Records (per ADR-005, ADR-006); their service functions work with `serde_json::Value` internally but the public surface uses typed input/output structs.
- `with_store()` is the correct mechanism for store access in the CLI and is not changing.

## Phase 0 E0603 error locations (violations to fix in Phase 3)

These are the exact CLI import sites that fail to compile after `pub(crate)` was applied:

| File | Line | Functions imported |
|---|---|---|
| `crates/srs-cli/src/commands/note.rs` | 8 | `add_member`, `list_members`, `remove_member`, `is_member` |
| `crates/srs-cli/src/commands/record.rs` | 7 | `add_member`, `list_members`, `remove_member`, `is_member` |
| `crates/srs-cli/src/commands/tag.rs` | 7 | `add_member`, `list_members`, `remove_member`, `is_member` |
| `crates/srs-cli/src/commands/relation.rs` | 6 | `list_members` |
| `crates/srs-cli/src/commands/container.rs` | 9–10 | `add_member`, `remove_member` |
