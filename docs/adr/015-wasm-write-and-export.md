# ADR-015: WASM write bindings and pure `to_srsj_string` export

- **Status:** proposed
- **Date:** 2026-06-07
- **Supersedes:** —

## Context

The `srs-bindings` WASM module currently exposes only read operations. The srs-web governance
editor needs create/update/delete/relation/lifecycle operations and a way to export the edited
repository as a `.srsj` string for browser download.

`JsonStore::flush` already assembles the `.srsj` envelope but writes to disk — unusable in WASM.
The write bindings must run in an in-memory `JsonStore` with no filesystem access.

## Decision

1. Extract `JsonStore::to_srsj_string(&self) -> Result<String, RepositoryError>` — pure, no I/O.
   `flush()` delegates to it. This is the export primitive for WASM.
2. The WASM `SrsRepository` struct wraps a `JsonStore` (already using `RefCell` for interior
   mutability). All write binding methods use `&self` — no `&mut self` needed.
3. Each write binding (B6: create/update/delete record; B7: relations + lifecycle; B8: export)
   calls exactly one service function from `srs-repository`. No business logic in `srs-bindings`.
4. The in-memory `JsonStore` is the editor's working copy. The browser downloads the output of
   `export_srsj()` (which calls `to_srsj_string()`). No filesystem access in WASM context.

## Consequences

**Positive:**
- The WASM write surface matches the existing `FileStore`-backed CLI — same service functions.
- `to_srsj_string` is independently testable without WASM.
- `flush()` is not broken; it just delegates to the new method.

**Negative / trade-offs:**
- The `JsonStore` must be initialized from a `.srsj` string (already supported via `from_srsj`).
  A fresh empty repo needs a well-formed manifest — the WASM binding for new-repo creation is
  deferred to a future issue.
