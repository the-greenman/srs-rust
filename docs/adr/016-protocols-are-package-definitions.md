# ADR-016: Protocols Are Package Definitions, Not Tier 2 Records

- **Status:** accepted
- **Date:** 2026-06-26
- **Supersedes:** ADR-006
- **Superseded by:** —

## Context

ADR-006 ("Protocol Definitions Are Generic Tier 2 Records With Typed Validation") decided
protocols would be stored as generic Tier 2 Records at `records/protocols/<slug>.json`,
indexed in `manifest.json`. That design required a prerequisite: creating a
`com.semanticops.srs/protocol@1` type in the spec package before any Rust implementation
could begin.

The implementation in srs-rust#170 went a different direction. Protocols are stored as
**package definitions** under `package/protocols/`, registered in the boundary's
`package.json → protocols[]` array — exactly parallel to Blueprints. This is the design
that was actually built and shipped.

ADR-006 is now factually stale. Any agent or developer reading it receives guidance that
contradicts the actual CLI behaviour.

## Decision

Protocols are **Package definitions**, not Tier 2 Records:

- **Storage path:** `package/protocols/<slug>-<id-prefix>.json` (relative to the boundary prefix)
- **Index:** registered in the boundary's `package.json → protocols[]` (paths relative to the boundary prefix)
- **Identification:** `protocolId` field within the JSON file (not a SRS record `id`)
- **Not instance Records:** protocols do not appear in `manifest.json`, are not queryable
  via `srs record list`, and do not require a `com.semanticops.srs/protocol@1` type in the
  spec package

**Why the Tier-2-Record design was abandoned:**

1. **Prerequisite gate removed.** The Tier-2-Record approach required a
   `com.semanticops.srs/protocol@1` type and field definitions to be authored in the spec
   package before any Rust implementation could begin. This was an unnecessary coupling that
   blocked progress.
2. **Parity with Blueprints.** Blueprints use the package-definition model. Having both
   protocols and blueprints as package definitions is simpler and more consistent than a
   hybrid where blueprints are package definitions and protocols are Tier 2 Records.
3. **Protocols are definition artefacts, not instance data.** They do not benefit from the
   generic record query infrastructure (`record list`, `record get`, relations).

**Rust model:**

- `Protocol` and `ProtocolStage` structs in `srs-core` are used for **semantic validation
  only** — they are not the storage representation.
- Stored values are preserved verbatim as `serde_json::Value` so that stage fields beyond
  the typed struct (`question`, `completionCriteria`, `contributesTo`, `aiGuidance`, …)
  survive a read/write round-trip.
- There is no golden schema file in `schemas/payload/` for the inner stage shape.

Stage dependency invariants (no self-dependency, no cycles, `order` consistent with
`dependsOn` partial order) still require typed Rust structs for validation graph traversal —
this hybrid logic is preserved from the original ADR-006 decision.

## Consequences

**Positive:**
- No prerequisite gate on the spec package. Protocols can be created and used without a
  corresponding SRS type being authored first.
- Parity with Blueprints: same storage model, same registration pattern, same load path.
- Value-centric storage preserves all stage fields verbatim across read/write round-trips,
  including fields beyond the typed `ProtocolStage` struct.

**Negative / trade-offs:**
- Protocol definitions are not queryable as generic Records via `srs record list`.
- `repo validate` does not yet cover protocol or blueprint definitions. JSON Schema files
  for validation are tracked in srs-rust#174; the `repo validate` coverage extension is a
  separate follow-on issue.
- The compiled `Package` model does not yet include protocols (tracked: srs-rust#176); read
  paths scan `package.json` ad hoc on each call rather than consuming a pre-loaded model.

**Neutral:**
- Protocol execution (runs, sessions, stage advancement, `AttentionState`) remains out of
  scope, as in ADR-006. This ADR covers definition storage only.
- Protocol definitions belong to the repository's package data; they are not global
  primitives.
- The `protocol stages <id>` command returns ordered stage summaries — a read-only
  projection, not a mutation.
