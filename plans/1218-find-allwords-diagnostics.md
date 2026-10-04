# Plan: find all-words contentMatch, unknown-filter diagnostic, stale text fixes

> Issue the-greenman/srs-rust#1218. Branch `feat/1218-find-allwords-diagnostic`. Owner-approved plan (relay map), decisions below ruled autonomously from the charter and ADR-048.

## Summary

`find` is one `srs-repository` service (`discovery_service::find`). Four fixes: (1) `contentMatch` matches records containing every whitespace-separated word in any field, any order; (2) a type/typeId/containerId naming nothing yields a `warning:` diagnostic; (3) `DESC_FIND` states Tier 0 + Tier 2 and the `namespace/name` type form; (4) MCP instructions advertise `composition/<id>`, the URI kind `uri.rs` accepts.

## Spec gate (Stage 1.5)

No spec change. `discovery.json` defines the query; `contentMatch` is a recall floor (RFC-012), so a superset of the phrase match is conformant. Diagnostics are an implementation-level result field (`Vec<String>`). Decision mode: complicated (rules apply). Layer test: all logic in the core service; CLI/MCP/WASM already pass the result through.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead | Sonnet session (single crate, no new role) |

## Architecture Decisions

| ADR / rule | Decision | Status |
|---|---|---|
| ADR-001/010 | Change lives only in `discovery_service`; adapters untouched except description/instruction text in `srs-mcp-core` | accepted |
| RFC-012 recall floor | All-words = every normalized word is a substring of some segment; phrase present in one segment implies all words present, so superset holds | decided |
| One way per goal | Single-word and phrase queries both go through the word path; no parallel phrase matcher | decided |
| Word semantics | Words may sit in different fields; `matchedFields` = fields with a segment containing any word; snippet windows on the first word found in the first matching segment (a real match location, #1221 window kept) | decided |
| Diagnostics | Existing `Vec<String>` result; strings prefixed `warning:`; unknown container short-circuits to zero hits, unknown type warns only | decided |
| Type check | `typeId` checked against `list_types` ids; `typeNamespace`/`typeName` against namespace/name pair (unspecified half is wildcard) | decided |

## Tasks

- [x] `discovery_service.rs`: word list, `match_content`, `unresolved_filters`
- [x] Tests: all-words (phrase superset, reversed, cross-field, missing word), unknown type/container warnings
- [x] `tools.rs` DESC_FIND, `lib.rs` INSTRUCTIONS `composition/`
- [x] `srs-usage.md` find line (srs repo, separate PR)

## Out of scope

Ranking, stemming, quoted-phrase syntax (a future Layer-2 index concern).

## Final Acceptance

`cargo test`, `cargo clippy --all-targets -- -D warnings`.
