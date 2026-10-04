# Plan: Navigable find hits (#1227)

## Summary

An agent that gets a `find` hit cannot follow it without assembling a URI, `agent-index` dead-ends on types and file-path entry points, and `record/{id}` fails on a Tier-0 note. Add `uri`/`typeId`/`containerIds` to hits, `uri` to neighbours, `typeId` and resolved entry points to agent-index, and make `record/{id}` resolve notes. No spec change (result shaping and adapter fields; RFC-012 defines the query, not the hit envelope).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (single session) |
| Verification | Sonnet architecture reviewer x2 |

## Architecture Decisions

| ADR / source | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | Dated amendment (2026-10-04, #1227) records all decisions below | accepted |
| capability-layering | URI builder for instance/type/container lives in `srs_repository::resource_uri`; `srs-mcp-core::uri::format` delegates. Hits and neighbours are service types, so the field reaches CLI, WASM and MCP at once | accepted |
| RFC-034 / RFC-043 | `containerIds` = declared membership only (a container's own outline), never `contains` traversal or nested effective membership; one `membership_index` pass per page | accepted |
| ADR-011 | `entryPoints` string[] to `{path, instanceId?, uri?}[]`: breaking, accepted. Consumers checked (srs-vscode, srs-web, srs-bindings, muDemocracy.org): none read it. A parallel `entryPointUris` array rejected (one way per goal) | accepted |
| ADR-011 | Golden schemas unchanged: payloads embed service types as opaque values; `generate-schemas` run shows no diff | verified |
| ADR-048 / charter | Layer: core service + adapter exposure; decision mode: complicated (owner-triaged issue, coordinator-approved) | accepted |
| `record/{id}` | Resolve through `get_instance_by_id` (any tier), serve Note JSON for Tier 0 | accepted |

No new ADR: ADR-037 amendment only.

## Contracts

- CLI: `find` hit gains `uri`, `typeId?`, `containerIds`; `relation neighbours` neighbour gains `uri`; `repo agent-index` changes as above. Golden schemas unchanged (opaque embeds); `cargo test --test payload_contracts` passes.
- Entity schemas: none touched.

## Scope

In: `crates/srs-repository/src/{resource_uri,discovery_service,context_query_service,agent_index_service,container_service}.rs`, `crates/srs-mcp-core/src/{uri,lib}.rs`, `crates/srs-cli/src/commands/repo.rs` (render). Out of scope: BM25 ranking (#1228), spec changes, `uri` on other payloads.

## Phases

1. `resource_uri` + hit fields + neighbour uri. Gate: `cargo test -p srs-repository`.
2. agent-index typeId and entry points. Gate: same.
3. MCP `record/{id}` note resolution + test. Gate: `cargo test -p srs-mcp`.
4. Docs (ADR-037 amendment, dogfooding S51), srs-usage note on srs#881.

## Final Acceptance

- [x] `cargo test`, `cargo clippy --all-targets -- -D warnings`
- [x] `cargo run --bin generate-schemas` leaves no diff
- [x] Dogfood S51 against muSrs over real `srs mcp serve`
