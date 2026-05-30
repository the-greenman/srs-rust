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
| 6: Test Coverage | Minimal — most planned tests not written |

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

The Documentation Worker must either create `docs/adr/0002-package-boundary-model.md` or add an
explicit note to `ARCHITECTURE.md` stating why a separate ADR is not needed. This must be resolved
before Phase 2 begins — Phase 1's milestone gate blocks Phase 2.

---

## Scope

- Add `PackageSelector`, `PackageBoundary`, `DefinitionKind`, `OwnedField`, `OwnedType` types.
- Add `list_package_boundaries`, `load_package_boundary`, `save_package_boundary_metadata`,
  `register_package_boundary`, `add_definition_to_boundary`, `remove_definition_from_boundary`,
  `resolve_definition_owner` to `RepositoryStore`.
- Implement new methods for FileStore, MemoryStore, and JsonStore.
- Migrate package services off raw `load_package_json`/`save_package_json` calls.
- Add `import_package_local` and `update_package_metadata` services.
- Add field/type create targeting a specific package boundary.
- Add CLI commands: `srs package import`, `srs package update`, `srs slice create`.
- Deprecate `srs package enable/disable` (hide from help, leave working).
- Write all tests specified in each phase.

**Out of scope:**

- Repository creation and full-repository portability (covered by lifecycle plan).
- Container storage alignment (covered by container plan).
- SQL adapter implementation.
- Registry or network-backed package import.
- Repository slice export/import from RFC-003.
- Changing the current file-backed package layout.
- `slice create` diverging semantically from `package create` — it is a permanent alias in this plan.

---

## Known Limitations

**`resolve_definition_owner` is O(n×m):** The implementation walks every boundary, loads every
definition file, and compares the `id` field. This is correct but linear. For large repos with many
packages and definitions it will be slow. The trait method doc must note this; implementors may
cache. A future SQL adapter may index by ID. This limitation is acceptable for v1.

---

## Phases

### Phase 1: Architecture Contract

**Goal:** The package boundary model is documented before implementation. Phase 2 does not begin
until this phase's milestone gate passes.

**Agent:** Lead Integrator + Documentation Worker

#### Tasks

- [x] Update `srs-rust/ARCHITECTURE.md` with package boundary rules.
- [x] State that package services use selectors and adapters use files/tables.
- [x] Document packages as definition/meta boundaries.
- [x] Document that raw `package.json`, package paths, and package file indexes must not appear in service APIs.
- [ ] Resolve the ADR: either create `docs/adr/0002-package-boundary-model.md` or add a paragraph
  to `ARCHITECTURE.md` under a new "## Why No Separate Package ADR" heading explaining that the
  Package Boundaries section is sufficient.

#### Acceptance Criteria

- [x] `ARCHITECTURE.md` names packages as logical definition boundaries.
- [x] `ARCHITECTURE.md` prohibits path-shaped package service APIs.
- [ ] ADR is created, or a written rationale for deferral exists in `ARCHITECTURE.md`.

#### Milestone gate (blocks Phase 2)

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 2: Package Boundary Types and Store Contract

**Goal:** `RepositoryStore` exposes logical package operations. After this phase, services can be
migrated off raw `load_package_json`/`save_package_json` calls.

**Prerequisite:** Phase 1 milestone gate must pass before this phase begins.

**Agent:** Package Boundary Worker

---

#### New types — `crates/srs-repository/src/package_types.rs` (new file)

These types must be in a non-test module so they can appear in `RepositoryStore` trait signatures.
`MemoryStore` is `#[cfg(test)]` but the trait and its methods are not — the types must be
available in production scope.

```rust
/// Identifies a package boundary within a repository.
/// `None` = primary package ("package/"); `Some(path)` = sub-package boundary path.
pub type PackageSelector = Option<String>;

/// Metadata describing one package boundary. Does not include definition content.
#[derive(Debug, Clone)]
pub struct PackageBoundary {
    pub selector: PackageSelector,   // None = primary
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub field_paths: Vec<String>,    // package-relative paths (e.g. "fields/foo.json")
    pub type_paths: Vec<String>,     // package-relative paths
}

/// Which definitions are tracked per boundary.
/// `RelationType` is included so relation type definitions can be tracked if needed in future,
/// but boundary methods are only required to handle `Field` and `Type` in this plan.
/// `View` and `DocumentView` are tracked for completeness; implementations may treat them
/// as no-ops if not yet needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind {
    Field,
    Type,
    View,
    DocumentView,
    RelationType,
}

/// A field carried with its source boundary. Used by merged listing services.
#[derive(Debug, Clone)]
pub struct OwnedField {
    pub field: srs_core::types::field::Field,
    pub owner: PackageSelector,
}

/// A type carried with its source boundary.
#[derive(Debug, Clone)]
pub struct OwnedType {
    pub record_type: srs_core::types::record_type::RecordType,
    pub owner: PackageSelector,
}
```

Export from `lib.rs`:
```rust
pub mod package_types;
pub use package_types::{DefinitionKind, OwnedField, OwnedType, PackageBoundary, PackageSelector};
```

---

#### New `RepositoryStore` trait methods

Add after the `// --- Package index ---` block in `store.rs`. The `DefinitionKind` doc must include
the O(n×m) complexity warning on `resolve_definition_owner`.

```rust
// --- Package boundaries ---

/// Return metadata for all package boundaries (primary + all registered sub-packages).
fn list_package_boundaries(&self) -> Result<Vec<PackageBoundary>, RepositoryError>;

/// Return metadata for one boundary. Returns `PackageNotFound` if missing.
fn load_package_boundary(&self, selector: &PackageSelector) -> Result<PackageBoundary, RepositoryError>;

/// Persist metadata fields (id, namespace, name, version) for one boundary.
/// Creates the boundary storage (package.json / table row) if it does not exist.
/// Must not overwrite field_paths or type_paths.
fn save_package_boundary_metadata(&self, boundary: &PackageBoundary) -> Result<(), RepositoryError>;

/// Register a boundary so it is visible in list_package_boundaries.
/// For sub-packages: adds to manifest packageRefs if absent.
/// For the primary package (selector = None): no-op.
/// Idempotent — calling twice with the same selector is not an error.
fn register_package_boundary(&self, selector: &PackageSelector) -> Result<(), RepositoryError>;

/// Append a definition path to the boundary's index for the given kind.
/// Path is package-relative (e.g. "fields/foo-abc123.json").
/// Idempotent — appending a path that is already present is not an error.
fn add_definition_to_boundary(
    &self,
    selector: &PackageSelector,
    kind: DefinitionKind,
    path: &str,
) -> Result<(), RepositoryError>;

/// Remove a definition path from the boundary's index.
/// No-op if the path is not present.
fn remove_definition_from_boundary(
    &self,
    selector: &PackageSelector,
    kind: DefinitionKind,
    path: &str,
) -> Result<(), RepositoryError>;

/// Return the selector of the boundary that owns the definition with the given id.
/// Returns `DefinitionNotFound` if no boundary contains a definition with that id.
///
/// # Performance
/// This is a linear scan: O(boundaries × definitions_per_boundary). Each definition
/// file is loaded and its `id` field compared. Implementors may cache if performance
/// is a concern; the trait makes no caching guarantee.
fn resolve_definition_owner(
    &self,
    id: &str,
    kind: DefinitionKind,
) -> Result<PackageSelector, RepositoryError>;
```

---

#### New `RepositoryError` variants — add to `error.rs`

```rust
/// A package boundary with the given selector was not found.
PackageNotFound { selector: PackageSelector },

/// A definition with the given id was not found in any boundary.
DefinitionNotFound { id: String },
```

(`PackageAlreadyRegistered` is added in Phase 3.)

---

#### FileStore implementation

- `list_package_boundaries`: call `load_package_json()` for primary; iterate `packageRefs` from
  manifest and call `load_instance_json("{path}/package.json")` for each. Build `PackageBoundary`
  from each JSON object's `id`, `namespace`, `name`, `version`, `fields` array, `types` array.
- `load_package_boundary(None)`: read `package/package.json`. Return `PackageNotFound` if missing.
- `load_package_boundary(Some(path))`: read `{path}/package.json`. Return `PackageNotFound` if missing.
- `save_package_boundary_metadata(b)`: load the appropriate `package.json`, overwrite `id`,
  `namespace`, `name`, `version` keys only, write back. Must not touch `fields`/`types` arrays.
- `register_package_boundary(None)`: no-op, return `Ok(())`.
- `register_package_boundary(Some(path))`: load manifest, check `packageRefs` for existing entry
  with same path, append `{mode: "local", path}` if absent, save manifest.
- `add_definition_to_boundary(sel, kind, path)`: load the appropriate `package.json`, push path
  into the correct array (`fields`/`types`/`views`/`documentViews`/`relationTypes`) if absent,
  write back.
- `remove_definition_from_boundary(sel, kind, path)`: load, filter, write back.
- `resolve_definition_owner(id, kind)`: call `list_package_boundaries()`, iterate; for each
  boundary, iterate paths for the given kind, call `load_instance_json` for each path (prefixed
  with the boundary's filesystem prefix: `package/` for primary, `{path}/` for sub-packages),
  compare `val["id"]`. Return first match. Return `DefinitionNotFound` if exhausted.

---

#### MemoryStore implementation

MemoryStore lives in `#[cfg(test)]` in `store.rs`. Add a `boundaries` field:

```rust
boundaries: RefCell<HashMap<Option<String>, PackageBoundary>>,
```

Pre-populate in `MemoryStore::empty()` with a primary boundary (`None` → empty `PackageBoundary`).

- `list_package_boundaries`: return `boundaries.borrow().values().cloned().collect()`.
- `load_package_boundary(sel)`: lookup by key, return `PackageNotFound` if absent.
- `save_package_boundary_metadata(b)`: update the `id`/`namespace`/`name`/`version` fields of the
  existing entry; do not replace `field_paths`/`type_paths`.
- `register_package_boundary(None)`: no-op.
- `register_package_boundary(Some(path))`: insert a new empty `PackageBoundary` entry if absent;
  also add to manifest `packageRefs`.
- `add_definition_to_boundary(sel, kind, path)`: push to `field_paths` or `type_paths` of the
  boundary entry if not already present. Return `PackageNotFound` if selector absent.
- `remove_definition_from_boundary(sel, kind, path)`: filter the vec. No-op if absent.
- `resolve_definition_owner(id, kind)`: iterate `boundaries`; for each boundary, iterate the
  appropriate paths, load `data["package/{path}"]` (primary) or `data["{boundary_path}/{path}"]`
  (sub-package), compare `val["id"]`. Return first match or `DefinitionNotFound`.

---

#### JsonStore implementation

JsonStore stores everything in its `data: HashMap<String, serde_json::Value>`. It has no separate
`boundaries` struct — boundaries are reconstructed from the `data` entries, the same way FileStore
reconstructs from disk.

- Implement all seven methods with the same logic as FileStore, but substituting
  `self.load_instance_json(path)?` for filesystem reads and `self.save_instance_json(path, val)?`
  for writes. The flush-on-every-mutation behavior of JsonStore handles persistence automatically.

---

#### Phase 2 Tasks

- [ ] Create `crates/srs-repository/src/package_types.rs` with `PackageSelector`, `PackageBoundary`,
  `DefinitionKind`, `OwnedField`, `OwnedType`. **Not gated behind `#[cfg(test)]`.**
- [ ] Add `pub mod package_types;` and re-exports to `lib.rs`.
- [ ] Add `PackageNotFound` and `DefinitionNotFound` variants to `error.rs`.
- [ ] Add the seven new methods to the `RepositoryStore` trait with the doc comments above.
- [ ] Implement all seven methods on `FileStore`.
- [ ] Add `boundaries: RefCell<HashMap<Option<String>, PackageBoundary>>` to `MemoryStore` and
  implement all seven methods. Pre-populate primary boundary in `empty()` / `with_field()` /
  `with_type()`.
- [ ] Implement all seven methods on `JsonStore`.

#### Phase 2 Acceptance Criteria

- [ ] `package_types.rs` compiles outside `#[cfg(test)]`.
- [ ] All three stores compile with no warnings after adding the seven methods.
- [ ] MemoryStore `boundaries` is keyed by `PackageSelector`, not by path strings.
- [ ] FileStore preserves current `package/` and `packageRefs` layout.
- [ ] JsonStore implements all seven methods using `load_instance_json`/`save_instance_json`.
- [ ] `resolve_definition_owner` doc includes the O(n×m) complexity note.
- [ ] `DefinitionKind::RelationType` is present but implementations may treat it as a no-op for
  the `field_paths`/`type_paths` tracking arrays; this is acceptable for Phase 2.

#### Phase 2 Testing — add to `store.rs` `#[cfg(test)]`

- `memory_store_package_types_are_not_cfg_test` — compile-time: `PackageBoundary` used from a
  non-test module (if possible to assert structurally; otherwise document and skip).
- `memory_store_list_package_boundaries_returns_primary` — fresh `MemoryStore` has one boundary
  with selector `None`.
- `memory_store_register_sub_package_adds_to_boundaries` — `register_package_boundary(Some("pkg/ext".into()))` makes it visible in `list_package_boundaries`.
- `memory_store_add_definition_to_boundary_updates_field_paths` — `add_definition_to_boundary(None, Field, "fields/foo.json")` appears in `load_package_boundary(None).field_paths`.
- `memory_store_add_definition_idempotent` — calling twice does not duplicate the path.
- `memory_store_remove_definition_from_boundary` — path is gone after remove; no error if absent.
- `memory_store_resolve_definition_owner_primary` — a field stored via `save_field` in primary boundary resolves to `None`.
- `memory_store_resolve_definition_owner_sub_package` — a field registered in a sub-package resolves to `Some(path)`.
- `memory_store_resolve_definition_not_found` — unknown id returns `DefinitionNotFound`.
- `memory_store_save_boundary_metadata_does_not_overwrite_paths` — save metadata with different name; `field_paths` unchanged.
- `file_store_list_package_boundaries_returns_primary` — FileStore on a minimal fixture returns at least one boundary.
- `file_store_register_sub_package_adds_to_manifest` — after `register_package_boundary(Some("pkg/ext"))`, manifest `packageRefs` contains the entry.
- `json_store_package_boundaries_roundtrip` — register sub-package, add field path, drop store,
  open new `JsonStore::open`, verify boundary and path present. Use `tempfile::TempDir`.

#### Phase 2 Milestone gate (blocks Phase 3 migration tasks and Phase 4 migration tasks)

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 3: Package Lifecycle Services

**Goal:** Package lifecycle is fully implemented through boundary methods. `import_package_local`
and `update_package_metadata` are added. Services do not call `load_package_json` directly.

**Prerequisite:** Phase 2 milestone gate must pass before migration tasks begin. `import_package_local`
and `update_package_metadata` may be implemented before Phase 2 using current store methods, then
migrated once Phase 2 lands.

**Agent:** Package Service Worker

---

#### `import_package_local` — clarification on source_path

`source_path` is a path **relative to the repository root** pointing to a directory that already
contains a `package.json`. For FileStore this means the directory exists on disk. The service reads
`store.load_instance_json("{source_path}/package.json")` — no filesystem operations directly.

The service does **not** copy any files. It reads the existing `package.json`, validates it,
extracts metadata, and registers the boundary. If the directory or `package.json` does not exist,
`load_instance_json` will return an error that the service propagates.

---

#### `create_package` idempotency — explicit policy

`create_package` with a path that is already registered should return
`RepositoryError::PackageAlreadyRegistered { id }`. This is **not** idempotent by design — calling
it twice is a caller error. The existing implementation checks for duplicate registration in the
manifest ref list (silently skips) but does not check the package id. After Phase 3 migration,
`create_package` must explicitly check via `list_package_boundaries` whether a boundary with the
same path or same id is already registered and return `PackageAlreadyRegistered` if so.

---

#### Phase 3 Tasks

- [ ] Migrate `create_package` to call `store.register_package_boundary` and
  `store.save_package_boundary_metadata`. Add explicit duplicate check (same path or id already
  registered → `PackageAlreadyRegistered`).
- [ ] Migrate `list_packages` to call `store.list_package_boundaries`. Map `PackageBoundary` to
  `PackageBoundaryInfo`. Remove direct `load_package_json` call.
- [ ] Implement `import_package_local`:
  - Input: `ImportPackageLocalInput { source_path: String }`.
  - Read `store.load_instance_json("{source_path}/package.json")` — no `std::fs`.
  - Extract `id`, `namespace`, `name`, `version`. Return `PackageLoad` error if missing fields.
  - Check `store.list_package_boundaries()` for an existing boundary with the same `id`. Return
    `PackageAlreadyRegistered { id }` if found.
  - Call `store.register_package_boundary(Some(source_path.clone()))`.
  - Output: `ImportPackageLocalResult { selector: Some(source_path), id, namespace, name }`.
- [ ] Implement `update_package_metadata`:
  - Input: `UpdatePackageMetadataInput { selector: PackageSelector, namespace: Option<String>, name: Option<String>, version: Option<String> }`.
  - Call `store.load_package_boundary(&input.selector)`. Return `PackageNotFound` if absent.
  - Patch non-None fields onto the loaded `PackageBoundary`.
  - Call `store.save_package_boundary_metadata(&patched)`.
  - Output: `UpdatePackageMetadataResult { boundary: PackageBoundary }`.
  - `field_paths` and `type_paths` must be unchanged.
- [ ] Add `RepositoryError::PackageAlreadyRegistered { id: String }` to `error.rs`.

#### Phase 3 Acceptance Criteria

- [ ] `create_package` uses boundary methods; returns `PackageAlreadyRegistered` on duplicate.
- [ ] `list_packages` uses `list_package_boundaries`; no direct `load_package_json` call.
- [ ] `import_package_local` callable through `&dyn RepositoryStore`; no `std::fs` calls.
- [ ] `update_package_metadata` only patches id/namespace/name/version; `field_paths`/`type_paths` unchanged.
- [ ] Duplicate `create_package` or `import_package_local` returns `PackageAlreadyRegistered`.

#### Phase 3 Testing

- `create_package_auto_registers_boundary` — after `create_package`, `list_package_boundaries` returns the new boundary.
- `create_package_rejects_primary_path` — `boundary_path = "package"` returns error.
- `create_package_rejects_duplicate_path` — same path twice returns `PackageAlreadyRegistered`.
- `import_package_local_registers_logical_package` — seeded path; `list_packages` returns it after import.
- `import_package_local_rejects_duplicate_id` — import same id twice returns `PackageAlreadyRegistered`.
- `import_package_local_rejects_missing_source` — nonexistent path returns `PackageLoad` or `Io` error.
- `update_package_metadata_changes_name` — new name visible in `list_packages`.
- `update_package_metadata_does_not_rewrite_paths` — add a field path first; call update; field path still present.
- `update_package_metadata_rejects_missing_boundary` — unknown selector returns `PackageNotFound`.

#### Phase 3 Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 4: Package-Aware Definition Services

**Goal:** Field/type CRUD uses boundary methods. No raw `load_package_json`/`save_package_json`
calls remain in definition services.

**Prerequisite:** Phase 2 milestone gate must pass before migration tasks begin.

**Agent:** Package Service Worker

---

#### `create_field` / `create_type` signature change — migration strategy

Adding `selector: PackageSelector` to `create_field` and `create_type` is a breaking change for
all existing callers. To avoid a compile flag day between Phase 4 (service) and Phase 5 (CLI):

1. Add a new function `create_field_in_package(store, field, selector: PackageSelector)` that
   implements the boundary-aware logic.
2. Keep `create_field(store, field)` as a thin wrapper calling
   `create_field_in_package(store, field, None)`.
3. Phase 5 updates CLI handlers to call `create_field_in_package` with the user-supplied selector.
4. After Phase 5 is complete, `create_field` can be removed or kept as a convenience wrapper.

Apply the same pattern to `create_type`.

---

#### Phase 4 Tasks

- [ ] Add `create_field_in_package(store, field, selector: PackageSelector) -> Result<CreateFieldResult>`:
  - Call `store.ensure_fields_dir()` (or boundary-equivalent).
  - Compute filename as today.
  - Call `store.save_field(&filename, &field)` with the appropriate prefix for the boundary.
  - Call `store.add_definition_to_boundary(&selector, DefinitionKind::Field, &filename)`.
  - Return `CreateFieldResult`.
- [ ] Add `create_type_in_package` with the same pattern.
- [ ] Keep `create_field` and `create_type` as wrappers calling `*_in_package(..., None)`.
- [ ] Migrate `find_field_path` to use `store.resolve_definition_owner(id, DefinitionKind::Field)`,
  then reconstruct the full path from the returned selector. Remove the manual walk.
- [ ] Migrate `find_type_path` the same way.
- [ ] Migrate `delete_field` to call `store.remove_definition_from_boundary` after deleting the file.
- [ ] Migrate `delete_type` the same way.
- [ ] Migrate `list_fields_internal` to build provenance from `store.list_package_boundaries()`
  and `boundary.field_paths` instead of calling `load_package_json`.
- [ ] Migrate `list_types_internal` the same way.
- [ ] Verify no field/type service function calls `load_package_json` or `save_package_json`.

#### Phase 4 Acceptance Criteria

- [ ] `create_field_in_package` / `create_type_in_package` exist and accept a selector.
- [ ] `create_field` / `create_type` remain as wrappers; existing CLI callers compile unchanged.
- [ ] No field/type service function calls `load_package_json` or `save_package_json`.
- [ ] `find_field_path` / `find_type_path` use `resolve_definition_owner`.
- [ ] `delete_field` / `delete_type` call `remove_definition_from_boundary`.
- [ ] All existing tests continue to pass.

#### Phase 4 Testing

- `create_field_in_sub_package` — field created with `selector = Some("pkg/ext".into())` appears
  in that boundary's `field_paths` and not in the primary.
- `create_field_default_selector_targets_primary` — `create_field` (no selector) goes to primary.
- `delete_field_removes_from_boundary_index` — after delete, path gone from boundary `field_paths`.
- `delete_type_removes_from_boundary_index` — same for types.
- `update_field_resolves_owner_via_boundary_methods` — update a field in a sub-package succeeds.
- `update_type_resolves_owner_via_boundary_methods` — same for types.
- `list_fields_by_package_filters_correctly` — field in sub-package not returned for primary filter.
- `list_types_by_package_filters_correctly` — same for types.
- `list_fields_includes_source_package` — `source_package` in summary matches boundary path.
- `list_types_includes_source_package` — same for types.

#### Phase 4 Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 5: CLI

**Goal:** Package lifecycle and package-aware definition queries fully available. Legacy
`enable/disable` hidden.

**Prerequisite:** Phase 4 milestone gate (all service changes complete and tested).

**Agent:** CLI Worker

---

#### `srs slice create` — permanent alias

`slice create` is a permanent CLI alias for `package create` and will not diverge semantically.
It exists to let users think in terms of "definition slices" without exposing the word "package"
in that context. The implementation calls the same `cmd_package_create` function directly — no
separate service logic. This is documented in `CLAUDE.md`.

---

#### Phase 5 Tasks

- [ ] `srs package import`: add `PackageCommand::Import { #[arg(long)] path: String }` to `mod.rs`.
  Handler in `package.rs` calls `import_package_local`. Output envelope:
  `{ "selector": ..., "id": ..., "namespace": ..., "name": ... }`.
- [ ] `srs package update`: add `PackageCommand::Update { #[arg(long)] selector: Option<String>, #[arg(long)] namespace: Option<String>, #[arg(long)] name: Option<String>, #[arg(long)] version: Option<String> }`.
  Omitted `--selector` maps to `None` (primary package). Handler calls `update_package_metadata`.
  Output: updated boundary as JSON object.
- [ ] `srs slice create`: add `PackageCommand::SliceCreate { ... }` with the same args as
  `PackageCommand::Create`. Dispatch arm calls `cmd_package_create` with the same arguments.
- [ ] `srs field create --package`: add `#[arg(long)] package: Option<String>` to
  `FieldCommand::Create`. Dispatch passes selector to `create_field_in_package`.
- [ ] `srs type create --package`: same for `TypeCommand::Create` and `create_type_in_package`.
- [ ] Deprecate `package enable/disable`: add `#[arg(hide = true)]` and update help strings to
  "Deprecated: use `srs package import` instead." Keep implementations.
- [ ] Update `CLAUDE.md`: add `srs package import`, `srs package update`, `srs slice create`;
  document `--package` on `field create` and `type create`; mark `enable/disable` as deprecated.

#### Phase 5 Acceptance Criteria

- [ ] `srs package import --path <rel>` creates and registers a boundary from an existing path.
- [ ] `srs package update` changes name/namespace/version only; field/type counts unchanged.
- [ ] `srs slice create` with same args as `srs package create` produces identical output shape.
- [ ] `srs field create --package pkg/ext` calls `create_field_in_package(..., Some("pkg/ext"))`.
- [ ] `srs type create --package pkg/ext` calls `create_type_in_package(..., Some("pkg/ext"))`.
- [ ] `srs package enable/disable` still exit 0 but are hidden from `--help`.
- [ ] All output envelopes are stable JSON (no removed keys).
- [ ] `CLAUDE.md` reflects all changes.

#### Phase 5 Testing — `crates/srs-cli/tests/`

- `package_create_happy_path` — exits 0; `ok: true`; `boundaryPath` present in payload.
- `package_import_local_happy_path` — seed a directory with a valid `package.json`, import it,
  verify it appears in `srs package list` output.
- `package_update_metadata_only` — create package, update `--name new-name`, verify name changed
  and `fieldCount` / `typeCount` unchanged in `srs package list`.
- `slice_create_output_matches_package_create` — same args; both return `ok: true` with
  `boundaryPath` present.
- `field_create_in_sub_package` — `srs field create --package pkg/ext < field.json` routes to
  sub-package; `srs field list --package pkg/ext` returns it.
- `field_list_with_package_filter` — fields in sub-package absent from `srs field list` without filter; present with `--package`.
- `type_list_with_package_filter` — same for types.

#### Phase 5 Milestone gate

```bash
cargo test
cargo clippy -- -D warnings
```

---

### Phase 6: Test Coverage for Existing Work

**Goal:** All behaviors implemented in the partial Phase 3/4 work that landed before Phase 2 have
explicit test coverage. This phase catches any invariants that slipped through.

**Prerequisite:** Phases 3, 4, 5 milestone gates passed.

**Agent:** Verification

---

#### Phase 6 Tasks

- [ ] Verify or write:
  - `package_service::tests::list_packages_returns_primary_and_sub_packages`
  - `package_service::tests::list_fields_by_package_filters_correctly` (may be written in Phase 4)
  - `package_service::tests::list_types_by_package_filters_correctly` (may be written in Phase 4)
  - `package_service::tests::list_fields_includes_source_package` (may be written in Phase 4)
  - `package_service::tests::list_types_includes_source_package` (may be written in Phase 4)
  - `package_service::tests::create_package_auto_registers_boundary` (may be written in Phase 3)
  - `package_service::tests::create_package_rejects_primary_path` (may be written in Phase 3)
  - `package_service::tests::field_delete_removes_from_package_json` (exists, verify still passes)
  - `package_service::tests::type_delete_removes_from_package_json` (exists, verify still passes)
  - `store::tests::memory_store_save_field_uses_package_prefix_key` — asserts that after
    `save_field("fields/foo.json", ...)`, the MemoryStore data map contains the key
    `"package/fields/foo.json"` and not `"fields/foo.json"`. This invariant is load-bearing for
    `resolve_definition_owner` correctness and **should be promoted to Phase 2 acceptance criteria**
    if not already present.
  - `store::tests::json_store_field_roundtrip` — `create_field` then `list_fields` via JsonStore.

#### Phase 6 Milestone gate (Final Acceptance gate)

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
- [ ] ADR created or deferral explicitly documented in `ARCHITECTURE.md`.
- [ ] `package_types.rs` is not gated behind `#[cfg(test)]`.
- [ ] No service function calls `load_package_json` or `save_package_json` directly.
- [ ] MemoryStore `boundaries` is keyed by `PackageSelector`, not path strings.
- [ ] FileStore preserves current `package/` and `packageRefs` layout.
- [ ] JsonStore implements all seven boundary methods.
- [ ] `resolve_definition_owner` doc contains the O(n×m) complexity note.
- [ ] `DefinitionKind::RelationType` is present in the enum; implementations may treat as no-op.
- [ ] `create_package` returns `PackageAlreadyRegistered` on duplicate.
- [ ] `import_package_local` is implemented and contains no `std::fs` calls.
- [ ] `update_package_metadata` does not touch `field_paths` or `type_paths`.
- [ ] `create_field_in_package` / `create_type_in_package` exist with selector parameter.
- [ ] `create_field` / `create_type` remain as wrappers; all existing callers compile unchanged.
- [ ] `srs package import`, `srs package update`, `srs slice create` available in CLI.
- [ ] `srs field create --package` and `srs type create --package` available.
- [ ] `srs package enable/disable` hidden from help, still functional.
- [ ] `memory_store_save_field_uses_package_prefix_key` test exists and passes.
- [ ] `CLAUDE.md` updated with new commands.
- [ ] A future SQL adapter can implement all seven boundary methods without changing service APIs.

---

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behavior summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass,
  update the plan checkboxes, then commit.
- Verification Agent runs after each major phase and before final sign-off.

## Ordering Constraints

```
Phase 1 → Phase 2 → Phase 3 (migration tasks) → Phase 4 → Phase 5 → Phase 6
                  ↘ Phase 3 (import/update new code, no migration) can start in parallel
```

Phase 3's `import_package_local` and `update_package_metadata` implementations can be written
against the current store interface before Phase 2 lands, then their internals migrated to boundary
methods once Phase 2 is complete.

## Assumptions

- The current file-backed package layout remains supported throughout.
- `package import` v1 supports local source only (no registry/network).
- `package update` v1 is metadata/binding only (no definition sync).
- `slice create` is a permanent CLI alias for `package create`; it will not diverge semantically.
- SQL implementation is not part of this plan.
- `DefinitionKind::View`, `DefinitionKind::DocumentView`, `DefinitionKind::RelationType` are
  included in the enum for completeness but boundary tracking for those kinds is not required to
  be fully functional in this plan. Implementations may return `Ok(())` for add/remove and skip
  them in `resolve_definition_owner`.
