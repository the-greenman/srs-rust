# Plan: Storage-Agnostic Package Boundaries

## Summary

This plan refactors package management so packages are logical definition boundaries rather than
filesystem-shaped subdirectories. Packages segment reusable definitions and metadata; services must
address them through package selectors and boundary models while FileStore, MemoryStore, and a
future SqlStore map those operations to their own storage representation.

## Progress Summary (as of 2026-05-30)

| Phase | Status |
|---|---|
| 1: Architecture Contract | Mostly done — ADR decision still open |
| 2: Package Boundary Types and Store Contract | Not started |
| 3: Package Lifecycle Services | Partial — create/list done; import/update missing |
| 4: Package-Aware Definition Services | Mostly done — field/type create targeting sub-package missing |
| 5: CLI | Partial — create/list/filters done; import/update/slice/deprecations missing |
| Tests | Minimal — most planned tests not written |

---

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Package Boundary Worker | — |
| Package Service Worker | — |
| CLI Worker | — |
| Documentation Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| TBD | Services address packages through logical selectors; adapters own package files/tables | proposed |

The Documentation Worker must either create an ADR under `srs-rust/docs/adr/` or explicitly record
in `ARCHITECTURE.md` why the existing rules are sufficient and no separate ADR is needed.

---

## Scope

- Add package selectors and package boundary types.
- Add `load_effective_package`, `list_package_boundaries`, `load_package_boundary`,
  `register_package_boundary`, `save_package_boundary_metadata`, `resolve_definition_owner` to
  `RepositoryStore`.
- Implement new methods for FileStore and MemoryStore. JsonStore gets the same.
- Migrate package services off raw `load_package_json` calls to the new boundary methods.
- Add `import_package_local` and `update_package_metadata` services.
- Add field/type create targeting a specific package boundary.
- Add CLI commands: `srs package import`, `srs package update`, `srs slice create`.
- Deprecate `srs package enable/disable` or route them through package lifecycle services.
- Write the tests planned in each phase that do not yet exist.

**Out of scope:**

- Repository creation and full-repository portability (covered by lifecycle plan).
- Container storage alignment (covered by container plan).
- SQL adapter implementation.
- Registry or network-backed package import.
- Repository slice export/import from RFC-003.
- Changing the current file-backed package layout.

---

## Phases

### Phase 1: Architecture Contract

**Goal:** The package boundary model is documented before implementation.

**Agent:** Lead Integrator + Documentation Worker

#### Tasks

- [x] Update `srs-rust/ARCHITECTURE.md` with package boundary rules.
- [x] State that package services use selectors and adapters use files/tables.
- [x] Document packages as definition/meta boundaries.
- [x] Document that raw `package.json`, package paths, and package file indexes must not appear in service APIs.
- [ ] Decide whether to add or update an ADR under `srs-rust/docs/adr/`. Either create
  `docs/adr/0002-package-boundary-model.md` or add a note to `ARCHITECTURE.md` explaining why a
  separate ADR is not needed.

#### Acceptance Criteria

- [x] `ARCHITECTURE.md` names packages as logical definition boundaries.
- [x] `ARCHITECTURE.md` prohibits path-shaped package service APIs.
- [ ] ADR decision is either created/updated or explicitly deferred with rationale.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 2: Package Boundary Types and Store Contract

**Goal:** `RepositoryStore` exposes logical package operations. Services no longer call
`load_package_json` directly — they call boundary methods that FileStore and MemoryStore
translate to their own storage shape.

**Agent:** Package Boundary Worker

#### New types — add to `crates/srs-repository/src/package_types.rs` (new file)

```rust
/// Identifies a package boundary within a repository.
/// `None` = primary package ("package/"); `Some(path)` = sub-package.
pub type PackageSelector = Option<String>;

/// Metadata describing one package boundary.
#[derive(Debug, Clone)]
pub struct PackageBoundary {
    pub selector: PackageSelector,       // None = primary
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub field_paths: Vec<String>,        // package-relative paths
    pub type_paths: Vec<String>,
}

/// A field or type merged from all boundaries, carrying its source.
#[derive(Debug, Clone)]
pub struct OwnedField {
    pub field: srs_core::types::field::Field,
    pub owner: PackageSelector,
}

#[derive(Debug, Clone)]
pub struct OwnedType {
    pub record_type: srs_core::types::record_type::RecordType,
    pub owner: PackageSelector,
}
```

Export from `lib.rs`: `pub use package_types::{PackageBoundary, PackageSelector, OwnedField, OwnedType};`

#### New trait methods — add to `RepositoryStore` in `store.rs`

Add after the existing `// --- Package index ---` block:

```rust
// --- Package boundaries ---

/// Return metadata for all package boundaries (primary + all sub-packages).
fn list_package_boundaries(&self) -> Result<Vec<PackageBoundary>, RepositoryError>;

/// Return metadata for one boundary. Returns `PackageNotFound` if missing.
fn load_package_boundary(&self, selector: &PackageSelector) -> Result<PackageBoundary, RepositoryError>;

/// Persist metadata for one boundary (id, namespace, name, version).
/// Creates the boundary's `package.json` if it does not exist.
fn save_package_boundary_metadata(&self, boundary: &PackageBoundary) -> Result<(), RepositoryError>;

/// Register a new boundary in the manifest (add to packageRefs for sub-packages).
/// No-op if already registered. For the primary package this is also a no-op.
fn register_package_boundary(&self, selector: &PackageSelector) -> Result<(), RepositoryError>;

/// Add a definition path to a boundary's index (e.g. "fields/foo.json").
fn add_definition_to_boundary(&self, selector: &PackageSelector, kind: DefinitionKind, path: &str) -> Result<(), RepositoryError>;

/// Remove a definition path from a boundary's index.
fn remove_definition_from_boundary(&self, selector: &PackageSelector, kind: DefinitionKind, path: &str) -> Result<(), RepositoryError>;

/// Find which boundary owns a field or type by ID.
fn resolve_definition_owner(&self, id: &str, kind: DefinitionKind) -> Result<PackageSelector, RepositoryError>;
```

Add `DefinitionKind` enum to `package_types.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind { Field, Type, View, DocumentView, RelationType }
```

#### FileStore implementation

- `list_package_boundaries`: read `load_package_json()` for primary + each `packageRefs` entry.
- `load_package_boundary(None)`: read `package/package.json`.
- `load_package_boundary(Some(path))`: read `{path}/package.json`.
- `save_package_boundary_metadata`: write the metadata fields into the appropriate `package.json`.
- `register_package_boundary(None)`: no-op.
- `register_package_boundary(Some(path))`: add `{mode:local, path}` to manifest `packageRefs` if absent.
- `add_definition_to_boundary`: load the `package.json`, push the path into the `fields`/`types`/etc. array, write back.
- `remove_definition_from_boundary`: load, filter out the path, write back.
- `resolve_definition_owner`: walk each boundary in order; for each path in the matching array, load the file and compare `id` field.

#### MemoryStore implementation

- Store boundary metadata in a new `boundaries: RefCell<HashMap<Option<String>, PackageBoundary>>`.
- `list_package_boundaries`: return all values from `boundaries`.
- `load_package_boundary(sel)`: lookup in `boundaries`, return `PackageNotFound` if absent.
- `save_package_boundary_metadata`: update `boundaries[sel]`.
- `register_package_boundary`: insert entry in `boundaries` if absent; for `Some(path)` also add to manifest `packageRefs`.
- `add_definition_to_boundary`: push to `boundaries[sel].field_paths` or `type_paths`.
- `remove_definition_from_boundary`: filter the vec.
- `resolve_definition_owner`: walk `boundaries`; for each path in the kind array, load `data[package/{path}]` or `data[{boundary_path}/{path}]` and compare `id`.

#### JsonStore implementation

- Same pattern as FileStore (JsonStore uses `data` map, so reads/writes go through `load_instance_json`/`save_instance_json`).

#### Tasks

- [ ] Create `crates/srs-repository/src/package_types.rs` with `PackageSelector`, `PackageBoundary`, `OwnedField`, `OwnedType`, `DefinitionKind`.
- [ ] Export from `lib.rs`.
- [ ] Add the seven new methods to the `RepositoryStore` trait.
- [ ] Implement all seven methods on `FileStore`.
- [ ] Implement all seven methods on `MemoryStore` (using `boundaries` RefCell, not path-shaped data).
- [ ] Implement all seven methods on `JsonStore`.
- [ ] Add new `RepositoryError` variant `PackageNotFound { selector: PackageSelector }` to `error.rs`.

#### Acceptance Criteria

- [ ] New package methods are sufficient for package services without calling `load_package_json`.
- [ ] MemoryStore package state is keyed by `PackageSelector`, not fake file paths.
- [ ] FileStore preserves current `package/` and `packageRefs` layout.
- [ ] JsonStore implements all methods and passes the same tests as MemoryStore.
- [ ] Trait remains synchronous.
- [ ] All three implementers compile with no warnings.

#### Testing — add to `store.rs` `#[cfg(test)]` block

- `memory_store_list_package_boundaries_returns_primary` — empty store has primary boundary.
- `memory_store_register_sub_package_adds_to_boundaries` — `register_package_boundary(Some("pkg/ext"))` adds entry.
- `memory_store_add_definition_to_boundary_updates_paths` — field path appears in `field_paths`.
- `memory_store_resolve_definition_owner_primary` — field stored in primary boundary resolves to `None`.
- `memory_store_resolve_definition_owner_sub_package` — field stored in sub-package resolves to `Some(path)`.
- `file_store_package_boundary_maps_existing_layout` — `list_package_boundaries` on a real FileStore fixture returns primary + registered sub-packages.
- `json_store_package_boundaries_roundtrip` — register, add definition, open new JsonStore from same file, verify boundary and path present.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 3: Package Lifecycle Services

**Goal:** Package lifecycle behavior is fully implemented through `RepositoryStore` boundary methods.
Services do not call `load_package_json` directly.

**Agent:** Package Service Worker

#### Current state

- `create_package` — done but calls `load_manifest`/`save_manifest` and `validate_package_ref_path` directly instead of `register_package_boundary`. Migrate after Phase 2.
- `list_packages` — done but reads raw `load_package_json`. Migrate to `list_package_boundaries` after Phase 2.
- `import_package_local` — not implemented.
- `update_package_metadata` — not implemented.

#### Tasks

- [ ] Migrate `create_package` to call `store.register_package_boundary` and
  `store.save_package_boundary_metadata` instead of hand-writing manifest/packageRefs mutations.
- [ ] Migrate `list_packages` to call `store.list_package_boundaries` and map to
  `PackageBoundaryInfo`. Remove direct `load_package_json` call.
- [ ] Implement `import_package_local(store, input: ImportPackageLocalInput) -> Result<ImportPackageLocalResult>`:
  - `ImportPackageLocalInput`: `{ source_path: String }` — path relative to repo root of a directory containing a `package.json`.
  - Validate the source directory contains a valid `package.json` (read via `store.load_instance_json`).
  - Extract `id`, `namespace`, `name`, `version` from source `package.json`.
  - If a boundary with the same `id` is already registered, return `RepositoryError::PackageAlreadyRegistered { id }`.
  - Copy the source `package.json` to `{source_path}/package.json` in store (it's already there for FileStore; for MemoryStore/JsonStore write via `save_instance_json`).
  - Call `store.register_package_boundary(Some(source_path))`.
  - Return `ImportPackageLocalResult { selector: Some(source_path), id, namespace, name }`.
- [ ] Implement `update_package_metadata(store, selector, input: UpdatePackageMetadataInput) -> Result<UpdatePackageMetadataResult>`:
  - `UpdatePackageMetadataInput`: `{ namespace: Option<String>, name: Option<String>, version: Option<String> }`.
  - Load current boundary via `store.load_package_boundary(&selector)`.
  - Apply non-None fields.
  - Call `store.save_package_boundary_metadata(&updated)`.
  - Return `UpdatePackageMetadataResult { boundary }`.
  - Must not touch `fields`/`types` arrays — metadata only.
- [ ] Add `RepositoryError::PackageAlreadyRegistered { id: String }` to `error.rs`.

#### Acceptance Criteria

- [ ] `create_package` uses boundary methods, not raw manifest/packageRefs writes.
- [ ] `list_packages` uses `list_package_boundaries`, not `load_package_json`.
- [ ] `import_package_local` is callable through `&dyn RepositoryStore`.
- [ ] `update_package_metadata` only touches id/namespace/name/version — never field/type arrays.
- [ ] No service function in this phase calls `std::fs`.
- [ ] Duplicate import returns deterministic `PackageAlreadyRegistered` error.

#### Testing — add to `package_service.rs` `#[cfg(test)]`

- `create_package_auto_registers_boundary` — after `create_package`, `list_package_boundaries` returns the new boundary.
- `create_package_rejects_primary_path` — `boundary_path = "package"` returns error.
- `import_package_local_registers_logical_package` — import a pre-seeded sub-package path; `list_packages` returns it.
- `import_package_local_rejects_duplicate` — second import of same `id` returns `PackageAlreadyRegistered`.
- `import_package_local_rejects_missing_source` — missing `package.json` returns error.
- `update_package_metadata_does_not_rewrite_definitions` — add a field path, call `update_package_metadata`, verify field path still present.
- `update_package_metadata_changes_name` — name update is visible in `list_packages`.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 4: Package-Aware Definition Services

**Goal:** Field/type CRUD uses boundary methods so there are no raw filename assumptions.

**Agent:** Package Service Worker

#### Current state

- `create_field`, `create_type` — write to primary package only; index update still calls `load_package_json`/`save_package_json` directly.
- `find_field_path`, `find_type_path` — use `load_package_json` + `load_instance_json` walk. Migrate to `resolve_definition_owner`.
- `update_field`, `delete_field`, `update_type`, `delete_type` — use `find_field_path`/`find_type_path`; should migrate to boundary methods after Phase 2.
- `list_fields_internal`, `list_types_internal` — build provenance by walking `load_package_json`. Migrate to `list_package_boundaries`.

#### Tasks

- [ ] Migrate `create_field` to accept `selector: PackageSelector` (default `None`). Use
  `store.add_definition_to_boundary(&selector, DefinitionKind::Field, &filename)` instead of
  directly mutating the `package.json` array.
- [ ] Migrate `create_type` the same way.
- [ ] Migrate `find_field_path` to use `store.resolve_definition_owner(id, DefinitionKind::Field)`,
  then reconstruct the path from the boundary. Remove the manual walk.
- [ ] Migrate `find_type_path` the same way.
- [ ] Migrate `delete_field` / `delete_type` to call
  `store.remove_definition_from_boundary(&selector, kind, &path)` after deleting the file.
- [ ] Migrate `list_fields_internal` provenance map to use `store.list_package_boundaries()` and
  `boundary.field_paths` instead of calling `load_package_json`.
- [ ] Migrate `list_types_internal` the same way.

#### Acceptance Criteria

- [ ] `create_field` / `create_type` accept an optional package selector; default is primary.
- [ ] No field/type service function calls `load_package_json` or `save_package_json` directly.
- [ ] Field/type update/delete resolve owner through boundary methods.
- [ ] `list_fields` / `list_types` provenance uses `list_package_boundaries`.
- [ ] All existing tests continue to pass.

#### Testing — add to `package_service.rs` `#[cfg(test)]`

- `list_fields_by_package_filters_correctly` — field in sub-package not returned for primary filter.
- `list_types_by_package_filters_correctly` — same for types.
- `list_fields_includes_source_package` — `source_package` in summary matches boundary path.
- `list_types_includes_source_package` — same for types.
- `create_field_in_sub_package` — field created with `selector = Some("pkg/ext")` appears in
  that boundary's `field_paths` and not in the primary.
- `delete_field_removes_from_boundary_index` — after delete, field path gone from boundary.
- `update_field_resolves_owner_package` — field in sub-package can be updated.
- `delete_type_resolves_owner_package` — type in sub-package can be deleted.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 5: CLI

**Goal:** Package lifecycle and package-aware definition queries fully available through thin CLI
handlers. Legacy `enable/disable` deprecated or routed through lifecycle services.

**Agent:** CLI Worker

#### Current state

- `srs package list` — done; output uses `PackageBoundaryInfo` JSON.
- `srs package create` — done.
- `srs field list --package` — done.
- `srs type list --package` — done.
- `srs package enable/disable` — still live; calls raw `add_package_ref`/`remove_package_ref`
  from `manifest_service`. Needs deprecation or routing through `import_package_local` /
  `create_package`.
- `srs package import` — not implemented.
- `srs package update` — not implemented.
- `srs slice create` — not implemented.
- `srs field create` / `srs type create` — do not yet accept `--package`.

#### Tasks

- [ ] `srs package import` — add `PackageCommand::Import { path: String }` variant to `mod.rs`.
  Handler calls `import_package_local(store, ImportPackageLocalInput { source_path: path })`.
  Output: `{ "selector": "...", "id": "...", "namespace": "...", "name": "..." }`.
- [ ] `srs package update` — add `PackageCommand::Update { selector: Option<String>, namespace: Option<String>, name: Option<String>, version: Option<String> }`.
  `--selector` omitted = primary package.
  Handler calls `update_package_metadata`. Output: updated boundary JSON.
- [ ] `srs slice create` — add `PackageCommand::SliceCreate` as an alias variant that calls
  `cmd_package_create` internally. Both commands appear in help. Document in `CLAUDE.md`.
- [ ] `srs field create --package <path>` — add `--package` arg to `FieldCommand::Create`.
  Pass `selector` to `create_field`.
- [ ] `srs type create --package <path>` — same for `TypeCommand::Create`.
- [ ] Deprecate `package enable/disable`: add `#[arg(hide = true)]` to both variants and add a
  note in their help text pointing to `package import`. Keep the implementations working for now —
  do not remove.
- [ ] Update `CLAUDE.md` CLI reference section: add `srs package import`, `srs package update`,
  `srs slice create`; document `--package` on `field create` and `type create`; mark
  `enable/disable` as deprecated.

#### Acceptance Criteria

- [ ] `srs package import` creates and registers a boundary from an existing local path.
- [ ] `srs package update` changes metadata only; does not alter definition arrays.
- [ ] `srs slice create` is callable and produces the same result as `srs package create`.
- [ ] `srs field create --package` / `srs type create --package` route to the correct boundary.
- [ ] `srs package enable/disable` still work but are hidden from default help.
- [ ] All output envelopes remain stable JSON (no removed keys).

#### Testing — add to `crates/srs-cli/tests/` integration tests

- `package_create_happy_path` — `srs package create --id ... --namespace ... --name ... --path pkg/ext` exits 0; `ok: true`; `boundaryPath` present.
- `package_import_local_happy_path` — seed a directory with a `package.json`, run `srs package import --path pkg/seed`, verify boundary listed in `srs package list`.
- `package_update_metadata_only` — create package, update name, verify new name; verify field count unchanged.
- `slice_create_matches_package_create` — same args to both commands produce equivalent output shapes.
- `field_create_in_sub_package` — `srs field create --package pkg/ext` routes to sub-package boundary.
- `field_list_with_package_filter` — fields in sub-package appear only with matching `--package`.
- `type_list_with_package_filter` — same for types.

#### Milestone gate

```bash
cargo test
cargo clippy -- -D warnings
```

---

### Phase 6: Test Coverage for Existing Work

**Goal:** All service and store behaviors introduced in the partial Phase 3/4 implementation have
test coverage, not just the new Phase 2–5 work.

**Agent:** Verification

#### Tasks

- [ ] Verify the following tests exist and pass (write them if absent):
  - `package_service::tests::list_packages_returns_primary_and_sub_packages`
  - `package_service::tests::list_fields_by_package_filters_correctly`
  - `package_service::tests::list_types_by_package_filters_correctly`
  - `package_service::tests::list_fields_includes_source_package`
  - `package_service::tests::list_types_includes_source_package`
  - `package_service::tests::create_package_auto_registers_boundary`
  - `package_service::tests::create_package_rejects_primary_path`
  - `package_service::tests::field_delete_removes_from_package_json` (already exists, verify)
  - `package_service::tests::type_delete_removes_from_package_json` (already exists, verify)
  - `store::tests::memory_store_save_field_uses_package_prefix_key` — proves MemoryStore stores at `package/fields/...` not `fields/...`.
  - `store::tests::json_store_field_roundtrip` — JsonStore create_field / list_fields roundtrip.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] `ARCHITECTURE.md` documents logical package boundaries.
- [ ] ADR is created or explicitly deferred with rationale in ARCHITECTURE.md.
- [ ] No service function calls `load_package_json` or `save_package_json` directly.
- [ ] MemoryStore package state is keyed by `PackageSelector`, not path strings.
- [ ] FileStore preserves current package layout.
- [ ] JsonStore implements all boundary methods.
- [ ] Field/type package provenance and package filtering work.
- [ ] `srs package import`, `srs package update`, `srs slice create` are available in the CLI.
- [ ] `srs field create --package` and `srs type create --package` are available.
- [ ] `srs package enable/disable` are deprecated (hidden, with migration hint).
- [ ] A future SQL adapter can implement all boundary methods without changing service APIs.

---

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behavior summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass,
  update the plan checkboxes, then commit.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Repository lifecycle and full-repository portability are handled by `storage-agnostic-repository-lifecycle.md`.
- Container storage alignment is handled by `storage-agnostic-container-boundaries.md`.
- The current file-backed package layout remains supported.
- `package import` v1 supports local source only.
- `package update` v1 is metadata/binding only.
- `slice create` is a CLI alias for package creation in this phase.
- SQL implementation is not part of this plan.
- Phase 2 must land before Phase 3 migration tasks and Phase 4 migration tasks begin.
  Phase 3 `import_package_local` and `update_package_metadata` can be implemented
  before Phase 2 if they are written to use the current `load_instance_json`/`save_instance_json`
  interface, then migrated in Phase 3's migration tasks.
