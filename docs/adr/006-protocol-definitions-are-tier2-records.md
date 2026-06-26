# ADR-006: Protocol Definitions Are Generic Tier 2 Records With Typed Validation

- **Status:** superseded
- **Date:** 2026-05-29
- **Supersedes:** —
- **Superseded by:** [ADR-016](016-protocols-are-package-definitions.md)

## Context

The CLI command structure plan introduces `srs protocol list/get/stages/validate/export/import` commands. Protocol definitions are epistemically ordered processes with typed structure: stages with `stageId`, `order`, `dependsOn[]`, `completionCriteria`, `contributesTo[]`, and optional `outputType`. Invariants include: no self-dependency, no circular dependencies, `order` consistent with the partial order implied by `dependsOn`.

Two questions need answering:

1. Where do Protocol definitions live in a repository?
2. Does `Protocol` need a native Rust struct in `srs-core`, or can generic records suffice?

Two options were considered:

**Option A — Native core type:** Define `Protocol` and `ProtocolStage` structs in `srs-core` as storage and validation types, with dedicated service functions in `srs-repository`. Storage path and manifest tier managed by a dedicated service.

**Option B — Generic Tier 2 Record with typed validation structs:** Protocol definitions are stored as generic Tier 2 Records. Typed in-memory structs exist in `srs-core` solely for validation logic — they are not the storage representation. The validation service deserializes `fieldValues` into these structs, runs invariant checks, then discards them. Storage and retrieval use the generic record services.

## Decision

**Storage (Option B):** Protocol definitions are generic Tier 2 Records bound to a `com.semanticops.srs/protocol@1` type. This type must be created in the spec package (`srs/srs/package/types/protocol.json`) with corresponding field definitions before any Rust implementation begins. Instances are stored as `records/protocols/<slug>.json` where slug is derived from the protocol name, indexed in `manifest.json` at `tier: 2`.

**Rust model (hybrid):** Protocol *validation* requires typed in-memory structs (`Protocol`, `ProtocolStage`) in `srs-core`. These structs exist for validation logic only — they are not the storage model. The validation service deserializes the generic Record's `fieldValues` into these structs, runs invariant checks, and returns diagnostics. Storage and retrieval use the generic record services.

Stage dependency invariants (no self-dependency, no cycles, `order` consistent with `dependsOn` partial order) cannot be expressed as field-level value type checks alone — they require traversal over the full stage graph. This is why typed Rust structs are necessary for validation even though storage is generic.

**Protocol execution is out of scope.** This ADR covers definition storage only. Protocol runs, sessions, stage advancement, and `AttentionState` are deferred to a future plan and a future ADR.

## Consequences

**Positive:**
- Protocol definitions are first-class queryable records. Any repository can declare its own protocols in its package — they are not hardcoded in the Rust library.
- `protocol list/get` reuse the generic record infrastructure with no duplication.
- Stage dependency validation catches broken protocols before they are used.
- Protocol execution state (runs, sessions) is cleanly separate — it is never part of the definition record.

**Negative / trade-offs:**
- A `com.semanticops.srs/protocol@1` type and its fields must be created in the spec package (`srs/srs/package/`) before any Rust implementation begins. This is a prerequisite gate.
- The `Protocol` and `ProtocolStage` validation structs in `srs-core` must stay aligned with the spec type's field IDs. Changes to the spec type require corresponding Rust changes.
- `protocol export/import` must serialize/deserialize through the generic Record shape (`fieldValues[]`), not directly through the typed validation structs.

**Neutral:**
- Protocol definitions belong to the repository's package data. A repository declares what protocols it supports; protocol definitions are not global primitives.
- The `protocol stages <id>` command deserializes the `stages` field value and returns ordered stage summaries — this is a read-only projection, not a mutation.
- Protocol execution is a later design. This ADR makes no claims about where run/session state lives.

## Prerequisites for Implementation

> **Superseded — do not act on the steps below.** ADR-016 replaced this design; the `com.semanticops.srs/protocol@1` type was never created and must not be created. See [ADR-016](016-protocols-are-package-definitions.md).

Before Phase 4 agents can implement protocol services:

1. Create `srs/srs/package/types/protocol.json` defining the `com.semanticops.srs/protocol@1` type.
2. Create the corresponding field definitions in `srs/srs/package/fields/` for: `id`, `namespace`, `name`, `version`, `description`, `targetType`, `stages`, `tags`, `createdAt`.
3. Update `srs/srs/package/package.json` to include the new type and field paths.
