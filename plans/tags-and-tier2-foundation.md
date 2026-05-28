# Plan: Tags and Tier 2 Foundation

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.
>
> Save this file to `plans/<slug>.md` before assigning agents. Agents receive the plan file as their primary brief.

## Summary

Notes are freely taggable with raw strings — that remains the baseline and no migration is required.

**`TagDefinition` is a core SRS type, not a user-defined package type.** It is a peer to `Field` and `RecordType` in the spec: defined natively in `srs-core`, with dedicated service functions in `srs-repository`, and a stable CLI surface. Tags do not require a `TagDefinition` to be used on a Note; definitions are additive enrichment that give a tag meaning, roles, and aliases.

The generic Tier 2 Record infrastructure (Phases 1–3) is still correct and valuable for user-defined types. But `TagDefinition` is pulled out of that path and given first-class treatment — the same way `Note` is first-class for Tier 0.

The practical consequence: `FOUNDATION_SIGNAL_TAGS` (a hardcoded CLI constant) is replaced by `TagDefinition` records with `roles: ["foundation"]` — but the lookup is done through the core tag service, not the generic `list_records_by_type`.

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
| [ADR-003](../docs/adr/003-tagdefinition-is-core.md) | TagDefinition is a core SRS type, not a pluggable package type | accepted |

---

## Scope

- ~~Add `Field`, `RecordType`, `Record`, `FieldValue` types to `srs-core`~~ ✓ done
- ~~Add `validate_record` to `srs-core`~~ ✓ done
- ~~Add `load_package` + `Package` to `srs-repository`~~ ✓ done
- ~~Add `list_records_by_type`, `get_record_by_id`, `create_record` to `srs-repository`~~ ✓ done
- Add `TagDefinition` as a native type to `srs-core` (peer to `Note`)
- Add `TagDefinitionService` to `srs-repository` (dedicated service, not via generic record store)
- Add `tag-definition` type definition to the `srs/` spec package (for schema/validation purposes)
- Add `srs tag list/get/create` CLI commands backed by the core service
- Replace `FOUNDATION_SIGNAL_TAGS` constant with data-driven lookup via the tag service
- Write ADR-003

**Out of scope:**

- Tier 1 (TypedRecord) — no concrete use case yet
- Async storage boundary — deferred to Phase 3 of the storage refactor plan
- `srs-bindings` Python surface — deferred to storage refactor Phase 5
- Tag definition records in `srs/records/` — creating actual TagDefinition instance records is a follow-on task after the type exists
- Migration of existing raw tags — not needed; raw string tags on Notes remain valid indefinitely; TagDefinitions are additive

---

## Phases

### Phase 1: Core Tier 2 Types

**Status:** `complete`

`Field`, `RecordType`, `Record`, `FieldValue`, `validate_record` all exist in `srs-core`.

---

### Phase 2: Package Loader

**Status:** `complete`

`load_package`, `Package`, `resolve_type`, `resolve_field`, `resolve_type_by_name` all exist in `srs-repository`.

---

### Phase 3: Generic Record Operations

**Status:** `complete`

`list_records_by_type`, `get_record_by_id`, `create_record` all exist in `srs-repository/src/record_store.rs`.

---

### Phase 4: Write ADR-003

**Status:** `complete`

**Goal:** The architectural decision that `TagDefinition` is core is recorded before implementation begins.

**Agent:** Lead Integrator

**Write scope:** `docs/adr/`

#### Tasks

- [ ] Create `docs/adr/003-tagdefinition-is-core.md`

**ADR content to capture:**
- Context: tags are first-class on Notes (Tier 0), used universally; `TagDefinition` gives meaning to tags rather than being a repo-specific type
- Decision: `TagDefinition` is a native `srs-core` type with dedicated service functions — a peer to `Note`, not a user-defined Tier 2 Record
- Consequences: `TagDefinition` gets typed Rust structs and dedicated `list_tag_definitions` / `get_tag_definition` / `create_tag_definition` service functions; the generic Record infrastructure remains for user-defined types; the tag CLI commands delegate to the tag service, not `record_store`

#### Milestone gate

```bash
git commit
```

---

### Phase 5: `TagDefinition` as a Native `srs-core` Type

**Status:** `complete`

**Goal:** `srs-core` has a typed `TagDefinition` struct alongside `Note`, with its own validation.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-core/src/types/tag_definition.rs` | Create |
| `crates/srs-core/src/types/mod.rs` | Edit — add `pub mod tag_definition;` |
| `crates/srs-core/src/validation/tag_definition.rs` | Create |
| `crates/srs-core/src/validation/mod.rs` | Edit — add `pub mod tag_definition;` |
| `crates/srs-core/src/error.rs` | Edit — add `EmptyTagKey` variant |

#### `TagDefinition` type shape

```rust
// crates/srs-core/src/types/tag_definition.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDefinition {
    #[serde(default)]
    pub instance_id: String,
    /// The raw tag string this definition describes. Must be non-empty.
    /// Matches the string used in Note.tags / NoteSection.tags.
    pub tag_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Semantic roles this tag plays. Well-known values: "foundation", "navigation", "lifecycle".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,   // "draft" | "active" | "deprecated" | "obsolete"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl TagDefinition {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.as_ref()
            .map(|rs| rs.iter().any(|r| r == role))
            .unwrap_or(false)
    }
}
```

**New `error.rs` variant:**
```rust
#[error("tag key must be non-empty")]
EmptyTagKey,
```

**`validation/tag_definition.rs`:**
```rust
pub fn validate_tag_definition(td: &TagDefinition) -> Result<(), CoreError>
```
Checks:
- `tag_key` is non-empty → `Err(CoreError::EmptyTagKey)`
- All strings in `roles` and `aliases` are non-empty (reuse `EmptyTag` error or new `EmptyRole` variant — use `EmptyTag` for consistency)

#### Tasks

- [ ] Create `crates/srs-core/src/types/tag_definition.rs` with `TagDefinition` and `has_role`
- [ ] Add `pub mod tag_definition;` to `crates/srs-core/src/types/mod.rs`
- [ ] Add `EmptyTagKey` variant to `crates/srs-core/src/error.rs`
- [ ] Create `crates/srs-core/src/validation/tag_definition.rs` with `validate_tag_definition`
- [ ] Add `pub mod tag_definition;` to `crates/srs-core/src/validation/mod.rs`

#### Tests (inline `#[cfg(test)]`)

- `tag_definition_roundtrips_json` — construct `TagDefinition` with all fields, serialize → deserialize, assert equal
- `tag_definition_minimal_roundtrips` — only `instance_id` + `tag_key` present; all optional fields absent → `Ok`
- `tag_definition_extra_fields_survive` — unknown JSON key survives round-trip via `extra`
- `has_role_returns_true_when_present` — `roles: ["foundation", "navigation"]`, `has_role("foundation")` → `true`
- `has_role_returns_false_when_absent` — `has_role("lifecycle")` on above → `false`
- `has_role_returns_false_when_no_roles` — `roles: None`, `has_role("anything")` → `false`
- `validate_tag_definition_passes_minimal` — `tag_key: "foundation"`, no optional fields → `Ok(())`
- `validate_tag_definition_empty_key_fails` — `tag_key: ""` → `Err(CoreError::EmptyTagKey)`
- `validate_tag_definition_empty_role_fails` — `roles: [""]` → `Err(CoreError::EmptyTag)`

#### Milestone gate

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
git commit
```

---

### Phase 6: Tag Definition Service in `srs-repository`

**Status:** `complete`

**Goal:** `srs-repository` has dedicated, typed service functions for `TagDefinition` — not routed through the generic record store.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-repository/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-repository/src/tag_service.rs` | Create |
| `crates/srs-repository/src/lib.rs` | Edit — add `pub mod tag_service;` |
| `crates/srs-repository/src/loader.rs` | Edit — add `load_tag_definition`, `load_tag_definition_relative` |
| `crates/srs-repository/src/writer.rs` | Edit — add `write_tag_definition`, `upsert_tag_definition_index_entry` |
| `crates/srs-repository/src/error.rs` | Edit — add `TagDefinitionLoad`, `TagDefinitionWrite`, `TagDefinitionValidation` variants |

#### `tag_service.rs` public API

```rust
use srs_core::types::tag_definition::TagDefinition;

/// Summary for list operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDefinitionSummary {
    pub instance_id: String,
    pub tag_key: String,
    pub label: Option<String>,
    pub roles: Option<Vec<String>>,
    pub status: Option<String>,
}

pub enum GetTagDefinitionResult {
    Found(Box<TagDefinition>),
    NotFound,
}

pub struct CreateTagDefinitionResult {
    pub tag_definition: TagDefinition,
    pub path: String,
}

pub fn list_tag_definitions(
    repo_root: &Path,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError>

pub fn list_tag_definitions_by_role(
    repo_root: &Path,
    role: &str,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError>

pub fn get_tag_definition_by_id(
    repo_root: &Path,
    id: &str,
) -> Result<GetTagDefinitionResult, RepositoryError>

pub fn get_foundation_signal_tags(
    repo_root: &Path,
) -> Result<Vec<String>, RepositoryError>
// Returns tag_key strings for all TagDefinitions with role "foundation".
// Returns Ok(vec![]) if no definitions exist — not an error.

pub fn create_tag_definition(
    repo_root: &Path,
    tag_definition: TagDefinition,
) -> Result<CreateTagDefinitionResult, RepositoryError>
```

**`get_foundation_signal_tags` is the replacement for `FOUNDATION_SIGNAL_TAGS`.** It moves into the library where it belongs — the CLI no longer owns tag policy.

**Tier in manifest:** `TagDefinition` instances use `tier: 3` in the manifest index. This distinguishes them from Tier 0 Notes (`tier: 0`) and generic Tier 2 Records (`tier: 2`). Add `is_tag_definition()` method to `InstanceIndexEntry`.

**Storage path:** `records/tag-definitions/<slug>.json` where slug is derived from `tag_key` using the same `slugify_title` function.

**New error variants:**
```rust
#[error("failed to load tag definition at {path}: {source}")]
TagDefinitionLoad { path: PathBuf, source: serde_json::Error },
#[error("tag definition validation failed at {path}: {source}")]
TagDefinitionValidation { path: PathBuf, source: srs_core::error::CoreError },
#[error("failed to write tag definition at {path}: {source}")]
TagDefinitionWrite { path: PathBuf, source: std::io::Error },
```

#### Tasks

- [ ] Add `TagDefinitionLoad`, `TagDefinitionValidation`, `TagDefinitionWrite` to `error.rs`
- [ ] Add `load_tag_definition(path)` and `load_tag_definition_relative(repo_root, path)` to `loader.rs`
- [ ] Add `write_tag_definition(td, path)` and `upsert_tag_definition_index_entry(manifest, td, path)` to `writer.rs`
- [ ] Add `is_tag_definition()` method to `InstanceIndexEntry` in `index.rs` — returns `self.tier == 3`
- [ ] Create `tag_service.rs` with all five public functions
- [ ] Add `pub mod tag_service;` to `lib.rs`

#### Tests (inline `#[cfg(test)]`)

- `list_tag_definitions_empty_repo` — temp repo with no tag definitions → `Ok(vec![])`
- `create_tag_definition_writes_file_and_updates_manifest` — create a `TagDefinition`, assert file at `records/tag-definitions/<slug>.json`, manifest updated with `tier: 3`
- `get_tag_definition_by_id_finds_created` — create then get by returned `instance_id` → `GetTagDefinitionResult::Found`
- `get_tag_definition_by_id_not_found` — unknown id → `GetTagDefinitionResult::NotFound`
- `list_tag_definitions_by_role_filters_correctly` — create two definitions, one with `roles: ["foundation"]`, one without; `list_by_role("foundation")` → 1 result
- `get_foundation_signal_tags_returns_tag_keys` — create definition with `roles: ["foundation"]`, `tag_key: "purpose"`; `get_foundation_signal_tags` → `vec!["purpose"]`
- `get_foundation_signal_tags_empty_when_none_defined` — empty repo → `Ok(vec![])`

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
git commit
```

---

### Phase 7: `srs/` Spec Package — Tag Definition Type

**Status:** `complete`

> Note: `node scripts/validate-all.mjs` reports a pre-existing failure due to missing `/home/greenman/dev/semanticops/schemas/` directory — unrelated to this work. The tag field and type files are present in `srs/package/`.


**Goal:** `com.semanticops.srs/tag-definition@1` exists as a type definition in the `srs/` spec package. This serves as the schema definition and validation reference — it is not what the Rust library uses to load TagDefinitions (the Rust struct is authoritative for that).

**Agent:** Lead Integrator

**Write scope:** `srs/` spec repository (not `srs-rust/`)

#### Fields to create (in `srs/package/fields/`)

Each file follows the pattern of `srs/package/fields/status.json`. Assign new UUID4s.

| File | `name` | `valueType` | Notes |
|---|---|---|---|
| `tag-key.json` | `tag-key` | `string` | The raw tag string (e.g. `"foundation"`) |
| `tag-label.json` | `tag-label` | `string` | Human-readable display name |
| `tag-description.json` | `tag-description` | `text` | What this tag means |
| `tag-roles.json` | `tag-roles` | `multiselect` | `allowedValues: ["foundation", "navigation", "lifecycle"]` |
| `tag-aliases.json` | `tag-aliases` | `text` | Comma-separated alternate forms |

**Reuse existing field:** `status` field UUID `e6f7a8b9-c0d1-4e2f-3a4b-5c6d7e8f9a0b` — include as optional.

#### Type to create: `srs/package/types/tag-definition.json`

Follow the pattern of `srs/package/types/meta.extension.json`. Fields in order:
- `tag-key` (required)
- `tag-label`, `tag-description`, `tag-roles`, `tag-aliases`, `status` (all optional)

#### `srs/package/package.json` edits

Add the 5 new field paths to `fields[]` and `types/tag-definition.json` to `types[]`.

#### Tasks

- [ ] Create `srs/package/fields/tag-key.json` (new UUID)
- [ ] Create `srs/package/fields/tag-label.json` (new UUID)
- [ ] Create `srs/package/fields/tag-description.json` (new UUID)
- [ ] Create `srs/package/fields/tag-roles.json` with `allowedValues` (new UUID)
- [ ] Create `srs/package/fields/tag-aliases.json` (new UUID)
- [ ] Create `srs/package/types/tag-definition.json`
- [ ] Update `srs/package/package.json`
- [ ] Run `node scripts/validate-all.mjs` and fix any issues

#### Milestone gate

```bash
# From srs/
node scripts/validate-all.mjs
git commit
```

---

### Phase 8: CLI — Wire Tag Service, Remove `FOUNDATION_SIGNAL_TAGS`

**Status:** `partial` — service wiring and `FOUNDATION_SIGNAL_TAGS` removal are complete; integration tests for `srs tag` commands not yet written

**Goal:** `srs tag list/get/create` delegate to the core tag service. `FOUNDATION_SIGNAL_TAGS` is gone. `cmd_note_foundations` calls `get_foundation_signal_tags`.

**Agent:** CLI Worker

**Write scope:** `crates/srs-cli/src/`

#### What currently exists (do not recreate)

- `crates/srs-cli/src/commands/tag.rs` — exists, routes through `record_store`; has placeholder UUIDs; needs to be rewritten to use `tag_service`
- `crates/srs-cli/src/commands/mod.rs` — `TagCommand` enum and dispatch already wired

#### Files to modify

| File | Action |
|---|---|
| `crates/srs-cli/src/commands/tag.rs` | Rewrite — delegate to `tag_service` instead of `record_store` |
| `crates/srs-cli/src/commands/note.rs` | Edit — update `cmd_note_foundations` |

#### `tag.rs` rewrite

Remove all references to `record_store`, `TAG_DEF_TYPE_ID`, `TAG_KEY_FIELD_ID`, `TAG_ROLES_FIELD_ID`. Replace with calls to `srs_repository::tag_service`:

```rust
use srs_repository::tag_service::{
    list_tag_definitions, list_tag_definitions_by_role,
    get_tag_definition_by_id, create_tag_definition,
};
use srs_core::types::tag_definition::TagDefinition;
```

**`cmd_tag_list`**: calls `list_tag_definitions(repo)` or `list_tag_definitions_by_role(repo, role)` if `--role` provided. Return: `output::ok("tag list", json!({ "tagDefinitions": summaries }))`.

**`cmd_tag_get`**: calls `get_tag_definition_by_id(repo, id)`. Returns `{ "tagDefinition": td }` or `ok: false`.

**`cmd_tag_create`**: reads JSON from stdin, deserialises to `TagDefinition`, calls `create_tag_definition(repo, td)`. Returns `{ "tagDefinition": created }`.

Keep `record_has_role` and `get_tag_key` helpers — delete them (logic now lives in `TagDefinition::has_role` and `td.tag_key`). Keep `collect_foundation_signal_tags` — delete it (now `tag_service::get_foundation_signal_tags`).

#### `cmd_note_foundations` update (in `note.rs`)

Replace the `FOUNDATION_SIGNAL_TAGS`-based lookup with:

```rust
use srs_repository::tag_service::get_foundation_signal_tags;

let signal_tags = get_foundation_signal_tags(&repo_root).unwrap_or_default();
let signal_tag_refs: Vec<&str> = signal_tags.iter().map(|s| s.as_str()).collect();
collect_foundation_notes(&repo_root, &signal_tag_refs)
```

#### Tasks

- [x] Rewrite `crates/srs-cli/src/commands/tag.rs` to use `tag_service` instead of `record_store`
- [x] Remove `record_has_role`, `get_tag_key`, `collect_foundation_signal_tags`, and all field UUID constants from `tag.rs`
- [x] Update `cmd_note_foundations` in `commands/note.rs` to call `get_foundation_signal_tags`
- [x] Confirm `FOUNDATION_SIGNAL_TAGS` does not appear anywhere in the codebase
- [x] Confirm `TAG_DEF_TYPE_ID` / `TAG_KEY_FIELD_ID` / `TAG_ROLES_FIELD_ID` constants removed

#### Integration tests (in `crates/srs-cli/tests/integration_tests.rs`)

- [ ] `tag_list_returns_ok_envelope` — `srs tag list` against live srs repo → `ok: true`, `payload.tagDefinitions` is array (may be empty)
- [ ] `tag_create_and_retrieve_in_temp_repo` — temp repo, create a TagDefinition with `{"tagKey": "test", "roles": ["foundation"]}` via stdin, retrieve by returned id; assert `tag_key == "test"`. Use existing `create_temp_repo()` fixture and `run_srs_stdin_in_dir` helper.

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
- [ ] `srs note foundations` compiles and runs without any hardcoded tag list (may return empty list until TagDefinition records are created in the repo)
- [ ] `FOUNDATION_SIGNAL_TAGS` does not appear anywhere in the codebase
- [ ] `TAG_DEF_TYPE_ID` / `TAG_KEY_FIELD_ID` / `TAG_ROLES_FIELD_ID` constants do not appear in `tag.rs`
- [ ] `TagDefinition` has a typed Rust struct in `srs-core` — it is not loaded via the generic `Record` path
- [ ] `get_foundation_signal_tags` is a library function in `srs-repository`, not a CLI concern
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

- Phases 1–3 are complete — generic Tier 2 infrastructure is in place.
- Phase 8 is a rewrite of existing `tag.rs`, not a fresh file — preserve the `TagCommand` enum and dispatch wiring in `mod.rs`.
- Raw string tags on Notes are valid at all times — no `TagDefinition` is required for a tag to be used.
- `get_foundation_signal_tags` returning `Ok(vec![])` when no definitions exist is correct — not an error.
- `tier: 3` is assigned to `TagDefinition` instances to distinguish them from Notes (`tier: 0`) and generic Records (`tier: 2`). If this conflicts with the spec, the Lead Integrator resolves before Phase 6 starts.
- The `tag-definition` type in the `srs/` package (Phase 7) is the schema reference for the spec. The Rust `TagDefinition` struct (Phase 5) is authoritative for loading — they must stay aligned but are maintained separately.
