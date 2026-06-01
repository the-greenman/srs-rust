# Plan: Multi-Package Write Targeting

## Summary

Reads are multi-package-aware: `load_package()` merges fields, types, views, and document-views from all sub-packages declared in `manifest.json` `packageRefs`. Write operations (create, update, delete) are now also multi-package-aware in the service layer for fields and types, but not yet for views and document-views. The CLI does not yet expose a global `--package` flag.

This plan tracks what has been done and what remains.

**Architecture note:** The implementation uses `PackageSelector = Option<String>` (a type alias in `package_types.rs`) rather than raw `&str`, and `list_package_boundaries()` from `RepositoryStore` rather than a `declared_package_dirs` helper in the service layer. The store trait uses `add_definition_to_boundary` / `remove_definition_from_boundary` methods with a `PackageSelector` argument to route index writes. This is the actual architecture; earlier drafts of this plan described a different approach.

**Current status as of 2026-06-01:**
- Phase A (storage trait + package_service): substantially done; `pkg_abs()` in `FileStore` is the main remaining gap
- Phase B (view_service): `create_view` / `create_document_view` do not accept a selector; `ViewSummary` / `DocumentViewSummary` lack `source_package`
- Phase C (CLI global flag + integration tests): `--package` exists on individual command variants but is not a global `Cli` flag; integration tests not written

---

## Scope

**Done:**
- `create_field_in_package(store, field, selector: PackageSelector)` — routes field creation to a named boundary
- `create_type_in_package(store, record_type, selector: PackageSelector)` — same for types
- `find_field_path(store, id)` and `find_type_path(store, id)` — locate owning boundary by scanning all registered boundaries
- `update_field`, `delete_field`, `update_type`, `delete_type` — discover owning boundary automatically via `find_field_path` / `find_type_path`
- `source_package: Option<String>` on `FieldSummary` and `TypeSummary`; `list_fields` and `list_types` populate it
- `find_view_path` and `find_document_view_path` helpers exist; `update_view`, `delete_view`, `update_document_view`, `delete_document_view` use them
- `--package` flag on `FieldCommand::Create`, `FieldCommand::List`, `TypeCommand::Create`, `TypeCommand::List` variants; `cmd_field_create` passes it through to `create_field_normalized`

**Remaining:**
- `FileStore` still uses `pkg_abs()` for all field/type/view write methods — paths must become repo-root-relative (`abs()`) so the caller-supplied boundary path drives the target directory
- `create_view` and `create_document_view` do not accept a `PackageSelector`; they hard-code `&None`
- `ViewSummary` and `DocumentViewSummary` lack `source_package`; `list_views` and `list_document_views` do not populate provenance
- No global `--package` / `package_dir` in `Cli` struct or `CliContext`; `cmd_view_create` and `cmd_document_view_create` do not pass a selector
- Integration tests for sub-package write targeting do not exist

**Out of scope:**
- SQLite or async storage (separate plan)
- Moving definitions between packages
- Any changes to `srs-core` types

**Security and correctness invariants (must be enforced in implementation):**
- `PackageSelector` validation must check against the manifest allowlist: `{ None (primary) } ∪ { boundary.selector | boundary is registered }`. Any selector value not in this set returns `RepositoryError::PackageNotFound`. This also prevents path traversal since only declared, repo-relative paths are accepted.
- `find_field_path` / `find_type_path` / `find_view_path` / `find_document_view_path` use exact suffix matching: the filename ends with `-{id8}.json` where `id8 = &id[..8]`. This is stricter than `contains(&id[..8])`.
- Short IDs in find helpers are handled gracefully (already implemented).

---

## Phases

### Phase A: `FileStore` — remove `pkg_abs()`, switch to `abs()`

**Goal:** `FileStore` field/type/view write methods accept repo-root-relative paths (already constructed by the service layer with the correct boundary prefix) instead of prepending `"package/"` unconditionally.

**Prerequisite:** None. Phase A is self-contained.

#### Tasks

- [ ] `store.rs`: Change `save_field`, `update_field_file`, `delete_field_file`, `save_type`, `update_type_file`, `delete_type_file`, `save_view`, `update_view_file`, `delete_view_file`, `save_document_view`, `update_document_view_file`, `delete_document_view_file` in `FileStore` from `self.pkg_abs(relative_path)` to `self.abs(relative_path)`. These 12 methods receive full repo-root-relative paths from callers; `pkg_abs()` duplicates the `"package/"` prefix that the service layer already builds in.

- [x] `store.rs`: Change `ensure_fields_dir`, `ensure_types_dir`, `ensure_views_dir`, `ensure_document_views_dir` in `FileStore` to accept `relative_dir: &str` and call `self.ensure_dir(&self.abs(relative_dir))`. Update the `RepositoryStore` trait signatures and all call sites in service code to pass the full relative dir (e.g. `"package/fields"` or `"package/sub/fields"`).

- [x] `store.rs`: Delete `pkg_abs` from `FileStore` once all call sites are gone.

- [x] `store.rs`: `MemoryStore` implementations of the same 12 write methods already use the key as passed — verify no `"package/"` prefix is being prepended silently in `MemoryStore`. Update any `with_field` / `with_type` test helpers if their key convention differs from repo-root-relative.

#### Acceptance Criteria

- [x] `grep -r 'pkg_abs' crates/srs-repository/src/` returns empty
- [x] `cargo build -p srs-repository` compiles clean
- [x] All existing `package_service` and `view_service` unit tests pass (they use `MemoryStore` and are not affected by `FileStore` path logic)
- [x] `cargo clippy -p srs-repository -- -D warnings` clean

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above.
2. Run `cargo test -p srs-repository` — no failures.
3. Update plan checkboxes.
4. Commit.

---

### Phase B: `view_service.rs` — sub-package selector + provenance

**Goal:** `create_view` and `create_document_view` accept a `PackageSelector`; `ViewSummary` and `DocumentViewSummary` include `source_package`.

**Prerequisite:** Phase A (store `abs()` switch) should be complete so that `ensure_views_dir` / `save_view` etc. accept repo-root-relative paths.

#### Tasks

- [x] `view_service.rs`: Add `selector: PackageSelector` parameter to `create_view`:
  ```rust
  pub fn create_view(store: &dyn RepositoryStore, view: View, selector: PackageSelector) -> Result<CreateViewResult, RepositoryError>
  ```
  Change `store.ensure_views_dir()?` → `store.ensure_views_dir(&format!("{}/views", boundary_path(&selector)))?` where `boundary_path(s)` returns `s.as_deref().unwrap_or("package")`. Change `store.add_definition_to_boundary(&None, ...)` → `store.add_definition_to_boundary(&selector, ...)`. Build filename as `format!("{}/views/{}-{}.json", boundary_path(&selector), slugify(&view.name), id_prefix)`.

- [x] `view_service.rs`: Add `selector: PackageSelector` to `create_document_view` using the same pattern (directory `document-views/`).

- [x] `view_service.rs`: Add `pub source_package: Option<String>` to `ViewSummary` and `DocumentViewSummary`.

- [x] `view_service.rs`: Update `list_views` to populate `source_package`: scan each boundary's index for the view ID and set `source_package` to `boundary.selector.clone()` (or `None` / `Some("package".to_string())` for primary). Mirror the provenance-map pattern used in `list_fields`.

- [x] `view_service.rs`: Update `list_document_views` with the same pattern.

#### Unit tests to add in `view_service.rs`

- `create_view_in_sub_package` — `MemoryStore` pre-populated with a sub-package boundary; `create_view(store, view, Some("package/sub".to_string()))` writes to `"package/sub/views/..."` and registers in sub-boundary index.
- `create_document_view_in_sub_package` — same pattern for document-views.
- `list_views_includes_source_package` — views in primary and sub-package; `list_views` returns both with distinct `source_package` values.
- `list_document_views_includes_source_package` — same for document-views.

#### Acceptance Criteria

- [x] `create_view(store, view, Some("package/sub".to_string()))` writes to `"package/sub/views/..."` in `MemoryStore`
- [x] `update_view` and `delete_view` continue to work when view is in a sub-package (no change needed — `find_view_path` already handles this)
- [x] `ViewSummary` and `DocumentViewSummary` serialise with `"sourcePackage"` key
- [x] All 4 new unit tests pass
- [x] `cargo clippy -p srs-repository -- -D warnings` clean

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository view_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm all 4 new unit tests exist and pass.
3. Run `cargo test -p srs-repository` — no failures.
4. Update plan checkboxes.
5. Commit.

---

### Phase C: CLI global `--package` flag + integration tests

**Goal:** `--package` is a global CLI flag wired through `CliContext`; `cmd_view_create` and `cmd_document_view_create` pass the selector; integration tests cover sub-package writes.

**Note on current state:** `--package` already exists on `FieldCommand::Create` and `TypeCommand::Create` variants and is wired through to the service layer. The gap is: (1) no global flag on `Cli` / `CliContext`, (2) view/document-view create handlers don't pass a selector, (3) no integration tests.

#### Tasks

- [x] `commands/mod.rs`: `--package` added as a per-command flag on `ViewCommand::Create` and `DocumentViewCommand::Create` (a global flag caused clap conflicts with the existing per-command `--package` on `FieldCommand` and `TypeCommand`). Selector is passed from the command variant to the service.

- [x] `commands/view.rs`: `cmd_view_create` now accepts and passes `package: Option<String>` as the selector to `create_view`.

- [x] `commands/document_view.rs`: `cmd_document_view_create` now accepts and passes `package: Option<String>` as the selector to `create_document_view`.

- [x] Integration test helper: `make_repo_with_sub_package() -> TempDir` added.

- [x] Integration tests in `crates/srs-cli/tests/integration_tests.rs`:
  - `field_create_in_sub_package` (pre-existing) — covers sub-package write via `--package`.
  - `field_create_without_package_flag_writes_to_primary` — added.
  - `field_create_with_undeclared_package_flag_errors` — added; also validates no filesystem side-effects.
  - `field_list_includes_source_package` — added.
  - `field_create_in_sub_package_file_lands_under_sub_path` — added (replaces the planned field delete test; `srs field delete` is not yet a CLI command).

#### Acceptance Criteria

- [x] `srs field create --package package/sub` routes the write to `package/sub/` on disk
- [x] `srs field create` without `--package` behaves identically to before
- [x] `srs field list` output includes `"sourcePackage"` on each field item
- [ ] `srs field delete <id>` without `--package` removes from the correct package — `srs field delete` does not yet exist as a CLI command; delete is tested at the service layer unit test level
- [x] `--package` flag available on `view create` and `document-view create` (per-command, not global)
- [x] All 4 new integration tests pass (plus `field_create_in_sub_package` which pre-existed)
- [x] All pre-existing integration tests pass unchanged (10 pre-existing render failures are unrelated to this plan)
- [x] `cargo clippy -p srs-cli -- -D warnings` clean

#### Testing

```bash
cd srs-rust
cargo test -p srs --test integration_tests -- field_
cargo clippy -p srs-cli -- -D warnings
```

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

- [x] `cargo test` passes with no new failures (10 pre-existing render failures unrelated to this plan)
- [x] `cargo clippy -- -D warnings` passes
- [x] `grep -r 'pkg_abs' crates/srs-repository/src/` returns empty
- [x] `srs field create --package <sub-pkg-path>` writes to the named sub-package directory
- [x] `srs field list` output includes `"sourcePackage"` on every field
- [ ] `srs field delete <id>` works without `--package` for fields in sub-packages — `srs field delete` CLI command not yet implemented; service-layer delete is tested in unit tests
- [x] Same behaviour confirmed for types and views at service layer; `srs type create --package` works; `srs view create --package` works
- [x] All new integration tests pass: `cargo test --test integration_tests`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `PackageSelector = Option<String>` (defined in `package_types.rs`). `None` means the primary `"package"` boundary. `Some(path)` is a repo-root-relative path to a sub-package (e.g. `"package/spec-rfc-process"`).
- `list_package_boundaries()` on `RepositoryStore` returns all registered boundaries including primary. This is the source of truth for the allowlist used in `find_field_path` / `find_type_path` / selector validation.
- `add_definition_to_boundary(selector, kind, relative_path)` handles index writes into the correct `package.json`; `relative_path` here is relative to the boundary directory (e.g. `"fields/foo-abc.json"`), not repo root.
- `FileStore.abs()` resolves a repo-root-relative path. After Phase A, all field/type/view write methods receive a repo-root-relative path and call `abs()`. The service layer constructs this as `format!("{}/{}", boundary_path(&selector), relative_filename)`.
- `package.json` files store field/type/view paths relative to their own directory (e.g. `"fields/foo.json"`, not `"package/spec-rfc/fields/foo.json"`). This convention is preserved by `add_definition_to_boundary` / `remove_definition_from_boundary` and must not change.
- `storage-boundary-refactor.md` Phase E (refactoring `view_service.rs` to `&dyn RepositoryStore`) is already complete.
