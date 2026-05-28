# Plan: Tags and Tier 2 Foundation

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.
>
> Save this file to `plans/<slug>.md` before assigning agents. Agents receive the plan file as their primary brief.

## Summary

Notes are freely taggable with raw strings — that remains the baseline and no migration is required. This plan adds an optional layer: `TagDefinition` Tier 2 Records that attach meaning to a tag (description, roles, aliases). A tag does not need a `TagDefinition` to be valid on a Note; definitions are purely additive enrichment.

The primary goal is the generic Tier 2 Record infrastructure (`Field`, `RecordType`, `Record` types + `list_records_by_type` / `create_record` service functions). `TagDefinition` is the first concrete use of that infrastructure. It also enables data-driven foundation note selection: notes with tags that have a `TagDefinition` with `roles: ["foundation"]` are selected — replacing the hardcoded `FOUNDATION_SIGNAL_TAGS` CLI constant. Notes with raw tags not backed by any definition continue to work normally.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library is the primary deliverable; CLI is one consumer | accepted |
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | Tier 2 record operations are generic; no type-specific library code | accepted |

---

## Scope

- Add `Field`, `RecordType`, `Record`, `FieldValue` types to `srs-core`
- Add `validate_record` to `srs-core`
- Add `load_package` + `Package` to `srs-repository`
- Add `list_records_by_type`, `get_record_by_id`, `create_record` to `srs-repository`
- Add `tag-definition` type definition (and supporting fields) to the `srs/` spec package
- Add `srs tag list/get/create` CLI commands
- Replace `FOUNDATION_SIGNAL_TAGS` constant with data-driven lookup from TagDefinition records

**Out of scope:**

- Tier 1 (TypedRecord) — no concrete use case yet
- Async storage boundary — deferred to Phase 3 of the storage refactor plan
- `srs-bindings` Python surface — deferred to storage refactor Phase 5
- Tag definition records in `srs/records/` — creating actual TagDefinition instance records is a follow-on task after the type exists
- Migration of existing raw tags — not needed; raw string tags on Notes remain valid indefinitely; TagDefinitions are additive

---

## Phases

### Phase 1: Core Tier 2 Types

**Goal:** `srs-core` has `Field`, `RecordType`, `Record`, `FieldValue` structs and `validate_record`.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-core/src/types/field.rs` | Create |
| `crates/srs-core/src/types/record_type.rs` | Create |
| `crates/srs-core/src/types/record.rs` | Create |
| `crates/srs-core/src/types/mod.rs` | Edit — add `pub mod field; pub mod record_type; pub mod record;` |
| `crates/srs-core/src/validation/record.rs` | Create |
| `crates/srs-core/src/validation/mod.rs` | Edit — add `pub mod record;` |
| `crates/srs-core/src/error.rs` | Edit — add `MissingRequiredField`, `UnknownField` variants |

#### Type shapes

All structs use `#[serde(rename_all = "camelCase")]` and `#[serde(skip_serializing_if = "Option::is_none")]` on optional fields. Use `#[serde(flatten)] pub extra: HashMap<String, serde_json::Value>` for forward-compatibility passthrough. `ValueType` is an enum, not a struct — the shapes below annotate each definition type.

**`types/field.rs`**
```rust
// struct
pub struct Field {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub value_type: ValueType,
    pub description: Option<String>,
    pub ai_guidance: Option<serde_json::Value>,
    pub allowed_values: Option<Vec<String>>,
    pub default_value: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// enum — serialises to lowercase strings: "string", "text", "multiselect", etc.
#[serde(rename_all = "lowercase")]
pub enum ValueType { String, Text, Number, Boolean, Date, Url, Select, Multiselect }
```

**`multiselect` serialisation:** A `FieldValue` whose field has `valueType: "multiselect"` stores its value as `serde_json::Value::Array` of `serde_json::Value::String` items. Example: `{ "fieldId": "...", "value": ["foundation", "navigation"] }`. The `FieldValue.value` field is always `serde_json::Value` — no special enum variant is needed. Validation does not check array contents against `allowedValues` in this phase (deferred).

**`types/record_type.rs`**
```rust
pub struct RecordType {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub fields: Vec<FieldAssignment>,
    pub description: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub struct FieldAssignment {
    pub field_id: String,
    pub order: u32,
    pub required: Option<bool>,      // None means true
    pub display_label: Option<String>,
}
```

**`types/record.rs`**
```rust
pub struct Record {
    pub instance_id: String,
    pub type_id: String,
    pub type_version: u32,
    pub type_namespace: Option<String>,
    pub type_name: Option<String>,
    pub field_values: Vec<FieldValue>,
    pub tags: Option<Vec<String>>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub struct FieldValue {
    pub field_id: String,
    pub value: serde_json::Value,
}
```

**`validation/record.rs`**
```rust
pub fn validate_record(record: &Record, record_type: &RecordType) -> Result<(), CoreError>
```
Checks:
- All FieldAssignments where `required` is `true` or `None` have a matching FieldValue
- All FieldValues reference a `field_id` that exists in the RecordType's `fields`

**New `error.rs` variants:**
```rust
#[error("missing required field: {field_id}")]
MissingRequiredField { field_id: String },
#[error("unknown field in record: {field_id}")]
UnknownField { field_id: String },
```

#### Tasks

- [ ] Create `crates/srs-core/src/types/field.rs` with `Field` and `ValueType`
- [ ] Create `crates/srs-core/src/types/record_type.rs` with `RecordType` and `FieldAssignment`
- [ ] Create `crates/srs-core/src/types/record.rs` with `Record` and `FieldValue`
- [ ] Update `crates/srs-core/src/types/mod.rs`
- [ ] Add `MissingRequiredField` and `UnknownField` to `crates/srs-core/src/error.rs`
- [ ] Create `crates/srs-core/src/validation/record.rs` with `validate_record`
- [ ] Update `crates/srs-core/src/validation/mod.rs`

#### Tests (inline `#[cfg(test)]`)

- `field_roundtrips_json` — Field with `valueType: "select"` and `allowedValues` serializes and deserializes correctly; unknown extra fields survive the round-trip via `extra`
- `field_extra_fields_survive_roundtrip` — Field JSON with an unknown key (e.g. `"editorHint"`) deserializes into `extra` and re-serializes with the key present
- `record_type_roundtrips_json` — RecordType with two FieldAssignments round-trips; `required: null`/absent treated as required
- `record_roundtrips_json` — Record with two FieldValues round-trips; `$schema` key in JSON survives via `extra`
- `record_null_optional_field_roundtrips` — Record with `"updatedAt": null` in JSON round-trips; null optional field is not an error
- `validate_record_passes_with_all_required_fields` — all required fields present → `Ok(())`
- `validate_record_missing_required_field` — required field absent → `Err(MissingRequiredField { field_id })`
- `validate_record_optional_field_absent_is_ok` — `required: false` field absent → `Ok(())`
- `validate_record_unknown_field` — FieldValue with unrecognised `field_id` → `Err(UnknownField { field_id })`
- `multiselect_field_value_is_array` — FieldValue with `value: ["a", "b"]` round-trips as `serde_json::Value::Array`

#### Milestone gate

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
git commit
```

---

### Phase 2: Package Loader

**Goal:** `srs-repository` can load a `Package` (fields + types) from a repo's `package/` directory and resolve types and fields by ID.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-repository/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-repository/src/package.rs` | Create |
| `crates/srs-repository/src/lib.rs` | Edit — add `pub mod package;` |
| `crates/srs-repository/src/error.rs` | Edit — add `PackageLoad`, `TypeNotFound`, `FieldNotFound` variants |

#### `package.rs` public API

```rust
pub struct Package {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub fields: Vec<Field>,
    pub record_types: Vec<RecordType>,
    /// The repo root (not the package/ subdirectory). All paths in package.json
    /// are relative to this root, so callers join against repo_root, not package/.
    pub root: PathBuf,
}

impl Package {
    /// Find a type by (typeId, typeVersion). Returns None if not present.
    pub fn resolve_type(&self, type_id: &str, version: u32) -> Option<&RecordType>
    /// Find a field by fieldId UUID. Returns None if not present.
    pub fn resolve_field(&self, field_id: &str) -> Option<&Field>
    /// Find a type by (namespace, name) — useful for listing without knowing the UUID.
    pub fn resolve_type_by_name(&self, namespace: &str, name: &str) -> Option<&RecordType>
    /// All fields (for completeness, symmetric with resolve_field).
    pub fn fields(&self) -> &[Field] { &self.fields }
}

pub fn load_package(repo_root: &Path) -> Result<Package, RepositoryError>
```

**`load_package` implementation:** reads `<repo_root>/package/package.json`, uses the `fields[]` and `types[]` path arrays (relative to repo root) to load each individual field and type JSON file, populates `Package`.

**New error variants:**
```rust
#[error("failed to load package at {path}: {source}")]
PackageLoad { path: PathBuf, source: serde_json::Error },
#[error("type not found: {type_id}@{version}")]
TypeNotFound { type_id: String, version: u32 },
#[error("field not found: {field_id}")]
FieldNotFound { field_id: String },
```

#### Tasks

- [ ] Add `PackageLoad`, `TypeNotFound`, `FieldNotFound` to `crates/srs-repository/src/error.rs`
- [ ] Create `crates/srs-repository/src/package.rs` with `Package`, `load_package`, `resolve_type`, `resolve_field`
- [ ] Add `pub mod package;` to `crates/srs-repository/src/lib.rs`

#### Tests (inline `#[cfg(test)]`)

- `load_package_from_live_repo` — load from `/home/greenman/dev/semanticops/srs`, assert `fields.len() > 20`, `record_types.len() > 5`
- `resolve_type_by_name_finds_meta_extension` — load live package, call `resolve_type_by_name("com.semanticops.srs", "meta.extension")`, assert `Some(t)` where `t.version == 1`
- `resolve_field_by_name_finds_status` — find field where `name == "status"` using `fields()`, assert it exists and `value_type == ValueType::Select`
- `resolve_type_returns_none_for_unknown` — `resolve_type("nonexistent-uuid", 1)` → `None`

> Note: Tests use name-based lookups against the live repo rather than hardcoded UUIDs, which could change if type definitions are regenerated. If you need to test UUID-based `resolve_type` specifically, load the type by name first, then use its `.id` as the input.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
git commit
```

---

### Phase 3: Generic Record Operations

**Goal:** `srs-repository` can list, get, and create any Tier 2 Record using the package type definitions.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-repository/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-repository/src/record_store.rs` | Create |
| `crates/srs-repository/src/lib.rs` | Edit — add `pub mod record_store;` |
| `crates/srs-repository/src/error.rs` | Edit — add `RecordLoad`, `RecordWrite`, `RecordValidation` variants |

#### `record_store.rs` public API

```rust
pub fn list_records_by_type(
    repo_root: &Path,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError>

pub fn get_record_by_id(
    repo_root: &Path,
    id: &str,
) -> Result<Option<Record>, RepositoryError>

pub fn create_record(
    repo_root: &Path,
    type_id: &str,
    type_version: u32,
    field_values: Vec<FieldValue>,
    relative_dir: &str,          // e.g. "records/tag-definitions"
) -> Result<Record, RepositoryError>
```

**Implementation notes:**
- `list_records_by_type`: load manifest, filter entries where `tier == 2` (package-bound Records only; Tier 1 TypedRecords are out of scope for this plan), load each as `Record`, filter by `type_namespace` + `type_name` matching. Skip entries that fail to deserialise as `Record` (treat as diagnostic, continue).
- `get_record_by_id`: find entry in manifest index by `instance_id`, load file as `Record`. Returns `Ok(None)` if not in index; returns `Err(RecordLoad)` if file is missing or unparseable.
- `create_record`: load package → `resolve_type_by_id(type_id, type_version)` (return `Err(RepositoryError::TypeNotFound)` if absent) → build `Record` → `validate_record(&record, &resolved_type)` → mint `instanceId` → write JSON to `<repo_root>/<relative_dir>/<instanceId>.json` → upsert manifest entry with `tier: 2` → write manifest
- Tier in manifest entry: hardcoded `2` for all records created by `create_record`

**New error variants:**
```rust
#[error("failed to load record at {path}: {source}")]
RecordLoad { path: PathBuf, source: serde_json::Error },
#[error("failed to write record at {path}: {source}")]
RecordWrite { path: PathBuf, source: std::io::Error },
#[error("record validation failed at {path}: {source}")]
RecordValidation { path: PathBuf, source: srs_core::error::CoreError },
```

#### Tasks

- [ ] Add `RecordLoad`, `RecordWrite`, `RecordValidation` to `error.rs`
- [ ] Create `crates/srs-repository/src/record_store.rs` with `list_records_by_type`, `get_record_by_id`, `create_record`
- [ ] Add `pub mod record_store;` to `lib.rs`

#### Tests (inline `#[cfg(test)]`)

- `list_records_by_type_from_live_repo` — list `("com.semanticops.srs", "meta.extension")` from `/home/greenman/dev/semanticops/srs`, assert `count > 5`. To verify the anchor: first call `resolve_type_by_name` from the loaded package, confirm it exists, then list — this detects if the type was renamed.
- `get_record_by_id_returns_known_record` — call `list_records_by_type` first, take the `instance_id` of the first result, call `get_record_by_id` with that id, assert `Some(r)` where `r.instance_id` matches
- `get_record_by_id_returns_none_for_unknown` — `get_record_by_id(repo, "nonexistent-id-12345")` → `Ok(None)`
- `create_record_in_temp_repo` — temp repo fixture with minimal `package/package.json` + one RecordType (one required field, one optional field) + the field definitions; call `create_record` with the required field value only; assert: file written at `records/test-records/<instanceId>.json`, manifest updated with `tier: 2`, returned `Record.instance_id` is non-empty
- `create_record_missing_required_field_fails` — same temp fixture, call `create_record` with no field values → `Err(RecordValidation { .. })`

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
git commit
```

---

### Phase 4: TagDefinition Type in the SRS Package

**Goal:** `com.semanticops.srs/tag-definition@1` exists as a type definition in the `srs/` spec package. No Rust code is specific to tags.

**Agent:** Lead Integrator (spec data change, not Rust code)

**Write scope:** `srs/` spec repository (not `srs-rust/`)

#### Fields to create (in `srs/package/fields/`)

Each file follows the pattern of `srs/package/fields/status.json`. Assign new UUID4s.

| File | `name` | `valueType` | `required` in type | Notes |
|---|---|---|---|---|
| `tag-key.json` | `tag-key` | `string` | true | The raw tag string (e.g. `"foundation"`) |
| `tag-label.json` | `tag-label` | `string` | false | Human-readable display name |
| `tag-description.json` | `tag-description` | `text` | false | What this tag means |
| `tag-roles.json` | `tag-roles` | `multiselect` | false | `allowedValues: ["foundation", "navigation", "lifecycle"]` |
| `tag-aliases.json` | `tag-aliases` | `text` | false | Comma-separated alternate forms |

**Reuse existing field:** `status` field UUID `e6f7a8b9-c0d1-4e2f-3a4b-5c6d7e8f9a0b` — include as optional.

#### Type to create: `srs/package/types/tag-definition.json`

Follow the pattern of `srs/package/types/meta.extension.json`. Fields in order: `tag-key` (required), `tag-label`, `tag-description`, `tag-roles`, `tag-aliases`, `status` (all optional).

#### `srs/package/package.json` edits

Add the 5 new field paths to `fields[]` and `types/tag-definition.json` to `types[]`.

#### Tasks

- [ ] Create `srs/package/fields/tag-key.json`
- [ ] Create `srs/package/fields/tag-label.json`
- [ ] Create `srs/package/fields/tag-description.json`
- [ ] Create `srs/package/fields/tag-roles.json` (with `allowedValues`)
- [ ] Create `srs/package/fields/tag-aliases.json`
- [ ] Create `srs/package/types/tag-definition.json`
- [ ] Update `srs/package/package.json` to include new fields and type
- [ ] Run `node scripts/validate-all.mjs` (from `srs/`) and fix any issues

#### Milestone gate

```bash
# From srs/
node scripts/validate-all.mjs

# From srs-rust/ — package loader should now find tag-definition
cargo test -p srs-repository load_package_from_live_repo
git commit
```

---

### Phase 5: `srs tag` CLI Commands

**Goal:** `srs tag list/get/create` work. `FOUNDATION_SIGNAL_TAGS` is removed. `cmd_note_foundations` derives its tag list from TagDefinition records with `foundation` role.

**Agent:** CLI Worker

**Write scope:** `crates/srs-cli/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-cli/src/commands/tag.rs` | Create |
| `crates/srs-cli/src/commands/mod.rs` | Edit — add `Tag(TagCommand)`, remove `FOUNDATION_SIGNAL_TAGS`, add `pub mod tag;` |
| `crates/srs-cli/src/commands/note.rs` | Edit — update `cmd_note_foundations` |

#### `commands/tag.rs` command surface

```
srs tag list  [--repo PATH] [--role ROLE]
srs tag get   [--repo PATH] <ID>
srs tag create [--repo PATH]             # reads JSON from stdin
```

**`cmd_tag_list`**: calls `list_records_by_type(repo, "com.semanticops.srs", "tag-definition")`, if `--role` provided filter records where the `tag-roles` FieldValue contains the role string. Return envelope: `{ "tagDefinitions": [...] }`.

**`cmd_tag_get`**: calls `get_record_by_id(repo, id)`. Returns `{ "tagDefinition": <record> }` or `ok: false` if not found.

**`cmd_tag_create`**: reads JSON from stdin, deserialises to `Vec<FieldValue>`, calls `create_record(repo, TAG_DEF_TYPE_ID, 1, field_values, "records/tag-definitions")`. Returns `{ "tagDefinition": <created record> }`.

```rust
// In tag.rs — the only tag-specific constants in the codebase.
// TAG_DEF_TYPE_ID and TAG_KEY_FIELD_ID are filled in during Phase 4 once UUIDs are assigned.
const TAG_DEF_NAMESPACE: &str = "com.semanticops.srs";
const TAG_DEF_TYPE_NAME: &str = "tag-definition";
const TAG_DEF_TYPE_ID: &str = "<uuid assigned in Phase 4>";
const TAG_KEY_FIELD_ID: &str = "<uuid of tag-key field assigned in Phase 4>";
const TAG_ROLES_FIELD_ID: &str = "<uuid of tag-roles field assigned in Phase 4>";
```

**Helper functions** — define these as private free functions in `tag.rs` (not in `srs-repository`; they are CLI-local utilities):

```rust
/// Returns true if the record has the given role in its tag-roles FieldValue.
/// tag-roles is a multiselect stored as a JSON array of strings.
fn record_has_role(record: &Record, role: &str) -> bool {
    record.field_values.iter()
        .find(|fv| fv.field_id == TAG_ROLES_FIELD_ID)
        .and_then(|fv| fv.value.as_array())
        .map(|arr| arr.iter().any(|v| v.as_str() == Some(role)))
        .unwrap_or(false)
}

/// Returns the string value of the tag-key field, if present.
fn get_tag_key(record: &Record) -> Option<&str> {
    record.field_values.iter()
        .find(|fv| fv.field_id == TAG_KEY_FIELD_ID)
        .and_then(|fv| fv.value.as_str())
}
```

**`cmd_note_foundations` update** (in `note.rs`):
Replace `FOUNDATION_SIGNAL_TAGS` lookup with:
```rust
use crate::commands::tag::{TAG_DEF_NAMESPACE, TAG_DEF_TYPE_NAME, record_has_role, get_tag_key};

let tag_defs = list_records_by_type(&repo, TAG_DEF_NAMESPACE, TAG_DEF_TYPE_NAME)
    .unwrap_or_default();
let signal_tags: Vec<&str> = tag_defs.iter()
    .filter(|r| record_has_role(r, "foundation"))
    .filter_map(|r| get_tag_key(r))
    .collect();
collect_foundation_notes(&repo, &signal_tags)
```
If `tag_defs` is empty (no TagDefinition records yet), `signal_tags` will be empty and `foundations` returns an empty list — acceptable transitional state.

Make `record_has_role` and `get_tag_key` `pub(crate)` so `note.rs` can import them.

#### Remove `FOUNDATION_SIGNAL_TAGS`

Delete the `pub const FOUNDATION_SIGNAL_TAGS` declaration from `commands/mod.rs`. Ensure no other code references it.

#### Tasks

- [ ] Create `crates/srs-cli/src/commands/tag.rs` with `TagCommand`, `cmd_tag_list`, `cmd_tag_get`, `cmd_tag_create`, `record_has_role` (pub crate), `get_tag_key` (pub crate)
- [ ] Add `pub mod tag;` and `Tag(TagCommand)` to `commands/mod.rs`
- [ ] Remove `FOUNDATION_SIGNAL_TAGS` from `commands/mod.rs`; ensure no remaining references
- [ ] Update `cmd_note_foundations` in `commands/note.rs` to use `record_has_role` and `get_tag_key` from `tag.rs`
- [ ] Fill in `TAG_DEF_TYPE_ID`, `TAG_KEY_FIELD_ID`, `TAG_ROLES_FIELD_ID` constants with UUIDs assigned in Phase 4

#### Integration tests (in `crates/srs-cli/tests/integration_tests.rs`)

- `tag_list_returns_ok_envelope` — `srs tag list` against live srs repo → `ok: true`, `payload.tagDefinitions` is array (may be empty)
- `tag_create_and_retrieve_in_temp_repo` — temp repo with full package, create a TagDefinition with `tag-key: "test"`, retrieve by returned id

#### Milestone gate

```bash
cargo test -p srs-cli
cargo test --test integration_tests
cargo clippy -- -D warnings
git commit
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `srs tag list` returns `ok: true` against the live srs repo
- [ ] `srs note foundations` compiles and runs without `FOUNDATION_SIGNAL_TAGS` (may return empty list until TagDefinition records are created)
- [ ] `FOUNDATION_SIGNAL_TAGS` constant does not exist anywhere in the codebase
- [ ] No `TagDefinition`-specific functions exist in `srs-repository` (only in `srs-cli/commands/tag.rs`)
- [ ] `node scripts/validate-all.mjs` passes in `srs/`
- [ ] All existing integration tests still pass

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **Agents must run the milestone gate (lint + tests + commit) before marking a phase complete.** A phase is not done until its gate passes.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Phases 1–3 are pure Rust — no changes to the `srs/` spec repo.
- Phase 4 requires the spec repo to be valid after changes (`validate-all.mjs` must pass).
- Raw string tags on Notes are valid at all times — no TagDefinition is required for a tag to be used. TagDefinitions are optional enrichment.
- `cmd_note_foundations` returning an empty list when no TagDefinition records exist is acceptable as a transitional state.
- The `TAG_KEY_FIELD_ID`, `TAG_ROLES_FIELD_ID`, and `TAG_DEF_TYPE_ID` constants in the CLI will be filled in during Phase 4 once UUIDs are assigned.
