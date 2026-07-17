# ADR-032: Migration Registry — Static Function-Pointer Pattern

- **Status:** accepted
- **Date:** 2026-07-17
- **Supersedes:** —
- **Superseded by:** —

## Context

Repository migrations (`migrate-identity`, `repo upgrade`) existed as independent, ad-hoc entry points with no shared discovery model. A client cannot ask "what migrations apply to this repo?" without hard-coding a list. Issue #461 adds a migration registry to solve this.

Three design axes were decided:

1. **How migrations are registered** (static compile-time vs dynamic plugin vs trait objects).
2. **What `apply_fn` returns** (typed sum type vs `serde_json::Value`).
3. **Status representation** (string enum vs discriminated struct).

## Decision

### Registration: static `&[MigrationDefinition]` with `fn` pointer fields

Migrations are registered at compile time in a `static MIGRATIONS: &[MigrationDefinition]` slice defined in `migration_registry_service.rs`. Each entry holds:

```rust
struct MigrationDefinition {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    status_fn: fn(&dyn RepositoryStore) -> Result<MigrationStatus, RepositoryError>,
    apply_fn:  fn(&dyn RepositoryStore) -> Result<serde_json::Value, RepositoryError>,
}
```

`fn` pointers (not closures) are used because closures that capture environment cannot be stored in `static` without heap allocation.

Adding a new migration requires: one new `MigrationDefinition` literal appended to `MIGRATIONS`, one `status_fn` implementation, and one `apply_fn` implementation — no changes to CLI handlers, WASM bindings, or downstream clients.

**Rejected alternative — trait objects:** `Box<dyn Migration>` per entry is more idiomatic for open-ended plugin scenarios but introduces heap allocation in `static` context (requires `LazyLock`), adds lifetime complexity, and is unnecessary when all migrations are known at compile time. The static slice is sufficient for the known-migrations use case; if runtime-loadable plugins become a requirement, this ADR can be revisited.

### Apply return type: `serde_json::Value`

`apply_fn` returns `serde_json::Value` rather than a typed sum type. A sum type would require a new enum variant per migration — violating the "one entry in the registry, no client changes" principle. Callers (CLI, WASM, web UI) already consume the payload as opaque JSON; they display or surface it without deep interpretation.

**ADR-010 compliance:** each `apply_fn` must construct its result through a `#[derive(Serialize)]` struct and call `serde_json::to_value(&typed_struct)`. The `serde_json::json!()` macro must not appear in any `apply_fn` body. Serialization failures are mapped to `RepositoryError::InvalidSnapshotData` rather than `RepositoryError::Serialize` (which carries a file path and would be misleading here).

### `MigrationStatus` representation in the CLI payload

The CLI payload uses a discriminated struct `{ needed: bool, alreadyApplied: bool, notApplicable: bool }` rather than a string enum. Rationale: JSON Schema sum types (oneOf/enum) are awkward to evolve without breaking clients; a discriminated struct is stable and easy to check in TypeScript. **Exactly one of the three booleans is `true`** — this is an invariant guaranteed by the `From<MigrationStatus>` impl and documented as a contract in `payload.rs`. A future `MigrationStatus` variant requires a corresponding bool field and a `From` update.

### `MigrationStatus` semantics

- `Needed` — the migration should be run (repo is not yet in the target state).
- `AlreadyApplied` — the target state is already achieved (migration has run, or nothing would change if it ran). For `repo-upgrade`, this includes repos with zero instances (nothing to rename = target state achieved).
- `NotApplicable` — the migration makes no sense on this store/repo (e.g. no root container set for `migrate-identity`).

## Consequences

**Positive:**

- Adding a migration is a one-file change (`MIGRATIONS` literal) with zero impact on CLI, WASM, or web client code.
- Static slice is zero-cost: no heap allocation, no runtime registry initialisation.
- `status_fn` / `apply_fn` both accept `&dyn RepositoryStore`, so they work on both `FileStore` (CLI) and `JsonStore` (WASM) without branching.
- The discriminated status struct is easy to check (`status.needed`, `status.alreadyApplied`) in any language without a pattern match.

**Negative / trade-offs:**

- `fn` pointers cannot close over environment — any migration that needs configuration must receive it through the `RepositoryStore` or through the migrate-specific service function, not through the registry entry itself.
- `apply_fn` returning `serde_json::Value` means migration-specific payload fields are invisible to the Rust type system at the registry level; callers must know the migration id to interpret the payload.
- If a migration needs to run only on a specific `StoreBackend`, it must check at runtime inside `status_fn` and return `NotApplicable` — there is no static dispatch on store type.

**Neutral:**

- The `MigrationStatus` Rust enum still exists in the service layer; only the CLI payload representation uses the discriminated struct.
- The two initial migrations (`migrate-identity`, `repo-upgrade`) cover the post-load stateful cases the registry was designed for. Two additional operations were investigated in #594 and determined to be **not registry candidates**:
  - **`migrate_rfc014`** (`srsj_migration_service.rs`) is a pre-load string transformer: it operates on raw `.srsj` JSON bytes before any `RepositoryStore` is constructed. On the WASM/JSON-store path, `load_from_srsj` applies it at load time so `status_fn` would always return `AlreadyApplied`. On the file-store path there is no `.srsj` bundle to migrate so `status_fn` would always return `NotApplicable`. In neither case can `status_fn` return `Needed` — registering it would produce noise, not signal. Pre-load bundle-format migrations belong in their own module (`srsj_migration_service.rs`) and remain standalone entry points.
  - **`srs migrate packet`** (`analysis.rs` / `build_migration_packet`) is a read-only analysis/export tool that assembles a handoff packet for external AI migration tooling. It does not modify the repository and has no `Needed`/`AlreadyApplied`/`NotApplicable` semantics. It is not a migration; it does not belong in `MIGRATIONS`.
  - **Registry scope rule (derived from #594):** only post-load stateful operations that (a) accept `&dyn RepositoryStore`, (b) mutate repository state, and (c) have a meaningful idempotency status are candidates for `MIGRATIONS`. Pre-load transformations and read-only analysis commands are excluded by definition.
- `cmd_repo_apply_migration --id repo-upgrade` accepts both `FileStore` and `JsonStore` backends (via `with_store`), unlike `cmd_repo_upgrade` which restricts to `FileStore`. This is correct: `upgrade_repository_paths` takes `&dyn RepositoryStore` and the restriction in `cmd_repo_upgrade` was conservative, not a correctness requirement. The new apply path is intentionally broader.
