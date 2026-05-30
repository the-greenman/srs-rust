# Plan: JsonStore — Single-File Repository Store

## Summary

`FileStore` and `MemoryStore` are the only `RepositoryStore` implementations. A `JsonStore` backed by a single `.srsj` JSON file would make repositories portable (one file to copy/email/commit), useful for tooling and offline scenarios, and a clean test of the storage boundary: if the trait truly abstracts all I/O, a third implementation should require no service-layer changes. The repository lifecycle and portability services are already implemented and ready to drive a new store adapter.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| JsonStore Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs needed — this plan implements the storage-boundary trait established in the now-completed `storage-boundary-refactor` plan. All architectural constraints already apply: services use `&dyn RepositoryStore`, adapters own I/O, `std::fs` is confined to store implementations.

---

## Scope

- New `JsonStore` struct implementing `RepositoryStore` in `crates/srs-repository/src/json_store.rs`
- Single `.srsj` file on disk; full repository state in one JSON envelope
- Public export from `srs-repository` crate root
- Unit tests in `json_store.rs` covering lifecycle, flush correctness, and portability integration
- `MemoryStore` stays `#[cfg(test)]` — no changes to it

**Out of scope:**
- CLI commands using `JsonStore` (a future plan can add `--store json` or similar)
- Lazy/batched flush optimisation
- Encryption or compression of the `.srsj` file
- SQL adapter

---

## Phases

### Phase 1: JsonStore Implementation

**Goal:** `JsonStore` implements `RepositoryStore`, compiles, and all unit tests pass.

**Agent:** JsonStore Worker

#### Tasks

- [x] Create `crates/srs-repository/src/json_store.rs` with:
  - Private `JsonStoreFile` envelope struct (the on-disk format)
  - Private `JsonStoreState` struct
  - Public `JsonStore` struct
  - `JsonStore::create(file_path)` constructor — errors if file already exists; returns uninitialized store
  - `JsonStore::open(file_path)` constructor — deserializes existing file; errors if missing or malformed
  - Private `flush(&self) -> Result<(), RepositoryError>` helper
  - Full `RepositoryStore for JsonStore` implementation (all trait methods)
- [x] Add `pub mod json_store;` to `crates/srs-repository/src/lib.rs`
- [x] Add `pub use json_store::JsonStore;` to `crates/srs-repository/src/lib.rs`
- [x] Write tests in `json_store.rs` (see Testing section below)

#### On-Disk Format

The `.srsj` file is a pretty-printed JSON object:

```json
{
  "srsj": "1",
  "manifest": { "instanceIndex": [], "repositoryId": "...", "namespace": "...", ... },
  "data": {
    "package/package.json": { ... },
    "package/fields/abc12345.json": { ... },
    "records/notes/xyz.json": { ... },
    "relations/relations-collection.json": { ... }
  }
}
```

- `srsj` — format version string, currently `"1"`
- `manifest` — serialized `Manifest` struct (the `root` field is `#[serde(skip)]` so it is excluded automatically)
- `data` — flat `HashMap<String, serde_json::Value>` keyed by forward-slash normalized relative paths

`package` is **not** stored as a separate top-level key; it is reconstructed from `data["package/package.json"]` on each `load_package()` call.

The corresponding Rust type:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct JsonStoreFile {
    srsj: String,
    manifest: serde_json::Value,
    data: HashMap<String, serde_json::Value>,
}
```

#### Internal Struct Layout

```rust
pub struct JsonStore {
    file_path: PathBuf,
    state: RefCell<JsonStoreState>,
}

struct JsonStoreState {
    initialized: bool,
    manifest: Manifest,
    data: HashMap<String, serde_json::Value>,
}
```

`JsonStore` is `!Sync` (contains `RefCell`) — same restriction as `MemoryStore`. Services take `&dyn RepositoryStore` and are single-threaded, so this is acceptable.

#### Key Method Behaviours

| Method | Behaviour |
|---|---|
| `repository_root()` | `file_path.parent().unwrap().to_path_buf()` |
| `repository_exists()` | `Ok(state.initialized)` — no filesystem check |
| `initialize_repository(input)` | Write manifest + `data["package/package.json"]`, set `initialized = true`, flush |
| `load_manifest()` | Return clone of `state.manifest` with `root` set to `repository_root()` |
| `save_manifest(m)` | Store in `state.manifest`, flush |
| `load_package()` | Read `data["package/package.json"]`; reconstruct `Package` using same logic as `MemoryStore::load_package()` in `store.rs` |
| `load_package_json()` | `data.get("package/package.json")` → clone or `NotFound` |
| `save_package_json(v)` | `data.insert("package/package.json", v)`, flush |
| `save_field(path, f)` | `data.insert(path, to_value(f))`, flush |
| `update_field_file(path, f)` | Same as `save_field` |
| `delete_field_file(path)` | `data.remove(path)`, flush |
| `ensure_fields_dir()` | No-op |
| `save_type / update_type_file / delete_type_file` | Same insert/remove pattern, flush each |
| `ensure_types_dir()` | No-op |
| `save_relation_type_definition(path, rt)` | `data.insert(path, to_value(rt))`, flush |
| `ensure_relation_types_dir()` | No-op |
| `save_view / update_view_file / delete_view_file` | Same insert/remove pattern, flush |
| `ensure_views_dir()` | No-op |
| `save_document_view / update_document_view_file / delete_document_view_file` | Same |
| `ensure_document_views_dir()` | No-op |
| `load_instance_json(path)` | `data.get(path)` → clone or `NotFound` |
| `save_instance_json(path, v)` | `data.insert(path, v)`, flush |
| `delete_instance_file(path)` | `data.remove(path)`, flush |
| `ensure_instance_dir(_)` | No-op |
| `list_instance_files(dir)` | Keys starting with `"{dir}/"`, ending `.json`, with no further `/` after prefix — same depth filter as `MemoryStore` |
| `load_relations_json(path)` | `data.get(path)` → clone or `NotFound` |
| `save_relations_json(path, v)` | `data.insert(path, v)`, flush |
| `ensure_relations_dir(_)` | No-op |
| `load_container_json(path)` | `data.get(path)` → clone or `NotFound` |
| `save_container_json(path, v)` | `data.insert(path, v)`, flush |
| `delete_container_file(path)` | `data.remove(path)`, flush |
| `ensure_containers_dir()` | No-op |
| `list_files_recursive(dir)` | All `data` keys starting with `"{dir}/"` |
| `load_text_file(path)` | `data.get(path)` → extract as JSON string value, or `NotFound` |
| `validate_package_ref_path(_)` | `Ok(())` unconditionally — no directory structure to validate |

#### Flush Helper

```rust
fn flush(&self) -> Result<(), RepositoryError> {
    let state = self.state.borrow();
    let manifest_value = serde_json::to_value(&state.manifest)
        .map_err(|e| RepositoryError::Serialize { path: self.file_path.clone(), source: e })?;
    let envelope = JsonStoreFile {
        srsj: "1".to_string(),
        manifest: manifest_value,
        data: state.data.clone(),
    };
    let json = serde_json::to_string_pretty(&envelope)
        .map_err(|e| RepositoryError::Serialize { path: self.file_path.clone(), source: e })?;
    std::fs::write(&self.file_path, &json)
        .map_err(|e| RepositoryError::Io { path: self.file_path.clone(), source: e })
}
```

#### Acceptance Criteria

- [x] `JsonStore::create` / `JsonStore::open` compile and work as constructors
- [x] `JsonStore` implements all methods of `RepositoryStore` with no `unimplemented!()` or `todo!()`
- [x] `repository_exists()` returns `false` before `initialize_repository()` and `true` after
- [x] `initialize_repository()` followed by `JsonStore::open` roundtrips manifest and package
- [x] Every mutating method flushes — verified by re-opening the file in tests
- [x] `list_instance_files` returns direct children only (no nested paths)
- [x] `validate_package_ref_path` returns `Ok(())` unconditionally
- [x] `ensure_*` methods are all no-ops (return `Ok(())`)

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository json_store
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `json_store.rs` under `#[cfg(test)]`:

**Lifecycle**
- `json_store_create_then_open_roundtrips` — create, initialize, drop, re-open; verify `repository_exists()` and manifest namespace survive
- `json_store_create_rejects_existing_file` — `create()` on a path that already has a file returns an error
- `json_store_open_rejects_missing_file` — `open()` on non-existent path returns `RepositoryError::Io`
- `json_store_open_rejects_malformed_json` — write garbage bytes, `open()` returns a parse error
- `json_store_initialize_rejects_duplicate` — two `initialize_repository()` calls return `RepositoryAlreadyExists`

**Flush correctness**
- `json_store_flush_on_save_instance` — save instance JSON, re-open from disk, verify instance is retrievable
- `json_store_flush_on_delete` — save then delete instance, re-open, verify absent

**Trait method correctness**
- `json_store_manifest_roundtrip` — `save_manifest` / `load_manifest`
- `json_store_package_json_roundtrip` — `save_package_json` / `load_package_json`
- `json_store_list_instance_files_direct_children_only` — nested path must not appear in results
- `json_store_list_files_recursive_returns_all_depths` — paths at multiple depths all returned
- `json_store_load_text_file_returns_string_value` — store a string JSON value, `load_text_file` returns it

**Portability integration** (use `repository_portability::{copy_repository, export_repository_snapshot}`)
- `json_store_copy_from_memory_store` — `copy_repository(&memory_store, &json_store)`, re-open `JsonStore`, verify snapshot matches original
- `json_store_copy_to_file_store` — initialize `JsonStore` with a field and an instance, `copy_repository(&json_store, &file_store)`, verify `FileStore` has expected files on disk
- `json_store_import_rejects_non_empty_target` — `initialize_repository` on target first, then `copy_repository` fails with `RepositoryNotEmpty` (or `RepositoryAlreadyExists`)

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed above exists and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update this plan: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `JsonStore` is publicly exported from `srs-repository` as `srs_repository::JsonStore`
- [x] A repository initialized in `MemoryStore` can be round-tripped through `JsonStore` to `FileStore` via `copy_repository` and validates cleanly
- [x] No service code or CLI code was modified — the trait boundary held

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The `.srsj` extension is a convention; `JsonStore::create` and `JsonStore::open` accept any path and do not enforce the extension.
- `MemoryStore` stays `#[cfg(test)]`. `JsonStore` duplicates the HashMap mutation pattern. The two types serve different purposes: `MemoryStore` is a test double, `JsonStore` is a production store adapter.
- `load_package()` reconstructs a `Package` from `data["package/package.json"]` on every call. No caching is needed at this stage — repository operations are not hot-path code.
- `repository_root()` returning `file_path.parent()` may return `.` if the file is in the current working directory. Callers that display paths should handle this gracefully, but this is not a `JsonStore` concern.
- The `RepositoryStore` trait currently contains methods added post-refactor (`save_relation_type_definition`, `ensure_relation_types_dir`). Verify the complete trait method list in `store.rs` before starting implementation — the trait may have grown since this plan was written.
