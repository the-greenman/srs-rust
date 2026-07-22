# ADR-041: Storage-backend design guardrails (keeping SQL/NoSQL backends feasible)

- **Status:** accepted
- **Date:** 2026-07-22
- **Supersedes:** —
- **Superseded by:** —
- **Related:** ADR-007 (write-before-index ordering), ADR-009 (adapter-owns-storage),
  ADR-010 (service boundary contract), ADR-021 (batch write mode), ADR-038 (Vfs seam / tree
  primary model), ADR-039 (`.srs` pure-tree archive); `ARCHITECTURE.md` (Storage Direction,
  Repository Lifecycle And Portability, Package/Container Boundaries, Store Matrix Testing);
  `docs/architecture/capability-layering.md` (three-layer model).

## Context

ADR-038/039 (#694) made the **in-memory VFS file tree the primary operational model** — a `Vfs`
seam (`DiskVfs`/`MemVfs`) under a single `FileStore`, tree sessions, and a pure-tree `.srs`
archive. That work is deliberately file-shaped, which raised a fair question: has the SRS
implementation quietly foreclosed a future **SQL/table-based (and later NoSQL) storage backend**?

This ADR records the evaluation and fixes the guardrails so the answer stays "no" by
construction. It **consolidates and sharpens** intent already scattered across `ARCHITECTURE.md`
("Keep storage boundaries visible so a database-backed implementation can be introduced later";
"A future SQL adapter must be able to implement package boundaries / containers as table rows
without changing service APIs") into a single decision an implementer can check against. No SQL
backend is built here.

### What the evaluation found

1. **The backend seam is `RepositoryStore`** (`crates/srs-repository/src/store.rs`), not the
   `Vfs`. Its own doc comment already names "(filesystem, SQLite, in-memory)" as swappable;
   service functions accept `&dyn RepositoryStore`; the CLI (`with_store`) and MCP (`open_store`)
   only construct a concrete store and hand it in. **A SQL backend is one new
   `impl RepositoryStore`.**

2. **ADR-038's Vfs/tree lives *below* `FileStore` and is bypassed by a non-file backend.** A SQL
   store implements `RepositoryStore` directly; it never touches `Vfs`, `MemVfs`, tree sessions,
   or the `.srs` archive. The file-tree-only capabilities are already gated behind safe
   discriminators — `as_tree_snapshot() -> Option<…>` defaults to `None`,
   `is_file_tree_store() -> bool` defaults to `false` — and the batch seam
   (`begin/commit/abort_batch`). **#694 did not wire a file backend into the semantics.** The
   standing risk is *regressing* this isolation, not anything #694 already did.

3. **Pre-existing friction (predates #694) that will ossify if left unmanaged.** Most of the
   trait is **path-string + `serde_json::Value`** shaped: `save_field(relative_path, …)`,
   `load_instance_json(relative_path) -> serde_json::Value`, `save_instance_json(path, value)`,
   `list_instance_files(dir) -> Vec<String>`, every `ensure_*_dir`. Instance enumeration is free
   functions walking `manifest.instance_index` by `InstanceIndexEntry.path`
   (`record_store.rs`), not a store-answerable query. A SQL adapter written against the trait as
   it stands is a **path-keyed JSON blob store** that gains nothing from being a database.

4. **Containers already demonstrate the fix.** `save_container(&Container)` / `load_container(id)`
   / `list_container_summaries()` address entities by **logical id**, with the path-based methods
   `#[deprecated]` and `ContainerIndexEntry.path` reduced to an adapter-private field. This is the
   template the other entity types have not yet followed.

5. **Owner steer on reach (2026-07-22):** embedded **SQLite is the in-scope backend** for a local
   single-user app and fits the current **synchronous** trait as-is. A **multi-user / networked**
   environment "may not use srs-rust as native tooling" and would consume SRS through
   **export/import** (the `RepositorySnapshot` codec, `.srs`/`.srsj`), *not* by implementing
   `RepositoryStore` over a network database. The synchronous store contract is therefore a
   **deliberate boundary, not debt** — async is explicitly out of scope.

## Decision

Adopt the following guardrails. New or changed storage-touching code must satisfy them; a future
SQL/NoSQL backend must be admissible under them without editing service APIs.

### A. Seam & isolation

- **G1 — `RepositoryStore` is the only primary-backend seam.** A new backend is exactly one
  object-safe `impl RepositoryStore`; keep the trait `dyn`-compatible. For **repository storage**,
  `std::fs`, `Vfs`, `DiskVfs`/`MemVfs`, the concrete `FileStore` type, and repo-relative path
  literals must never appear in `srs-repository` service functions, `srs-bindings`, or clients.
  The narrow, sanctioned carve-outs are (a) reading **external, user-supplied sources** that are
  not the repository's own storage — e.g. a federation registry file (`registry_service`) or a
  package install-source bundle (`package_install_service`), the library analogue of the CLI's
  process-boundary file reads — and (b) `#[cfg(test)]` fixture builders. Neither persists the
  repository's own entities outside the store. (Existing rule — keep enforcing; see
  `ARCHITECTURE.md` Authority Boundaries / Repository Lifecycle And Portability and `CLAUDE.md`
  Storage Boundary Rules.)

- **G2 — The Vfs/tree seam stays `FileStore`-internal.** File-tree-only capabilities
  (`as_tree_snapshot`, `is_file_tree_store`, byte-identical round-trip, `export_tree`, the `.srs`
  pure-tree archive) remain behind `Option`/`bool` discriminators with `None`/`false` defaults.
  **No service may assume a file tree exists, that files round-trip byte-identically, or that a
  store can produce a tree snapshot.** ADR-038/039 already comply; this guardrail forbids
  regressing that isolation.

### B. Trait shape — pay down before it ossifies

- **G3 — Migrate entity persistence from path-strings to logical identity, following the
  container precedent.** New or changed persistence methods for records, notes, relations,
  fields, types, and views take **typed entities keyed by logical id**
  (`save_record(&Record)`, `load_record_by_id(id)`, `delete_record(id)`), not
  `(relative_path, serde_json::Value)`. Deprecate the path/Value methods exactly as containers
  did. This is what makes a SQL adapter *table rows* rather than *path-keyed blobs*.

- **G4 — No `serde_json::Value` as the store contract's currency for domain entities.** Typed in,
  typed out (mirrors the ADR-010 service-function contract). `serde_json::Value` methods are
  transitional and must carry a deprecation path.

- **G5 — `InstanceIndexEntry.path` is an adapter detail; enumeration and query must be
  store-answerable.** Provide a `list_records(filter)` / query-shaped capability a backend can
  satisfy with a native query, rather than only free functions that walk `manifest.instance_index`
  by path in the service layer. Treat `path` as an opaque backend key in the contract — the same
  status `ContainerIndexEntry.path` already has.

### C. Transactions & ordering

- **G6 — Multi-write operations use the batch seam.** Compound writes go through
  `begin_batch` / `commit_batch` / `abort_batch` (ADR-021) so a SQL backend maps them to a real
  transaction and a file backend keeps its deferred-flush behaviour. Respect ADR-007's
  write-entity-before-index ordering on create and index-before-entity on delete. Do not add
  write paths that bypass the batch seam.

### D. Sync boundary & networked integration

- **G7 — The `RepositoryStore` contract stays synchronous and embedded; networked backends
  integrate via export/import.** Embedded SQLite (`rusqlite`) is the sanctioned local-app backend
  and fits the sync trait as-is. **Do not introduce `async` into the store trait** (consistent
  with `ARCHITECTURE.md` Storage Direction and `CLAUDE.md`). Multi-user / networked environments
  integrate through the **backend-neutral portability engine** (`RepositorySnapshot`,
  `export_repository_snapshot` / `import_repository_snapshot`, `.srs`/`.srsj`), which is the
  sanctioned interop boundary. Its lossless, layout-faithful round-trip and byte-fidelity
  guarantees must be protected as a first-class contract, not an incidental feature.

### E. Accelerators (Layer 2) — the recommended first SQL build

- **G8 — SQL/graph/vector acceleration is an optional derived projection in `srs-projection`,
  never a replacement for ground truth.** It is rebuildable from the primary store and gated by
  the capability-layering **consistency rule**: structured results (type/container/tag/lifecycle
  filters) MUST equal the Layer-1 deterministic contract exactly; content matching MAY add
  semantically-related recall and MAY reorder by score but MUST NOT drop anything the contract
  matched. Ground truth remains the primary `RepositoryStore`. Building this first is the
  lowest-risk way to introduce SQL, and it exercises the query-shaped seams (G5), de-risking any
  later primary-store adapter.

### F. Safety net

- **G9 — A shared `RepositoryStore` conformance/parity suite is the admission gate for any new
  backend.** Promote the existing store-matrix testing (FileStore + JsonStore + MemoryStore) into
  a reusable conformance harness every backend must pass, including cross-store round-trips
  (memory → json → file → sql) via the portability engine. No new backend is admissible until it
  is green against the harness. (Extends `ARCHITECTURE.md` Store Matrix Testing and the
  `CLAUDE.md` cross-store-roundtrip rule.)

## Consequences

**Positive:**
- The primary-store door is kept open *by a checkable contract*, not by hope: a SQL/SQLite
  backend is one `impl RepositoryStore` that returns `None`/`false` for tree-shaped methods and
  maps logical ids to rows.
- The container precedent (G3–G5) becomes the explicit target shape, converting a vague "SQLite
  is possible" claim into a concrete, incremental migration.
- The synchronous boundary and the export/import interop path are stated as intentional, so the
  networked/multi-user question no longer looks like an unhandled gap — it has a sanctioned answer
  (G7) that also raises the priority of protecting the portability engine's fidelity guarantees.
- Layer-2 accelerators (FTS, graph, vector) get a safe, endorsed home (G8) without destabilising
  ground truth.

**Negative / trade-offs:**
- G3–G5 imply real refactoring debt on the record/note/relation/field/type/view methods before a
  SQL adapter is more than a blob store. That work is filed as follow-ups, not done here.
- The "SQLite-ready" property is asserted but still **unexercised** against a non-path paradigm; a
  SQLite spike (filed follow-up) is the cheapest way to find remaining path/Value assumptions.
- Choosing export/import (not a native async store) as the networked path means a multi-user
  server built on a network DB does not reuse `srs-repository` as its live store — it re-implements
  or wraps the semantics on its own side and exchanges SRS artifacts. Accepted per owner steer.

**Neutral:**
- No code changes in this ADR; it is doctrine plus reciprocal pointers from `ARCHITECTURE.md` and
  `capability-layering.md`. The concrete debt (logical-id migration, conformance harness, SQLite
  spike, optional Layer-2 scoping) is tracked as linked follow-up issues.
- Guardrails G1, G2, G6 already hold in the current codebase; they are recorded to prevent
  regression, not to demand new work.
