# Plan: Views and Document Views CRUD

## Summary

Add `srs view` and `srs document-view` command groups for full CRUD over View (L1) and DocumentView (L2) definitions stored in the package. Views and DocumentViews are package-defined artefacts. The types, validation, and package loading machinery all exist. The `view_service.rs` module has read-only list/get. The CLI has `srs render document-view` but no management surface at all. This plan delivers create, update, delete, and CLI commands.

**Dependency on `storage-boundary-refactor.md`**: Phase A of this plan (service layer) requires the `RepositoryStore` trait and `FileStore` from `storage-boundary-refactor.md` Phase A. If that work is not yet done, Phase A here should be deferred and Phases B/C (CLI only) cannot be completed either. **Recommended: run `storage-boundary-refactor.md` first**; then this plan's Phase A is already done as part of Phase E of that plan, and only Phases B/C (CLI commands) remain here.

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
| ADR (new) | Introduce `PackageStore` trait to abstract file I/O from service logic | proposed |

Key decisions:

- **`PackageStore` trait**: service functions receive `&dyn PackageStore` (or `&impl PackageStore`), not a `&Path`. All `std::fs` calls are inside `FilePackageStore`. This is the storage boundary referenced in ARCHITECTURE.md — "keep storage boundaries visible so a database-backed implementation can be introduced later."
- **`FilePackageStore`** lives in `srs-repository` alongside the service. It holds the `repo_root: PathBuf` and implements all read/write/delete operations for package JSON files.
- **CLI passes `FilePackageStore`**: `srs-cli` instantiates `FilePackageStore::new(ctx.repo)` and passes it to service functions. The CLI itself does no I/O.
- **Spec `ext:views-l1`**: Validation enforced in `validate_view()` (`srs-core/src/validation/view.rs`).
- **Spec `ext:views-l2`**: Validation enforced in `validate_document_view()`.
- **Storage layout** (file adapter only): `package/views/{slug}-{id_prefix}.json`, `package/document-views/{slug}-{id_prefix}.json`, indexed in `package/package.json`.
- **I/O ordering** (file adapter only): file-first on create; index-first on delete. This is an implementation detail of `FilePackageStore`, not of the service layer.

---

## Scope

- New `PackageStore` trait in `srs-repository` with `FilePackageStore` concrete implementation
- Extend `view_service.rs` in `srs-repository` with full CRUD for both View and DocumentView — all functions take `&dyn PackageStore`, no `&Path`, no `std::fs`
- New error variants for `ViewNotFound` and error variants needed for create/update
- `srs view` CLI command group: `list`, `get`, `create`, `update`, `delete`
- `srs document-view` CLI command group: `list`, `get`, `create`, `update`, `delete`
- New `crates/srs-cli/src/commands/view.rs` and `crates/srs-cli/src/commands/document_view.rs`
- File layout (FilePackageStore): `package/views/{slug}-{id_prefix}.json` and `package/document-views/{slug}-{id_prefix}.json`
- Unit tests in `view_service.rs` using an in-memory `MockPackageStore`; integration tests in `integration_tests.rs` using `FilePackageStore`

**Out of scope:**

- Retrofitting `package_service.rs` to use `PackageStore` (tracked separately — do not touch `package_service.rs` in this plan)
- Validation that `FieldView.fieldId` exists in the bound Type's effective field list (cross-entity referential integrity — deferred)
- Validation that `DocumentSection.renderViewId` resolves to a View (cross-entity referential integrity — deferred)
- Rendering (already exists as `srs render document-view`)
- Any changes to `srs-core` types or validation (they are complete)

---

## Storage Design

### `PackageStore` trait

Defines all I/O operations needed by the service layer. Lives in `crates/srs-repository/src/package_store.rs`.

```rust
pub trait PackageStore {
    fn load_views(&self) -> Result<Vec<View>, RepositoryError>;
    fn load_document_views(&self) -> Result<Vec<DocumentView>, RepositoryError>;
    fn save_view(&self, view: &View) -> Result<(), RepositoryError>;
    fn update_view(&self, view_id: &str, view: &View) -> Result<(), RepositoryError>;
    fn delete_view(&self, view_id: &str) -> Result<(), RepositoryError>;
    fn save_document_view(&self, view: &DocumentView) -> Result<(), RepositoryError>;
    fn update_document_view(&self, view_id: &str, view: &DocumentView) -> Result<(), RepositoryError>;
    fn delete_document_view(&self, view_id: &str) -> Result<(), RepositoryError>;
}
```

### `FilePackageStore`

Concrete file-backed implementation in the same module. Holds `repo_root: PathBuf`. Stores views under `package/views/{slug}-{id_prefix}.json` and document-views under `package/document-views/{slug}-{id_prefix}.json`. Tracks both in `package/package.json` `views[]` / `documentViews[]` arrays.

`slug` = lowercase name, spaces→hyphens, strip non-alphanumeric. `id_prefix` = `&id[..id.len().min(8)]`.

I/O ordering (adapter detail, not service logic): write file first on create; update `package.json` first on delete.

### `MockPackageStore`

In-memory implementation for unit tests. Stores `Vec<View>` and `Vec<DocumentView>` in a `RefCell`. No file system access. Lives in `#[cfg(test)]` in `package_store.rs`.

---

## Command Surface

```
srs view list [--namespace <ns>] [--type-id <uuid>]
    # --namespace: filter by view.namespace
    # --type-id: filter by view.typeId (views bound to that Type)
srs view get <view-id>
srs view create                 # reads full View JSON from stdin
srs view update <view-id>       # reads full View JSON from stdin (full replace, not patch)
srs view delete <view-id>

srs document-view list [--namespace <ns>] [--container-type <type>]
    # --namespace: filter by documentView.namespace
    # --container-type: filter by documentView.containerType
srs document-view get <document-view-id>
srs document-view create        # reads full DocumentView JSON from stdin
srs document-view update <document-view-id>   # reads full DocumentView JSON from stdin
srs document-view delete <document-view-id>
```

`update` is a full replace: the caller supplies the complete View/DocumentView JSON. Partial patching is not supported (same as `update_field` / `update_type`).

---

## Phases

### Phase A: `PackageStore` Trait + Repository Service

**Goal:** `PackageStore` trait and `FilePackageStore` exist; `view_service.rs` provides full CRUD through the trait with no `std::fs` calls; unit tests pass against `MockPackageStore`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/package_store.rs` — `PackageStore` trait, `FilePackageStore`, and `MockPackageStore` (cfg test)
- [ ] Add `ViewNotFound` and `DocumentViewNotFoundById` to `crates/srs-repository/src/error.rs` if not already present
- [ ] Extend `crates/srs-repository/src/view_service.rs` with `ViewSummary`, `DocumentViewSummary`, and CRUD functions — all taking `&dyn PackageStore`
- [ ] Add `pub mod package_store;` to `crates/srs-repository/src/lib.rs`

#### Error variants to add (check `error.rs` first)

```rust
#[error("view not found: {view_id}")]
ViewNotFound { view_id: String },

#[error("document view not found: {document_view_id}")]
DocumentViewNotFoundById { document_view_id: String },
```

Add to `impl PartialEq for RepositoryError`:
```rust
(RepositoryError::ViewNotFound { view_id: a }, RepositoryError::ViewNotFound { view_id: b }) => a == b,
(RepositoryError::DocumentViewNotFoundById { document_view_id: a }, RepositoryError::DocumentViewNotFoundById { document_view_id: b }) => a == b,
```

Note: `DocumentViewNotFound { view_id }` already exists for the render path — keep it unchanged. Add `DocumentViewNotFoundById` as a distinct variant for the CRUD path.

#### Summary types in `view_service.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub type_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub container_type: Option<String>,
    pub section_count: usize,
}
```

#### Service function signatures (`view_service.rs`)

All service functions take `store: &dyn PackageStore`. No `&Path`. No `std::fs`.

```rust
pub fn list_views_summary(store: &dyn PackageStore) -> Result<Vec<ViewSummary>, RepositoryError>
// store.load_views() → map to ViewSummary; filtering is in the CLI layer

pub fn list_document_views_summary(store: &dyn PackageStore) -> Result<Vec<DocumentViewSummary>, RepositoryError>

pub fn get_view_by_id(store: &dyn PackageStore, view_id: &str) -> Result<GetViewResult, RepositoryError>
// store.load_views() → find by id

pub fn get_document_view_by_id(store: &dyn PackageStore, view_id: &str) -> Result<GetDocumentViewResult, RepositoryError>

pub fn create_view(store: &dyn PackageStore, view: View) -> Result<View, RepositoryError>
// validate_view(&view) → ViewValidation on failure
// store.save_view(&view)
// return view

pub fn update_view(store: &dyn PackageStore, view_id: &str, view: View) -> Result<View, RepositoryError>
// validate_view(&view) → ViewValidation on failure
// store.update_view(view_id, &view) → propagates ViewNotFound
// return view

pub fn delete_view(store: &dyn PackageStore, view_id: &str) -> Result<String, RepositoryError>
// store.delete_view(view_id) → propagates ViewNotFound
// return view_id.to_string()

pub fn create_document_view(store: &dyn PackageStore, view: DocumentView) -> Result<DocumentView, RepositoryError>
pub fn update_document_view(store: &dyn PackageStore, view_id: &str, view: DocumentView) -> Result<DocumentView, RepositoryError>
pub fn delete_document_view(store: &dyn PackageStore, view_id: &str) -> Result<String, RepositoryError>
// mirror view functions for DocumentView
```

#### CLI wiring pattern

The CLI constructs `FilePackageStore::new(&ctx.repo)` and passes `&store` to service functions:

```rust
// in view.rs handler:
let store = FilePackageStore::new(&ctx.repo);
let result = create_view(&store, view)?;
```

The existing `view_service::list_views` and `view_service::get_view_by_id` (which take `&Path` and use `load_package`) should be replaced or superseded by the new trait-based versions. If render_service depends on the old signatures, update render_service to use `FilePackageStore` instead of `&Path`.

#### Unit tests in `view_service.rs`

Use `MockPackageStore` — no temp directory, no file system. This is the key difference from the old pattern.

View tests:
- `create_view_stores_and_returns_view` — create; MockPackageStore has the view; return matches
- `list_views_summary_returns_all` — populate mock with two views; list returns both summaries
- `get_view_by_id_finds_view` — populate mock; get by id; Found
- `get_view_by_id_not_found` — empty mock; get → NotFound
- `update_view_replaces_view` — save view; update with new description; load_views returns updated
- `update_view_not_found` — update unknown id → ViewNotFound error
- `delete_view_removes_view` — save then delete; load_views returns empty
- `delete_view_not_found` — delete unknown id → ViewNotFound error

DocumentView tests (same 8, prefixed `document_view_`).

Total: 16 unit tests, all run without touching the file system.

#### Acceptance Criteria

- [ ] `PackageStore` trait defined; `FilePackageStore` and `MockPackageStore` implement it
- [ ] All service functions in `view_service.rs` take `&dyn PackageStore` — no `&Path`, no `std::fs` imports
- [ ] `create_view` / `create_document_view` call the validator before delegating to the store
- [ ] `update_view` / `update_document_view` propagate `ViewNotFound` / `DocumentViewNotFoundById` from the store
- [ ] Unit tests pass using `MockPackageStore` only — no temp dirs in Phase A tests
- [ ] `FilePackageStore` integration: `srs-repository` compiles; existing `srs render document-view` still works

#### Testing

```bash
cargo test -p srs-repository view_service
cargo test -p srs-repository package_store
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 16 unit tests exist and pass (8 view + 8 document_view), all using MockPackageStore.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

### Phase B: CLI — `srs view` Command Group

**Goal:** `srs view list|get|create|update|delete` is wired and integration tests pass.

**Agent:** CLI Worker

#### Tasks

- [ ] Create `crates/srs-cli/src/commands/view.rs` — dispatch and handlers
- [ ] Modify `crates/srs-cli/src/commands/mod.rs` — add `pub mod view;`, `ViewCommand` enum, dispatch arm
- [ ] Add `view list/get/create/update/delete` integration tests to `crates/srs-cli/tests/integration_tests.rs`

#### `mod.rs` additions

Add `pub mod view;` at top of module list.

Add to `Commands` enum:
```rust
/// View (L1) definition commands
#[command(subcommand)]
View(ViewCommand),
```

Add enum:
```rust
#[derive(Subcommand)]
pub enum ViewCommand {
    /// List view definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by bound typeId
        #[arg(long = "type-id")]
        type_id: Option<String>,
    },
    /// Get a view definition by ID
    Get { view_id: String },
    /// Create a new view definition (reads JSON from stdin)
    Create,
    /// Update a view definition (reads full JSON from stdin)
    Update { view_id: String },
    /// Delete a view definition
    Delete { view_id: String },
}
```

Add to `dispatch` match:
```rust
Commands::View(cmd) => view::dispatch(ctx, cmd),
```

#### Handler payload shapes

| Handler | command string | Payload |
|---------|---------------|---------|
| `cmd_list` | `"view list"` | `{ "views": [ViewSummary] }` |
| `cmd_get` | `"view get"` | `{ "view": <View> }` |
| `cmd_create` | `"view create"` | `{ "view": <View> }` |
| `cmd_update` | `"view update"` | `{ "view": <View> }` |
| `cmd_delete` | `"view delete"` | `{ "viewId": "..." }` |

List filtering (namespace, type_id) is done in the handler after `list_views_summary()`, not in the service.

CLI handlers construct `FilePackageStore::new(&ctx.repo)` and pass `&store` to service functions. No `&Path` passed to services.

#### Integration tests

Helper: `make_view_test_repo() -> TempDir` — minimal manifest, package with `views: [], documentViews: []`.

Minimal valid View fixture (camelCase JSON):
```json
{
  "id": "00000000-0000-4000-8000-000000000001",
  "namespace": "com.test",
  "name": "test-view",
  "version": 1,
  "description": "A test view",
  "typeId": "00000000-0000-4000-8000-000000000010",
  "typeVersion": 1,
  "fieldViews": [
    { "fieldId": "00000000-0000-4000-8000-000000000020", "order": 0 }
  ],
  "createdAt": "2026-01-01T00:00:00Z"
}
```

Tests:
- `view_list_returns_empty_initially` — fresh repo; `{ "views": [] }`
- `view_create_returns_view` — create; response `view.id` matches
- `view_get_returns_created_view` — create then get
- `view_list_returns_created_view` — create; list contains view summary
- `view_list_filters_by_namespace` — create two views with different namespaces; `--namespace` filter returns only matching
- `view_list_filters_by_type_id` — create two views with different typeIds; `--type-id` filter returns only matching
- `view_update_replaces_view` — create, update description, get; description updated
- `view_delete_removes_view` — create, delete, list; list empty
- `view_get_not_found_returns_error` — get unknown id → error in diagnostics

#### Acceptance Criteria

- [ ] `srs view list` returns `{ "views": [] }` on fresh repo
- [ ] `srs view create` reads stdin JSON; returns view
- [ ] `srs view get <id>` returns the view
- [ ] `srs view update <id>` replaces the view; subsequent get shows new content
- [ ] `srs view delete <id>` removes the view; subsequent list is empty
- [ ] `--namespace` and `--type-id` filters work on `srs view list`
- [ ] Get/delete on unknown id returns an error payload (not a panic)

#### Testing

```bash
cargo test -p srs --test integration_tests -- view_
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 9 `view_*` integration tests exist and pass.
3. Run:

```bash
cargo build
cargo clippy -- -D warnings
cargo test -p srs --test integration_tests -- view_
```

4. Update plan checkboxes.
5. Commit.

---

### Phase C: CLI — `srs document-view` Command Group

**Goal:** `srs document-view list|get|create|update|delete` is wired and integration tests pass.

**Agent:** CLI Worker

#### Tasks

- [ ] Create `crates/srs-cli/src/commands/document_view.rs` — dispatch and handlers
- [ ] Modify `crates/srs-cli/src/commands/mod.rs` — add `pub mod document_view;`, `DocumentViewCommand` enum, dispatch arm
- [ ] Add `document_view_*` integration tests to `crates/srs-cli/tests/integration_tests.rs`

#### `mod.rs` additions

Add `pub mod document_view;` at top of module list.

Add to `Commands` enum:
```rust
/// Document View (L2) definition commands
#[command(subcommand)]
DocumentView(DocumentViewCommand),
```

Add enum:
```rust
#[derive(Subcommand)]
pub enum DocumentViewCommand {
    /// List document view definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by containerType
        #[arg(long = "container-type")]
        container_type: Option<String>,
    },
    /// Get a document view definition by ID
    Get { document_view_id: String },
    /// Create a new document view definition (reads JSON from stdin)
    Create,
    /// Update a document view definition (reads full JSON from stdin)
    Update { document_view_id: String },
    /// Delete a document view definition
    Delete { document_view_id: String },
}
```

Add to `dispatch` match:
```rust
Commands::DocumentView(cmd) => document_view::dispatch(ctx, cmd),
```

#### Handler payload shapes

| Handler | command string | Payload |
|---------|---------------|---------|
| `cmd_list` | `"document-view list"` | `{ "documentViews": [DocumentViewSummary] }` |
| `cmd_get` | `"document-view get"` | `{ "documentView": <DocumentView> }` |
| `cmd_create` | `"document-view create"` | `{ "documentView": <DocumentView> }` |
| `cmd_update` | `"document-view update"` | `{ "documentView": <DocumentView> }` |
| `cmd_delete` | `"document-view delete"` | `{ "documentViewId": "..." }` |

#### Integration tests

Minimal valid DocumentView fixture:
```json
{
  "id": "00000000-0000-4000-8000-000000000002",
  "namespace": "com.test",
  "name": "test-doc-view",
  "version": 1,
  "description": "A test document view",
  "sections": [
    {
      "sectionId": "section-1",
      "order": 0,
      "source": { "type": "fixed-instances", "instanceIds": [] }
    }
  ],
  "createdAt": "2026-01-01T00:00:00Z"
}
```

Tests:
- `document_view_list_returns_empty_initially`
- `document_view_create_returns_document_view`
- `document_view_get_returns_created`
- `document_view_list_returns_created`
- `document_view_list_filters_by_namespace`
- `document_view_list_filters_by_container_type`
- `document_view_update_replaces_document_view`
- `document_view_delete_removes_document_view`
- `document_view_get_not_found_returns_error`

#### Acceptance Criteria

- [ ] `srs document-view list` returns `{ "documentViews": [] }` on fresh repo
- [ ] `srs document-view create` reads stdin JSON; returns documentView
- [ ] `srs document-view get <id>` returns the document view
- [ ] `srs document-view update <id>` replaces the document view; subsequent get shows new content
- [ ] `srs document-view delete <id>` removes the document view; subsequent list is empty
- [ ] `--namespace` and `--container-type` filters work on `srs document-view list`
- [ ] Get/delete on unknown id returns an error payload

#### Testing

```bash
cargo test -p srs --test integration_tests -- document_view_
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 9 `document_view_*` integration tests exist and pass.
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
- [ ] All `view_*` CLI integration tests pass: `cargo test -p srs --test integration_tests -- view_`
- [ ] All `document_view_*` CLI integration tests pass: `cargo test -p srs --test integration_tests -- document_view_`
- [ ] All `view_service` unit tests pass: `cargo test -p srs-repository view_service`
- [ ] `srs render document-view` still works (no regression in render path)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `View` and `DocumentView` structs in `srs-core` are complete and serialize to camelCase JSON via `serde` — inspect `crates/srs-core/src/types/view.rs` to confirm field names before writing fixtures.
- `validate_view()` and `validate_document_view()` in `srs-core/src/validation/view.rs` are the canonical validators — call them on create and update inside service functions, before delegating to the store.
- `slugify()` is a `FilePackageStore` private helper, not a service concern. Copy pattern from `package_service.rs` but keep it inside `FilePackageStore`.
- `package.json` `views` and `documentViews` keys may be absent in older repos — `FilePackageStore` must treat missing keys as empty arrays (consistent with `#[serde(default)]` in `PackageMetadata`).
- `DocumentViewNotFound { view_id }` already exists in `error.rs` for the render path — leave it; add `DocumentViewNotFoundById { document_view_id }` as a separate variant for the CRUD path.
- **Service functions must not import `std::fs`, `std::path::Path`, or call any I/O.** If a compiler error or test requires `&Path` in a service function, that is a design error — fix the design, not the rule.
- `FilePackageStore` integration tests (CLI integration tests in Phase B/C) use a real temp directory. Unit tests in Phase A use only `MockPackageStore`.
- `srs render document-view` currently calls `view_service::get_document_view_by_id(repo_root, id)` which takes a `&Path`. After this plan, that function will take `&dyn PackageStore`. The render command handler in `render.rs` must be updated to construct a `FilePackageStore` and pass it — this is a required change in Phase A's acceptance criteria ("existing `srs render document-view` still works").
