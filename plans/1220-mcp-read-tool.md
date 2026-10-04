# Plan: MCP `read` tool for any srs:// resource (#1220)

## Summary

claude.ai (browser MCP relay, srs-web#306) calls tools but not resources, so map, agent-index, tree, `record/{id}`, `context/…`, `composition/{id}`, `type/{id}`, `protocol` are unreachable. Add one tool `read {uri}` that returns exactly what `resources/read` returns, through the same dispatch (`srs_resources::read_resource`), plus a byte cap for the browser relay (~128 KB). No spec change (spec gate: `srs://` is tooling, ADR-037 §6).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | this session |
| MCP Adapter Worker | this session (crates/srs-mcp-core only) |
| Verification | haiku reviewer |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | Resources + tools dual exposure (as `type_schema`, Amendment #692). `read` is a dated amendment, not a new ADR: it adds no constraint beyond "one dispatch". | accepted; amendment added |
| ADR-048 / charter "one way per goal" | `read` calls `srs_resources::read_resource` verbatim: no per-URI logic in the tool. Only a size policy is added, URI-agnostic. | applied |
| ADR-011 | Tool input is a shadow struct (`ReadToolInput`); no service input to mirror (the URI is the whole input). | applied |
| decision (oversize) | Cap the returned text at `MAX_READ_BYTES` = 96_000 (under the ~128 KB relay limit, leaving headroom for JSON framing/escaping). Over the cap: the text is cut on a UTF-8 boundary, a trailing notice states shown/total bytes and points to bounded alternatives, `structuredContent` carries `{uri, mimeType, truncated, totalBytes, shownBytes}`. Not an error: the head of the document is still useful. The pointer is generic text naming `find {limit}`, `srs://…/tree/{instanceId}`, `container_outline`, `record/{id}`. Applies only to the tool; `resources/read` stays unchanged (stdio clients have no limit). | decided here, recorded in ADR-037 amendment (`read` is the one size-bounded tool; `resources/read` is not) |
| decision (errors) | "Errors as today": `read_resource`'s `McpApplicationError` (invalid uri -32602, not found -32002) propagates as the protocol error; no second error mapping. | decided |
| decision (placement) | Tool name/description/input in `tools.rs` (single owner of the catalogue); dispatch in `SrsMcpApplication::call` (`tools/call`), because it alone knows `repository_id`, which `call_tool(store,…)` lacks. | decided |
| Interop register | Agent-facing surface: alignment-opportunities item 1 (MCP, adopt) is the surface being extended; no contradiction. | cited |

No new ADR.

## Contracts

- CLI output contract: no CLI command or payload changes.
- Entity schema sync: none.
- MCP tool catalogue grows 28 -> 29 (`surface.rs` `tool_catalogue_has_all_thirty_tools…`, tools.rs `list_tools_advertises_all_thirty_with_schemas`).

## Scope

- `TOOL_READ`, `DESC_READ`, `ReadToolInput {uri}` in `crates/srs-mcp-core/src/tools.rs`, listed in `list_tools`.
- `tools/call` `read` handled in `crates/srs-mcp-core/src/lib.rs` via `read_resource` + cap.
- `INSTRUCTIONS` text: start with `find {limit: 0}` for the map; use `read` for any srs:// resource (coordinate with #1218: minimal local edit, rebase).
- Tests; ADR-037 amendment; `srs-rust/CLAUDE.md` tool-count wording if stale; `docs/dogfooding.md` row.
- Finding (report only): does `record/{id}` work for a Tier-0 note. Fix is srs-rust#1227.

**Out of scope:** fixing note reads (#1227); paging/offset on `read`; changing `resources/read`; per-resource bounded variants; guard changes (read is not a write tool).

## Phases

### Phase 1: read tool + cap

**Agent:** MCP Adapter Worker

#### Tasks

- [x] `ReadToolInput`, `TOOL_READ`, `DESC_READ`, list entry in tools.rs
- [x] `tools/call` branch in lib.rs: `read` -> `srs_resources::read_resource` -> tool result with cap
- [x] Update tool-count tests (28 -> 29)
- [x] INSTRUCTIONS wording
- [x] Tests: read equals resources/read text for map and record; unknown uri errors as resources/read does; oversize truncates with notice and metadata; wrong-repo uri invalid params
- [x] Tier-0 note read check (report)

#### Acceptance Criteria

- [x] for results at or under the cap, `read {uri}` text == `resources/read` contents[0].text (test `read_tool::read_equals_resources_read_and_errors_as_there`)
- [x] oversize result is <= cap + notice, valid UTF-8, flagged truncated
- [x] No per-URI logic outside `read_resource`

#### Testing

```bash
cargo test -p srs-mcp-core -p srs-mcp
cargo clippy -- -D warnings
```

#### Milestone gate

Tests and clippy pass; checkboxes updated; commit.

## Final Acceptance

- [x] `cargo test` passes
- [x] `cargo clippy -- -D warnings` passes
- [x] no payload structs/schemas changed
- [x] dogfood: real repo via `srs mcp serve` (or McpDispatcher) `read` on map, record, oversize

## Coordination Rules

- #1218 touches INSTRUCTIONS and DESC_FIND; #1229 touches tools.rs. Keep edits local, rebase before PR.

## Assumptions

- The relay limit is ~128 KB per message; 96,000 bytes of text leaves headroom.
- A tool result with only `content` text (no duplicated structured payload) is acceptable to claude.ai.

## Review resolutions

Plan reviews: no blockers. Adopted: ADR wording (cap is a deliberate `read`-only bound), ADR-048 checklist, guard-bypass test, `shownBytes`, CLAUDE.md wording, named tests. Declined with reason: mapping not-found to `isError` tool results (issue says "errors as today"; one error contract per address, recorded in the amendment). Dogfood success = automated assertions via dispatcher plus a manual `srs mcp serve` run.

## Finding (report only, fix is #1227)

`read srs://…/record/{id}` of a Tier-0 note fails: `failed to load record at "records/notes/….json": missing field typeId` (`get_record_by_id` -> `load_record_by_id` deserialises every record as a typed Record).
