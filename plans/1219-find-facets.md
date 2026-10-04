# Plan: find facets, the derived repository map (#1219)

## Summary

A remote agent cannot see what a repository holds. `find` returns hits and a `total`, but nothing says which types, tags or select-field values exist. Add `facets` to `DiscoveryResult`: counts over the WHOLE match set (before paging, same rule as `total`) by type, by tag, and by each closed string field (the successor of select/multiselect, RFC-032 R3). `find {limit: 0}` with no filters is then the whole-repository map; with a type filter it is that type's keyword map. One core service change; CLI payload, MCP `find` and WASM `find` all serialise the same `DiscoveryResult`, so no adapter logic. Facet output is bounded (top N values per facet plus an `other` count; capped number of fields) to stay under the 128 KB relay limit.

**Spec gate:** no spec change. `discovery.json` defines only the query; `limit`/`offset`/`facets` shape the result and are implementation-level (as for #1217/#1227).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main agent |
| Repository Worker | main agent (`discovery_service.rs`, `text_projection.rs`) |
| CLI Worker | main agent (`payload.rs` only if schema needs it, schemas) |
| MCP Adapter Worker | main agent (`DESC_FIND` text only) |
| Verification | haiku Verification Agent |

## Architecture Decisions

Charter check (ADR-048): rule 1 spec-first: RFC-012 defines the query and the recall floor, result shaping is implementation-level, no spec ruling is being made. Rule 2 layer test: counting is semantics, so it lives once in the `srs-repository` service; adapters only serialise (capability-layering.md). Rule 3 one way per goal: reuse `find`'s hit loop, `build_field_text_index` (one package pass) and `container_service::membership_index`; no second scan, no new "map" command (the `repo map` resource stays the counts-of-everything-unfiltered view; facets are the filterable one). Rule 4: payload schema golden file regenerated; srs-vscode payload-contract mirror is a tracking item; srs-usage#881 comment. Rule 5: clear/complicated, Door 1 (additive result field).

| ADR / decision | Decision | Status |
|---|---|---|
| [ADR-019](../docs/adr/019-discovery-service.md) | Discovery composes existing services; amended with one line: `facets` are computed in the same pass, over the pre-paging match set | accepted (amended) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `FindPayload` embeds `DiscoveryResult`; golden schema regenerated if it changes | accepted |
| [ADR-013](../docs/adr/013-wasm-bindings.md) / [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | Bindings and MCP stay thin: same service call, same typed result | accepted |
| D1 Always present, not opt-in | `facets` is always in the result. A flag would be a second mode; cost is one extra counting pass over hits already in memory, and size is bounded | decided |
| D2 Facet fields = closed `string` fields | Select/multiselect = `datatype: string`, `valueDomain: closed`; list cardinality = multiselect. Counted by stored value (vocabularyRef values need no resolution). Open strings, numbers, dates and composites are not facets (unbounded cardinality) | decided |
| D3 Field facets keyed by `Field.name` | RFC-039 records key values by name, so the name is the only join key. Two fields sharing a name aggregate into one facet | decided (known ceiling, noted in doc comment) |
| D4 Shape: ordered lists, bounded | `FacetCounts {values:[{value,count}], other}`; values sorted count desc then value asc (deterministic); top `FACET_TOP_N = 20`; `other` = occurrences in omitted values (omitted when 0). Fields: at most `FACET_MAX_FIELDS = 25`, ranked by total occurrences then name | decided |
| D5 Notes | Tier 0 notes have no type: counted in `facets.notes`, not under a sentinel type key. They still contribute tags | decided |
| D6 Counting unit | One count per instance per value (a record's duplicate tags count once); a multiselect value counts once per instance | decided |
| D7 No new ADR | An additive result field implementing ADR-019; amend ADR-019 (result shape, stale pagination line) and ADR-037 (MCP `find` reply carries facets; `limit: 0` is the map call) | decided |
| D8 Closed means closed everywhere | A name is a facet only if every package field of that name is a closed string (review finding: a same-named open field would inject unbounded values) | decided |
| D9 Reply cost | Facets ride every `find` reply, so the default-limit MCP call is measured in S52 as well as `limit: 0`. Adapter-side omission rejected: adapter policy that changes semantics | decided |
| D10 Relation to other maps | `repo map` (unfiltered package/instance counts) and `agent-index` (orientation: types, entry points) remain; `find` facets are the filterable value-level map and do not duplicate their fields. S52 checks `facets.byType` totals agree with `repo map` instance counts. A Layer-2 index (#1239) must leave facets unchanged: they are counted over the Layer-1 match set | decided |

## Contracts

### CLI output contract (ADR-011)
`FindPayload { result: DiscoveryResult }` gains `result.facets`. `DiscoveryResult` is embedded as an opaque value in the golden schema (ADR-037 #1227 amendment), so the facets shape is pinned by the service unit tests plus the MCP and bindings tests, not by `payload_contracts`. Still run `cargo run --bin generate-schemas` and `cargo test --test payload_contracts`; expect no schema diff.

### Entity schema sync
No schema under `srs/docs/schema/2.0/` changes.

## Scope

- `DiscoveryFacets`, `FacetCounts`, `FacetCount`, `FieldFacet` types in `discovery_service.rs` (serde camelCase, `skip_serializing_if` empty).
- Per-candidate facet material (tags, closed-field values) collected in `find_tier2` / `find_tier0`, aggregated in `find` before paging.
- `FieldTextIndex` gains the set of closed-string field names (same package pass).
- `limit: 0` returns no hits but full facets (already true of `take(0)`; pinned by a test). CLI `--limit 0` and MCP `limit: 0` need no code.
- `DESC_FIND` and MCP instructions mention facets and `limit: 0`.
- ADR-019 amendment, `docs/dogfooding.md` scenario S52, plan, `srs-usage.md` note via issue comment on srs#881.

**Out of scope:** facets over numeric/date fields; facet drill-down syntax (the agent re-queries with the existing filters); facet filter on field value (a `fieldValue` query axis would be a spec change to `DiscoveryQuery`; file as follow-up); resolving vocabulary labels.

## Phases

### Phase 1: Service

**Goal:** `find` returns bounded, pre-paging facets.
**Agent:** Repository Worker

#### Tasks
- [ ] `text_projection.rs`: add `closed_names: HashSet<String>` to `FieldTextIndex` (built in `build_field_text_index` from `f.field_type.datatype == String && is_closed()`), accessor `is_closed_name`.
- [ ] `discovery_service.rs`: types above; `Candidate { hit, tags, selects }` internal; `find_tier2`/`find_tier0` return candidates; `build_facets(&[Candidate]) -> DiscoveryFacets`; `DiscoveryResult.facets`; the early-return branch (unknown container filter, nothing can match) returns `DiscoveryFacets::default()`.
- [ ] Unit tests (MemoryStore; `facets_count_the_whole_match_set_independent_of_paging`, `facets_follow_the_filters_and_count_notes`, `facet_values_are_bounded_with_an_other_count`): counts independent of limit/offset; same name closed and open is not a facet; non-string and empty-array values skipped; `limit:0` yields facets and no hits; tag counted once per instance; multiselect counts each value; notes counted in `notes`; top-N truncation with `other`; content/type filters narrow facets.

#### Acceptance Criteria
- [ ] `facets` counts equal those of an unpaged query
- [ ] Output bounded: more than N values yields N plus `other`
- [ ] No extra store scans (package loaded once, membership index unchanged)

#### Testing
```bash
cargo test -p srs-repository discovery
```

#### Milestone gate
`cargo test -p srs-repository`, `cargo clippy --workspace --all-targets -- -D warnings`, commit.

### Phase 2: Adapters, schema, docs

**Goal:** the same facets over CLI, MCP, WASM; docs and dogfood updated.
**Agent:** CLI Worker / MCP Adapter Worker

#### Tasks
- [ ] `cargo run --bin generate-schemas` (expect no diff); `cargo test --test payload_contracts`
- [ ] `DESC_FIND` (`crates/srs-mcp-core/src/tools.rs`) mentions facets and `limit: 0`; test in `crates/srs-mcp/tests/tools.rs` asserting `structuredContent.facets` with `limit: 0` returns no hits
- [ ] bindings test in `crates/srs-bindings/tests/find.rs` asserting camelCase keys `facets`, `byType`, `fields`; no binding code change
- [ ] amend `docs/adr/019-discovery-service.md` and `docs/adr/037-mcp-adapter-surface.md`; add S52 and a coverage matrix row in `docs/dogfooding.md`; comment on the-greenman/srs#881; file srs-vscode payload-mirror tracking issue linked under the same epic (srs-web#306)

#### Acceptance Criteria
- [ ] `srs find --limit 0` on a repo prints facets, no hits
- [ ] MCP `find {limit:0}` reply is under 128 KB on real muSrs

#### Milestone gate
`cargo test --workspace`, clippy, `payload_contracts`, commit.

## Final Acceptance

- [ ] `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] `cargo test --test payload_contracts` passes
- [ ] Dogfood against `/home/greenman/dev/muDemocracy.org/muSrs`: `find {limit:0}` size and top facets recorded in S52

## Coordination Rules

- Workers keep to write scopes. PR #1239 (BM25, `FindPage.rank`, `DiscoveryIndex`) edits `find`; PR #1239 is open (BEHIND) at planning time; rebase over it if it lands first. `facets.notes` is 0 under any Tier-2-only predicate, since those exclude notes.

## Assumptions

- The match set is held in memory as hits already (it is today); facet counting adds no I/O.
- Real muSrs closed fields (layer, pole, claimkind, warrantlevel, remedystatus, theme) are the field facets; the issue's `problem.kind` is data in another shape and will be checked in dogfooding.
