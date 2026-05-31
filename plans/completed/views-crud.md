# Plan: Views and Document Views CRUD

## Summary

Add `srs view` and `srs document-view` command groups for full CRUD over View (L1) and DocumentView (L2) definitions stored in the package. Views and DocumentViews are package-defined artefacts. The types, validation, and package loading machinery all exist. The `view_service.rs` module has read-only list/get. The CLI has `srs render document-view` but no management surface at all. This plan delivers create, update, delete, and CLI commands.

**Storage boundary work is complete.** The earlier dependency on `storage-boundary-refactor.md` is resolved — `RepositoryStore` already carries all view I/O methods. The `PackageStore` trait design from the original plan is obsolete; do not implement it.

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
| ADR-001 | CLI is a thin consumer of library crates; no business logic in handlers | accepted |
| ADR-010 | Every service function takes a typed input struct and returns a typed result struct | accepted |

Key decisions:

- **No new trait or store type.** `RepositoryStore` already has `save_view`, `update_view_file`, `delete_view_file`, `ensure_views_dir`, and the DocumentView equivalents. `add_definition_to_boundary` and `remove_definition_from_boundary` already handle `DefinitionKind::View` and `DefinitionKind::DocumentView`, which map to `"views"` and `"documentViews"` in `package.json`.
- **`find_view_path` helper pattern.** To locate a view file for update/delete, use `resolve_definition_owner(id, DefinitionKind::View)` to find the boundary, then scan `load_package_json()["views"]` and `load_instance_json` to match by `id`. This mirrors `find_field_path` in `package_service.rs`. Note: `PackageBoundary` only carries `field_paths` and `type_paths` — there is no `view_paths` field, so the service reads view paths directly from the raw `package.json` JSON.
- **CLI handler pattern.** Handlers use `with_store(&ctx, |store| Ok(view_service::fn(store, input)?))`. No direct `FileStore` construction in handlers (enforced by an existing integration test).
- **Unit tests use `FileStore` with `tempfile::TempDir`.** `MemoryStore::save_view` stores at the raw path key (without `package/` prefix), so `find_view_path` calling `load_instance_json("package/views/...")` would miss the entry. Use `FileStore` for all view service unit tests. `tempfile` is already in dev-dependencies.
- **Storage layout:** `package/views/{slug}-{id_prefix}.json`, `package/document-views/{slug}-{id_prefix}.json`, tracked in `package/package.json`.
- **`DocumentViewNotFound { view_id }` is kept unchanged** — the render path uses it. Add `DocumentViewNotFoundById { document_view_id }` as a separate variant for the CRUD path.

---

## Scope

- Add `ViewNotFound` and `DocumentViewNotFoundById` error variants to `error.rs`
- Extend `view_service.rs` with summary structs, result structs, and full CRUD for both View and DocumentView — all functions take `store: &dyn RepositoryStore`
- New `crates/srs-cli/src/commands/view.rs` and `crates/srs-cli/src/commands/document_view.rs`
- Wire `srs view` and `srs document-view` into `crates/srs-cli/src/commands/mod.rs`
- Unit tests in `view_service.rs` using `FileStore` + temp dir
- Integration tests in `crates/srs-cli/tests/integration_tests.rs`

**Out of scope:**

- No `PackageStore` trait, no `FilePackageStore`, no `MockPackageStore`
- No new `pub mod` in `lib.rs` (`view_service` is already declared)
- No changes to `package.rs`, `store.rs`, or `writer.rs`
- Validation that `FieldView.fieldId` exists in the bound Type's field list (cross-entity referential integrity — deferred)
- Validation that `DocumentSection.renderViewId` resolves to a View (cross-entity referential integrity — deferred)
- Rendering (already exists as `srs render document-view`)
- Any changes to `srs-core` types or validation (they are complete)

---

## What already exists (do not re-implement)

- `View`, `DocumentView` structs — `crates/srs-core/src/types/view.rs`
- `validate_view()`, `validate_document_view()` — `crates/srs-core/src/validation/view.rs`
- `list_views`, `get_view_by_id`, `list_document_views`, `get_document_view_by_id` — `crates/srs-repository/src/view_service.rs`
- `GetViewResult { Found(Box<View>), NotFound }`, `GetDocumentViewResult` — `crates/srs-repository/src/view_service.rs`
- All store I/O methods for views — `RepositoryStore` trait in `store.rs`, implemented by `FileStore`
- `add_definition_to_boundary`, `remove_definition_from_boundary`, `resolve_definition_owner` — `RepositoryStore` in `store.rs`
- `DefinitionKind::View` → `"views"`, `DefinitionKind::DocumentView` → `"documentViews"` — `store.rs:1394`
- `new_instance_id()` — `crates/srs-repository/src/writer.rs`
- `slugify()` private fn — `crates/srs-repository/src/package_service.rs:566` (copy locally)
- `tempfile` crate already in dev-dependencies

---

## Command Surface

```
srs view list [--namespace <ns>] [--type-id <uuid>]
    # --namespace: filter by view.namespace (handler-side filter on list_views_summary result)
    # --type-id: filter by view.typeId (handler-side filter)
srs view get <id>
srs view create                 # reads full View JSON from stdin
srs view update <id>            # reads full View JSON from stdin (full replace, not patch)
srs view delete <id>

srs document-view list [--namespace <ns>] [--container-type <type>]
    # --namespace: filter by documentView.namespace (handler-side filter)
    # --container-type: filter by documentView.containerType (handler-side filter)
srs document-view get <id>
srs document-view create        # reads full DocumentView JSON from stdin
srs document-view update <id>   # reads full DocumentView JSON from stdin
srs document-view delete <id>
```

`update` is a full replace. Filtering on `list` is done in the handler after `list_views_summary()` / `list_document_views_summary()`.

---

## Phases

### Phase A: Repository Service

**Goal:** `view_service.rs` provides full CRUD for both View and DocumentView through `&dyn RepositoryStore`. All unit tests pass using `FileStore` with a temp directory.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `ViewNotFound` and `DocumentViewNotFoundById` variants to `crates/srs-repository/src/error.rs`; add matching arms to `impl PartialEq for RepositoryError` before the `_ => false` catch-all
- [x] Add the service module doc comment header to `view_service.rs` (see template in `tag_service.rs`)
- [x] Add additional imports to `view_service.rs`: `DefinitionKind`, `new_instance_id`, `validate_view`, `validate_document_view`
- [x] Add `ViewSummary` and `DocumentViewSummary` summary structs (camelCase serde)
- [x] Add result structs: `CreateViewResult`, `UpdateViewResult`, `DeleteViewResult`, `CreateDocumentViewResult`, `UpdateDocumentViewResult`, `DeleteDocumentViewResult`
- [x] Add private `slugify(name: &str) -> String` helper (copy from `package_service.rs:566`)
- [x] Add private `find_view_path(store, id) -> Result<Option<(String, PackageSelector)>>` — uses `resolve_definition_owner(id, DefinitionKind::View)` then scans `load_package_json()["views"]` via `load_instance_json`
- [x] Add private `find_document_view_path` — mirrors `find_view_path` for `documentViews`
- [x] Add `list_views_summary(store) -> Result<Vec<ViewSummary>>` — maps `list_views` result
- [x] Add `create_view(store, view: View) -> Result<CreateViewResult>` — validate → assign id if empty → ensure dir → save file → add to boundary → return
- [x] Add `update_view(store, view_id, view) -> Result<UpdateViewResult>` — validate → find path → overwrite → return
- [x] Add `delete_view(store, view_id) -> Result<DeleteViewResult>` — find path → delete file (best-effort) → remove from boundary → return id
- [x] Add `list_document_views_summary`, `create_document_view`, `update_document_view`, `delete_document_view` — mirror View functions; use `DocumentViewNotFoundById` for not-found errors; use `"document-views/{slug}-{prefix}.json"` filenames

#### Error variants to add (`error.rs`)

```rust
#[error("view not found: {view_id}")]
ViewNotFound { view_id: String },

#[error("document view not found: {document_view_id}")]
DocumentViewNotFoundById { document_view_id: String },
```

PartialEq arms:
```rust
(RepositoryError::ViewNotFound { view_id: a }, RepositoryError::ViewNotFound { view_id: b }) => a == b,
(RepositoryError::DocumentViewNotFoundById { document_view_id: a }, RepositoryError::DocumentViewNotFoundById { document_view_id: b }) => a == b,
```

#### Key service function signatures

```rust
pub fn list_views_summary(store: &dyn RepositoryStore) -> Result<Vec<ViewSummary>, RepositoryError>
pub fn list_document_views_summary(store: &dyn RepositoryStore) -> Result<Vec<DocumentViewSummary>, RepositoryError>
pub fn create_view(store: &dyn RepositoryStore, view: View) -> Result<CreateViewResult, RepositoryError>
pub fn update_view(store: &dyn RepositoryStore, view_id: &str, view: View) -> Result<UpdateViewResult, RepositoryError>
pub fn delete_view(store: &dyn RepositoryStore, view_id: &str) -> Result<DeleteViewResult, RepositoryError>
pub fn create_document_view(store: &dyn RepositoryStore, document_view: DocumentView) -> Result<CreateDocumentViewResult, RepositoryError>
pub fn update_document_view(store: &dyn RepositoryStore, document_view_id: &str, document_view: DocumentView) -> Result<UpdateDocumentViewResult, RepositoryError>
pub fn delete_document_view(store: &dyn RepositoryStore, document_view_id: &str) -> Result<DeleteDocumentViewResult, RepositoryError>
```

#### `create_view` orchestration

```rust
pub fn create_view(store: &dyn RepositoryStore, mut view: View) -> Result<CreateViewResult, RepositoryError> {
    validate_view(&view).map_err(|e| RepositoryError::ViewValidation {
        path: std::path::PathBuf::from("package/views"), source: e,
    })?;
    if view.id.is_empty() { view.id = new_instance_id(); }
    store.ensure_views_dir()?;
    let id_prefix = &view.id[..view.id.len().min(8)];
    let filename = format!("views/{}-{}.json", slugify(&view.name), id_prefix);
    store.save_view(&filename, &view)?;
    store.add_definition_to_boundary(&None, DefinitionKind::View, &filename)?;
    Ok(CreateViewResult { view })
}
```

#### `find_view_path` implementation

```rust
fn find_view_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::View) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let pkg_json = store.load_package_json()?;
    let paths = pkg_json["views"].as_array().cloned().unwrap_or_default();
    let prefix = match &owner { None => "package".to_string(), Some(p) => p.clone() };
    for entry in &paths {
        if let Some(rel) = entry.as_str() {
            if let Ok(val) = store.load_instance_json(&format!("{prefix}/{rel}")) {
                if val["id"].as_str() == Some(id) {
                    return Ok(Some((rel.to_string(), owner)));
                }
            }
        }
    }
    Ok(None)
}
```

#### Unit tests in `view_service.rs`

Use `FileStore` with `tempfile::TempDir`. Provide a `setup_minimal_repo(root)` helper that writes `.srs/`, `manifest.json` (`{"instanceIndex":[]}`), and `package/package.json` with empty `fields`, `types`, `relationTypes`, `views`, `documentViews` arrays.

View tests (8):
- `create_view_assigns_id_and_registers_in_package_json`
- `create_view_fails_with_empty_field_views`
- `list_views_summary_returns_created_view`
- `get_view_by_id_finds_created_view`
- `get_view_by_id_not_found_returns_not_found`
- `update_view_overwrites_description`
- `update_view_not_found_returns_view_not_found_error`
- `delete_view_removes_from_package_json`

DocumentView tests (8, mirror with `document_view_` prefix):
- `create_document_view_assigns_id_and_registers_in_package_json`
- `create_document_view_fails_with_empty_sections`
- `list_document_views_summary_returns_created`
- `get_document_view_by_id_finds_created`
- `get_document_view_by_id_not_found_returns_not_found`
- `update_document_view_overwrites_description`
- `update_document_view_not_found_returns_error`
- `delete_document_view_removes_from_package_json`

#### Acceptance Criteria

- [x] `ViewNotFound` and `DocumentViewNotFoundById` exist in `error.rs`; `DocumentViewNotFound` is unchanged
- [x] All service functions in `view_service.rs` take `store: &dyn RepositoryStore` — no `&Path`, no `std::fs` imports
- [x] `create_view` / `create_document_view` call the validator before writing
- [x] `update_view` / `update_document_view` propagate `ViewNotFound` / `DocumentViewNotFoundById`
- [x] All 16 unit tests pass using `FileStore` + temp dir

#### Testing

```bash
cargo test -p srs-repository view_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 16 unit tests exist and pass.
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

- [x] Create `crates/srs-cli/src/commands/view.rs` — dispatch and handlers
- [x] Modify `crates/srs-cli/src/commands/mod.rs` — add `pub mod view;`, `ViewCommand` enum, `Commands::View` variant, dispatch arm
- [x] Add `view_*` integration tests to `crates/srs-cli/tests/integration_tests.rs`

#### `mod.rs` additions

Add `pub mod view;` at top of module list.

Add to `Commands` enum:
```rust
/// View (L1) definition management
#[command(subcommand)]
View(ViewCommand),
```

Add `ViewCommand` enum (near `TagCommand`):
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
    Get { id: String },
    /// Create a new view definition (reads JSON from stdin)
    Create,
    /// Update a view definition (reads full JSON from stdin)
    Update { id: String },
    /// Delete a view definition
    Delete { id: String },
}
```

Add to dispatch match:
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
| `cmd_delete` | `"view delete"` | `{ "id": "..." }` |

List filtering (namespace, type_id) is done in the handler after `list_views_summary()`, not in the service.

**Do NOT** construct `FileStore` directly in handlers — use `with_store`. An existing integration test enforces this.

#### Handler pattern

```rust
fn cmd_view_create(ctx: CliContext) -> Result<String> {
    let view: View = serde_json::from_reader(io::stdin())?;
    match with_store(&ctx, |store| Ok(create_view(store, view)?)) {
        Ok(CreateViewResult { view }) => Ok(output::ok("view create", json!({ "view": view }))),
        Err(e) => Ok(output::err("view create", vec![e.to_string()])),
    }
}
```

#### Integration tests

Helper `create_temp_repo_with_views() -> TempDir` — minimal `.srs/`, `manifest.json`, and `package/package.json` with `views: [], documentViews: []`.

Minimal valid View fixture (no `id` — service assigns it):
```json
{
  "namespace": "com.test",
  "name": "test-view",
  "version": 1,
  "description": "A test view",
  "typeId": "00000000-0000-4000-a000-000000000001",
  "typeVersion": 1,
  "fieldViews": [{ "fieldId": "f1", "order": 0 }],
  "createdAt": "2026-01-01T00:00:00Z"
}
```

Tests:
- `view_list_returns_ok_envelope` — fresh repo; `payload.views` is `[]`
- `view_create_returns_view_with_id` — create; `payload.view.id` is non-empty
- `view_get_returns_created_view` — create then get; name matches
- `view_list_contains_created_view` — create then list; summary present
- `view_update_changes_description` — create, update, get; description updated
- `view_delete_removes_view` — create, delete, list; list empty
- `view_get_not_found_returns_error` — unknown UUID; `ok: false`
- `view_create_fails_validation` — empty `fieldViews`; `ok: false`

#### Acceptance Criteria

- [x] `srs view list` returns `{ "views": [] }` on fresh repo
- [x] `srs view create` reads stdin JSON; returns view with assigned id
- [x] `srs view get <id>` returns the view
- [x] `srs view update <id>` replaces the view; subsequent get shows new content
- [x] `srs view delete <id>` removes the view; subsequent list is empty
- [x] Get/delete on unknown id returns an error payload (not a panic)
- [x] `--namespace` and `--type-id` filters narrow list results

#### Testing

```bash
cargo test -p srs --test integration_tests -- view_
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 8 `view_*` integration tests exist and pass.
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

- [x] Create `crates/srs-cli/src/commands/document_view.rs` — dispatch and handlers
- [x] Modify `crates/srs-cli/src/commands/mod.rs` — add `pub mod document_view;`, `DocumentViewCommand` enum, dispatch arm
- [x] Add `document_view_*` integration tests to `crates/srs-cli/tests/integration_tests.rs`

#### `mod.rs` additions

Add `pub mod document_view;`.

Add to `Commands` enum — **use `#[command(name = "document-view")]`** so the CLI word is `document-view`:
```rust
/// Document view (L2 render view) management
#[command(subcommand, name = "document-view")]
DocumentView(DocumentViewCommand),
```

This does not conflict with `Commands::Render(RenderCommand::DocumentView {...})` — they are at different branches of the command tree.

Add `DocumentViewCommand` enum (same variants as `ViewCommand`, with `container_type` filter instead of `type_id`):
```rust
#[derive(Subcommand)]
pub enum DocumentViewCommand {
    List {
        #[arg(long)] namespace: Option<String>,
        #[arg(long = "container-type")] container_type: Option<String>,
    },
    Get { id: String },
    Create,
    Update { id: String },
    Delete { id: String },
}
```

Add dispatch arm:
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
| `cmd_delete` | `"document-view delete"` | `{ "id": "..." }` |

#### Integration tests

Minimal DocumentView fixture:
```json
{
  "namespace": "com.test",
  "name": "test-doc-view",
  "version": 1,
  "description": "A test document view",
  "sections": [{ "sectionId": "s1", "order": 0, "source": { "type": "fixed-instances", "instanceIds": [] } }],
  "createdAt": "2026-01-01T00:00:00Z"
}
```

Tests (same 8 as view, prefixed `document_view_`), plus:
- `render_document_view_not_broken` — `srs render document-view --view <uuid>` still returns `command: "render document-view"` (regression guard)

#### Acceptance Criteria

- [x] `srs document-view list` returns `{ "documentViews": [] }` on fresh repo
- [x] `srs document-view create` reads stdin JSON; returns documentView with assigned id
- [x] `srs document-view get <id>` returns the document view
- [x] `srs document-view update <id>` replaces; subsequent get shows new content
- [x] `srs document-view delete <id>` removes; subsequent list is empty
- [x] Get/delete on unknown id returns an error payload
- [x] `srs render document-view --view <uuid>` still works (no regression)

#### Testing

```bash
cargo test -p srs --test integration_tests -- document_view_
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all `document_view_*` integration tests exist and pass.
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

- [x] `cargo test` passes with no failures (497 tests)
- [x] `cargo clippy -- -D warnings` passes
- [x] All `view_*` integration tests pass: `cargo test -p srs --test integration_tests -- view_`
- [x] All `document_view_*` integration tests pass: `cargo test -p srs --test integration_tests -- document_view_`
- [x] All `view_service` unit tests pass: `cargo test -p srs-repository view_service`
- [x] `srs render document-view` still works (no regression in render path)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `View` and `DocumentView` structs serialize to camelCase JSON via `serde` — confirmed by inspecting `crates/srs-core/src/types/view.rs`.
- `validate_view()` and `validate_document_view()` are the canonical validators — call them on create and update inside service functions, before delegating to the store.
- `slugify()` is a private helper, not a service concern. Copy the 5-line implementation from `package_service.rs:566`.
- `package.json` `views` and `documentViews` keys may be absent in older repos — `add_definition_to_boundary` will fail if the key is not an array. Ensure test repos always include these keys. In production repos, `ensure_views_dir` should be called before saving (which creates the directory but does not add the key). If needed, a pre-check can be added to `create_view`.
- `MemoryStore::save_view` stores at a key without the `package/` prefix, so `find_view_path` (which calls `load_instance_json("package/views/...")`) will not find entries in MemoryStore. All view service unit tests must use `FileStore` with a temp directory.
- `FileStore::save_view` uses `pkg_abs(relative_path)` which prepends `package/` — so `"views/foo.json"` becomes `package/views/foo.json` on disk. `load_instance_json("package/views/foo.json")` also reads from `package/views/foo.json`. These are consistent for `FileStore`.
