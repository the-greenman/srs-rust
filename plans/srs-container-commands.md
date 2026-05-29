# Plan: SRS Container Commands

## Summary

Add a `srs container` command group for full CRUD over multiple named containers per repository. Containers are first-class SRS 2.0 grouping entities — repos may have several (e.g. `Sprint 1`, `Backlog`) stored as separate JSON files in `containers/`. No Rust struct, service, or CLI surface exists for containers today. This plan fills that gap in the same entity-first pattern used for `note`, `tag`, and `relation`.

**Why containers matter for live facilitation:** `AttentionState.containerId` is mandatory (Invariant 34) — every document-space `Address` requires a `containerId` as its root context specifier. Containers are the scope boundary for `ext:addressability`, Protocol stage tracking, and Context Queries. A correct container CRUD implementation is a prerequisite for any live facilitation or TSS work: without resolvable containers, `AttentionState` cannot be validated and conversation chunks cannot be associated with the correct document scope.

**CLI container scoping:** This plan also adds a global `--container <container-id>` flag. When set, any create command (`note create`, `record create`, `tag create`, etc.) automatically adds the new instance's `instanceId` to that container's `memberInstanceIds` after creation. This is the pragmatic CLI substitute for a live `AttentionState` cursor — no persistent file, passed per-invocation. `AttentionState` as a spec concept (with `Revision`, `SourceReference`, and `Context Query`) is out of scope for this plan.

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

No new ADRs for behavior. Key spec references:

- **Invariant 20** (`invariant-020.json`): `Container.containerId` is not an instance ID and must not appear in `rootInstanceIds`, `memberInstanceIds`, `Relation.sourceInstanceId`, or `Relation.targetInstanceId`. This is enforced in `validate_container_invariants`.
- **Invariant 21** (spec §08-9): `rootInstanceIds` and `memberInstanceIds`, when present, must reference valid SRS instance IDs. Also enforced in `validate_container_invariants`.
- **Invariant 34** (`invariant-034.json`): `AttentionState.containerId` must reference a valid `Container.containerId`. This plan's CRUD is the foundation for that invariant — once containers are resolvable, `AttentionState` can be validated.
- **Spec §04-6**: Container struct has `namespace` and `name` optional fields (for addressing) in addition to `title`. Both are included in the `Container` struct in this plan.
- **Addressability integration**: document-space `Address` uses `containerId` as its mandatory root context. Correct container resolution is a prerequisite for `ext:addressability` conformance.

One transitional storage note: the manifest JSON schema (`crates/srs-schema/schemas/2.0/manifest.json`) has `additionalProperties: false` and does not declare `containerIndex`. Storing `containerIndex` in `manifest.extra` creates schema drift — the live manifest will not validate against the published schema. Schema validation is currently deferred in the codebase (not enforced at runtime), so this is safe to ship as-is. A future plan must add `containerIndex` to the manifest schema before schema validation is turned on.

This plan implements the entity-first command pattern established in [srs-cli-command-structure.md](srs-cli-command-structure.md) and the manifest-extra storage pattern from `manifest_service.rs`.

---

## Scope

- `Container` struct and `ContainerIndexEntry` struct in `srs-core`
- Pure `validate_container` function in `srs-core` (invariants only)
- `container_service` module in `srs-repository` with full CRUD plus membership management
- `srs container` CLI command group: `list`, `create`, `get`, `update`, `delete`, `members`, `roots`, `validate`
- Storage: `containers/{slug}-{id_prefix}.json` files tracked via `containerIndex` in `manifest.extra`; IDs must be UUIDs (format validated at create time)
- Global `--container <container-id>` flag: when set, all create commands auto-add the new instance to that container's `memberInstanceIds`

**Out of scope:**

- Relation-graph traversal in validation (checking containerId does not appear in relations)
- Any changes to the legacy manifest-embedded `container` field (left untouched)
- Migration from single-embedded container to containerIndex

---

## Storage Design

Container files live in `containers/`, one file per container:
```
containers/{slug}-{containerId_prefix}.json
```

`containerId` must be a UUID (enforced by `validate_container`). The filename prefix uses the first 8 hex chars of the UUID: `&container_id[..container_id.len().min(8)]` (safe slice — UUIDs are always ≥ 8 chars after minting, and `new_instance_id()` always returns a full UUID). Do not use `[..8]` directly; use `.len().min(8)` to be panic-safe for any caller-supplied ID.

`manifest.extra["containerIndex"]` tracks them:
```json
"containerIndex": [
  { "containerId": "550e8400-e29b-41d4-a716-446655440000", "title": "Sprint 1", "path": "containers/sprint-1-550e8400.json" }
]
```

Slug derived from `title`: lowercase, spaces→hyphens, strip non-alphanumeric. Same `slugify` pattern as `tag_service.rs`.

---

## Command Surface

```
srs container list
srs container create                            # reads JSON from stdin
srs container get <container-id>
srs container update <container-id>             # partial patch from stdin
srs container delete <container-id>
srs container members list <container-id>
srs container members add <container-id> <instance-id>
srs container members remove <container-id> <instance-id>
srs container roots list <container-id>
srs container roots add <container-id> <instance-id>
srs container roots remove <container-id> <instance-id>
srs container validate <container-id>
```

`update` stdin: partial patch — only present fields applied; `rootInstanceIds` / `memberInstanceIds` are not patchable via `update`.
`validate` scope: invariants only — `containerId` + `title` non-empty; all member/root IDs exist in `manifest.instance_index`.

---

## Phases

### Phase A: Core Type

**Goal:** `Container` and `ContainerIndexEntry` structs exist in `srs-core` with serialization and pure validation.

**Agent:** Core Model Worker

#### Tasks

- [ ] Create `crates/srs-core/src/types/container.rs` — `Container` struct (mirrors `TagDefinition` pattern) and `ContainerIndexEntry` struct
- [ ] Modify `crates/srs-core/src/types/mod.rs` — add `pub mod container;`
- [ ] Create `crates/srs-core/src/validation/container.rs` — `validate_container` function
- [ ] Modify `crates/srs-core/src/validation/mod.rs` — add `pub mod container;`

#### Struct definitions

`Container` in `crates/srs-core/src/types/container.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub container_id: String,
    pub title: String,
    // namespace + name: optional addressability metadata (spec §04-6)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_instance_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_instance_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerIndexEntry {
    pub container_id: String,
    pub title: String,
    pub path: String,
}
```

`validate_container` in `crates/srs-core/src/validation/container.rs`:
```rust
use crate::error::CoreError;
use crate::types::container::Container;

pub fn validate_container(container: &Container) -> Result<(), CoreError> {
    if container.container_id.is_empty() {
        return Err(CoreError::MissingRequiredField { field_id: "containerId".to_string() });
    }
    if container.title.is_empty() {
        return Err(CoreError::MissingRequiredField { field_id: "title".to_string() });
    }
    Ok(())
}
```

Note: `CoreError::MissingRequiredField` takes `field_id: String` — use `.to_string()` on string literals.

#### Acceptance Criteria

- [ ] `Container` serializes to camelCase JSON; optional fields absent when None
- [ ] `Container` deserializes from camelCase JSON; unknown fields land in `extra`
- [ ] `ContainerIndexEntry` round-trips correctly
- [ ] `validate_container` returns `Ok` for minimal valid container
- [ ] `validate_container` returns `MissingRequiredField` for empty `containerId` or `title`

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Tests to write (all in `crates/srs-core/src/types/container.rs`):
- `container_roundtrips_all_fields` — full round-trip with all optional fields set
- `container_minimal_roundtrips` — required fields only; optional fields absent from JSON
- `container_extra_fields_survive` — unknown JSON fields preserved in `extra`

Tests to write (in `crates/srs-core/src/validation/container.rs`):
- `validate_container_passes_minimal` — minimal valid container
- `validate_container_empty_container_id_fails` — empty `container_id`
- `validate_container_empty_title_fails` — empty `title`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 6 tests exist and pass.
3. Run:

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

### Phase B: Repository Service

**Goal:** `container_service` provides full CRUD plus membership management for containers in `srs-repository`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Modify `crates/srs-repository/src/error.rs` — add `ContainerNotFound` and `ContainerValidation` variants + PartialEq arms
- [ ] Create `crates/srs-repository/src/container_service.rs` — all service functions and helper types
- [ ] Modify `crates/srs-repository/src/lib.rs` — add `pub mod container_service;`

#### Error variants to add to `error.rs`

```rust
#[error("container not found: {container_id}")]
ContainerNotFound { container_id: String },

#[error("container validation failed: {source}")]
ContainerValidation { source: srs_core::error::CoreError },
```

Add to `impl PartialEq for RepositoryError`:
```rust
(RepositoryError::ContainerNotFound { container_id: a }, RepositoryError::ContainerNotFound { container_id: b }) => a == b,
(RepositoryError::ContainerValidation { source: sa }, RepositoryError::ContainerValidation { source: sb }) => sa == sb,
```

#### Types in `container_service.rs`

```rust
pub struct ContainerSummary { pub container_id: String, pub title: String, pub path: String }
pub struct ContainerPatch {
    pub title: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub container_type: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta: Option<serde_json::Value>,
}
pub struct ContainerValidationReport { pub ok: bool, pub errors: Vec<String> }
```

`ContainerPatch` derives `Deserialize`. `ContainerSummary` and `ContainerValidationReport` derive `Serialize`. All use `#[serde(rename_all = "camelCase")]`.

#### Private helpers

```rust
fn load_container_index(manifest: &Manifest) -> Vec<ContainerIndexEntry>
// manifest.extra.get("containerIndex") → serde_json::from_value → unwrap_or_default()

fn save_container_index(manifest: &mut Manifest, index: Vec<ContainerIndexEntry>) -> Result<(), RepositoryError>
// manifest.extra.insert("containerIndex", serde_json::to_value(&index)?) → Ok(())

fn load_container_file(path: &Path) -> Result<Container, RepositoryError>
// std::fs::read_to_string → serde_json::from_str → ManifestParse on error

fn write_container_file(container: &Container, path: &Path) -> Result<(), RepositoryError>
// serde_json::to_string_pretty → std::fs::write; create parent dir first

fn slugify_title(title: &str) -> String
// lowercase, spaces→hyphens, strip chars that are not alphanumeric or '-'
```

#### Service functions

```rust
pub fn list_containers(repo_root: &Path) -> Result<Vec<ContainerSummary>, RepositoryError>
// load_manifest → load_container_index → map to ContainerSummary

pub fn create_container(repo_root: &Path, mut container: Container) -> Result<Container, RepositoryError>
// Step 1: mint ID first (before validation) if container.container_id.is_empty()
//   container.container_id = new_instance_id()
// Step 2: validate_container → ContainerValidation on failure
//   (validation rejects empty containerId, so ID must exist before validate is called)
// Step 3: create_dir_all("containers/")
// Step 4: slug = slugify_title(&container.title)
//   id_prefix = &container.container_id[..container.container_id.len().min(8)]
//   filename = format!("{}-{}.json", slug, id_prefix)
// Step 5: file_path = repo_root.join("containers").join(&filename)
// Step 6: write_container_file(&container, &file_path)
// Step 7: load_manifest → load_container_index → push ContainerIndexEntry { container_id, title, path: relative_path }
// Step 8: save_container_index → write_manifest → return container

pub fn get_container(repo_root: &Path, container_id: &str) -> Result<Container, RepositoryError>
// load_manifest → load_container_index → find by container_id
// ContainerNotFound if absent
// load_container_file(repo_root.join(&entry.path))

pub fn update_container(repo_root: &Path, container_id: &str, patch: ContainerPatch) -> Result<Container, RepositoryError>
// Step 1: get_container → apply non-None patch fields
// Step 2: validate_container → ContainerValidation on failure
// Step 3: load_manifest → load_container_index → find entry by container_id
// Step 4: write_container_file to entry.path (unchanged — no file rename on title change)
// Step 5: if patch.title is Some, update the ContainerIndexEntry.title in the index
//   (stale title in containerIndex causes srs container list to show old title after update)
// Step 6: save_container_index → write_manifest → return updated container

pub fn delete_container(repo_root: &Path, container_id: &str) -> Result<String, RepositoryError>
// load_manifest → load_container_index → find entry → ContainerNotFound if absent
// std::fs::remove_file(repo_root.join(&entry.path))
// remove entry from index → save_container_index → write_manifest
// return container_id.to_string()

pub fn list_members(repo_root: &Path, container_id: &str) -> Result<Vec<String>, RepositoryError>
// get_container → container.member_instance_ids.unwrap_or_default()

pub fn add_member(repo_root: &Path, container_id: &str, instance_id: &str) -> Result<Vec<String>, RepositoryError>
// get_container → get or init member_instance_ids
// if already contains instance_id: return as-is (idempotent)
// push, sort, write_container_file, return updated list

pub fn remove_member(repo_root: &Path, container_id: &str, instance_id: &str) -> Result<Vec<String>, RepositoryError>
// get_container → filter out instance_id
// if list now empty: set member_instance_ids = None
// write_container_file → return updated list (empty vec if None)

pub fn list_roots(repo_root: &Path, container_id: &str) -> Result<Vec<String>, RepositoryError>
pub fn add_root(repo_root: &Path, container_id: &str, instance_id: &str) -> Result<Vec<String>, RepositoryError>
pub fn remove_root(repo_root: &Path, container_id: &str, instance_id: &str) -> Result<Vec<String>, RepositoryError>
// mirror member functions but for root_instance_ids

pub fn validate_container_invariants(repo_root: &Path, container_id: &str) -> Result<ContainerValidationReport, RepositoryError>
// get_container (returns ContainerNotFound if missing)
// run validate_container → collect error if fails
// load_manifest → build known_ids set from instance_index
// Invariant 20: check containerId does NOT appear in rootInstanceIds or memberInstanceIds
//   (containerId is not an instance ID and must not be used as one)
// Invariant 21: check each memberInstanceIds ID exists in known_ids; collect errors
// Invariant 21: check each rootInstanceIds ID exists in known_ids; collect errors
// return ContainerValidationReport { ok: errors.is_empty(), errors }
```

#### Unit tests in `container_service.rs`

Helper: `make_minimal_container_repo(temp: &TempDir) -> PathBuf` — writes `manifest.json` with `instanceIndex: []` and `containerIndex: []` plus creates `.srs/` marker. Pattern: copy from `tag_service.rs` tests.

Tests:
- `create_container_writes_file_and_index` — create returns container; file exists; containerIndex has entry
- `create_container_mints_id_if_empty` — container with empty container_id gets a UUID assigned
- `list_containers_returns_all` — create two; list returns both summaries
- `get_container_returns_container` — create then get by id
- `get_container_missing_returns_error` — get unknown id → ContainerNotFound
- `update_container_patches_title` — update title field; get returns new title
- `update_container_list_shows_updated_title` — update title; list returns new title in summary (tests containerIndex sync)
- `update_container_preserves_other_fields` — update title only; description unchanged
- `delete_container_removes_file_and_index` — create then delete; file gone; list returns empty
- `delete_container_missing_returns_error` — delete unknown id → ContainerNotFound
- `add_member_adds_id` — add member; list_members returns it
- `add_member_is_idempotent` — add same id twice; list_members returns it once
- `remove_member_removes_id` — add then remove; list_members returns empty
- `remove_member_noop_when_absent` — remove id not present; no error
- `remove_member_clears_field_when_list_empty` — remove last member; field serialized as absent
- `add_root_adds_id` — add root; list_roots returns it
- `remove_root_removes_id` — add then remove; list_roots returns empty
- `validate_invariants_passes_clean` — minimal valid container; report.ok == true
- `validate_invariants_fails_invalid_member_id` — member id not in instance_index; report.ok == false
- `validate_invariants_fails_invalid_root_id` — root id not in instance_index; report.ok == false
- `validate_invariants_fails_container_id_in_member_ids` — containerId appears in memberInstanceIds (Invariant 20); report.ok == false
- `validate_invariants_fails_container_id_in_root_ids` — containerId appears in rootInstanceIds (Invariant 20); report.ok == false

#### Acceptance Criteria

- [ ] `create_container` writes file and appends to containerIndex
- [ ] `get_container` returns `ContainerNotFound` for unknown id
- [ ] `update_container` patches only supplied fields
- [ ] `delete_container` removes file and containerIndex entry
- [ ] `add_member` / `add_root` are idempotent
- [ ] `remove_member` / `remove_root` set field to None when list empties
- [ ] `validate_container_invariants` catches invalid member/root IDs (Invariant 21)
- [ ] `validate_container_invariants` catches containerId appearing in memberInstanceIds or rootInstanceIds (Invariant 20)

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 22 tests exist and pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

### Phase C: CLI Commands

**Goal:** `srs container` command group is wired up with all subcommands and 12 integration tests pass.

**Agent:** CLI Worker

#### Tasks

- [ ] Create `crates/srs-cli/src/commands/container.rs` — dispatch and handlers
- [ ] Modify `crates/srs-cli/src/commands/mod.rs` — add module, enums, dispatch arm
- [ ] Modify `crates/srs-cli/tests/integration_tests.rs` — add 12 test-first integration tests (write tests first, then implement)

#### `mod.rs` additions

Add `pub mod container;` at top.

Add to `Commands` enum:
```rust
/// Container grouping and membership commands
#[command(subcommand)]
Container(ContainerCommand),
```

Add enums:
```rust
#[derive(Subcommand)]
pub enum ContainerCommand {
    /// List all containers
    List,
    /// Create a new container (reads JSON from stdin)
    Create,
    /// Get a container by ID
    Get { container_id: String },
    /// Update a container (reads partial JSON patch from stdin)
    Update { container_id: String },
    /// Delete a container by ID
    Delete { container_id: String },
    /// Member instance management
    #[command(subcommand)]
    Members(ContainerMembersCommand),
    /// Root instance management
    #[command(subcommand)]
    Roots(ContainerRootsCommand),
    /// Validate container invariants
    Validate { container_id: String },
}

#[derive(Subcommand)]
pub enum ContainerMembersCommand {
    List { container_id: String },
    Add { container_id: String, instance_id: String },
    Remove { container_id: String, instance_id: String },
}

#[derive(Subcommand)]
pub enum ContainerRootsCommand {
    List { container_id: String },
    Add { container_id: String, instance_id: String },
    Remove { container_id: String, instance_id: String },
}
```

Add to `dispatch` match:
```rust
Commands::Container(cmd) => container::dispatch(ctx, cmd),
```

#### `container.rs` handler structure (mirrors `repo.rs`)

```rust
use crate::commands::{CliContext, ContainerCommand, ContainerMembersCommand, ContainerRootsCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::container_service::{...};

pub fn dispatch(ctx: CliContext, cmd: ContainerCommand) -> Result<String> {
    match cmd {
        ContainerCommand::List => cmd_list(ctx),
        ContainerCommand::Create => cmd_create(ctx),
        ContainerCommand::Get { container_id } => cmd_get(ctx, container_id),
        ContainerCommand::Update { container_id } => cmd_update(ctx, container_id),
        ContainerCommand::Delete { container_id } => cmd_delete(ctx, container_id),
        ContainerCommand::Members(sub) => dispatch_members(ctx, sub),
        ContainerCommand::Roots(sub) => dispatch_roots(ctx, sub),
        ContainerCommand::Validate { container_id } => cmd_validate(ctx, container_id),
    }
}
```

#### Handler payload shapes

| Handler | `output::ok` command string | Payload |
|---------|----------------------------|---------|
| `cmd_list` | `"container list"` | `{ "containers": [...] }` |
| `cmd_create` | `"container create"` | `{ "container": <Container> }` |
| `cmd_get` | `"container get"` | `{ "container": <Container> }` |
| `cmd_update` | `"container update"` | `{ "container": <Container> }` |
| `cmd_delete` | `"container delete"` | `{ "containerId": "..." }` |
| `members list` | `"container members list"` | `{ "containerId": "...", "memberInstanceIds": [...] }` |
| `members add` | `"container members add"` | `{ "containerId": "...", "instanceId": "...", "memberInstanceIds": [...] }` |
| `members remove` | `"container members remove"` | `{ "containerId": "...", "instanceId": "...", "memberInstanceIds": [...] }` |
| `roots list` | `"container roots list"` | `{ "containerId": "...", "rootInstanceIds": [...] }` |
| `roots add` | `"container roots add"` | `{ "containerId": "...", "instanceId": "...", "rootInstanceIds": [...] }` |
| `roots remove` | `"container roots remove"` | `{ "containerId": "...", "instanceId": "...", "rootInstanceIds": [...] }` |
| `cmd_validate` (pass) | `"container validate"` | `{ "ok": true, "errors": [] }` |
| `cmd_validate` (fail) | `output::err("container validate", report.errors)` | — |

Stdin for `create`/`update`: `serde_json::from_reader(std::io::stdin())`. On parse failure: `output::err("container create/update", vec![format!("Failed to parse JSON: {e}")])`.

#### Integration tests in `integration_tests.rs`

Helper: `make_container_test_repo() -> TempDir` — minimal manifest with `instanceIndex: []` and `containerIndex: []`. Pattern: copy from existing test helpers.

Tests (write test-first before implementing handlers):

All test fixtures must use full UUIDs for `containerId`, `memberInstanceIds`, and `rootInstanceIds` — the schema requires `format: uuid` and `validate_container` enforces non-empty. Use `"00000000-0000-4000-8000-000000000001"` etc. as fixture IDs.

- `container_list_returns_empty_initially` — fresh repo; list returns `{ "containers": [] }`
- `container_create_returns_container` — create with `{"containerId":"00000000-0000-4000-8000-000000000001","title":"Test"}` via stdin; response has `container.containerId == "00000000-0000-4000-8000-000000000001"`
- `container_get_returns_created_container` — create then get; ids match
- `container_update_patches_title` — create, update title, get; title updated
- `container_update_list_reflects_new_title` — create, update title, list; summary shows new title
- `container_delete_removes_container` — create, delete, list; list returns empty
- `container_members_list_returns_ids` — create, add member; list_members returns it
- `container_members_add_adds_id` — add member; response memberInstanceIds contains the id
- `container_members_remove_removes_id` — add then remove; memberInstanceIds empty
- `container_roots_list_returns_ids` — create, add root; list_roots returns it
- `container_roots_add_adds_id` — add root; response rootInstanceIds contains the id
- `container_roots_remove_removes_id` — add then remove; rootInstanceIds empty
- `container_validate_passes_clean` — create; validate; `ok == true`

#### Acceptance Criteria

- [ ] `srs container list` returns `{ "containers": [] }` on fresh repo
- [ ] `srs container create` reads stdin JSON; returns container with assigned id
- [ ] `srs container get <id>` returns the container
- [ ] `srs container update <id>` patches title without touching description
- [ ] `srs container delete <id>` removes the container; subsequent list is empty
- [ ] `srs container members add` / `remove` / `list` operate correctly
- [ ] `srs container roots add` / `remove` / `list` operate correctly
- [ ] `srs container validate <id>` returns `ok: true` for clean container

#### Testing

```bash
cargo test -p srs --test integration_tests -- container
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 13 integration tests exist and pass.
3. Run:

```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

4. Update plan checkboxes.
5. Commit.

---

### Phase D: Global --container Flag

**Goal:** Any create command auto-adds the new instance to the named container's `memberInstanceIds` when `--container <id>` is passed globally.

**Agent:** CLI Worker

#### Tasks

- [ ] Modify `crates/srs-cli/src/commands/mod.rs` — add `container_id: Option<String>` to `CliContext` and `--container` global arg to `Cli`
- [ ] Modify `crates/srs-cli/src/commands/mod.rs` — pass `container_id` through `dispatch` into `CliContext`
- [ ] Modify `crates/srs-cli/src/commands/note.rs` — after successful `create_note`, if `ctx.container_id.is_some()`, call `add_member`
- [ ] Modify `crates/srs-cli/src/commands/tag.rs` — same pattern for `tag create`
- [ ] Modify `crates/srs-cli/src/commands/record.rs` — same pattern for `record create`
- [ ] Modify `crates/srs-cli/src/commands/relation.rs` — `relation create` is exempt (relations are not instances; no `instanceId` to add)
- [ ] Modify `crates/srs-cli/tests/integration_tests.rs` — add 3 integration tests

#### `mod.rs` changes

Add to `Cli`:
```rust
/// Container context: auto-add created instances to this container's memberInstanceIds
#[arg(long, global = true)]
pub container_id: Option<String>,
```

Add to `CliContext`:
```rust
pub container_id: Option<String>,
```

Add to `dispatch` → `CliContext` construction:
```rust
container_id: cli.container_id,
```

#### Create handler pattern

After any successful create that returns an `instanceId`, if `ctx.container_id.is_some()`:

```rust
if let Some(ref cid) = ctx.container_id {
    // silently ignore if container not found or add_member fails —
    // the primary create succeeded; container membership is best-effort at CLI layer
    let _ = add_member(&ctx.repo, cid, &created_instance_id);
}
```

**Silent failure rationale**: the record was already written. A missing or invalid container ID should not roll back the create. The caller can always call `srs container members add` manually. Implementations may optionally include a warning in `diagnostics[]` rather than silently swallowing the error — both are acceptable.

Affected create handlers: `cmd_note_create`, `cmd_tag_create`, `cmd_record_create`. Not `cmd_field_create` or `cmd_type_create` (package definitions, not instances; no `instanceId` in `instanceIndex`).

#### Integration tests

- `container_flag_note_create_adds_to_container` — `srs --container <id> note create` with valid container; `container members list` returns the new note's instanceId
- `container_flag_record_create_adds_to_container` — same for `record create`
- `container_flag_invalid_container_create_still_succeeds` — `--container <nonexistent-id> note create`; note is created successfully despite bad container id

#### Acceptance Criteria

- [ ] `srs --container <id> note create` adds the note instanceId to container memberInstanceIds
- [ ] `srs --container <id> record create` adds the record instanceId to container memberInstanceIds
- [ ] Create succeeds even if `--container <id>` references a non-existent container
- [ ] Commands without `--container` are unaffected

#### Testing

```bash
cargo test -p srs --test integration_tests -- container_flag
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 3 integration tests exist and pass.
3. Run:

```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

4. Update plan checkboxes.
5. Commit.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] All 13 CLI integration tests pass: `cargo test -p srs --test integration_tests -- container`
- [ ] All 3 --container flag integration tests pass: `cargo test -p srs --test integration_tests -- container_flag`
- [ ] All 22 service unit tests pass: `cargo test -p srs-repository container`
- [ ] All 6 core type/validation tests pass: `cargo test -p srs-core container`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `CoreError::MissingRequiredField` exists and accepts `field_id: String` — use `.to_string()` on string literals (e.g. `"containerId".to_string()`). Do not use `&str` or `&'static str`.
- `new_instance_id()` from `srs-repository::writer` generates UUID v4.
- `write_manifest()` from `srs-repository::writer` does a full pretty-print rewrite preserving `manifest.extra`.
- `load_manifest()` from `srs-repository::manifest` populates `manifest.extra` via `#[serde(flatten)]`.
- Existing manifest-embedded `container` field in `manifest.extra` is independent of `containerIndex` — both can coexist.
- Spec `Container.namespace` and `Container.name` are optional addressing metadata (§04-6) — included in the struct but not required by validation.
- Invariant 20 (`invariant-020.json`) is a core invariant — `containerId` must never appear as a member/root ID. Checked in `validate_container_invariants`, not in `validate_container` (which is pure type-level validation without repo context).
- `AttentionState` (ext:addressability, Invariant 34) requires a resolvable `containerId` — this plan delivers that foundation; the `AttentionState` struct itself is out of scope.
- The `--container` flag is a single-invocation substitute for `AttentionState` — it has no persistence, no session concept, and no Protocol integration. It is intentionally minimal.
- `add_member` failure when `--container` is set is silent at the CLI layer because the primary write already succeeded. This is intentional — membership is best-effort in the absence of transactions.
