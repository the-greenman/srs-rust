# ADR-025: Implicit Core Package Merge

- **Status:** accepted
- **Date:** 2026-07-09
- **Supersedes:** —
- **Superseded by:** —

## Context

RFC-018 requires that `com.semanticops.core/*` types (initially `purpose`, used
as the mandatory identity record type) are resolvable in every SRS repository
with zero per-repo configuration. Before this decision, the Rust implementation
hardcoded the core type UUIDs as compile-time constants
(`core_purpose::PURPOSE_TYPE_ID`, etc.) because there was no type-resolution
mechanism that could find them without a package declaration.

Three mechanisms were considered:

- **A — Implicit merge:** embed the canonical core package artifact in
  `srs-repository`; merge its fields and types into every `load_package()`
  result transparently. Repos need no `packageRefs` entry. Conflicts are
  detected and rejected loudly.
- **B — Synthetic field/type injection:** inject hardcoded `Field` and
  `RecordType` structs at startup, keeping them as code rather than as a
  JSON asset. Simpler but diverges from the package model and makes updates
  harder.
- **C — Explicit `packageRef` in every repo's manifest:** add a local or
  remote reference to the core package. Requires migration of every existing
  repo and forces users to declare an implementation detail.

## Decision

**Mechanism A — implicit merge.** The canonical core bundle
(`com.semanticops.core` package, version 1.0.0) is embedded in
`crates/srs-repository/assets/core-bundle.srsj` via `include_str!` and parsed
lazily once at startup (`core_package::core_package()`).

`RepositoryStore::load_package` — in `FileStore`, `MemoryStore`, and `JsonStore`
(the WASM backend, per ADR-013) — merges the core package's fields and types
into the returned `Package` **after** all explicit `packageRefs` are folded in:

- If a repo already has a field or type with the same ID as a core definition
  but a different namespace or name, `load_package` returns
  `CorePackageConflict`. Repos must not declare their own
  `com.semanticops.core/*` definitions.
- If the IDs match and the namespace + name also match (the repo was copied
  from a state that already had core fields embedded), the merge is idempotent
  — the duplicate is silently skipped.
- If the IDs are absent (the normal case), the core fields and types are
  silently appended.

A drift-check integration test (`tests/core_bundle_drift.rs`) compares the
embedded artifact against the canonical copy in `srs/packages/com.semanticops.core/`
when that repo is present, so CI catches staleness.

### Amendment (srs-rust#685): the seven canonical relation types

The bundle also carries `relationTypes[]` — the seven canonical relation types
(`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`,
`precedes`) documented in `srs-usage.md` as shipping "in the core package"
(RFC-005 / relation principle R3). Until #685, `EmbeddedCorePackage` didn't
deserialize `relationTypes` at all, so every freshly created repo rejected all
seven with `E1UnknownRelationType`.

Relation types merge with **different conflict semantics than fields/types**,
because `srs-core`'s `resolve_definition` resolves a relation by bare `key`,
not by id: two definitions sharing a key with different id/content is an
`E1Conflict` at relation-validation time, regardless of which package
contributed which. So the merge skips a canonical relation type whenever the
repo already has *any* definition — its own, or the same canonical one
carried over by a prior merge — using that key, rather than matching by id
the way fields/types do. This is deliberately permissive: a repo's own
definition always wins, which also covers repos that pre-date #685 and
worked around the missing canonical types by declaring their own namespaced
definitions (the documented `srs relation-type create` workaround the issue
itself calls "a trap for agents"). There is no `CorePackageConflict` error
path for relation types — key collision is always a silent skip, never a
hard error.

## Consequences

**Positive:**
- Existing repos (including the `srs/srs/` spec repo) gain core type
  resolution with no manifest changes.
- `package.resolve_type_by_name("com.semanticops.core", "purpose")` returns
  `Some` on every repository.
- `srs type list` and discovery surfaces core types automatically.
- Hardcoded UUID constants in `core_purpose.rs` can be retired once callers
  are updated (#434).

**Negative / trade-offs:**
- `load_package()` result always contains the core fields, types, and relation
  types even if the caller only cares about the repo's own package. This is a
  minor overhead (2 fields, 1 type, 7 relation types) and is unlikely to cause
  issues.
- A repo cannot define a `com.semanticops.core/*` type even if it wants to
  experiment; it will get a loud error. This is intentional — the core
  namespace is reserved.
- The embedded artifact must be kept in sync with `srs/packages/com.semanticops.core/`.
  The drift test provides the safety net.

**Neutral:**
- The core package is NOT represented as a `packageRefs` boundary in the
  manifest. `list_package_boundaries` does not include it. `srs type list --package`
  (boundary-filtered) will not list core types for any boundary selector —
  only the unfiltered `srs type list` shows them.
- `MemoryStore` callers that construct stores with a pre-built `Package`
  (test doubles) will now have core types merged in. Tests that checked
  `fields.is_empty()` must be updated.
