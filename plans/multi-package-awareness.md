# Plan: Multi-Package Write Targeting

## Summary

`load_package()` already merges fields, types, views, and document-views from all sub-packages declared in `manifest.json` `packageRefs` — reads are multi-package-aware. But every write operation (create, update, delete) hard-codes the primary `package/` directory. The root cause is `FileStore.pkg_abs()`, which prepends `"package/"` to all field/type/view relative paths. There is no `--package` flag, no `sourcePackage` in list output, and no way to create a field in `package/spec-rfc-process/` through the service layer or CLI. This plan fixes all three layers: `RepositoryStore` trait, service functions, and CLI.

**Prerequisite:** `storage-boundary-refactor.md` Phase E must be complete before the view_service tasks in Phase B begin. The package_service tasks in Phase A can start immediately — `package_service.rs` is already refactored to `&dyn RepositoryStore`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Storage + Service Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs. This plan implements the storage boundary established in `storage-boundary-refactor.md` (ADR-008) and extends it to cover sub-package write routing.

---

## Scope

- Add `load_package_json_for(package_dir)` and `save_package_json_for(package_dir, value)` to `RepositoryStore` trait
- Add `relative_dir: &str` parameter to all four `ensure_*_dir` trait methods
- Change `FileStore` implementations of `save_field`, `update_field_file`, `delete_field_file`, `save_type`, `update_type_file`, `delete_type_file`, `save_view`, `update_view_file`, `delete_view_file`, `save_document_view`, `update_document_view_file`, `delete_document_view_file` from `pkg_abs()` to `abs()` — relative paths now carry the full repo-root prefix
- Delete `pkg_abs` from `FileStore`
- Add `package_dir: Option<&str>` to `create_field` and `create_type` in `package_service.rs`
- Add `find_field_location` and `find_type_location` private helpers; update `update_field`, `delete_field`, `update_type`, `delete_type` to use them
- Add `source_package: String` to `FieldSummary` and `TypeSummary`; update `list_fields` and `list_types`
- Add `package_dir: Option<&str>` to `create_view` and `create_document_view` in `view_service.rs`; add `find_view_location` and `find_document_view_location`; add `source_package` to `ViewSummary` and `DocumentViewSummary`
- Add `--package <relative-path>` as a global flag to `Cli`; add `package_dir: Option<String>` to `CliContext`; wire through `dispatch()`
- Update `cmd_field_create`, `cmd_view_create`, `cmd_document_view_create` to pass `ctx.package_dir.as_deref()`
- Update `MemoryStore` path key conventions for field/type/view methods to match the new repo-root-relative prefix

**Out of scope:**

- SQLite or async storage (separate plan)
- `--package` filter on list commands (list already shows all packages; provenance is in `sourcePackage`)
- Moving definitions between packages
- Any changes to `srs-core` types

**Security and correctness invariants (must be enforced in implementation):**

- `--package` / `package_dir` must be validated against the manifest allowlist: `{ "package" } ∪ { ref.path | ref.mode == "local" }`. Any value not in this set returns `RepositoryError::InvalidPackageDir`. This also prevents path traversal since only declared, repo-relative paths are accepted.
- ID lookup in `find_field_location` (and equivalents) uses exact suffix matching: the filename ends with `-{id8}.json` where `id8 = &id[..8]`. This is stricter than `contains(&id[..8])` — slug text cannot collide with the 8-char ID suffix at the end of the filename.
- `id[..8]` slice is only taken after validating `id.len() >= 8`; short IDs return `RepositoryError::InvalidId("ID must be at least 8 characters")` rather than panicking.

---

## Phases

### Phase A: `RepositoryStore` Trait + `package_service.rs`

**Goal:** `RepositoryStore` has sub-package-aware package.json I/O; `create_field` and `create_type` accept `package_dir`; update/delete discover the owning package automatically; `FieldSummary` and `TypeSummary` include `source_package`.

**Agent:** Storage + Service Worker

#### Tasks

- [ ] `store.rs`: Add to `RepositoryStore` trait after `save_package_json`:
  ```rust
  fn load_package_json_for(&self, package_dir: &str) -> Result<serde_json::Value, RepositoryError>;
  fn save_package_json_for(&self, package_dir: &str, value: &serde_json::Value) -> Result<(), RepositoryError>;
  ```
  `FileStore` implementation: `self.read_json(&self.repo_root.join(package_dir).join("package.json"))` and `self.write_json(...)`. `MemoryStore` implementation: key is `format!("{}/package.json", package_dir)` in `self.data`.

- [ ] `store.rs`: Change signatures of the four `ensure_*_dir` trait methods to take `relative_dir: &str`:
  ```rust
  fn ensure_fields_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
  fn ensure_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
  fn ensure_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
  fn ensure_document_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
  ```
  `FileStore`: `self.ensure_dir(&self.abs(relative_dir))`. `MemoryStore`: no-op `Ok(())`.

- [ ] `store.rs`: In `FileStore`, change `save_field`, `update_field_file`, `delete_field_file`, `save_type`, `update_type_file`, `delete_type_file`, `save_view`, `update_view_file`, `delete_view_file`, `save_document_view`, `update_document_view_file`, `delete_document_view_file` from `self.pkg_abs(relative_path)` to `self.abs(relative_path)`. Delete `pkg_abs` method.

- [ ] `store.rs`: Update `MemoryStore` test helpers (`with_field`, `with_type`, `with_view`, `with_document_view` if they exist) — path keys change from e.g. `"fields/foo.json"` to `"package/fields/foo.json"` to match the new convention.

- [ ] `package_service.rs`: Add `package_dir: Option<&str>` parameter to `create_field`:
  ```rust
  pub fn create_field(store: &dyn RepositoryStore, field: Field, package_dir: Option<&str>) -> Result<CreateFieldResult, RepositoryError>
  ```
  Body: `let pkg_dir = package_dir.unwrap_or("package");` → if `package_dir.is_some()`, call `validate_package_dir(store, pkg_dir)?` → call `store.load_package_json_for(pkg_dir)` → `store.ensure_fields_dir(&format!("{}/fields", pkg_dir))` → full path `format!("{}/fields/{}-{}.json", pkg_dir, slug, id8)` for `store.save_field(...)` → index entry `format!("fields/{}-{}.json", slug, id8)` (paths in `package.json` are relative to the package directory, not repo root — preserving the existing `load_package_from_dir` convention) → `store.save_package_json_for(pkg_dir, ...)`.

- [ ] `package_service.rs`: Add `package_dir: Option<&str>` parameter to `create_type` using the same pattern (directory `types/`, index entry `"types/{slug}-{id8}.json"`).

- [ ] `package_service.rs`: Add private helper for allowlist derivation:
  ```rust
  fn declared_package_dirs(store: &dyn RepositoryStore) -> Result<Vec<String>, RepositoryError>
  // Returns ["package"] + [ref.path for local refs in manifest.packageRefs]
  ```
  Used by both `find_*_location` and `validate_package_dir`.

- [ ] `package_service.rs`: Add private helper for `package_dir` validation:
  ```rust
  fn validate_package_dir(store: &dyn RepositoryStore, package_dir: &str) -> Result<(), RepositoryError>
  // Errors with RepositoryError::InvalidPackageDir if package_dir not in declared_package_dirs()
  ```
  Called at the top of `create_field`, `create_type`, and later `create_view`, `create_document_view` when `package_dir` is `Some`. Prevents writes to arbitrary paths including traversal attempts.

- [ ] `package_service.rs`: Add private helper:
  ```rust
  fn find_field_location(store: &dyn RepositoryStore, id: &str) -> Result<(String, String), RepositoryError>
  // Returns (pkg_dir, index_relative_path) e.g. ("package/spec-rfc", "fields/foo-abc12.json")
  ```
  Algorithm: validate `id.len() >= 8` or return `RepositoryError::InvalidId`; extract `id8 = &id[..8]`; call `declared_package_dirs(store)?` → for each dir call `store.load_package_json_for(dir)` → search `fields` array for entry where the filename (last path segment) ends with `format!("-{}.json", id8)` (exact suffix — avoids false positives from slug text) → return first match or `RepositoryError::NotFound`.

- [ ] `package_service.rs`: Add `find_type_location` with the same shape (searches `types` array).

- [ ] `package_service.rs`: Update `update_field` and `delete_field` to use `find_field_location`:
  - `update_field`: call `find_field_location(store, &field.id)?` → full path `format!("{}/{}", pkg_dir, index_path)` → `store.update_field_file(&full_path, &field)`.
  - `delete_field`: call `find_field_location(store, id)?` → remove from index → `store.delete_field_file(&full_path)` → `store.save_package_json_for(&pkg_dir, ...)`.

- [ ] `package_service.rs`: Update `update_type` and `delete_type` to use `find_type_location` in the same pattern.

- [ ] `package_service.rs`: Add `pub source_package: String` to `FieldSummary` and `TypeSummary`.

- [ ] `package_service.rs`: Update `list_fields` to populate `source_package`:
  - Load full field data from `store.load_package()`.
  - Build `HashMap<id_prefix, pkg_dir>` by scanning each package dir's `package.json["fields"]` array (same pkg_dir list as `find_field_location`).
  - Map each field to `FieldSummary` with `source_package` from the map (default to `"package"` if not found).

- [ ] `package_service.rs`: Update `list_types` with the same pattern for `TypeSummary`.

- [ ] `package_service.rs`: Update all existing unit tests that call `store.save_field(...)` or pre-populate `MemoryStore` with field/type paths — update string keys from `"fields/foo.json"` to `"package/fields/foo.json"`. Update `store.load_package_json()` calls in tests to `store.load_package_json_for("package")`.

#### Unit tests to add in `package_service.rs`

- `create_field_in_sub_package` — `MemoryStore` pre-populated with `"package/sub/package.json": {"fields":[]}` and manifest with packageRef for `"package/sub"`; `create_field(store, field, Some("package/sub"))`; assert `store.data["package/sub/fields/foo-abc.json"]` exists and `store.data["package/sub/package.json"]["fields"]` contains `"fields/foo-abc.json"`; assert primary `package/package.json` unchanged.
- `create_field_defaults_to_primary` — `create_field(store, field, None)` → field lands in `"package/fields/"`.
- `create_field_rejects_undeclared_package` — `create_field(store, field, Some("package/intruder"))` where `"package/intruder"` is not in manifest; returns `Err(RepositoryError::InvalidPackageDir)`.
- `create_field_rejects_traversal_path` — `create_field(store, field, Some("package/../etc"))` returns `Err(RepositoryError::InvalidPackageDir)` (not in allowlist).
- `find_field_location_finds_in_sub_package` — manifest with packageRef; sub-package json with `"fields": ["fields/foo-abc12345.json"]`; `find_field_location(store, "abc12345-...")` (full UUID starting with `abc12345`) returns `("package/sub", "fields/foo-abc12345.json")`.
- `find_field_location_no_false_positive` — sub-package json with `"fields": ["fields/abc12345-xyz.json"]` (ID chars in slug, not suffix); `find_field_location(store, "99999999-...")` returns `RepositoryError::NotFound` — exact suffix match prevents slug collision.
- `find_field_location_returns_not_found` — empty stores; `find_field_location` returns `RepositoryError::NotFound`.
- `find_field_location_rejects_short_id` — `find_field_location(store, "abc")` returns `Err(RepositoryError::InvalidId)`.
- `update_field_discovers_sub_package` — field in sub-package; `update_field` updates the correct file path.
- `delete_field_discovers_sub_package` — field in sub-package; `delete_field` removes from correct index.
- `list_fields_includes_source_package` — primary and sub-package each have one field; `list_fields` returns both summaries with distinct `source_package` values.

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles clean
- [ ] `grep -r 'pkg_abs' crates/srs-repository/src/` returns empty
- [ ] `create_field(store, field, Some("package/spec-rfc"))` writes to `"package/spec-rfc/fields/..."` in `MemoryStore`
- [ ] `update_field` and `delete_field` operate correctly when field is in a sub-package
- [ ] `FieldSummary` and `TypeSummary` serialise with `"sourcePackage"` key
- [ ] All new unit tests pass; all pre-existing `package_service` unit tests pass after path key updates
- [ ] `cargo clippy -p srs-repository -- -D warnings` clean

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository package_service
cargo clippy -p srs-repository -- -D warnings
```

Tests to write or verify:
- `create_field_in_sub_package` — proves create routes to sub-package
- `create_field_defaults_to_primary` — proves existing behaviour preserved
- `create_field_rejects_undeclared_package` — enforce manifest allowlist
- `create_field_rejects_traversal_path` — traversal rejected via allowlist
- `find_field_location_finds_in_sub_package` — proves update/delete discovery
- `find_field_location_no_false_positive` — suffix match avoids slug collision
- `find_field_location_returns_not_found` — error path
- `find_field_location_rejects_short_id` — short IDs return `InvalidId`
- `update_field_discovers_sub_package` — update via discovery
- `delete_field_discovers_sub_package` — delete via discovery
- `list_fields_includes_source_package` — provenance in summaries

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all 11 new unit tests exist and pass; all existing tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Update plan checkboxes.
5. Commit.

---

### Phase B: `view_service.rs` Multi-Package Support

**Goal:** `create_view`, `create_document_view` accept `package_dir`; update/delete discover the owning package; `ViewSummary` and `DocumentViewSummary` include `source_package`.

**Agent:** Storage + Service Worker

**Prerequisite:** `storage-boundary-refactor.md` Phase E must be complete (`view_service.rs` must take `&dyn RepositoryStore` before this phase begins).

#### Tasks

- [ ] `view_service.rs`: Add `package_dir: Option<&str>` to `create_view`:
  ```rust
  pub fn create_view(store: &dyn RepositoryStore, view: View, package_dir: Option<&str>) -> Result<View, RepositoryError>
  ```
  Pattern identical to `create_field`: `pkg_dir = package_dir.unwrap_or("package")`, full file path `"{pkg_dir}/views/{slug}-{id8}.json"`, index entry `"views/{slug}-{id8}.json"`, `store.ensure_views_dir(&format!("{}/views", pkg_dir))`, `store.save_package_json_for(pkg_dir, ...)`.

- [ ] `view_service.rs`: Add `package_dir: Option<&str>` to `create_document_view` using the same pattern (directory `document-views/`).

- [ ] `view_service.rs`: Add `find_view_location(store, id) -> Result<(String, String), RepositoryError>` — searches `views` array in each package's `package.json`. Add `find_document_view_location` for `document_views` array.

- [ ] `view_service.rs`: Update `update_view` and `delete_view` to use `find_view_location`.

- [ ] `view_service.rs`: Update `update_document_view` and `delete_document_view` to use `find_document_view_location`.

- [ ] `view_service.rs`: Add `pub source_package: String` to `ViewSummary` and `DocumentViewSummary`. Update `list_views` and `list_document_views` to populate it (same scan pattern as `list_fields`).

#### Unit tests to add in `view_service.rs`

- `create_view_in_sub_package` — mirrors `create_field_in_sub_package`
- `create_document_view_in_sub_package` — mirrors above
- `find_view_location_finds_in_sub_package`
- `find_document_view_location_finds_in_sub_package`
- `list_views_includes_source_package`
- `list_document_views_includes_source_package`

#### Acceptance Criteria

- [ ] `create_view(store, view, Some("package/spec-rfc"))` writes to `"package/spec-rfc/views/..."` in `MemoryStore`
- [ ] `update_view` and `delete_view` work when view is in a sub-package
- [ ] `ViewSummary` and `DocumentViewSummary` serialise with `"sourcePackage"`
- [ ] All 6 new unit tests pass
- [ ] `cargo clippy -p srs-repository -- -D warnings` clean

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository view_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm all 6 new unit tests exist and pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Update plan checkboxes.
5. Commit.

---

### Phase C: CLI `--package` Flag + Integration Tests

**Goal:** `--package` is a global CLI flag; create commands route writes to the correct package; list outputs include `sourcePackage`; integration tests cover the sub-package write path.

**Agent:** CLI Worker

#### Tasks

- [ ] `crates/srs-cli/src/commands/mod.rs`: Add to `Cli` struct:
  ```rust
  /// Target package directory for create operations (relative to repo root).
  /// Example: package/spec-rfc-process. Defaults to primary package.
  #[arg(long = "package", global = true)]
  pub package_dir: Option<String>,
  ```
  Add `pub package_dir: Option<String>` to `CliContext`. Wire in `dispatch()` constructor (same pattern as `container_id`).

- [ ] `crates/srs-cli/src/commands/field.rs`: In `cmd_field_create`, pass `ctx.package_dir.as_deref()` as third argument to `create_field`. No change to `cmd_field_list`, `cmd_field_get`, `cmd_field_delete` — they don't take `package_dir`.

- [ ] `crates/srs-cli/src/commands/view.rs` (Phase A of views-crud.md): In `cmd_view_create`, pass `ctx.package_dir.as_deref()`. No change to other handlers.

- [ ] `crates/srs-cli/src/commands/document_view.rs` (Phase A of views-crud.md): In `cmd_document_view_create`, pass `ctx.package_dir.as_deref()`.

- [ ] Integration test helper: `make_repo_with_sub_package() -> TempDir` — creates a minimal manifest with a `packageRefs` entry for `"package/sub"`, a `package/package.json` with empty arrays, and a `package/sub/package.json` with empty arrays. Both directories exist on disk.

- [ ] Integration tests in `crates/srs-cli/tests/integration_tests.rs`:
  - `field_create_with_package_flag_writes_to_sub_package` — `make_repo_with_sub_package()`, run `srs field create --package package/sub` with valid field JSON; assert `package/sub/fields/foo.json` exists and `package/sub/package.json["fields"]` contains it; assert `package/package.json["fields"]` unchanged.
  - `field_create_without_package_flag_writes_to_primary` — fresh repo, `srs field create` without `--package`; assert `package/fields/foo.json` exists.
  - `field_create_with_undeclared_package_flag_errors` — `srs field create --package package/ghost` where `"package/ghost"` is not in manifest; assert `ok: false` in output and no files created under `package/ghost/`.
  - `field_list_includes_source_package` — repo with fields in both primary and sub-package; `srs field list`; each item in `payload.fields` has `"sourcePackage"` key.
  - `field_delete_without_package_removes_from_correct_package` — field in sub-package; `srs field delete <id>` without `--package`; assert file removed from `package/sub/fields/`; sub-package's `package.json["fields"]` no longer contains it.

#### Acceptance Criteria

- [ ] `srs field create --package package/spec-rfc-process` routes the write to `package/spec-rfc-process/` on disk
- [ ] `srs field create` without `--package` behaves identically to before (primary package)
- [ ] `srs field list` output includes `"sourcePackage"` on each field item
- [ ] `srs field delete <id>` without `--package` removes from the correct package (discovers it)
- [ ] `srs --help` shows `--package` in global flags
- [ ] All 5 new integration tests pass
- [ ] All pre-existing integration tests pass unchanged
- [ ] `cargo clippy -p srs-cli -- -D warnings` clean

#### Testing

```bash
cd srs-rust
cargo test -p srs --test integration_tests -- field_
cargo clippy -p srs-cli -- -D warnings
```

Specific tests:
- `field_create_with_package_flag_writes_to_sub_package`
- `field_create_without_package_flag_writes_to_primary`
- `field_create_with_undeclared_package_flag_errors`
- `field_list_includes_source_package`
- `field_delete_without_package_removes_from_correct_package`

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm all 5 new integration tests exist and pass.
3. Run:
```bash
cargo build
cargo test
cargo clippy -- -D warnings
```
4. Update plan checkboxes.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `grep -r 'pkg_abs' crates/srs-repository/src/` returns empty
- [ ] `srs field create --package <sub-pkg-path>` writes to the named sub-package directory
- [ ] `srs field list` output includes `"sourcePackage"` on every field
- [ ] `srs field delete <id>` works without `--package` for fields in sub-packages
- [ ] Same behaviour confirmed for types, views, and document-views
- [ ] All integration tests pass: `cargo test --test integration_tests`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `store.rs` `FileStore` has a private `abs()` method (resolves relative path against `repo_root`) and a private `pkg_abs()` method (same but prepends `"package/"`). The change in Phase A removes `pkg_abs` and switches field/type/view methods to `abs()`.
- `package.json` files store field/type/view paths relative to their own directory (e.g. `"fields/foo.json"`, not `"package/spec-rfc/fields/foo.json"`). This is the existing convention in `load_package_from_dir` and must be preserved. Only the `store.save_field()` / `store.update_field_file()` call sites in service code receive the full repo-root-relative path.
- `MemoryStore` stores all values by string key in a `RefCell<HashMap<String, serde_json::Value>>`. After Phase A, keys for field/type/view data use the full repo-root-relative path (e.g. `"package/fields/foo.json"`). Existing tests that pre-populate `MemoryStore` with path keys like `"fields/foo.json"` must be updated to `"package/fields/foo.json"`.
- `manifest.json` `packageRefs` entries have `{ "mode": "local", "path": "package/spec-rfc-process" }` format. The `path` value is relative to `repo_root`. Primary package is always at `"package"` (not listed in `packageRefs`).
- `storage-boundary-refactor.md` Phase E (refactoring `view_service.rs` to `&dyn RepositoryStore`) is a hard prerequisite for Phase B of this plan. Phase A has no such dependency.
- `list_fields` and `list_types` currently call `store.load_package()` for merged field data. The provenance derivation in Phase A adds a second pass (scanning each package's `package.json` index) without changing the merge semantics of `load_package()`.
