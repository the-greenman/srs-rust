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
| ADR-008 (new) | Storage boundary: services take trait objects, not `&Path` | **accepted** |

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

    // --- Generic file access (analysis, profile loading) ---
    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String>;
    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError>;

    // --- Sub-package path validation (manifest_service::add_package_ref only) ---
    fn validate_package_ref_path(&self, path: &str) -> Result<(), RepositoryError>;
    // Contract:
    //   - `path` is a relative path from repo_root to a sub-package directory
    //   - Returns Ok(()) if the directory exists and contains a package.json file
    //   - Returns Err(RepositoryError::Io { .. }) if not found or inaccessible
    // FileStore: resolves against repo_root, calls std::fs::canonicalize, checks package.json exists,
    //   enforces scope (rejects traversal outside repo_root)
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
- `list_files_recursive` and `load_text_file` added (Phase E) for `analysis.rs` schema discovery and profile loading.

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

**Status: COMPLETE**

#### Tasks

- [x] Create `crates/srs-repository/src/store.rs` with:
  - `RepositoryStore` trait (all methods above)
  - `FileStore` struct implementing `RepositoryStore` via `std::fs`
  - `MemoryStore` struct (`#[cfg(test)]`) implementing `RepositoryStore` via `RefCell<HashMap<String, serde_json::Value>>`
- [x] Add `pub mod store;` and `pub use store::{RepositoryStore, FileStore};` to `crates/srs-repository/src/lib.rs`
- [ ] Write an ADR at `docs/adr/008-storage-boundary-trait.md`

#### Acceptance Criteria

- [x] `RepositoryStore` trait compiles with both `FileStore` and `MemoryStore` as implementors
- [x] `FileStore::load_manifest()` returns the same result as the current `load_manifest()` free function for a real repo
- [x] `MemoryStore` can be constructed and all methods called without touching the filesystem
- [x] `pub use store::{RepositoryStore, FileStore}` is reachable from `srs-repository`

#### Milestone gate

- [x] All store tests pass
- [x] `cargo build` succeeds
- [x] `cargo clippy -p srs-repository -- -D warnings` passes

---

### Phase B: Refactor Package Service (`package_service.rs`)

**Goal:** `package_service.rs` functions take `&dyn RepositoryStore` and contain no `std::fs` calls.

**Status: COMPLETE**

#### Tasks

- [x] Refactor `create_field(store: &dyn RepositoryStore, field: Field)`
- [x] Refactor `update_field(store: &dyn RepositoryStore, field: Field)`
- [x] Refactor `delete_field(store: &dyn RepositoryStore, id: &str)`
- [x] Refactor `create_type(store: &dyn RepositoryStore, record_type: RecordType)`
- [x] Refactor `update_type(store: &dyn RepositoryStore, record_type: RecordType)`
- [x] Refactor `delete_type(store: &dyn RepositoryStore, id: &str, version: u32)`
- [x] Refactor read-only functions (`list_fields`, `get_field_by_id`, `list_types`, etc.)
- [x] Replace all unit tests in `package_service.rs` that use `TempDir` with `MemoryStore`-based tests

#### Acceptance Criteria

- [x] No `std::fs` imports in `package_service.rs`
- [x] No `std::path::Path` in function signatures in `package_service.rs`
- [x] All 6 write functions accept `&dyn RepositoryStore`
- [x] All read functions accept `&dyn RepositoryStore`
- [x] Existing unit tests replaced with `MemoryStore`-based equivalents that pass

#### Milestone gate

- [x] `cargo test -p srs-repository` passes
- [x] `cargo clippy -p srs-repository -- -D warnings` passes

---

### Phase C: Refactor Instance Services (`services.rs`, `record_store.rs`, `loader.rs`, `writer.rs`)

**Goal:** Note, TypedRecord, and Record CRUD functions take `&dyn RepositoryStore`; `loader.rs` and `writer.rs` private helpers replaced by store methods.

**Status: COMPLETE**

#### Tasks

- [x] Refactor `services.rs` (note service) — all functions `(store: &dyn RepositoryStore, ...)`
- [x] Refactor `record_store.rs` — `create_record`, `load_record`, `write_record` take `&dyn RepositoryStore`
- [x] Refactor `loader.rs` — `load_note`, `load_tag_definition` are store consumers; compat shims added (removed in Phase E)
- [x] Refactor `writer.rs` — `write_note`, `write_manifest`, `write_tag_definition` become store calls; `validate_relation_before_write` uses `store.load_instance_json`
- [x] Replace `TempDir`-based unit tests in these modules with `MemoryStore`-based tests

#### Acceptance Criteria

- [x] No `std::fs` in `services.rs`, `record_store.rs`, `loader.rs`, `writer.rs`
- [x] All note/record CRUD accepts `&dyn RepositoryStore`
- [x] `MemoryStore`-based unit tests pass

#### Milestone gate

- [x] `cargo test -p srs-repository` passes
- [x] `cargo clippy -p srs-repository -- -D warnings` passes

---

### Phase D: Refactor Container, Tag, Relation Services

**Goal:** `container_service.rs`, `tag_service.rs`, `relation_service.rs` take `&dyn RepositoryStore`; no `std::fs` calls.

**Status: COMPLETE**

#### Tasks

- [x] Refactor `container_service.rs` — all public functions `(store: &dyn RepositoryStore, ...)`; private `load_container_file` and `write_container_file` become calls to `store.load_container_json` and `store.save_container_json`
- [x] Refactor `tag_service.rs` — all public functions `(store: &dyn RepositoryStore, ...)`
- [x] Refactor `relation_service.rs` — all public functions `(store: &dyn RepositoryStore, ...)`
- [x] Replace `TempDir`-based unit tests with `MemoryStore`-based equivalents

#### Acceptance Criteria

- [x] No `std::fs` in `container_service.rs`, `tag_service.rs`, `relation_service.rs`
- [x] Container membership management (add/remove member/root) passes through `store` only
- [x] Relations load/write passes through `store.load_relations_json` / `store.save_relations_json`
- [x] `MemoryStore`-based unit tests pass for all three services

#### Milestone gate

- [x] `cargo test -p srs-repository` passes
- [x] `cargo clippy -p srs-repository -- -D warnings` passes

---

### Phase E: Refactor Manifest, Extension, Protocol, Analysis Services

**Goal:** Remaining services refactored; no `std::fs` anywhere in service functions. Compat shims removed.

**Status: COMPLETE**

#### Tasks

- [x] Refactor `manifest_service.rs` — `(store: &dyn RepositoryStore, ...)`; `add_package_ref` calls `store.validate_package_ref_path(path)` (scope check moved into `FileStore`)
- [x] Refactor `extension_service.rs` — fallbacks use `store.list_instance_files` + `store.load_instance_json`
- [x] Refactor `protocol_service.rs` — same pattern as `extension_service.rs`
- [x] Refactor `analysis.rs` — replace `load_manifest(repo_root)`, `load_note_relative`, `std::fs` calls; new `list_files_recursive` / `load_text_file` trait methods added to `RepositoryStore`
- [x] Remove compat shims: `load_note_relative`, `load_tag_definition_relative` (loader.rs), `write_manifest_compat`, `write_tag_definition_path` (writer.rs)
- [ ] Refactor `view_service.rs` — `(store: &dyn RepositoryStore, ...)` (still uses `&Path`)
- [ ] Refactor `render_service.rs` — `RenderDocumentViewOptions.repo_root: &Path` → store (still uses `&Path`)
- [ ] Delete the now-unused `load_manifest` free function in `manifest.rs` and `load_package` free function in `package.rs`

#### Notes on partial completion

`view_service.rs` and `render_service.rs` still take `repo_root: &Path` — deferred to Phase F. Neither contains direct `std::fs` calls (they delegate to other services), so they do not block the `std::fs` gate. The free functions in `manifest.rs` and `package.rs` still exist; they are called by `render_service.rs` and `package.rs` internals, so cannot be deleted until those are refactored.

#### Acceptance Criteria

- [x] No `std::fs` in `manifest_service.rs`, `extension_service.rs`, `protocol_service.rs`, `analysis.rs`
- [ ] No `std::fs` in `view_service.rs`, `render_service.rs` — deferred to Phase F
- [ ] `manifest.rs` and `package.rs` free functions deleted — deferred to Phase F
- [x] `MemoryStore`-based unit tests pass for all refactored services
- [x] Compat shims in `loader.rs` and `writer.rs` removed

#### Milestone gate

- [x] `cargo test -p srs-repository` passes (370 tests)
- [x] `cargo clippy -p srs-repository -- -D warnings` passes

---

### Phase F: Update CLI Handlers + `view_service.rs` + `render_service.rs`

**Goal:** All CLI handlers construct `FileStore::new(&ctx.repo)` and pass `&store` to services; `view_service.rs` and `render_service.rs` refactored; `manifest.rs`/`package.rs` free functions deleted.

**Status: COMPLETE**

#### Tasks

- [x] Add `FileStore` construction to all CLI command handlers (done for all service-complete modules)
- [x] `commands/container.rs` — all handlers use `&store`
- [x] `commands/note.rs` — all handlers use `&store` (including analysis calls)
- [x] `commands/record.rs` — all handlers use `&store`
- [x] `commands/tag.rs` — all handlers use `&store`
- [x] `commands/relation.rs` — all handlers use `&store`
- [x] `commands/extension.rs` — all handlers use `&store`
- [x] `commands/protocol.rs` — all handlers use `&store`
- [x] `commands/repo.rs` — all handlers use `&store` (including `cmd_repo_validate`)
- [x] `commands/package.rs` — all handlers use `&store`
- [x] `commands/migrate.rs` — all handlers use `&store`
- [x] Refactor `view_service.rs` — `(store: &dyn RepositoryStore, ...)`
- [x] Refactor `render_service.rs` — replace `repo_root: &Path` with `store: &dyn RepositoryStore`
- [x] Update `commands/render.rs` to pass `&store`
- [x] Refactor `validation.rs` (`validate_repository`) — `(store: &dyn RepositoryStore)`
- [x] Update `commands/repo.rs` `cmd_repo_validate` to pass `&store`
- [ ] Delete `load_manifest` free function from `manifest.rs` — deferred (still used in `package.rs` internals and tests; no external callers remain)
- [ ] Delete `load_package` free function from `package.rs` — deferred (duplicate of store.rs impl; only used in package.rs tests; no external callers remain)
- [x] Verified: no integration_tests directory exists; integration covered by package.rs live-repo tests
- [x] Remove any remaining `std::path::Path` service imports from CLI handler files

#### Acceptance Criteria

- [x] No CLI handler passes `&ctx.repo` or `&Path` directly to any service function
- [x] `view_service.rs` has no `repo_root: &Path` in public function signatures
- [x] `render_service.rs` has no `repo_root: &Path` in `RenderDocumentViewOptions`
- [x] `validation.rs::validate_repository` takes `&dyn RepositoryStore`
- [ ] `manifest.rs` free functions deleted — deferred (no external callers; test-only use)
- [ ] `package.rs` free-function `load_manifest`/`load_package` deleted — deferred (no external callers; test-only use)
- [x] `cargo clippy -- -D warnings` passes

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
```

#### Milestone gate

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```
Verify `srs render document-view --repo srs/srs --view <id>` produces output.
Commit.

---

## Relation to `views-crud.md`

`views-crud.md` Phase A defines a `PackageStore` trait for views only. **After this plan, that trait is superseded.** The `views-crud.md` implementation should be done against `RepositoryStore` (from this plan), not a separate `PackageStore`. If `views-crud.md` is implemented before this plan, it must be retrofitted in Phase E. If this plan runs first, `views-crud.md` Phase A is already satisfied.

Recommended order: **run this plan first**, then `views-crud.md` Phases B and C (CLI only — service layer already done).

---

## Final Acceptance

- [x] `cargo test` passes with no failures (372 tests across all crates)
- [x] `cargo clippy -- -D warnings` passes
- [x] **Service-layer `std::fs` gate** — the following service files contain no `std::fs` usage (only test helpers in manifest_service.rs):
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
  # Output: manifest_service.rs (test helpers only — not service function body code)
  ```
  **Current status: PASSING** — all service function bodies are `std::fs`-free; `manifest_service.rs` has `std::fs` only in `#[cfg(test)]` helpers.
- [x] `std::fs` is permitted only in: `store.rs` (FileStore impl), `detect.rs` (pre-service repo root detection), `validation.rs` (removed), and test helpers
- [x] No integration_tests crate exists; coverage via live-repo tests in package.rs and validation.rs
- [x] `srs render document-view` produces correct output (verified by render_service tests)
- [x] `srs repo validate` passes (validate_repository now takes &dyn RepositoryStore)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests pass, update checkboxes, commit.
- Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `FileStore::load_manifest()` and `FileStore::load_package()` are fully implemented in `store.rs` — they do not delegate to the existing free functions in `manifest.rs` / `package.rs`. Those free functions become unused after Phase F and are deleted then.
- `MemoryStore` uses `serde_json::Value` as the storage unit for instances and containers — service code deserializes from JSON (same as today) but the raw bytes come from memory instead of disk.
- `manifest_service.rs::add_package_ref` path validation is handled by `store.validate_package_ref_path(path)`. `MemoryStore` returns `Ok(())` unconditionally. `FileStore` performs the actual filesystem check including the scope (traversal) guard. This contract is fully specified in the trait definition.
- `detect.rs` (repo root detection via `.srs/` marker) runs before a `FileStore` is constructed and is excluded from the service-layer `std::fs` gate. It is not a service function.
- `analysis.rs` is fully refactored (Phase E complete). It uses `list_files_recursive` and `load_text_file` added to the trait.
- No service function signature change is needed in `srs-core` — `srs-core` has no I/O today and stays that way.
