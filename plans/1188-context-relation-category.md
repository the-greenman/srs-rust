# Plan: Relation-category filter on the record context read (#1188)

> Base: origin/master. DRAFT: the public API shape below awaits owner decision (Stage 2.4). Deferred from #1134 ("category filtering of structural edges; filter later if noisy").

## Summary

`get_record_context` (#1134) inlines every edge touching a record, structural ones (contains/precedes) included, each with the full neighbour. On hub records this is noisy; srs-web's essay editor currently filters structural edges client-side (`toAttachment`). Add one OPTIONAL filter, driven by `RelationTypeDefinition.category` (never by relation name), implemented once in the service and exposed through CLI, WASM and the MCP context resource. Default (no filter) is byte-identical to today.

**Spec gate (Stage 1.5):** no spec change. `RelationTypeDefinition.category` already exists in the spec and core; this is a read-only filter on an implementation projection (no new type/field/relation/schema; `srs://` is implementation tooling, ADR-037 sec. 6).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main session |
| Repository Worker | same session (service + test) |
| CLI Worker | same session (flag) |
| Bindings Worker | same session (doc only; input is serde) |
| MCP Adapter Worker | same session (URI query) |
| Verification | haiku subagent |

## Architecture Decisions

| Ref | Decision | Status |
|---|---|---|
| ADR-001/010 | Filter lives once in `context_query_service` as a typed field on `RecordContextQuery`; adapters map flags/URI to it | governs |
| ADR-011 | Output payload `ContextRecordPayload` is unchanged (filter is input-only): no golden-schema diff | governs |
| ADR-013 | WASM `context_record` stays deserialize -> one service call -> serialize; new key arrives via serde, no binding logic | governs |
| ADR-037 (+ #949 amendment) | Context URI arm stays a thin pass-through; query string parsed in `uri.rs` and handed to the service | governs; addendum sentence |
| ADR-048 / capability-layering | Spec-first: no spec change. Layer test: service once. One way per goal: extends the existing read, no twin. Structural-not-nominal: category from the installed definition, relation name never branched on. Decision mode: complicated | governs |
| Interop register (`srs/docs/research/alignment-opportunities.md`) | Item 1 (MCP surface) already adopted/shipped; it has no entry on context shaping or filtering, so nothing contradicted | cited |
| Other ADRs (002-009, 012, 014-036, 038-047, 049-050) | Read by title/relevance only; none governs a read-side filter on an existing service. A full read of all 51 was not done and a reviewer should confirm | noted |

No new ADR: implements ADR-010/037 patterns.

## Contracts

### CLI output contract (ADR-011)
Input-only change. `ContextRecordPayload` unchanged; `cargo test --test payload_contracts` must still pass (it does at the WIP commit).

### Entity schema sync
No.

## Scope

- `RecordContextQuery.exclude_relation_categories: Vec<RelationTypeCategory>` (`#[serde(default)]`, camelCase `excludeRelationCategories`). Empty = today's behaviour.
- Filter in `get_record_context`: resolve each edge's `relationType` via `Package::resolve_relation_type`, drop the edge when the definition's category is listed. Edges whose type has no installed definition are KEPT (cannot be classified).
- `FromStr for RelationTypeCategory` in srs-core (wire spelling, via serde) so adapters share one parser.
- CLI: `srs context record <id> --exclude-category <cat>` (repeatable).
- WASM: `context_record` JSON accepts `"excludeRelationCategories": ["composition","sequence"]`.
- MCP: resource `srs://<repo>/context/[{containerId}/]{instanceId}?excludeRelationCategories=composition,sequence`; unknown category -> `invalid_params`; unknown query key -> URI error. (No context tool exists; the resource is the only MCP surface.)

**Out of scope:** an "omitted count" in the payload; an include-list; a named `structural` preset; filtering `entry`/`subtree`; srs-web changes.

## Phases

### Phase 1: service + adapters

**Agent:** Repository / CLI / MCP workers

- [x] srs-core `FromStr` for `RelationTypeCategory` (WIP commit 90f2e95a)
- [x] service field + filter + test `record_context_excludes_relation_categories` (WIP)
- [x] CLI flag, WASM doc, MCP URI query + template description (WIP)
- [ ] URI parse/roundtrip test for the query form; MCP and CLI tests (invalid category, filter applied)
- [ ] ADR-037 one-line addendum; `docs/dogfooding.md` line

#### Acceptance Criteria
- [ ] No filter: output identical to master
- [ ] `excludeRelationCategories=[composition,sequence]` drops contains/precedes edges and keeps others, with no name matching
- [ ] Same result on CLI, WASM and MCP for the same input

#### Milestone gate
`cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --test payload_contracts`.

## Final Acceptance
- [ ] `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` exit 0
- [ ] `cargo test --test payload_contracts` exit 0, no schema diff
- [ ] `bash scripts/check-schema-sync.sh` exit 0 (no entity schema change)
- [ ] `rg 'precedes|contains' crates/srs-repository/src/context_query_service.rs` shows no relation-name branching

## Coordination Rules
Per TEMPLATE; worktree off fresh origin/master (srs-rust#874). Agents push branches only; owner opens the PR after review.

## Assumptions
- `RelationTypeDefinition.category` for core `contains`/`precedes` is composition/sequence (verify against the core package at implementation).
- srs-web drops its client filter after this ships and passes the two categories.
