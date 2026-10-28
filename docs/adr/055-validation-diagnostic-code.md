# ADR-055: An optional stable `code` on validation diagnostics

- **Status:** accepted
- **Date:** 2026-10-28
- **Supersedes:** —
- **Superseded by:** —

## Context

The engine computes a stable rule code for many validation findings and then flattens it into the
message text (`CatalogDiagnostic.code` becomes `"{code}: {message}"`, the arrangement validator's
`CODE_*` constants, the dependency finding's `code`). A client that wants to group or explain
diagnostics by rule has to parse that text, and ADR-001 forbids the client owning that parse.

The owner's ruling on srs-rust#1264 (2026-10-08) adopts stable diagnostic codes; decision bf4a5af5
(accepted 2026-10-27, option C then B) scopes this issue to part 1: an optional `code` on validation
diagnostics. Implementation-only, so no spec door applies.

## Decision

1. `ValidationDiagnostic` gains `code: Option<String>`. It serializes as `code` only when present,
   so a diagnostic without a code is byte-identical to before.
2. It is filled wherever the engine already computes a code: the catalog's diagnostics, the
   arrangement and container-pointer codes, the package-dependency findings, the relation-type
   core-key shadow, the in-memory migration notice, the dangling-reference and collection-retired
   checks, and the lifecycle relation checks.
3. `message` is unchanged, including its code prefix. Stripping the prefix is srs-rust#1496, the
   owner's condition on bf4a5af5; renaming legacy code spellings is srs-rust#1492. Both are out of
   scope here, so a code carries today's spelling.
4. WASM `validate()` and MCP `repo_validate` serialize the same struct, so they carry the field with
   no adapter change. The `repo validate` payload description and golden are updated.

## Consequences

**Positive:** clients can group by `code` on `repo validate` without parsing text.

**Negative / trade-offs:** diagnostics with no computed code still have none; the shared
`Diagnostic {severity, code, message, instanceId?}` type for the other binding surfaces
(srs-rust#1491, #1339) is not defined here.

**Neutral:** no `dataModelRevision` change and no new payload version; the field is additive.

## Implementation charter (ADR-048)

- [x] **Spec-first** — owner ruling on srs-rust#1264 (2026-10-08) and decision bf4a5af5.
- [x] **Layer test** — core service (`srs-repository` validation); CLI, WASM and MCP carry it unchanged.
- [x] **One way per goal** — the existing `ValidationDiagnostic` serializer, no second shape.
- [x] **Parity and mirror obligations** — `repo-validate.json` golden regenerated; schema mirrors and pin: none.
- [x] **Decision mode** — complicated.
