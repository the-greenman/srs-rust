# Plan: Paragraph context — extend `get_record_context` + MCP resource (#1134)

> Base: origin/master. Owner decisions 2026-10-03 recorded in Architecture Decisions.

## Summary

One read returns everything an agent needs about a paragraph (a container member): the record, every relation touching it in both directions with the neighbour record/note inline (comments, notes, problems, sources, challenges), and its arrangement subtree when a container is given. Per owner ruling D2 this EXTENDS `context_query_service::get_record_context` (no sibling fn): `RecordContextQuery` gains optional `containerId`; `relations` gains inbound edges (the "inbound deferred to #252" comment in the service; #252 itself closed, nothing open covers inbound edges) and per-edge `direction` + `neighbour`; result gains optional `entry`/`subtree` from `container_service::get_outline` (#1156). Exposed once each through CLI (`srs context record`, reusing the existing GLOBAL `--container` flag, which is documented as "scope to a container" and has no other meaning for this command, so no new flag name), MCP resource `srs://<repo>/context/{containerId}/{instanceId}` (plus `context/{instanceId}` container-less, same arm) and WASM `context_record` (optional `containerId`). Native and browser MCP share `srs_mcp_core::srs_resources::read_resource` (both wrap `SrsMcpApplication`), so identical output holds by construction; one parity test proves it.

Spec gate (Stage 1.5): NO spec change. Read-only projection, no new type/field/relation/schema; `srs://` is implementation tooling (ADR-037 sec. 6). No essay-package ids or relation names anywhere in srs-rust.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main session |
| Repository Worker | same session (service + tests) |
| CLI Worker | same session (payload, handler, golden schema) |
| MCP Adapter Worker | same session (write scope extended to `crates/srs-mcp-core/**`; agents.md line added) |
| Bindings Worker | same session |
| Verification | haiku subagent |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-001/010 | One sync service, typed in/out in srs-repository | governs |
| ADR-011 | `ContextRecordPayload` gains optional fields; regenerate golden schema | governs |
| ADR-013 | WASM `context_record` stays deserialize -> one service call -> serialize | governs |
| ADR-037 (+ #949 amendment) | New URI kind is a thin arm in srs-mcp-core serving the service result verbatim; amendment paragraph added | governs; amendment |
| ADR-048 | spec-first: no spec change; layer test: service once; one-way-per-goal: extend the existing context read, no twin; decision mode: complicated | governs |
| capability-layering.md | structural-not-nominal: `relationType` is a sort key / data, never branched on | governs |
| Owner D1/D3 | `relations` is ONE flat list of `{direction: out|in, relation fields (flattened RelationSummary), neighbour: full record/note}`, sorted relationType -> neighbour createdAt -> relationId. Evolves the existing outgoing list; no second list | ruled |
| Owner D2 | extend `get_record_context`; optional container id adds subtree; all new payload fields optional/additive | ruled |
| Owner D4 | subtree = outline entries only (id, depth, parent...) from `get_outline`; neighbours depth 1 | ruled |
| Owner D5 | URI `srs://<repo>/context/{containerId}/{instanceId}`, container-less `context/{instanceId}` via same arm | ruled |
| Research register | item 1 (MCP, adopt) already shipped; resource only | cited |

No new ADR (one amendment paragraph to ADR-037).

Notes: #1165 write guard untouched (read). RFC-046 `createdBy` rides inside serialized Record/Note unchanged. Flattened `RelationSummary` keeps existing consumers working (fields unchanged); they now also see inbound edges, distinguishable by `direction`; `sourceId`/`targetId` unchanged in meaning.

## Contracts

### CLI output contract (ADR-011)
`ContextRecordPayload` (`crates/srs-cli/src/payload.rs`) changes: `relations` element schema stays `Value` (no golden diff there), plus new optional `container_id`, `entry`, `subtree` (skip_serializing_if none). Run `cargo run --bin generate-schemas`, commit `schemas/payload/*`; `cargo test --test payload_contracts`.

### Entity schema sync
No.

## Scope

- `RecordContextQuery { record_id, container_id: Option<String> }` (`#[serde(default)]`, back-compat for the binding JSON).
- `RecordContextResult.relations: Vec<ContextRelation>`; `ContextRelation { direction: Out|In, #[serde(flatten)] relation: RelationSummary, neighbour: Option<ContextInstance> }`; `ContextInstance` tagged enum (record|note; `LoadedInstance` is not Serialize).
- `RecordContextResult.container_id/entry/subtree` optional: `get_outline`, locate the instance (not a member -> `InvalidInput`), `entry = entries[i]`, `subtree = entries[i+1..entry.run_end]`.
- Neighbour of a relation = the other endpoint; unresolved -> `neighbour: null` (no hard error; dangling edges already exist).
- CLI: `cmd_context_record` passes `ctx.container_id`.
- MCP: `SrsUri::Context{container_id: Option<String>, instance_id}`; parse `context/<id>` and `context/<cid>/<iid>`; `read_resource` arm; two templates; INSTRUCTIONS sentence.
- WASM: `context_record` JSON accepts optional `containerId` (doc comment updated).
- Docs: ADR-037 amendment, `plans/agents.md` scope line, `docs/dogfooding.md` scenario, srs-usage.md (separate srs PR).

**Out of scope:** comment-specific shaping; category filtering of structural edges (`precedes`/`contains` come through; filter later if noisy); depth-N expansion; subtree record inlining; `maxDepth`; filling `tagged_chunks` (#582); `resources/list` enumeration of every pair; JsonStore-specific work.

## Phases

### Phase 1: Service (Repository Worker)

**Goal:** `get_record_context` returns bidirectional edges with neighbours and optional subtree.

#### Tasks
- [x] Add `Direction`, `ContextInstance`, `ContextRelation`; extend query/result in `crates/srs-repository/src/context_query_service.rs`.
- [x] Implement inbound `list_relations(target=)`, neighbour load (cached per id), sort (relationType, neighbour `created_at` with None LAST, relationId), optional outline subtree.
- [x] Fix existing test `record_context_relations`; add new tests.

#### Acceptance Criteria
- [x] Inbound and outbound both present with correct `direction`; unrelated absent.
- [x] Record and Note neighbours both serialize; dangling -> null neighbour.
- [x] Subtree equals outline slice for nested fixture; non-member container errors; no container -> `entry`/`subtree` absent.
- [x] Ordering is chronological within a relationType.

#### Testing
`cargo test -p srs-repository context_query`: `record_context_inbound_and_outbound`, `record_context_orders_thread_by_created_at`, `record_context_note_neighbour`, `record_context_dangling_neighbour`, `record_context_subtree_slice`, `record_context_non_member_errors`.

#### Milestone gate
`cargo test -p srs-repository`; `cargo clippy -p srs-repository --all-targets -- -D warnings`; commit.

### Phase 2: CLI + WASM + MCP (CLI/Bindings/MCP workers)

**Goal:** the one read is reachable from CLI, WASM, MCP identically.

#### Tasks
- [x] CLI handler + payload fields; `cargo run --bin generate-schemas`; commit schemas.
- [x] WASM `context_record` docs/behaviour; test in `crates/srs-bindings/tests/context_query.rs`.
- [x] `crates/srs-mcp-core/src/uri.rs` Context variant, parse/format/templates, roundtrip test; `lib.rs` read arm, templates, instructions.
- [x] Parity test: `McpDispatcher<SrsMcpApplication<FileStore>>` vs browser `McpSession` over same data; equals service/binding output.
- [x] ADR-037 amendment (Phase 2 task, not blocking): update the section 6 URI list, the two templates, the INSTRUCTIONS sentence; agents.md scope line.

#### Acceptance Criteria
- [x] `srs context record <id> --container <cid>` returns subtree; without it unchanged shape plus new edges.
- [x] resources/read context URIs work (both forms); malformed -> invalid_params.
- [x] Native vs browser byte-equal.

#### Testing
`cargo test -p srs-cli -p srs-bindings -p srs-mcp-core -p srs-mcp`; `cargo test --test payload_contracts`.

#### Milestone gate
tests + `cargo clippy --all-targets -- -D warnings`; commit.

## Final Acceptance
- [ ] `cargo test` and `cargo clippy --all-targets -- -D warnings` pass
- [ ] `cargo test --test payload_contracts` passes; schemas regenerated
- [ ] `bash scripts/check-schema-sync.sh` passes (no entity schema change)
- [ ] Native/browser parity test passes
- [ ] `rg 'comments-on|mudemocracy' crates` empty
- [ ] srs-usage.md PR opened in `srs` and linked

## Coordination Rules
Per TEMPLATE; worktree off fresh origin/master (srs-rust#874).

## Assumptions
- `get_outline` and `OutlineEntry.run_end` (exclusive subtree end) on master (PR #1159).
- Subject must be a Tier-2 record: a Tier-0 note subject returns the existing not-found error (documented in srs-usage); notes are supported as neighbours. Global `--container` help text updated to mention `context record`.
- Hub records inline many neighbours; accepted for v1, revisit with category filtering.
- Neighbour inline is bounded by one record's edge count.
- srs-web#329 (AttachmentGlyph/HoverCard/PinnedPane) is the S2 consumer; no separate consumer issue needed.
