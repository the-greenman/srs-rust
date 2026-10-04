# Plan: Bounded neighbours service/tool and MCP tree controls (#1229)

## Summary

An agent cannot read a record's neighbours with a size limit: `context/{id}` inlines every neighbour in both directions (782 `authored-by` edges on a muSrs hub) and is a resource claude.ai cannot call, and the `tree` resources hard-code `TreeOptions::default()`. This plan adds (1) one core service `context_query_service::list_neighbours` returning a bounded, paged page of edge+neighbour summaries (never full records) with `total`, exposed through the CLI (`srs relation neighbours`), a WASM binding and an MCP `neighbours` tool; and (2) `maxDepth` / `relationType` / `typeFilter` query parameters on the `tree` and `tree/{instanceId}` resource URIs, which the existing `build_tree` already supports. No spec change: read and result-shaping over existing relations.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main session |
| Repository Worker | main session |
| CLI Worker | main session |
| Bindings Worker | main session |
| MCP Adapter Worker | main session |
| Verification | Verification Agent (haiku) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

Autonomous run: the owner delegated the design pause; each decision is resolved from the charter, ADR-048 and existing patterns.

| # | Decision | Rationale / governing rule |
|---|---|---|
| D1 | One service `list_neighbours(store, NeighboursQuery, NeighboursPage)` in `context_query_service.rs` (beside `get_record_context`, reusing `EdgeDirection`); CLI, WASM and MCP are thin adapters (capability-layering.md; ADR-010, ADR-013, ADR-037). No new module. | One way per goal (charter Conformance cell "one way over many"). |
| D2 | Filter edges with `relation_service::load_relations` (pub(crate)) rather than `list_relations`, so endpoint labels and neighbour loads happen only for the returned page, not for all 782 edges. Same filter semantics (source/target/type). | The point of the issue is bounded cost; `list_relations` resolves labels for every match before any paging. |
| D3 | Paging mirrors `FindPage{limit: Option<usize>, offset}` from #1221: service `limit: None` = all; the MCP tool defaults to 25 (adapter's choice, per FindPage doc). `total` counts all matching edges before paging. | Existing pattern (#1217/#1221). |
| D4 | Result: `{instanceId, total, neighbours: [{direction: out|in, relationId, relationType, neighbour: {instanceId, label?, typeNamespace?, typeName?}}]}`. Never the full record. Dangling neighbour: ids kept, label/type omitted. Self-relation appears once per direction (as in `get_record_context`). Order: `(relationType, createdAt none-last, relationId)`, identical to context. | Deterministic, same as the context service. |
| D5 | Direction omitted = both. `EdgeDirection` gains `FromStr` ("out"/"in") used by CLI and WASM; MCP deserialises it via serde. | Single parse point. |
| D6 | Missing subject instance -> `RepositoryError::NotFound` (tool error). Tier-0 notes are valid subjects. | Consistent with context. |
| D7 | Tree controls are `?maxDepth=&relationType=&typeFilter=` query params on the existing tree resource URIs (same mechanism as context's `excludeRelationCategories`, #1188), NOT a new tool. #1220's `read` tool passes URIs through, so claude.ai reaches them. `SrsUri::Tree` / `TreeFrom` carry a `TreeQuery`. Bad/unknown param or non-integer depth -> invalid params. | One way per goal; the issue allows either; avoids a second tree surface. |
| D8 | The neighbour `uri` field is DEFERRED to #1227, which owns the shared `uri` field and where the `srs://` builder lives (today only in the `srs-mcp-core` adapter, which the core cannot depend on). Neighbour carries `instanceId`, so the agent can already form `record/{id}`. A comment is left on #1227. | Avoids inventing a core-side URI builder in parallel with #1227 (parallel implementation = drift). |
| D9 | CLI surface: `srs relation neighbours <ID> [--relation-type T] [--direction out|in] [--limit N] [--offset N]`; payload `NeighboursPayload` (service type embedded via `schemars(with=Value)`, like `RelationListPayload`). | ADR-011. |

No new ADR: implements ADR-010/011/013/037/048.

---

## Contracts

### CLI output contract (ADR-011)

New command `relation neighbours` -> `NeighboursPayload` in `crates/srs-cli/src/payload.rs`; run `cargo run --bin generate-schemas`, commit `schemas/payload/`; `cargo test --test payload_contracts` must pass. Existing commands unchanged (`tree` already exposes depth/relation-type/type-filter).

### Entity schema sync

No `srs/docs/schema/2.0/` change.

---

## Scope

- `list_neighbours` service + `NeighboursQuery`, `NeighboursPage`, `NeighboursResult`, `NeighbourEdge`, `NeighbourSummary` types, `EdgeDirection::from_str`.
- CLI `relation neighbours`, WASM `SrsRepository::neighbours`, MCP tool `neighbours`.
- MCP tree resource query params.
- Docs: `srs-usage.md` CLI reference note, MCP server instructions text, `docs/dogfooding.md` scenario.

**Out of scope:**

- `uri` on neighbours (D8, #1227).
- A WASM `tree` binding (no existing one; not requested).
- Relation-category filter on neighbours (context has one; follow-up if needed).

---

## Phases

### Phase 1: Core service

**Goal:** `list_neighbours` returns a bounded page with total.

**Agent:** Repository Worker

#### Tasks

- [ ] `EdgeDirection` `FromStr`/`as_str` in `context_query_service.rs`
- [ ] Types + `list_neighbours` (D2, D4, D6)
- [ ] Unit tests in the same file

#### Acceptance Criteria

- [ ] total counts pre-paging edges; `limit`/`offset` slice after sort; both directions by default; `direction` and `relation_type` filter
- [ ] labels/types resolved only for the page
- [ ] missing subject is NotFound; dangling neighbour listed without label

#### Testing

```bash
cargo test -p srs-repository neighbours
```

- `neighbours_pages_with_total`, `neighbours_filters_direction_and_type`, `neighbours_missing_subject_not_found`, `neighbours_dangling_neighbour_has_no_label`

#### Milestone gate

`cargo test -p srs-repository`, `cargo clippy -p srs-repository -- -D warnings`, tick boxes, commit.

### Phase 2: CLI + WASM

**Goal:** `srs relation neighbours` and `SrsRepository::neighbours` call the service.

**Agent:** CLI Worker, Bindings Worker

#### Tasks

- [ ] `RelationCommand::Neighbours` in `commands/mod.rs`, handler in `commands/relation.rs`
- [ ] `NeighboursPayload`; regenerate schemas
- [ ] WASM `neighbours(...)` in `srs-bindings/src/lib.rs`

#### Acceptance Criteria

- [ ] CLI output matches payload schema; bad `--direction` is a clap error
- [ ] bindings compile (`cargo check -p srs-bindings`)

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

- CLI integration test: paged neighbours with `total`.

#### Milestone gate

`cargo test -p srs-cli`, clippy, commit.

### Phase 3: MCP

**Goal:** `neighbours` tool and tree URI params.

**Agent:** MCP Adapter Worker

#### Tasks

- [ ] `NeighboursToolInput` (camelCase, deny_unknown_fields), `TOOL_NEIGHBOURS`, `DESC_NEIGHBOURS`, list_tools entry, `call_tool` arm (default limit 25). Edits local to the new tool.
- [ ] `uri.rs`: `TreeQuery` + parse/format for `tree` and `tree/{id}` queries; `lib.rs` read arm passes them to `TreeOptions`; update server instructions + tree resource descriptions.
- [ ] Tests in `crates/srs-mcp/tests/{tools,resources}.rs` and `uri.rs`

#### Acceptance Criteria

- [ ] tool result equals the direct service result; default limit 25 with full `total`
- [ ] `tree?maxDepth=0` returns roots only; `relationType`/`typeFilter` reach `build_tree`; unsupported query -> invalid params

#### Testing

```bash
cargo test -p srs-mcp -p srs-mcp-core
```

#### Milestone gate

`cargo test -p srs-mcp -p srs-mcp-core`, clippy, commit.

---

## Final Acceptance

- [ ] `cargo test` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] No schema mirror change
- [ ] Rebased on master (after #1221) with no change to find tool code

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- #1221 touches `tools.rs`/`discovery_service.rs`; #1227 adds `uri` to hits. Keep `tools.rs` edits to the new tool; rebase before the PR.

## Assumptions

- #1220 (`read` tool) is not merged; tree params are on the URI so it passes them through unchanged.
- `Relation.created_at` is `Option<String>`.
