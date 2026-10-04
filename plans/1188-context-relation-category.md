# Plan: Relation-category filter on the record context read (#1188)

> Base: origin/master. Owner decisions 2026-10-04 recorded in Architecture Decisions. Deferred from #1134 ("category filtering of structural edges; filter later if noisy").

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
| Owner D1 | Exclude list `excludeRelationCategories`; empty = unchanged | ruled |
| Owner D2 | Raw `RelationTypeCategory` wire values, no `structural` preset (core: contains=composition, precedes=sequence, verified) | ruled |
| Owner D3 | Edges whose type has no installed definition are KEPT | ruled |
| Owner D4 | Silent drop: no payload / golden-schema change, no omitted count | ruled |
| Owner D5 | CLI repeatable `--exclude-category`; MCP formal RFC 6570 template `srs://<repo>/context/...{?excludeRelationCategories}` so it shows in the template list | ruled |
| Owner D6 | Shared `FromStr for RelationTypeCategory` in srs-core | ruled |
| ADR-025 | Relation types resolve via `Package::resolve_relation_type` over `load_package()` (implicit core merge), same `key` match as validation. A namespaced custom type whose definition key differs is unclassified and KEPT (D3) | governs |
| All other ADRs | Architecture Reviewer read all 51: none else governs a read-side filter | reviewed |

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

### Phase 1: service + adapters (WIP commit 90f2e95a done; remainder below)

**Agent:** Repository, CLI, MCP workers (same session, sequential)

- [x] `crates/srs-core/src/types/relation_type_definition.rs`: `FromStr for RelationTypeCategory` + every-variant round-trip test
- [x] `crates/srs-repository/src/context_query_service.rs`: `RecordContextQuery.exclude_relation_categories`, filter before neighbour load, package loaded only when filtering
- [x] `crates/srs-cli/src/commands/context.rs`: repeatable `--exclude-category` via clap `value_parser!`
- [x] `crates/srs-bindings/src/lib.rs` doc on `context_record`
- [x] `crates/srs-mcp-core/src/uri.rs` (`?excludeRelationCategories`, `&` split, `{?excludeRelationCategories}` template, round-trip) and `lib.rs` (parse to categories -> `invalid_params`; instructions sentence)
- [x] Docs: ADR-037 amendment, `docs/dogfooding.md` S48; srs-usage.md on a separate `srs` branch

#### Testing
- `record_context_excludes_relation_categories` (service): sequence edge dropped, association kept, empty list keeps both
- `from_str_round_trips_every_variant` (srs-core)
- `context_uri_query_parses_and_rejects_unknown_keys`, `uri_roundtrip_all_kinds` (uri.rs)
- `read_context_exclude_categories_matches_service_and_rejects_unknown` (srs-mcp resources)
- `context_record_query_accepts_exclude_relation_categories_key` (bindings: pins the WASM camelCase key)
- `unknown_category_is_rejected_known_is_parsed` (CLI)

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
- [ ] No relation-name branching in non-test code of `context_query_service.rs` (`rg -n '"precedes"|"contains"'` on it returns 0 matches)

## Coordination Rules
Per TEMPLATE; worktree off fresh origin/master (srs-rust#874). Agents push branches only; owner opens the PR after review.

## Assumptions
- `RelationTypeDefinition.category` for core `contains`/`precedes` is composition/sequence (verify against the core package at implementation).
- srs-web drops its client filter after this ships and passes the two categories.
