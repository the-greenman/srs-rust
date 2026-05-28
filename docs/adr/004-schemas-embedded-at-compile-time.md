# ADR-004: Rust Embeds a Pinned Schema Artifact

- **Status:** accepted
- **Date:** 2026-05-28
- **Supersedes:** -
- **Superseded by:** -

## Context

The canonical JSON Schemas live in `srs/docs/schema/2.0/` and are published at `srs.semanticops.com/schema/2.0/`. The Rust tooling must validate SRS data against those same schemas while remaining usable offline.

The first proposed design copied schemas from a sibling `srs` checkout during `cargo build` and embedded the copied files from `OUT_DIR`. That gives offline runtime validation, but it makes Rust builds depend on mutable files outside the Rust source tree. A checkout can build different binaries depending on which sibling spec repo happens to be present.

For schema-driven tooling, the better contract is:

- one authoritative schema source,
- generated or synced consumer artifacts with provenance,
- reproducible builds from checked-in source,
- automated drift checks that fail when generated artifacts diverge from the authority.

## Decision

Rust embeds schemas from a pinned schema artifact crate, `srs-schema`.

The authoritative source remains `srs/docs/schema/2.0/`. A sync script copies those files into `srs-rust/crates/srs-schema/schemas/2.0/` and writes a digest/provenance file such as `SHA256SUMS`. The schema files in `crates/srs-schema/` are generated artifacts, not an independent source of truth.

`srs-schema` embeds its checked-in schema snapshot with `include_str!` and exposes a registry keyed by canonical `$id`, for example:

```rust
SchemaRegistry::default().validate_by_id(
    "https://srs.semanticops.com/schema/2.0/note.json",
    &value,
)
```

CI and pre-commit checks run a drift command that compares `crates/srs-schema/schemas/2.0/` with the canonical `srs/docs/schema/2.0/`. A schema change in the spec repo must be accompanied by an updated Rust schema artifact before release.

The Rust build itself does not read from the sibling `srs` repo and does not fetch schemas over the network.

## Consequences

**Positive:**

- Rust builds are reproducible from the `srs-rust` checkout alone.
- Runtime validation works offline and has no deployment asset dependency beyond the binary/library.
- Schema drift becomes a CI failure instead of a memory burden.
- The same validation registry can be reused by `srs-core`, `srs-repository`, `srs-cli`, and future bindings.
- Schema provenance is auditable through checked-in digests.

**Negative / trade-offs:**

- There is a generated schema copy in the Rust repo. This is acceptable only because it is treated as an artifact and checked against the canonical source.
- Schema updates require running the sync script and committing the resulting artifact changes.
- Cross-repo CI must have access to the spec repo for drift checks, or it must run in a workspace where `SRS_SPEC_DIR` points at the canonical schema checkout.

**Rejected alternatives:**

- **Runtime fetch from `srs.semanticops.com`:** rejected because validation must work offline and should not depend on network availability or mutable remote content.
- **Sibling-repo `build.rs` copy:** rejected because it makes builds non-reproducible and dependent on files outside the crate source.
- **Shipping loose schema files next to the binary:** rejected because it adds a deployment-time asset dependency and creates another place for version skew.

## Implementation Notes

- `srs-schema` should register every schema under `docs/schema/2.0/`, not only note/record/type/field.
- Validation should use the declared `$schema` where possible and report unknown/missing schema IDs as diagnostics.
- Repository validation should also detect mismatches between `manifest.instanceIndex[].tier` and an instance file's declared `$schema`.
- Model contract tests in `srs-core` should serialize Rust values and validate them through `srs-schema` so schema/model drift is caught early.
