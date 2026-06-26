# ADR-016: Protocol Definitions Are Package Definitions, Not Tier 2 Records

- **Status:** accepted
- **Date:** 2026-06-26
- **Supersedes:** [ADR-006](006-protocol-definitions-are-tier2-records.md)
- **Superseded by:** —

## Context

ADR-006 decided that Protocol definitions would be stored as generic Tier 2 Records bound to a `com.semanticops.srs/protocol@1` type in the spec package. This required creating that type and its field definitions as a prerequisite before any Rust implementation could begin.

When implementation work began (srs-rust#170), a different direction was taken:

- Blueprints had already been established as **package definitions** (per ADR-009) — JSON files under `package/blueprints/`, registered in `package.json → blueprints[]`, owned by the package, loaded directly (not via record fieldValues).
- Requiring Protocols to be Tier 2 Records would have imposed a prerequisite gate: the `com.semanticops.srs/protocol@1` type and all its field definitions had to be authored in the spec package (`srs/srs/package/`) before Rust work could start.
- Protocols serve the same role as Blueprints in the package model: they are definitions that describe a process (Protocol) or a document structure (Blueprint) that the package author ships. Both are package-maintained metadata, not instance records.
- Storing Protocols as package definitions removes the spec-type prerequisite and creates structural parity between the two definition kinds.

## Decision

Protocol definitions are **package definitions**, stored under `package/protocols/<name>-<uuid>.json` and registered in `package.json → protocols[]`. This is structurally parallel to Blueprints (`package/blueprints/`, `package.json → blueprints[]`).

**Storage model:** a Protocol definition file is a JSON object with top-level fields prefixed `protocol*` (`protocolId`, `protocolNamespace`, `protocolName`, `protocolVersion`, `protocolDescription`?, `protocolTargetType`, `protocolStages`, `protocolTags`?, `protocolCreatedAt`). Fields marked `?` are optional. There is no `fieldValues` wrapper; the file is the definition.

**Rust model:** `Protocol` and `ProtocolStage` structs in `srs-core` serve as both the deserialization target and the validation model. The structs are loaded from the JSON file directly via serde (not reconstructed from `fieldValues`). Validation invariants (no self-dependency, no cycles, `order` consistent with `dependsOn` partial order) are enforced by the protocol service against these typed structs.

**Rich stage fields:** `ProtocolStage` carries optional fields beyond the minimal DAG shape — `question`, `completionCriteria`, `contributesTo`, `aiGuidance`, `purpose`, `outputType`. These fields are preserved verbatim when loading; there is no lossy projection through a narrow fieldValues schema. This value-centric storage model means stage fields the Rust struct does not know about are not rejected.

**Partial ADR-011 compliance for protocol commands:** `protocol list` follows ADR-011 fully, using `ProtocolListEntry` with a committed golden schema. `protocol stages` also follows ADR-011 fully, using `ProtocolStageEntry` — this struct is a **full projection** of all stage fields including `purpose`, `question`, `completionCriteria`, `contributesTo`, `aiGuidance`, and `outputType` (the latter two as `serde_json::Value`). The write/read commands — `protocol get`, `protocol create`, `protocol update`, `protocol import`, and `protocol export` — all return `ProtocolPayload { protocol: serde_json::Value }`, which embeds the full protocol body verbatim. This opaque wrapper is a deliberate carve-out to preserve the value-centric stored shape without projecting it through a typed struct; the inner body has no golden schema. New protocol CLI commands MUST define named payload structs per ADR-011; the `ProtocolPayload` opaque approach is not a template to copy.

## Consequences

**Positive:**
- No prerequisite gate: Protocol definitions can be authored and consumed without first creating a `com.semanticops.srs/protocol@1` type in the spec package.
- Structural parity with Blueprints: both definition kinds load the same way — direct JSON deserialization, no fieldValues mapping.
- Value-centric storage preserves rich stage fields authored in the package (e.g. `aiGuidance`, `purpose`) without lossy round-trips through a constrained schema.
- Protocol definitions are owned by the package maintainer, not by the repository's instance layer. A protocol is shipped with the package, not created per-instance.

**Negative / trade-offs:**
- Raw `serde_json::Value` is used for the `ProtocolPayload` inner body (returned by `protocol get`, `create`, `update`, `import`, and `export`) to preserve the protocol definition verbatim, bypassing ADR-011's typed-struct contract for those commands' inner bodies.
- There is no entity JSON Schema file (`protocol.json`) in `srs/docs/schema/2.0/` validating the stored shape. Authors writing protocol files by hand cannot use schema-on-save validation (tracked in srs-rust#174).
- `repo validate` does not currently schema-validate protocol definition files (blocked on srs-rust#174 and srs-rust#175).
- ADR-006's stated "Rust model (hybrid)" — where typed structs existed only for validation and storage used generic records — no longer holds. The `Protocol` struct is both storage type and validation type.
- This storage model is a deliberate departure from ADR-002 (Tier 2 Generic Record Operations), which established the generic Record pattern that ADR-006 applied to protocols. Future definition-like entities should choose between the ADR-002 pattern (Tier 2 Record) and this package-definition pattern explicitly; neither is automatically the default.

**Neutral:**
- The `com.semanticops.srs/protocol@1` type and field definitions described in ADR-006 were never created. They are not needed and should not be created.
- Protocol execution state (runs, sessions, stage advancement) remains out of scope, as in ADR-006. This ADR covers definition storage only.
- The spec formally records Protocol definitions as package data (not Tier 2 Records) in invariant 037 of the spec subsection 05-1-5-1.
