# ADR-009: Package Boundary Model

- **Status:** accepted
- **Date:** 2026-05-30
- **Supersedes:** —
- **Superseded by:** —

## Context

Packages were originally treated as filesystem directories: services called
`load_package_json` directly, used raw `packageRefs` paths, and assumed a
`package/` directory layout. This made it impossible to swap the storage
backend (e.g. SQLite, in-memory) without service logic knowing about files.

The goals are:
- Services must address packages through logical selectors, not file paths.
- Storage adapters (FileStore, MemoryStore, JsonStore, future SqlStore) own the
  mapping from logical selector to their own storage representation.
- No service function may call `load_package_json`, `save_package_json`, or
  refer to raw `packageRefs` paths directly.

## Decision

1. A **`PackageSelector`** (`Option<String>`) is the canonical identifier for a
   package boundary. `None` = primary package; `Some(path)` = sub-package.
2. The `RepositoryStore` trait exposes logical boundary methods:
   `list_package_boundaries`, `load_package_boundary`,
   `save_package_boundary_metadata`, `register_package_boundary`,
   `add_definition_to_boundary`, `remove_definition_from_boundary`,
   `resolve_definition_owner`. Services call these instead of raw JSON methods.
3. FileStore maps these methods onto the existing `package/` + `packageRefs`
   layout — the on-disk format is unchanged.
4. MemoryStore stores boundary metadata in a `RefCell<HashMap<PackageSelector,
   PackageBoundary>>` keyed by selector, not by path string.
5. A future SqlStore can implement all boundary methods against tables without
   any service-layer changes.

## Consequences

**Positive:**
- Services are storage-agnostic; any `RepositoryStore` implementor works.
- MemoryStore is a faithful test double without fake file paths.
- The on-disk `package/` layout is preserved, so existing repositories are
  unaffected.
- A SQL adapter can be added later without touching service logic.

**Negative / trade-offs:**
- `resolve_definition_owner` is O(n×m) in the FileStore/MemoryStore
  implementations (walks each boundary, loads each definition file). SQL
  adapters may use an index. This is acceptable for current repository sizes.
- Existing call sites that use `load_package_json` / `save_package_json`
  directly must be migrated (Phase 3 and 4 of the storage-agnostic package
  management plan).

**Neutral:**
- `DefinitionKind` includes `View`, `DocumentView`, and `RelationType` variants
  for completeness; implementations in this plan treat them as no-ops and may
  implement them in a future phase.
- `PackageSelector = Option<String>` is a type alias, not a newtype, for
  ergonomics. This may be revisited if disambiguation becomes necessary.
