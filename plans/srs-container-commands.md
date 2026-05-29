# Plan: SRS Container Commands

## Summary

Add a `srs container` command group for full CRUD over multiple named containers per repository. Containers are first-class SRS 2.0 grouping entities — repos may have several (e.g. `Sprint 1`, `Backlog`) stored as separate JSON files in `containers/`. No Rust struct, service, or CLI surface exists for containers today. This plan fills that gap in the same entity-first pattern used for `note`, `tag`, and `relation`.

**Why containers matter for live facilitation:** `AttentionState.containerId` is mandatory (Invariant 34) — every document-space `Address` requires a `containerId` as its root context specifier. Containers are the scope boundary for `ext:addressability`, Protocol stage tracking, and Context Queries. A correct container CRUD implementation is a prerequisite for any live facilitation or TSS work: without resolvable containers, `AttentionState` cannot be validated and conversation chunks cannot be associated with the correct document scope.

**CLI container scoping:** This plan also adds a global `--container <container-id>` flag that makes the container the **operational scope** for content commands — not merely a membership tag. The semantics are:

- **list**: returns only instances that belong to the container (`memberInstanceIds`)
- **create**: creates the instance and adds it to `memberInstanceIds`
- **get**: allowed regardless of container membership (addressing by ID is always unambiguous)
- **delete**: refused with an error if the instance is not in `memberInstanceIds` — the flag is a scope guard, not just a hint
- **update**: allowed regardless of container membership (updates the instance, not its membership)

A container should be self-contained: when you work within a container scope, you see only what belongs to it, and destructive operations are constrained to it. This is the pragmatic CLI substitute for a live `AttentionState` cursor — no persistent file, passed per-invocation.

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

**I/O ordering for file-backed CRUD:** Container CRUD follows the rule established in [ADR-007](../docs/adr/007-file-index-io-ordering.md) — file-first on create, index-first on delete — so the `containerIndex` is always consistent and dangling index entries cannot result from interrupted writes. See that ADR for full rationale and the repair strategy.

One transitional storage note: the manifest JSON schema (`crates/srs-schema/schemas/2.0/manifest.json`) has `additionalProperties: false` and does not declare `containerIndex`. Storing `containerIndex` in `manifest.extra` creates schema drift — the live manifest will not validate against the published schema. Schema validation is currently deferred in the codebase (not enforced at runtime), so this is safe to ship as-is. A future plan must add `containerIndex` to the manifest schema before schema validation is turned on.

This plan implements the entity-first command pattern established in [srs-cli-command-structure.md](srs-cli-command-structure.md) and the manifest-extra storage pattern from `manifest_service.rs`.

---

## Scope

- `Container` struct and `ContainerIndexEntry` struct in `srs-core`
- Pure `validate_container` function in `srs-core` (invariants only)
- `container_service` module in `srs-repository` with full CRUD plus membership management
- `srs container` CLI command group: `list`, `create`, `get`, `update`, `delete`, `members`, `roots`, `validate`
- Storage: `containers/{slug}-{id_prefix}.json` files tracked via `containerIndex` in `manifest.extra`; IDs must be UUIDs — `validate_container` checks UUID format explicitly (not just non-empty)
- Global `--container <container-id>` flag: makes the container the operational scope — list returns only container members, create adds to membership, delete is refused if instance not in container

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
srs container list [--type <container-type>] [--member <instance-id>] [--root <instance-id>]
                                                # --type: filter by containerType
                                                # --member: containers where instance appears in memberInstanceIds OR rootInstanceIds (i.e. belongs in any role)
                                                # --root: containers where instance appears specifically in rootInstanceIds
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

**Distinguishing container kinds:** use `containerType` (free-form string, e.g. `"meeting"`, `"project"`, `"research"`) as a lightweight label for filtering and display. For richer differentiation with typed fields (meeting date, project goal, research hypothesis), create a Tier 2 Record of the appropriate Type and list its `instanceId` in `rootInstanceIds` — the container holds the boundary, the anchor Record holds the semantic state. Both approaches are complementary.

---

## Phases

### Phase A: Core Type

**Goal:** `Container` and `ContainerIndexEntry` structs exist in `srs-core` with serialization and pure validation.

**Agent:** Core Model Worker

#### Tasks

- [x] Create `crates/srs-core/src/types/container.rs` — `Container` struct (mirrors `TagDefinition` pattern) and `ContainerIndexEntry` struct
- [x] Modify `crates/srs-core/src/types/mod.rs` — add `pub mod container;`
- [x] Create `crates/srs-core/src/validation/container.rs` — `validate_container` function
- [x] Modify `crates/srs-core/src/validation/mod.rs` — add `pub mod container;`

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
    // containerId must be a UUID — parse with the `uuid` crate (already a dependency via srs-repository)
    uuid::Uuid::parse_str(&container.container_id)
        .map_err(|_| CoreError::InvalidFieldValue {
            field_id: "containerId".to_string(),
            reason: "must be a valid UUID".to_string(),
        })?;
    if container.title.is_empty() {
        return Err(CoreError::MissingRequiredField { field_id: "title".to_string() });
    }
    Ok(())
}
```

Note: `CoreError::MissingRequiredField` and `CoreError::InvalidFieldValue` take `String` fields — use `.to_string()` on literals. Check that `CoreError::InvalidFieldValue` exists in `srs-core/src/error.rs`; add it if absent (same pattern as `MissingRequiredField`).

#### Acceptance Criteria

- [x] `Container` serializes to camelCase JSON; optional fields absent when None
- [x] `Container` deserializes from camelCase JSON; unknown fields land in `extra`
- [x] `ContainerIndexEntry` round-trips correctly
- [x] `validate_container` returns `Ok` for minimal valid container (UUID `containerId`, non-empty `title`)
- [x] `validate_container` returns `MissingRequiredField` for empty `containerId` or `title`
- [x] `validate_container` returns `InvalidFieldValue` for a non-UUID `containerId`

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
- `validate_container_non_uuid_container_id_fails` — non-empty but non-UUID string (e.g. `"not-a-uuid"`) → `InvalidFieldValue`
- `validate_container_empty_title_fails` — empty `title`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 7 tests exist and pass.
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

- [x] Modify `crates/srs-repository/src/error.rs` — add `ContainerNotFound` and `ContainerValidation` variants + PartialEq arms
- [x] Create `crates/srs-repository/src/container_service.rs` — all service functions and helper types
- [x] Modify `crates/srs-repository/src/lib.rs` — add `pub mod container_service;`

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
pub struct ContainerSummary {
    pub container_id: String,
    pub title: String,
    pub path: String,
    pub container_type: Option<String>,  // included so callers can filter/display without re-loading the file
}
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
pub fn list_containers(
    repo_root: &Path,
    container_type: Option<&str>,
    member_instance_id: Option<&str>,  // matches memberInstanceIds OR rootInstanceIds
    root_instance_id: Option<&str>,    // matches rootInstanceIds only
) -> Result<Vec<ContainerSummary>, RepositoryError>
// load_manifest → load_container_index → load each container file
// filter by container_type when Some
// filter by member_instance_id when Some: only containers where the id appears in
//   member_instance_ids OR root_instance_ids (the node belongs in any role)
// filter by root_instance_id when Some: only containers where the id appears in
//   root_instance_ids specifically
// Both member_instance_id and root_instance_id may be supplied simultaneously (AND semantics)
// ContainerSummary includes containerType for filtering and display

pub fn containers_for_instance(repo_root: &Path, instance_id: &str) -> Result<Vec<ContainerSummary>, RepositoryError>
// Returns all containers where instance_id appears in memberInstanceIds OR rootInstanceIds.
// Delegates to list_containers(repo_root, None, Some(instance_id), None).

pub fn create_container(repo_root: &Path, mut container: Container) -> Result<Container, RepositoryError>
// Step 1: mint ID first (before validation) if container.container_id.is_empty()
//   container.container_id = new_instance_id()
// Step 2: validate_container → ContainerValidation on failure
//   (validation rejects empty containerId and non-UUID containerId, so ID must exist before validate)
// Step 3: create_dir_all("containers/")
// Step 4: slug = slugify_title(&container.title)
//   id_prefix = &container.container_id[..container.container_id.len().min(8)]
//   filename = format!("{}-{}.json", slug, id_prefix)
// Step 5: file_path = repo_root.join("containers").join(&filename)
// Step 6: write_container_file(&container, &file_path)
// Step 7: load_manifest → load_container_index → push ContainerIndexEntry { container_id, title, path: relative_path }
// Step 8: save_container_index → write_manifest → return container
//
// I/O ordering note: file is written (Step 6) before the index is updated (Step 8).
// If Step 8 fails, the file exists on disk but is not in containerIndex — it is an orphan,
// invisible to list/get. This is recoverable (the file can be manually indexed or deleted).
// The inverse order (index first, file second) would leave a dangling index entry pointing
// to a missing file, which causes load errors on every list — worse failure mode.
// Accept the orphan risk; add a future `srs container repair` scan if needed.

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
// Step 1: remove entry from index → save_container_index → write_manifest
//   (index updated FIRST — if file removal then fails, the entry is already gone from the index;
//    the file is an orphan but not a dangling reference; recoverable)
// Step 2: std::fs::remove_file(repo_root.join(&entry.path))
//   (best-effort; log warning on failure but do not return error — index is already clean)
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
- `delete_container_removes_index_entry` — create then delete; containerIndex entry gone; list returns empty (file removal is best-effort per ADR-007 — test asserts index consistency, not file absence)
- `delete_container_file_is_absent_after_delete` — separate test: create then delete under normal I/O; file is also absent on disk (verifies the best-effort removal succeeds in the happy path)
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
- `containers_for_instance_returns_matching_containers` — create two containers; add instance as member of one; `containers_for_instance` returns only that container
- `containers_for_instance_includes_root_role` — create a container; add instance as a root (not member); `containers_for_instance` still returns it (root counts as "in" the container)
- `containers_for_instance_returns_empty_when_no_match` — instance not in any container in any role; returns empty vec
- `list_containers_root_filter_matches_root_only` — add instance as root of one container and member of another; `--root` filter returns only the first

#### Acceptance Criteria

- [x] `create_container` writes file and appends to containerIndex
- [x] `get_container` returns `ContainerNotFound` for unknown id
- [x] `update_container` patches only supplied fields
- [x] `delete_container` removes the containerIndex entry (hard requirement) and removes the file on disk (best-effort per ADR-007; returns success even if file unlink fails)
- [x] `add_member` / `add_root` are idempotent
- [x] `remove_member` / `remove_root` set field to None when list empties
- [x] `validate_container_invariants` catches invalid member/root IDs (Invariant 21)
- [x] `validate_container_invariants` catches containerId appearing in memberInstanceIds or rootInstanceIds (Invariant 20)

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 30 tests exist and pass.
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

- [x] Create `crates/srs-cli/src/commands/container.rs` — dispatch and handlers
- [x] Modify `crates/srs-cli/src/commands/mod.rs` — add module, enums, dispatch arm
- [x] Modify `crates/srs-cli/tests/integration_tests.rs` — add test-first integration tests (write tests first, then implement)

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
    /// List all containers, optionally filtered by containerType or membership role
    List {
        /// Filter by containerType (e.g. "meeting", "project", "research")
        #[arg(long = "type")]
        container_type: Option<String>,
        /// Return only containers where this instance appears in memberInstanceIds OR rootInstanceIds
        #[arg(long = "member")]
        member_instance_id: Option<String>,
        /// Return only containers where this instance appears specifically in rootInstanceIds
        #[arg(long = "root")]
        root_instance_id: Option<String>,
    },
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
        ContainerCommand::List { container_type, member_instance_id, root_instance_id } => cmd_list(ctx, container_type, member_instance_id, root_instance_id),
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
| `cmd_list` | `"container list"` | `{ "containers": [{ containerId, title, path, containerType? }] }` |
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
- `container_list_filters_by_type` — create containers with `containerType: "meeting"` and `"project"`; `--type meeting` returns only the meeting container
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
- `container_list_member_filter_returns_containing_containers` — create two containers; add an instance id to one via `members add`; `srs container list --member <instance-id>` returns only that container
- `container_list_member_filter_includes_root_role` — add instance as root of a container; `--member <instance-id>` still returns it
- `container_list_root_filter_returns_root_containers` — add instance as root of one container and member of another; `--root <instance-id>` returns only the first

#### Acceptance Criteria

- [x] `srs container list` returns `{ "containers": [] }` on fresh repo
- [x] `srs container create` reads stdin JSON; returns container with assigned id
- [x] `srs container get <id>` returns the container
- [x] `srs container update <id>` patches title without touching description
- [x] `srs container delete <id>` removes the container; subsequent list is empty
- [x] `srs container members add` / `remove` / `list` operate correctly
- [x] `srs container roots add` / `remove` / `list` operate correctly
- [x] `srs container validate <id>` returns `ok: true` for clean container

#### Testing

```bash
cargo test -p srs --test integration_tests -- container
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 11 `container*` integration tests exist and pass.
3. Run:

```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

4. Update plan checkboxes.
5. Commit.

---

### Phase D: Global --container Scope Flag

**Goal:** `--container <id>` makes the named container the operational scope for all content commands: list is filtered, create adds to membership, delete is refused if the instance is not a member.

**Agent:** CLI Worker

#### Tasks

- [x] Modify `crates/srs-cli/src/commands/mod.rs` — add `container_id: Option<String>` to `CliContext` and `--container` global arg to `Cli`
- [x] Modify `crates/srs-cli/src/commands/mod.rs` — pass `container_id` through `dispatch` into `CliContext`
- [x] Modify `crates/srs-repository/src/container_service.rs` — add `is_member(repo_root, container_id, instance_id)` helper
- [x] Modify `crates/srs-cli/src/commands/note.rs` — scope list, create, delete
- [x] Modify `crates/srs-cli/src/commands/tag.rs` — scope list, create, delete
- [x] Modify `crates/srs-cli/src/commands/record.rs` — scope list, create, delete
- [x] Modify `crates/srs-cli/src/commands/relation.rs` — scope list (both endpoints must be members)
- [x] Modify `crates/srs-cli/tests/integration_tests.rs` — add 8 integration tests

#### `mod.rs` changes

Add to `Cli`:
```rust
/// Container scope: constrains list/create/delete to this container's membership
#[arg(long, global = true)]
pub container_id: Option<String>,
```

Add to `CliContext`:
```rust
pub container_id: Option<String>,
```

#### Scoping rules per operation

| Operation | `--container` absent | `--container <id>` set |
|-----------|---------------------|------------------------|
| `list` | all instances | only `memberInstanceIds` of the container |
| `create` | creates instance normally | creates instance + adds to `memberInstanceIds`; error if container not found |
| `get` | by ID, unrestricted | by ID, unrestricted (get is always unambiguous) |
| `update` | by ID, unrestricted | by ID, unrestricted |
| `delete` | deletes instance | refused with error if instance not in `memberInstanceIds`; deletes and removes from membership if it is |
| `relation list` | all relations | only relations where **both** `sourceInstanceId` and `targetInstanceId` are in `memberInstanceIds` |
| `relation create/delete` | unrestricted | unrestricted (relations are edges between instances; membership controls the instances, not the edges) |
| `field/type create/delete` | unrestricted | unrestricted (package definitions, not instances) |

**Delete scope guard**: `output::err("note delete", vec!["Instance '{id}' is not a member of container '{cid}' — delete refused"])` with exit code 1. This prevents cross-container destructive operations when working in a scoped context.

**Create failure on bad container**: create with `--container <bad-id>` must return an error with no instance written. The container must be verified to exist **before** the create service is called — not after. Post-create `add_member` failure leaves a persisted-but-unregistered instance, which cannot satisfy the acceptance test `container_scope_note_create_fails_invalid_container`.

#### `is_member` helper in `container_service.rs`

```rust
pub fn is_member(repo_root: &Path, container_id: &str, instance_id: &str) -> Result<bool, RepositoryError>
// get_container → check member_instance_ids.contains(instance_id)
// ContainerNotFound if container absent
```

#### List scoping pattern (per handler)

```rust
if let Some(ref cid) = ctx.container_id {
    let members = list_members(&ctx.repo, cid)?;  // ContainerNotFound → error
    // filter result to only instances whose instanceId is in members
}
```

#### Create scoping pattern (per handler)

```rust
// BEFORE calling the create service — fail fast, no instance written yet:
if let Some(ref cid) = ctx.container_id {
    // get_container returns ContainerNotFound if absent; propagate as output::err
    match get_container(&ctx.repo, cid) {
        Err(RepositoryError::ContainerNotFound { .. }) => {
            return Ok(output::err("note create",
                vec![format!("Container '{}' not found — no note written", cid)]));
        }
        Err(e) => return Err(e.into()),
        Ok(_) => {}  // container exists; proceed to create
    }
}

// create the instance:
let result = create_note(&ctx.repo, note)?;

// add to container membership (container was verified above; failure here is unexpected I/O):
if let Some(ref cid) = ctx.container_id {
    add_member(&ctx.repo, cid, &result.note.instance_id)
        .map_err(|e| anyhow::anyhow!("Note created but failed to add to container: {e}"))?;
}
```

The pre-check uses `get_container` (which loads the container index and the container file). This is two reads before the create, which is acceptable — it eliminates the partial-failure window where the instance exists but container registration failed.

#### Delete scoping pattern (per handler)

```rust
if let Some(ref cid) = ctx.container_id {
    if !is_member(&ctx.repo, cid, &id)? {
        return Ok(output::err("note delete",
            vec![format!("Instance '{}' is not a member of container '{}' — delete refused", id, cid)]));
    }
    // Remove from membership FIRST, before deleting the instance file.
    // Order matters: if file delete succeeds but remove_member fails, the container
    // holds a dangling reference to a deleted instance (Invariant 21 violation).
    // If remove_member succeeds but file delete then fails, the instance file still
    // exists on disk and is still listed in instanceIndex — it is a valid SRS instance,
    // just no longer a container member. This is a recoverable state; a retry of the
    // delete will succeed. The dangling-reference failure (other order) is not recoverable
    // without manual index repair.
    remove_member(&ctx.repo, cid, &id)?;
}
// delete the instance after membership is clean:
delete_note(&ctx.repo, &id)?;
```

#### Integration tests

- `container_scope_note_list_filters_to_members` — two notes, one in container; `--container` list returns only the member
- `container_scope_note_create_adds_to_container` — `--container` note create; `container members list` includes the new id
- `container_scope_note_create_fails_invalid_container` — `--container <bad-id> note create`; returns error, no note written
- `container_scope_note_delete_refused_if_not_member` — note exists but not in container; `--container` delete returns error; note still exists
- `container_scope_note_delete_removes_membership` — note is member; `--container` delete succeeds; note gone and no longer in `memberInstanceIds`
- `container_scope_tag_list_filters_to_members` — same pattern for `tag list`
- `container_scope_record_list_filters_to_members` — same pattern for `record list`
- `container_scope_relation_list_filters_to_internal` — two relations: one between two container members, one crossing outside; `--container` list returns only the internal one

#### Acceptance Criteria

- [x] `srs --container <id> note list` returns only notes in that container's `memberInstanceIds`
- [x] `srs --container <id> note create` creates the note and adds it to `memberInstanceIds`; fails if container not found
- [x] `srs --container <id> note delete <non-member-id>` returns error and does not delete
- [x] `srs --container <id> note delete <member-id>` deletes and removes from `memberInstanceIds`
- [x] Same list/create/delete scoping behavior for `tag` and `record` commands
- [x] `srs --container <id> relation list` returns only relations where both endpoints are in `memberInstanceIds`
- [x] Commands without `--container` are unaffected
- [x] `get` and `update` are unrestricted by `--container`

#### Testing

```bash
cargo test -p srs --test integration_tests -- container_scope
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 8 integration tests exist and pass.
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

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] All `container*` CLI integration tests pass: `cargo test -p srs --test integration_tests -- container`
- [x] All 8 --container scope integration tests pass: `cargo test -p srs --test integration_tests -- container_scope`
- [x] All service unit tests pass: `cargo test -p srs-repository container_service`
- [x] All core type/validation tests pass: `cargo test -p srs-core container`
 
Plan close-out (2026-05-29):
- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes

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
- `--container` makes the container the **scope boundary**, not a membership hint: list filters, delete guards, create adds. This is the core design invariant for Phase D.
- `add_member` failure on create is a hard error — if a container scope was specified but the container doesn't exist, that is a mistake. Silent failure is explicitly rejected.
- `is_member` is a new helper in `container_service.rs` needed by delete scope guard. It loads the container and checks `memberInstanceIds`. It must be called before the file-level delete to avoid a TOCTOU window.
- `relation create/delete` and package definitions (`field`, `type`) are never container-scoped — they either have no `instanceId` in `instanceIndex` or are not instance content.
