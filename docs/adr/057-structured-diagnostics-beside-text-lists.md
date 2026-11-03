# ADR-057: A shared structured `Diagnostic` beside the text list on six payload surfaces

- **Status:** accepted
- **Date:** 2026-11-03
- **Supersedes:** —
- **Superseded by:** —

## Context

ADR-055 gave `repo validate` an optional `code`. The render, find, container view, blueprint
(schema and brief), repository navigation and `type_schema` results still return diagnostics as
`Vec<String>`, with the rule code only inside the text (`[R23] ...`, `section-container-ambiguous: ...`).
srs-web groups notices on exact message text, and a client wanting the code must parse text, which
ADR-001 forbids.

Decision bf4a5af5 (owner, 2026-10-27, option C then B) rules: deliver structured diagnostics on the
other surfaces as a parallel list beside the existing text list, which stays exactly as it is until
srs-rust#1496 retires it. This issue (srs-rust#1491) is the first consumer, so it defines the shared
shape that srs-rust#1339 reuses (ADR-048 rule 3: one shape, defined by its first consumer).

## Decision

1. **One type.** `srs_repository::diagnostic::Diagnostic { severity, code, message, instanceId? }`.
   `severity` reuses `DiagnosticSeverity` (`error`, `warning`, `info`). `message` is the text-list
   entry verbatim (code prefix included), so the two lists agree entry for entry. `instanceId` is
   omitted when absent.
2. **One field name on every surface: `diagnosticsStructured`.** Same name in the CLI payload, WASM
   result and MCP result. It is always present (an empty array when there are no findings). The
   existing `diagnostics: string[]` is untouched.
3. **Built where the text is built.** A `Diagnostics` collector holds both lists; a site cannot add
   to one and forget the other. Sites with a named code pass it (`find-field-unknown`,
   `section-container-ambiguous`, `resolve-container-view/composition-not-found`, ...). The render
   projection already leads every message with its tag, so its sites derive the code from that tag
   in one function (`[R23] ...` gives `R23`, `[section:s1] ...` gives `section`,
   `ext:type-inheritance: ...` gives `ext:type-inheritance`, an untagged message gives `render`),
   per the 2026-10-08 ruling that a site naming no code gets one derived mechanically.
4. **Nested results keep their codes.** `blueprint schema` prefixes each nested `type_schema` entry
   with its type id in the message and sets `instanceId` to that type id. Findings re-used from the
   tree and protocol services, which are outside this issue, carry the codes `tree-diagnostic` and
   `protocol-diagnostic` until srs-rust#1339 gives them their own.
5. Additive only: no field removed or retyped, no `dataModelRevision` change. Code spellings are
   today's; renames are srs-rust#1492.

## Consequences

**Positive:** srs-web#632 can group by `code`; no client parses text.

**Negative / trade-offs:** render codes are tag-derived rather than individually named, and the
untagged render messages share the code `render` until they are named; the payload carries each
finding twice until srs-rust#1496.

**Neutral:** golden payload schemas for `find`, `render-composition`, `blueprint-schema`,
`blueprint-brief` and `type-schema` gain the field; `container resolve-view` and `repo navigation`
payloads are opaque objects and carry it unchanged.

## Implementation charter (ADR-048)

- [x] **Spec-first** — owner ruling on srs-rust#1264 (2026-10-08) and decision bf4a5af5.
- [x] **Layer test** — `srs-repository` services; CLI, WASM and MCP carry the field unchanged.
- [x] **One way per goal** — one `Diagnostic` type and one collector; `repo validate` keeps its own
  `ValidationDiagnostic` (ADR-055), not duplicated.
- [x] **Parity and mirror obligations** — payload goldens regenerated; schema mirrors and pin: none.
- [x] **Decision mode** — complicated.
