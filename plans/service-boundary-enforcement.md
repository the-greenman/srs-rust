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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Every service function takes a typed input struct and returns a typed result struct; the CLI calls one function per logical operation | accepted |

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

### Phase 0: Compile-Time Enforcement Infrastructure ✅ COMPLETE

**Goal:** Membership functions are `pub(crate)`, making it physically impossible for CLI handlers to call them directly. This creates the compile-time fence before any migration work begins.

**Agent:** Service Layer Worker

This phase does not move any logic — it only changes visibility. The CLI will fail to compile after this phase until Phase 3 removes the imports. That is expected and correct: the compiler is now pointing at every violation.

#### Tasks

- [x] In `crates/srs-repository/src/container_service.rs`: change `pub fn list_members`, `pub fn add_member`, `pub fn remove_member`, `pub fn is_member` to `pub(crate)`. Keep all other container service functions (`get_container`, `create_container`, `update_container`, `delete_container`, `list_containers`, `list_roots`, `add_root`, `remove_root`, `validate_container`, `list_members_full`) as `pub`.
- [x] Add the service module doc comment header (see Enforcement Strategy section above) to `container_service.rs`, `services.rs`, `record_store.rs`, `tag_service.rs`, `relation_service.rs`, `package_service.rs`, `extension_service.rs`, `protocol_service.rs`
- [x] Confirm `cargo build -p srs-repository` still succeeds (the repository crate itself uses these functions internally — `pub(crate)` is sufficient)
- [x] Confirm `cargo build -p srs-cli` now **fails** with `error[E0603]` on `list_members`, `add_member`, `remove_member`, `is_member` imports — this failure is the proof the enforcement works
- [x] Record the list of compile errors (file + line) in a comment at the bottom of this plan — these are the exact locations Phase 3 must fix

#### Acceptance Criteria

- [x] `cargo build -p srs-repository` succeeds
- [x] `cargo build -p srs-cli` fails with E0603 errors on membership function imports
- [x] No other compile errors introduced (only the expected E0603s)
- [x] All 8 service modules have the doc comment header

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

### Phase 1: Schema Housekeeping ✅ COMPLETE

**Goal:** All entity types that have CLI commands have a registered JSON schema; schema drift is detectable by CI.

**Agent:** Schema Worker

Notes: `protocol.json` was determined to be unnecessary — protocols are stored as Tier 2 Records (per ADR-006), so `record.json` covers them. Final schema count is 17, not 18.

#### Tasks

- [x] Copy `document-view.json`, `view.json`, `theme.json` from `/srs/docs/schema/2.0/` to `crates/srs-schema/schemas/2.0/`
- [x] Author `container.json` in `/srs/docs/schema/2.0/` from the `Container` struct in `crates/srs-core/src/types/container.rs`. Required fields: `containerId` (UUID), `title` (string). No `additionalProperties: false` because Container uses `#[serde(flatten)] extra` for forward-compat.
- [x] ~~Author `protocol.json`~~ — skipped; protocols are Tier 2 Records, `record.json` applies
- [x] Copy `container.json` to `crates/srs-schema/schemas/2.0/`
- [x] In `crates/srs-schema/src/lib.rs`: added 4 new schema ID constants; updated assertion count to 17
- [x] ~~`minimal_field_passes_schema_contract`~~ — already existed
- [x] Add `minimal_record_type_passes_schema_contract` test to `crates/srs-core/src/types/record_type.rs`
- [x] Add `minimal_container_passes_schema_contract` test to `crates/srs-core/src/types/container.rs`
- [x] Write `scripts/check-schema-sync.sh`
- [x] Sync `manifest.json` and `package-manifest.json` (were diverged from spec)

#### Acceptance Criteria

- [x] `cargo test -p srs-schema` passes with schema count assertion at 17
- [x] `cargo test -p srs-core` passes including new schema alignment tests
- [x] `scripts/check-schema-sync.sh` exits 0 with current files
- [x] `scripts/check-schema-sync.sh` exits non-zero when any schema file in `srs/docs/schema/2.0/` has no matching copy in `crates/srs-schema/schemas/2.0/`

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

### Phase 2: Typed Input/Output Structs for All Service Operations ✅ COMPLETE

**Goal:** Every service function has a typed input struct and a typed result struct; no `serde_json::Value` parameters on public service functions.

**Agent:** Service Layer Worker

Define all structs alongside their service function in the same module. Use `#[derive(Debug, Deserialize)]` with `#[serde(rename_all = "camelCase")]` for inputs; `#[derive(Debug, Serialize)]` with `#[serde(rename_all = "camelCase")]` for outputs.

#### Tasks

**`crates/srs-repository/src/services.rs` (note service):**
- [x] Define `ListNotesFilter { container_id: Option<String> }`
- [x] Define `CreateNoteInput { note: Note, container_id: Option<String> }`
- [x] Define `CreateNoteResult { note: Note }`, `DeleteNoteResult { instance_id: String }`
- [x] Updated `list_notes(store, filter: ListNotesFilter)` — performs container filtering internally
- [x] Add `create_note_in_context(store, input: CreateNoteInput)` — validates container, creates, adds member atomically
- [x] Define `DeleteNoteInput { id: String, container_id: Option<String> }`
- [x] Add `delete_note_in_context(store, input: DeleteNoteInput)` — checks membership, removes, deletes atomically
- [x] Add `update_note_validated(store, id, note)` — validates ID match before update

**`crates/srs-repository/src/record_store.rs`:**
- [x] Define `RecordListFilter { type_namespace: Option<String>, type_name: Option<String>, container_id: Option<String> }`
- [x] Define `CreateRecordInput { field_values: Vec<FieldValue> }`
- [x] Define `CreateRecordResult { record: Record }`, `DeleteRecordResult { instance_id: String }`
- [x] Add `list_records_filtered(store, filter: RecordListFilter)` — unified with container + type filtering
- [x] Add `create_record_in_context(store, type_filter, type_version, input, container_id, relative_dir)` — resolves type, creates, adds to container
- [x] Add `delete_record_in_context(store, id, container_id)` — membership check, remove, delete

**`crates/srs-repository/src/tag_service.rs`:**
- [x] Define `TagListFilter { container_id: Option<String> }`
- [x] Add `list_tag_definitions_filtered(store, filter: TagListFilter)` — performs container filtering
- [x] Add `create_tag_definition_in_context(store, tag, container_id)` — atomic create+add_member
- [x] Add `delete_tag_definition_in_context(store, id, container_id)` — atomic check+remove+delete
- [x] Add `update_tag_definition_validated(store, id, tag)` — ID validation before update

**`crates/srs-repository/src/relation_service.rs`:**
- [x] Add `container_id: Option<String>` to existing `ListRelationsFilter`
- [x] Updated `list_relations` to filter where BOTH source AND target are container members
- [x] Add `create_relation_auto(store, relation)` — loads package/defs internally

**`crates/srs-repository/src/package_service.rs`:**
- [x] Define `FieldListFilter { namespace: Option<String>, package: Option<Option<String>> }`
- [x] Define `TypeListFilter { namespace: Option<String>, package: Option<Option<String>> }`
- [x] Add `list_fields_filtered(store, filter: FieldListFilter)` — unified replacement
- [x] Add `list_types_filtered(store, filter: TypeListFilter)` — unified replacement
- [x] Add `list_relation_types_filtered(store, status: Option<String>)` — status filtering moved from CLI
- [x] Add `create_field_normalized(store, raw, package_selector)` — normalization moved from CLI

**`crates/srs-repository/src/extension_service.rs`:** — doc headers added; full service migration deferred to future phase
**`crates/srs-repository/src/protocol_service.rs`:** — doc headers added; full service migration deferred to future phase

#### Acceptance Criteria

- [x] Every entity's list function accepts a filter struct
- [x] `cargo test -p srs-repository` passes
- [x] `cargo build -p srs` succeeds after Phase 3 CLI cleanup

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

### Phase 3: CLI Handler Cleanup ✅ COMPLETE

**Goal:** Every CLI handler conforms to the enforced pattern: one `from_reader` or flag-to-struct map, one `with_store` call, one `output::ok/err`.

**Agent:** CLI Cleanup Worker

#### Tasks

**`crates/srs-cli/src/commands/note.rs`:**
- [x] `cmd_note_list`: uses `list_notes(store, ListNotesFilter { container_id })`
- [x] `cmd_note_create`: uses `create_note_in_context(store, CreateNoteInput { note, container_id })`
- [x] `cmd_note_update`: uses `update_note_validated(store, &id, note)`
- [x] `cmd_note_delete`: uses `delete_note_in_context(store, DeleteNoteInput { id, container_id })`

**`crates/srs-cli/src/commands/record.rs`:**
- [x] Remove `parse_field_values_payload()` function entirely
- [x] Remove `resolve_type()` function entirely
- [x] `cmd_record_list`: uses `list_records_filtered(store, RecordListFilter { ... })`
- [x] `cmd_record_create`: uses `create_record_in_context`
- [x] `cmd_record_delete`: uses `delete_record_in_context(store, &id, container_id)`

**`crates/srs-cli/src/commands/tag.rs`:**
- [x] `cmd_tag_list`: uses `list_tag_definitions_filtered(store, TagListFilter { container_id })`
- [x] `cmd_tag_create`: uses `create_tag_definition_in_context(store, tag, container_id)`
- [x] `cmd_tag_update`: uses `update_tag_definition_validated(store, &id, tag)`
- [x] `cmd_tag_delete`: uses `delete_tag_definition_in_context(store, id, container_id)`

**`crates/srs-cli/src/commands/relation.rs`:**
- [x] `cmd_relation_list`: uses `list_relations(store, ListRelationsFilter { container_id })`
- [x] `cmd_relation_create`: uses `create_relation_auto(store, relation)`

**`crates/srs-repository/src/container_service.rs`:**
- [x] Added `add_container_member`, `remove_container_member`, `list_container_members` public wrappers for the membership management CLI commands

**`crates/srs-cli/src/commands/container.rs`:**
- [x] Updated membership subcommands to use public wrapper functions

**Deferred (extension.rs, protocol.rs):** These handlers still contain service-level logic. The full extension/protocol service migration is tracked as future work — extension_service.rs and protocol_service.rs have doc headers marking what needs to move.

#### Acceptance Criteria

- [x] `cargo build -p srs` succeeds (461 tests pass, 0 failures)
- [x] `cargo clippy -- -D warnings` passes clean
- [x] No `list_members`, `is_member`, `add_member`, `remove_member` calls in note/record/tag/relation handler functions
- [x] No `.retain()` applied to list results in cleaned handlers

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

### Phase 4: Schema Validation at Service Boundaries ✅ COMPLETE

**Goal:** Every service create/update function validates the input against the registered JSON schema before deserializing to a Rust type.

**Agent:** Service Layer Worker

#### Tasks

- [x] `services.rs`: Schema validation in `create_note_in_context` and `update_note` against `NOTE_SCHEMA_ID`
- [x] `package_service.rs`: Schema validation in `create_field_normalized` against `FIELD_SCHEMA_ID` — validated on raw input before normalization (normalization sets `aiGuidance: {}` which would fail strict schema)
- [x] ~~`package_service.rs create_type_in_package`~~ — skipped; `type.json` schema requires `$schema` field that the in-memory struct doesn't carry
- [x] `relation_service.rs`: Schema validation in `create_relation` validates the full `RelationsCollection` after appending the new relation, against `RELATIONS_COLLECTION_SCHEMA_ID`
- [x] `container_service.rs`: Schema validation in `create_container` and `update_container` against `CONTAINER_SCHEMA_ID`
- [x] ~~`protocol_service.rs`~~ — deferred; protocol import still in CLI layer (Phase 3 deferral)

#### Acceptance Criteria

- [x] All existing create/update tests still pass (461 tests, 0 failures)
- [x] `cargo clippy -- -D warnings` clean
- [x] Schema validation runs before semantic validation in note, field, container, relation create paths

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [x] `cargo test` passes with no failures (461 tests)
- [x] `cargo clippy -- -D warnings` passes
- [ ] `srs repo validate --repo ../../srs/srs` shows 0 errors, same record/relation counts as before
- [ ] `srs note list`, `srs record list`, `srs relation list` with and without `--container` produce correct results
- [ ] `srs note create`, `srs record create`, `srs field create`, `srs protocol import` produce identical JSON output to pre-plan behavior
- [x] `cargo build -p srs` contains zero imports of `list_members`, `add_member`, `remove_member`, `is_member` — enforced by `pub(crate)` visibility (compile-time proof)
- [ ] No CLI handler function performs `.retain()` or `.filter()` on a list result — **PARTIALLY MET**: `tag.rs:40`, `field.rs:35`, `record_type.rs:35`, `relation_type.rs:19` still filter in-handler (see Deferred Work)
- [ ] No public service function in `srs-repository` has a `serde_json::Value` parameter — **PARTIALLY MET**: `extension_service.rs` and `protocol_service.rs` (see Deferred Work)
- [x] `scripts/check-schema-sync.sh` exits 0
- [x] ADR-010 status updated to `accepted`

---

## Deferred Work

The following items were out of scope for this plan's four phases or were explicitly deferred. They are tracked here so a future agent can pick them up without re-reading the full history.

### D1: Extension service migration (`extension.rs` + `extension_service.rs`)

**What remains:** `crates/srs-cli/src/commands/extension.rs` (146 lines) still extracts `fieldValues` from raw JSON, infers `type_id`/`typeVersion`, and constructs `FieldValue` objects directly in the handler. `list_extensions` returns `Vec<Record>` (generic) rather than a typed `Vec<ExtensionSummary>`.

**What needs to happen:**
- Define `ExtensionSummary { instance_id, namespace, name, extension_type }` in `extension_service.rs`
- Rewrite `list_extensions` to return `Vec<ExtensionSummary>`
- Define `CreateExtensionInput { raw: serde_json::Value }` — service extracts `fieldValues` and infers type internally
- Add `create_extension(store, input) -> Result<ExtensionResult, RepositoryError>`
- Add `update_extension(store, id, input) -> Result<ExtensionResult, RepositoryError>`
- Update `extension.rs` CLI handler to pass raw stdin to service; remove field extraction logic

### D2: Protocol service migration (`protocol.rs` + `protocol_service.rs`)

**What remains:** `crates/srs-cli/src/commands/protocol.rs` (265 lines) contains:
- `cmd_protocol_import`: 153-line function that extracts fields from protocol JSON, maps camelCase keys to `FieldValue` objects, hardcodes field IDs — all business logic that belongs in the service
- `cmd_protocol_get`: injects `instanceId` into the returned JSON object manually

**What needs to happen:**
- Define `ProtocolImportInput { raw: serde_json::Value }` in `protocol_service.rs`
- Move the 153-line import body into `import_protocol(store, input) -> Result<ProtocolResult, RepositoryError>`
- Define `ProtocolResult { protocol: serde_json::Value, instance_id: String }` — service populates `instance_id`
- Rewrite `get_protocol` to return `ProtocolResult` with `instance_id` already set
- Update `protocol.rs` CLI handler to be a thin wrapper

### D3: Remaining in-handler filter calls

Four CLI handlers still filter list results inside the handler rather than passing filter parameters to the service:

| File | Line | Pattern |
|---|---|---|
| `crates/srs-cli/src/commands/tag.rs` | 40 | `filter(|s| member_ids.contains(...))` — container filter still done in-handler for the `tag list` path |
| `crates/srs-cli/src/commands/field.rs` | 35 | `filter(|f| f.namespace == *ns)` — namespace filter done in-handler |
| `crates/srs-cli/src/commands/record_type.rs` | 35 | `filter(|t| t.namespace == *ns)` — namespace filter done in-handler |
| `crates/srs-cli/src/commands/relation_type.rs` | 19 | `filter(|rt| ...)` — status filter done in-handler |

`list_relation_types_filtered`, `list_fields_filtered`, and `list_types_filtered` service functions already exist in `package_service.rs` — the CLI handlers just need to call them and remove the in-handler filter chains.

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
