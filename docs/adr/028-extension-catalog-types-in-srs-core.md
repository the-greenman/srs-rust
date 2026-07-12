# ADR-028: Extension Catalog File Types Belong in srs-core/extensions

- **Status:** accepted
- **Date:** 2026-07-12
- **Supersedes:** —
- **Superseded by:** —

## Context

The `ext:registry` extension (spec record `srs/srs/records/extensions/ext-registry.json`) defines a
package registry catalog format: `Registry` (top-level index) and `RegistryEntry` (one package per
entry). These describe an **external file format** consumed by the service layer — a catalog index
fetched from a URL or read from a local path, analogous to a package index in other ecosystems.

Two questions arise from the existing ADR landscape:

**Question 1 (ADR-002):** Should `Registry`/`RegistryEntry` be handled as generic Tier 2 Records?

ADR-002 established generic-record operations for SRS *instance* records stored inside a repository.
Registry catalog files are not SRS instance records: they are not stored in a repository's
`records/` directory, they are not bound to a `typeId`, and they do not appear in `manifest.json →
instanceIndex`. They are external data files consumed by the service layer. ADR-002 does not apply
to this class of data.

**Question 2 (ADR-005):** Should `srs-core/src/extensions/mod.rs` remain empty / be removed?

ADR-005 decided that extension *definition records* (`meta.extension` Tier 2 Records in the spec
package) do not need native Rust structs in `srs-core`. It noted the `extensions/mod.rs` stub "may
be removed." That ruling covered one class of use: the extension definition records themselves.

Extension catalog file types are a distinct class. `Registry` and `RegistryEntry` are the Rust
deserialization targets for a file format defined by the `ext:registry` extension — not the
definition record for the extension itself. The pattern precedent is ADR-016 (`Protocol`/`ProtocolStage`
in `srs-core/types/`), which introduced native structs for a package-definition file format without
using the Tier 2 generic-record path.

## Decision

**Extension catalog file types belong in `srs-core/src/extensions/<name>.rs`.**

Specifically:
- Types that model *external data file formats* defined by SRS extensions (catalog indexes,
  manifest variants, external reference files) get native Rust structs in `srs-core/src/extensions/`.
- Extension *definition records* (`meta.extension` Tier 2 Records) continue to use the generic
  record path per ADR-005 — that ruling is unchanged.
- `srs-core/src/extensions/mod.rs` is not removed; it is the module root for extension data types.

For `ext:registry` specifically:
- `Registry` and `RegistryEntry` live in `crates/srs-core/src/extensions/registry.rs`.
- Both structs use `#[serde(rename_all = "camelCase")]` with no `deny_unknown_fields`, so external
  catalog files from future registry versions are deserialized without error (unknown fields
  silently ignored).
- Both structs derive `Debug, Clone, Serialize, Deserialize` only — not `PartialEq`, following the
  pattern for large document-level types (`Blueprint`, `Protocol`, `Record`).

Future extension catalog types (e.g., `ext:import-tracking` provenance files if they become
external file formats) follow the same pattern: a new `crates/srs-core/src/extensions/<name>.rs`.

## Consequences

**Positive:**
- Clear home for extension data format types: `srs-core/extensions/` for data, generic record path
  for definition records.
- Parallel to the `types/` vs generic-record split already established by ADR-016.
- No prerequisite gate (no spec type or field definitions needed before Rust work can proceed).
- Service layer (#244 and beyond) gets typed structs with no fieldValues mapping boilerplate.

**Negative / trade-offs:**
- ADR-005's "may be removed" note on `extensions/mod.rs` is now effectively reversed. Future
  contributors reading ADR-005 must consult this ADR to understand why the module has content.
- There is no entity JSON Schema file (`registry-catalog.json`) in `srs/docs/schema/2.0/` for this
  format. Schema contract tests (ADR-004 pattern) are deferred until such a schema is added.
- This ADR does not cover extension-defined file formats that ARE SRS Tier 2 records (e.g., a
  hypothetical extension that adds new record subtypes). Those would follow ADR-002's generic path.

**Neutral:**
- The `ext:registry` spec record in `srs/srs/records/extensions/ext-registry.json` defines the
  schema in prose; the Rust struct is the implementation of that spec, not an authoritative source.
- Other extensions (currently all definition-record-only) are not affected.
