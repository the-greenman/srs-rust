# ADR-025: Implicit Core Package Merge

- **Status:** proposed
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

`RepositoryStore::load_package` — in both `FileStore` and `MemoryStore` —
merges the core package's fields and types into the returned `Package` **after**
all explicit `packageRefs` are folded in, using the existing
`PackageRefConflict` coalescing logic:

- If a repo already has a field or type with the same ID as a core definition,
  `load_package` returns `PackageRefConflict`. Repos must not declare their
  own `com.semanticops.core/*` definitions.
- If the IDs are absent (the normal case), the core fields and types are
  appended silently.

A drift-check integration test (`tests/core_bundle_drift.rs`) compares the
embedded artifact against the canonical copy in `srs/packages/com.semanticops.core/`
when that repo is present, so CI catches staleness.

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
- `load_package()` result always contains the core fields and types even if the
  caller only cares about the repo's own package. This is a minor overhead
  (2 fields, 1 type) and is unlikely to cause issues.
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
