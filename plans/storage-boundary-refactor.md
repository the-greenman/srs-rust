# Plan: Storage Boundary Refactor

## Summary

Every service function in `srs-repository` currently takes `repo_root: &Path` and calls `std::fs` directly. This makes the storage backend unswappable — adding SQLite requires rewriting every service. ARCHITECTURE.md states: "keep storage boundaries visible so a database-backed implementation can be introduced later." This plan introduces a storage trait layer between service logic and I/O, so that a future `SqliteStore` can satisfy the same traits without touching service code. The existing `agentic-storage-refactor-plan.md` Phase 3 described this intent; this plan is the concrete execution.

**Scope constraint:** This plan does NOT implement SQLite. It introduces the trait boundaries and refactors existing services through them. A future plan adds a SQLite implementation.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Storage Worker | — |
| Service Refactor Worker | — |
| CLI Wiring Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-008 (new) | Storage boundary: services take trait objects, not `&Path` | proposed |

Key decisions:

- **One `RepositoryStore` trait** (not 6-7 fine-grained traits). Services take `&dyn RepositoryStore`. A single trait is simpler to implement for SQLite; fine-grained decomposition can happen later if there is real pressure.
- **`FileStore`** is the concrete file-backed implementation. It holds `repo_root: PathBuf`. All `std::fs` calls move here and out of service functions.
- **`MemoryStore`** is the in-memory test fake. Lives in `#[cfg(test)]` in the same module. Eliminates `TempDir` from unit tests.
- **Services keep the same public function names** but signatures change from `(repo_root: &Path, ...)` to `(store: &dyn RepositoryStore, ...)`. This is a breaking change to callers — all CLI handlers are updated in the same plan.
- **CLI constructs `FileStore::new(&ctx.repo)`** and passes `&store` to services. The CLI itself does no I/O.
- **`load_manifest` and `load_package` become trait methods**, not free functions. The `Manifest` and `Package` types remain in `srs-core`/`srs-repository`.
- **No async.** Synchronous traits only, consistent with the existing codebase and ARCHITECTURE.md.

---

## Violation Summary

The audit found **40+ direct `std::fs` calls** across 11 files in `srs-repository`. Every service function takes `repo_root: &Path`. Full breakdown:

| Module | Violating functions | Operations |
|---|---|---|
| `package_service.rs` | `create_field`, `update_field`, `delete_field`, `create_type`, `update_type`, `delete_type` | read_to_string, write, create_dir_all, remove_file |
| `container_service.rs` | `load_container_file`, `write_container_file`, `delete_container` (private helpers) | read_to_string, create_dir_all, write, remove_file |
| `relation_service.rs` | `load_relations_collection`, `write_relations_collection` | read_to_string, create_dir_all, write |
| `tag_service.rs` | `create_tag_definition`, `delete_tag_definition` | create_dir_all, remove_file |
| `record_store.rs` | `create_record`, `load_record`, `write_record` | create_dir_all, read_to_string, write |
| `writer.rs` | `write_note`, `write_manifest`, `write_tag_definition`, `validate_relation_before_write` | write, read_to_string |
| `loader.rs` | `load_note`, `load_tag_definition` | read_to_string |
| `manifest.rs` | `load_manifest` | read_to_string |
| `extension_service.rs` | `list_records_by_type_fallback`, `get_record_by_id_fallback` | read_dir, read_to_string |
| `protocol_service.rs` | `list_records_by_type_fallback`, `get_record_by_id_fallback` | read_dir, read_to_string |
| `manifest_service.rs` | `add_package_ref` | canonicalize, exists |

`render_service.rs` does not call `std::fs` directly — it composes other services. It still needs its `repo_root: &Path` parameter replaced by `&dyn RepositoryStore`, but the change there is mechanical.

---

## Scope

**In scope:**
- New `crates/srs-repository/src/store.rs` — `RepositoryStore` trait + `FileStore` + `MemoryStore` (cfg test)
- Refactor all service functions to take `&dyn RepositoryStore` instead of `&Path`
- Remove all `std::fs` calls from service functions (move to `FileStore`)
- Update all CLI handlers to construct `FileStore::new(&ctx.repo)` and pass `&store`
- Update `render_service.rs` parameter to `&dyn RepositoryStore`
- Unit tests for each service module use `MemoryStore` only — no `TempDir`
- CLI integration tests continue using real file repos via `FileStore`

**Out of scope:**
- SQLite implementation (future plan)
- Async traits (explicitly deferred per ARCHITECTURE.md)
- `srs-file-store` crate extraction (deferred until a second adapter justifies the split)
- `srs-bindings` (Phase 5 of `agentic-storage-refactor-plan.md`, separate plan)
- Changing `srs-core` types

---

## `RepositoryStore` Trait Design

Lives in `crates/srs-repository/src/store.rs`. Covers all data categories identified in the audit:

```rust
pub trait RepositoryStore {
    // --- Manifest ---
    fn load_manifest(&self) -> Result<Manifest, RepositoryError>;
    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError>;

    // --- Package definitions (read) ---
    fn load_package(&self) -> Result<Package, RepositoryError>;

    // --- Fields ---
    fn save_field(&self, field: &Field, relative_path: &str) -> Result<(), RepositoryError>;
    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError>;
    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_fields_dir(&self) -> Result<(), RepositoryError>;

    // --- Types ---
    fn save_type(&self, record_type: &RecordType, relative_path: &str) -> Result<(), RepositoryError>;
    fn update_type_file(&self, relative_path: &str, record_type: &RecordType) -> Result<(), RepositoryError>;
    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_types_dir(&self) -> Result<(), RepositoryError>;

    // --- Views (L1) ---
    fn save_view(&self, view: &View, relative_path: &str) -> Result<(), RepositoryError>;
    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_views_dir(&self) -> Result<(), RepositoryError>;

    // --- Document Views (L2) ---
    fn save_document_view(&self, view: &DocumentView, relative_path: &str) -> Result<(), RepositoryError>;
    fn update_document_view_file(&self, relative_path: &str, view: &DocumentView) -> Result<(), RepositoryError>;
    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_document_views_dir(&self) -> Result<(), RepositoryError>;

    // --- Instances (Notes, TypedRecords, Records) ---
    fn load_instance_json(&self, relative_path: &str) -> Result<serde_json::Value, RepositoryError>;
    fn save_instance_json(&self, relative_path: &str, value: &serde_json::Value) -> Result<(), RepositoryError>;
    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_instance_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError>;

    // --- Relations ---
    fn load_relations_json(&self, relative_path: &str) -> Result<serde_json::Value, RepositoryError>;
    fn save_relations_json(&self, relative_path: &str, value: &serde_json::Value) -> Result<(), RepositoryError>;
    fn ensure_relations_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Containers ---
    fn load_container_json(&self, relative_path: &str) -> Result<serde_json::Value, RepositoryError>;
    fn save_container_json(&self, relative_path: &str, value: &serde_json::Value) -> Result<(), RepositoryError>;
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError>;

    // --- Package index (package.json) ---
    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError>;
    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError>;

    // --- Sub-package path validation (manifest_service::add_package_ref only) ---
    fn validate_package_ref_path(&self, path: &str) -> Result<(), RepositoryError>;
    // Contract:
    //   - `path` is a relative path from repo_root to a sub-package directory
    //   - Returns Ok(()) if the directory exists and contains a package.json file
    //   - Returns Err(RepositoryError::Io { .. }) if not found or inaccessible
    // FileStore: resolves against repo_root, calls std::fs::canonicalize, checks package.json exists
    // MemoryStore: returns Ok(()) unconditionally (path existence is not meaningful in memory)
    // This method is the only place path-existence validation for sub-packages is permitted
    // to use filesystem operations, and it is inside FileStore, not inside manifest_service.rs
}
```

**Design notes:**
- Methods operate on **relative paths** (e.g. `"fields/my-field-abc12345.json"`), not absolute paths. `FileStore` resolves them against `repo_root`. `MemoryStore` uses them as keys in a `HashMap<String, serde_json::Value>`.
- `load_package()` is a trait method so `MemoryStore` can return a pre-configured `Package` without reading files.
- JSON-level methods (`load_instance_json`, `save_instance_json`) keep deserialization in service functions where error context is meaningful, not in the store.
- `ensure_*_dir()` methods are no-ops in `MemoryStore`.

---

## Data Flow After Refactor

```
CLI handler
  → constructs FileStore::new(&ctx.repo)
  → calls service_function(&store, args...)

service_function(&dyn RepositoryStore, ...)
  → store.load_manifest()          // no path, no std::fs
  → store.load_package()
  → store.save_instance_json(...)
  → validate(...)                  // pure, no I/O

FileStore::save_instance_json(relative_path, value)
  → std::fs::write(self.repo_root.join(relative_path), ...)

MemoryStore::save_instance_json(relative_path, value)
  → self.data.borrow_mut().insert(relative_path.to_string(), value.clone())
```

---

## Phases

### Phase A: Define `RepositoryStore` Trait + `FileStore` + `MemoryStore`

**Goal:** The trait, file-backed implementation, and in-memory test fake exist and compile; no service functions changed yet.

**Agent:** Core Storage Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/store.rs` with:
  - `RepositoryStore` trait (all methods above)
  - `FileStore` struct implementing `RepositoryStore` via `std::fs`
  - `MemoryStore` struct (`#[cfg(test)]`) implementing `RepositoryStore` via `RefCell<HashMap<String, serde_json::Value>>`
- [ ] Add `pub mod store;` and `pub use store::{RepositoryStore, FileStore};` to `crates/srs-repository/src/lib.rs`
- [ ] Write an ADR at `docs/adr/008-storage-boundary-trait.md`

#### `FileStore` implementation notes

`FileStore` holds `repo_root: PathBuf`. Each method resolves relative paths: `self.repo_root.join(relative_path)`. Errors map to existing `RepositoryError` variants (`Io`, `ManifestMissing`, etc.).

`FileStore::load_manifest()` and `FileStore::load_package()` must be **fully implemented in `store.rs`** — they must not call the existing free functions in `manifest.rs` and `package.rs`. Those free functions call `std::fs` internally, and delegating to them would mean `store.rs` is not the only place with `std::fs` calls. The free functions in `manifest.rs` and `package.rs` become dead code once `FileStore` is complete; they may be removed at the end of Phase A or left for a follow-up cleanup, but they must not be called by `FileStore`.

**Migration path for `manifest.rs` / `package.rs` free functions**: Before Phase A, these are called by service code. After Phase A, `FileStore` provides the same functionality without calling them. Phases B–E then eliminate service-layer calls to those free functions. By end of Phase E, the free functions have no callers and can be deleted.

`FileStore::validate_package_ref_path(path: &str) -> Result<(), RepositoryError>` (see trait method below) uses `std::fs::canonicalize` and `.exists()`. This is the only place path-existence validation for sub-packages occurs.

#### `MemoryStore` implementation notes

```rust
#[cfg(test)]
pub struct MemoryStore {
    data: RefCell<HashMap<String, serde_json::Value>>,
    manifest: RefCell<Manifest>,
    package: RefCell<Package>,
}

impl MemoryStore {
    pub fn new(manifest: Manifest, package: Package) -> Self { ... }
    pub fn with_instance(mut self, path: &str, value: serde_json::Value) -> Self { ... }
}
```

`load_manifest()` returns a clone of `self.manifest`. `load_package()` returns a clone of `self.package`. `load_instance_json(path)` looks up `self.data`. All `ensure_*_dir()` methods are no-ops returning `Ok(())`.

#### Acceptance Criteria

- [ ] `RepositoryStore` trait compiles with both `FileStore` and `MemoryStore` as implementors
- [ ] `FileStore::load_manifest()` returns the same result as the current `load_manifest()` free function for a real repo
- [ ] `MemoryStore` can be constructed and all methods called without touching the filesystem
- [ ] `pub use store::{RepositoryStore, FileStore}` is reachable from `srs-repository`

#### Testing

```bash
cargo test -p srs-repository store
cargo clippy -p srs-repository -- -D warnings
```

Tests (in `store.rs`):
- `file_store_load_manifest_roundtrips` — write a minimal manifest.json to TempDir; FileStore returns it
- `memory_store_load_manifest_returns_configured` — MemoryStore returns the manifest it was built with
- `memory_store_save_and_load_instance_json` — save then load → same value
- `memory_store_delete_instance_removes_key` — save, delete, load → error

#### Milestone gate

1. All 4 store tests pass.
2. `cargo build` succeeds.
3. Run:
```bash
cargo clippy -p srs-repository -- -D warnings
```
4. Update checkboxes. Commit.

---

### Phase B: Refactor Package Service (`package_service.rs`)

**Goal:** `package_service.rs` functions take `&dyn RepositoryStore` and contain no `std::fs` calls.

**Agent:** Service Refactor Worker

#### Tasks

- [ ] Refactor `create_field(store: &dyn RepositoryStore, field: Field)`
- [ ] Refactor `update_field(store: &dyn RepositoryStore, field: Field)`
- [ ] Refactor `delete_field(store: &dyn RepositoryStore, id: &str)`
- [ ] Refactor `create_type(store: &dyn RepositoryStore, record_type: RecordType)`
- [ ] Refactor `update_type(store: &dyn RepositoryStore, record_type: RecordType)`
- [ ] Refactor `delete_type(store: &dyn RepositoryStore, id: &str, version: u32)`
- [ ] Refactor read-only functions (`list_fields`, `get_field_by_id`, `list_types`, etc.) — these call `load_package(repo_root)` which becomes `store.load_package()`
- [ ] Replace all unit tests in `package_service.rs` that use `TempDir` with `MemoryStore`-based tests

#### Signature changes (all functions)

Before: `fn create_field(repo_root: &Path, field: Field)`
After:  `fn create_field(store: &dyn RepositoryStore, field: Field)`

The function body:
- Calls `store.load_package_json()` instead of `std::fs::read_to_string(&package_json_path)`
- Calls `store.ensure_fields_dir()` instead of `std::fs::create_dir_all(...)`
- Calls `store.save_field(&field, &filename)` instead of `std::fs::write(&field_path, ...)`
- Calls `store.save_package_json(&updated)` instead of `std::fs::write(&package_json_path, ...)`
- No `std::fs` imports remain in this file

#### Acceptance Criteria

- [ ] No `std::fs` imports in `package_service.rs`
- [ ] No `std::path::Path` in function signatures in `package_service.rs`
- [ ] All 6 write functions accept `&dyn RepositoryStore`
- [ ] All read functions accept `&dyn RepositoryStore`
- [ ] Existing unit tests replaced with `MemoryStore`-based equivalents that pass

#### Testing

```bash
cargo test -p srs-repository package_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
Commit.

---

### Phase C: Refactor Instance Services (`services.rs`, `record_store.rs`, `loader.rs`, `writer.rs`)

**Goal:** Note, TypedRecord, and Record CRUD functions take `&dyn RepositoryStore`; `loader.rs` and `writer.rs` private helpers replaced by store methods.

**Agent:** Service Refactor Worker

#### Tasks

- [ ] Refactor `services.rs` (note service) — all functions `(store: &dyn RepositoryStore, ...)`
- [ ] Refactor `record_store.rs` — `create_record`, `load_record`, `write_record` take `&dyn RepositoryStore`
- [ ] Refactor `loader.rs` — `load_note`, `load_tag_definition` become internal store consumers or are inlined
- [ ] Refactor `writer.rs` — `write_note`, `write_manifest`, `write_tag_definition` become store calls; `validate_relation_before_write` uses `store.load_instance_json`
- [ ] Replace `TempDir`-based unit tests in these modules with `MemoryStore`-based tests

#### Note service pattern

```rust
// Before:
pub fn create_note(repo_root: &Path, note: Note) -> Result<CreateNoteResult, RepositoryError>

// After:
pub fn create_note(store: &dyn RepositoryStore, note: Note) -> Result<CreateNoteResult, RepositoryError>
// body calls: store.load_manifest(), store.save_instance_json(...), store.save_manifest(...)
```

#### Acceptance Criteria

- [ ] No `std::fs` in `services.rs`, `record_store.rs`, `loader.rs`, `writer.rs`
- [ ] All note/record CRUD accepts `&dyn RepositoryStore`
- [ ] `MemoryStore`-based unit tests pass

#### Testing

```bash
cargo test -p srs-repository services
cargo test -p srs-repository record_store
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
Commit.

---

### Phase D: Refactor Container, Tag, Relation Services

**Goal:** `container_service.rs`, `tag_service.rs`, `relation_service.rs` take `&dyn RepositoryStore`; no `std::fs` calls.

**Agent:** Service Refactor Worker

#### Tasks

- [ ] Refactor `container_service.rs` — all public functions `(store: &dyn RepositoryStore, ...)`; private `load_container_file` and `write_container_file` become calls to `store.load_container_json` and `store.save_container_json`
- [ ] Refactor `tag_service.rs` — all public functions `(store: &dyn RepositoryStore, ...)`
- [ ] Refactor `relation_service.rs` — all public functions `(store: &dyn RepositoryStore, ...)`
- [ ] Replace `TempDir`-based unit tests with `MemoryStore`-based equivalents

#### Acceptance Criteria

- [ ] No `std::fs` in `container_service.rs`, `tag_service.rs`, `relation_service.rs`
- [ ] Container membership management (add/remove member/root) passes through `store` only
- [ ] Relations load/write passes through `store.load_relations_json` / `store.save_relations_json`
- [ ] `MemoryStore`-based unit tests pass for all three services

#### Testing

```bash
cargo test -p srs-repository container_service
cargo test -p srs-repository tag_service
cargo test -p srs-repository relation_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
Commit.

---

### Phase E: Refactor Manifest, Extension, Protocol Services + `render_service.rs`

**Goal:** Remaining services refactored; `render_service.rs` takes `&dyn RepositoryStore`; no `std::fs` anywhere in service functions.

**Agent:** Service Refactor Worker

#### Tasks

- [ ] Refactor `manifest_service.rs` — `(store: &dyn RepositoryStore, ...)`; `add_package_ref` calls `store.validate_package_ref_path(path)` instead of calling `std::fs` directly
- [ ] Refactor `extension_service.rs` — `list_records_by_type_fallback` uses `store.list_instance_files` + `store.load_instance_json`
- [ ] Refactor `protocol_service.rs` — same pattern as `extension_service.rs`
- [ ] Refactor `view_service.rs` — `(store: &dyn RepositoryStore, ...)` (supersedes views-crud.md Phase A)
- [ ] Refactor `render_service.rs` — `RenderDocumentViewOptions.repo_root: &Path` → `RenderDocumentViewOptions.store: &dyn RepositoryStore`
- [ ] Refactor `analysis.rs` — replace `load_manifest(repo_root)` and other free-function calls with `store.load_manifest()` and `store.load_package()`; signature: `(store: &dyn RepositoryStore, ...)`
- [ ] Delete the now-unused `load_manifest` free function in `manifest.rs` and `load_package` free function in `package.rs` (all callers replaced by Phase E)

#### Acceptance Criteria

- [ ] No `std::fs` in `manifest_service.rs`, `extension_service.rs`, `protocol_service.rs`, `view_service.rs`, `render_service.rs`, `analysis.rs`
- [ ] `manifest.rs` and `package.rs` free functions deleted (or marked `#[deprecated]` if external crates reference them — check `srs-cli` first)
- [ ] `srs render document-view` still works end-to-end

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
Verify `srs render document-view --repo srs/srs --view <id>` produces output.
Commit.

---

### Phase F: Update CLI Handlers + Integration Tests

**Goal:** All CLI handlers construct `FileStore::new(&ctx.repo)` and pass `&store` to services; integration tests pass.

**Agent:** CLI Wiring Worker

#### Tasks

- [ ] Add `FileStore` construction to `CliContext` or as a per-handler local: `let store = FileStore::new(&ctx.repo);`
- [ ] Update every handler in `crates/srs-cli/src/commands/` to pass `&store` instead of `&ctx.repo`
- [ ] Verify `cargo test --test integration_tests` still passes
- [ ] Remove any `std::path::Path` service imports from CLI handler files

#### Option: store on `CliContext`

```rust
pub struct CliContext {
    pub repo: PathBuf,
    pub container_id: Option<String>,
    pub store: FileStore,   // constructed once in dispatch()
}
```

This avoids constructing `FileStore` in every handler. `FileStore::new` is cheap (just stores a `PathBuf`).

#### Acceptance Criteria

- [ ] No CLI handler passes `&ctx.repo` or `&Path` to any service function
- [ ] All CLI integration tests pass unchanged
- [ ] `cargo clippy -- -D warnings` passes

#### Testing

```bash
cargo test --test integration_tests
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```
Commit.

---

## Relation to `views-crud.md`

`views-crud.md` Phase A defines a `PackageStore` trait for views only. **After this plan, that trait is superseded.** The `views-crud.md` implementation should be done against `RepositoryStore` (from this plan), not a separate `PackageStore`. If `views-crud.md` is implemented before this plan, it must be retrofitted in Phase E. If this plan runs first, `views-crud.md` Phase A is already satisfied.

Recommended order: **run this plan first**, then `views-crud.md` Phases B and C (CLI only — service layer already done).

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] **Service-layer `std::fs` gate** — the following service files contain no `std::fs` usage:
  ```bash
  grep -l "std::fs" \
    crates/srs-repository/src/package_service.rs \
    crates/srs-repository/src/container_service.rs \
    crates/srs-repository/src/tag_service.rs \
    crates/srs-repository/src/relation_service.rs \
    crates/srs-repository/src/services.rs \
    crates/srs-repository/src/record_store.rs \
    crates/srs-repository/src/loader.rs \
    crates/srs-repository/src/writer.rs \
    crates/srs-repository/src/manifest_service.rs \
    crates/srs-repository/src/extension_service.rs \
    crates/srs-repository/src/protocol_service.rs \
    crates/srs-repository/src/view_service.rs \
    crates/srs-repository/src/render_service.rs \
    crates/srs-repository/src/analysis.rs
  # Expected output: empty (no files listed)
  ```
- [ ] `std::fs` is permitted only in: `store.rs` (FileStore impl), `detect.rs` (pre-service repo root detection), and test helpers
- [ ] All integration tests pass: `cargo test --test integration_tests`
- [ ] `srs render document-view` produces correct output on a real repo
- [ ] `srs repo validate` passes on `srs/srs/`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests pass, update checkboxes, commit.
- Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `FileStore::load_manifest()` and `FileStore::load_package()` are fully implemented in `store.rs` — they do not delegate to the existing free functions in `manifest.rs` / `package.rs`. Those free functions become unused after Phase E and are deleted then.
- `MemoryStore` uses `serde_json::Value` as the storage unit for instances and containers — service code deserializes from JSON (same as today) but the raw bytes come from memory instead of disk.
- `manifest_service.rs::add_package_ref` path validation is handled by `store.validate_package_ref_path(path)`. `MemoryStore` returns `Ok(())` unconditionally. `FileStore` performs the actual filesystem check. This contract is fully specified in the trait definition — implementing agents must not deviate from it.
- `detect.rs` (repo root detection via `.srs/` marker) runs before a `FileStore` is constructed and is excluded from the service-layer `std::fs` gate. It is not a service function.
- `analysis.rs` is a service-layer module and is included in the Phase E task list and the final acceptance grep gate. It must be refactored to accept `&dyn RepositoryStore`.
- No service function signature change is needed in `srs-core` — `srs-core` has no I/O today and stays that way.
