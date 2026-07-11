# ADR-028: Migration Registry Pattern

- **Status:** proposed
- **Date:** 2026-07-11
- **Supersedes:** —
- **Superseded by:** —

## Context

Repository migrations existed as ad-hoc, unconnected commands:
`repo migrate-identity`, `repo upgrade`, `migrate packet --foundation`, and the
`srsj_migration_service::migrate_rfc014` helper. Each had its own entry point with no shared
model. A caller (especially a web UI) had to already know a migration existed **and** guess
whether a given repo needed it — because no surface existed to ask "which migrations apply to
this repo?"

The srs-web UI (issue srs-web#180) uncovered this gap concretely: the identity migration
was only discoverable by accident via a `repo validate` warning, and the web client had no
way to detect or apply it without the CLI.

Three alternatives were considered for how to register migrations:

1. **Static compile-time `&[T]` array with fn-pointer detect/apply** — Each migration is one
   entry in a global slice. Adding a migration = adding one element; no trait vtable, no
   allocation, no `dyn Trait` overhead.

2. **Dynamic `Vec<Box<dyn Migration>>` built at call time** — Trait-object approach, more
   conventional OO. Requires defining a `Migration` trait, boxing each implementation, and
   building the vec per call or storing it globally behind a `Mutex`.

3. **Macro-generated registry** — Custom `#[register_migration]` proc-macro that appends
   to a link-section list. Avoids touching a central file, but adds macro complexity and
   is harder to read and test.

## Decision

Use **option 1: a static, compile-time `&[MigrationRegistryEntry]` array** in
`crates/srs-repository/src/migration_service.rs`.

Each `MigrationRegistryEntry` holds:
- `id: &'static str` — stable identifier clients store and pass back in apply calls
- `title: &'static str` — short human-readable name
- `description: &'static str` — longer explanation
- `detect: fn(&dyn RepositoryStore) -> Result<MigrationStatus, RepositoryError>` — pure-read applicability check
- `apply: fn(&dyn RepositoryStore) -> Result<serde_json::Value, RepositoryError>` — delegates to the existing typed migration function and serializes its result

`MigrationStatus` is an enum with variants `NotApplicable`, `Needed`, `AlreadyApplied`,
serialized via `#[serde(rename_all = "camelCase")]` to `"notApplicable"`, `"needed"`,
`"alreadyApplied"`.

The two public service functions are:
- `list_migrations(store) -> Result<Vec<MigrationEntry>, RepositoryError>` — iterates the
  registry, runs each detector, returns annotated entries.
- `apply_migration_by_id(store, id) -> Result<serde_json::Value, RepositoryError>` — finds
  the entry by id and calls its apply fn.

`apply_migration_by_id` returns `serde_json::Value` to support the generic interface: each
migration has a different typed result, but the apply fn in the registry converts it to JSON
internally, preserving type safety within each migration module.

Adding a new migration requires only: (a) implement two private fns (`detect_*`, `apply_*`)
in `migration_service.rs`, and (b) add one `MigrationRegistryEntry { ... }` to `MIGRATIONS`.
No CLI, binding, or client changes are needed to list or surface the new migration.

## Consequences

**Positive:**
- Zero allocations for listing migrations; the registry is a `&'static [...]` slice.
- Adding a migration is a single-file change to `migration_service.rs` — no trait impls,
  no `Box`, no registration code scattered across files.
- The detect fn and apply fn are private; only the stable `id` is public. Internal
  implementation changes inside a migration module do not affect the registry contract.
- `apply_migration_by_id` returning `serde_json::Value` keeps the generic interface open
  for future migrations with different result shapes, without modifying any enum.
- `list_migrations` and `apply_migration_by_id` obey ADR-010 (typed inputs/outputs at
  the service boundary) and ADR-013 (same service called by CLI and WASM bindings).

**Negative / trade-offs:**
- The central `MIGRATIONS` slice is a compilation-time list — adding a migration from an
  external crate is not possible without touching this file. This is acceptable: SRS
  migrations are all known at compile time within this monorepo.
- `apply_migration_by_id` loses the specific result type at the registry boundary
  (it returns `serde_json::Value` rather than, e.g., `MigrateIdentityResult`). Callers
  that need the typed result must call the specific service function directly.
- Migration ids become stable client-facing identifiers. If an id must change, all clients
  using the old id will get "unknown migration id" errors. Ids should be treated as
  permanent once published.

**Neutral:**
- The migration `id` format (kebab-case, e.g. `"migrate-identity"`) mirrors the CLI
  command name for discoverability, with no namespace prefix. This is a convention, not
  a guarantee; the contract is only that ids are stable strings.
- `MigrationStatus` is serialized as a `String` in CLI payload structs (to avoid coupling
  `schemars` into `srs-repository`) but as a typed enum with serde in `srs-repository`
  and the WASM binding path.
