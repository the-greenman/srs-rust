# Plan: Real golden schemas for `find` and `relation neighbours` (#1258)

## Summary

`find` and `relation neighbours` payload goldens embed `result` as `true` (any JSON), so `payload_contracts` cannot fail for any change to hits, facets or `NeighbourSummary`. Give both payloads real schemas via local mirror structs in `payload.rs`, as ADR-011 prescribes (`srs-core`/`srs-repository` stay schemars-free). The mirrors are what the handlers serialize, so the golden describes what the CLI emits. Each `From` impl destructures the service struct exhaustively, so an added/removed/renamed service field breaks compilation. Serde-attribute drift on the service type (`skip_serializing_if`, `flatten`, `rename`) is caught only by the fidelity test, which is the declared parity gate (ADR-048 rule 3) for these twins.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead |
| CLI Worker | Lead |
| Verification | Verification Agent |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Local mirror structs with `From` for service types; no schemars in library crates | accepted |
| [ADR-019](../docs/adr/019-discovery-service.md) / ADR-010 | `find` and `similar` both emit FindPayload; handler stays one service call plus `.into()` | accepted |
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) | One way per goal: reuse the existing mirror pattern (`NoteTagEntry`, `RepoDoctorFinding`), no new mechanism | accepted |

Mirrors are declared twins of the service types, parity gate = fidelity test. ADR-048 rules 1/2/5: no spec ruling, layer = CLI adapter only, mode clear, Door 1 (`gate:auto-merge`). MCP and WASM return service types directly and stay uncontracted (out of scope). No new ADR: this applies ADR-011 as written. Rejected alternative: deriving `JsonSchema` on the service types (adds schemars 0.8 to srs-repository, which already carries optional schemars 1 for MCP; ADR-011 forbids). Not a long-term design decision: JSON output is unchanged, so no design pause.

## Contracts

- CLI output: serialized JSON unchanged byte-for-byte; golden `find.json` and `relation-neighbours.json` change from opaque to full schemas.
- No entity schema (`srs/docs/schema/2.0`) change, so no spec RFC (Stage 1.5: none required).

## Scope

- Mirror structs in `crates/srs-cli/src/payload.rs`: `DiscoveryResultPayload`, `DiscoveryHitPayload`, `DiscoveryFacetsPayload`, `FacetCountsPayload`, `FacetCountPayload`, `FieldFacetPayload` (flattened counts), `NeighboursResultPayload`, `NeighbourEdgePayload`, `NeighbourSummaryPayload`, `EdgeDirectionPayload` enum (`out`/`in`, exhaustive match in `From`). Mirrors: DiscoveryResult, DiscoveryHit, DiscoveryFacets, FacetCounts, FacetCount, FieldFacet (srs_repository::discovery_service); NeighboursResult, NeighbourEdge, NeighbourSummary, EdgeDirection (srs_repository::context_query_service, which exists). Preserve `skip_serializing_if`, `default`, `rename_all = camelCase`.
- `From<service type>` impls with exhaustive destructuring.
- `FindPayload.result` / `NeighboursPayload.result` use the mirrors; handlers `find.rs`, `relation.rs` call `.into()`.
- Regenerate goldens with `cargo run --bin generate-schemas`.
- New test: serialization of real service values through the mirror equals serialization of the service value (guards mirror fidelity incl. skipped fields).
- Demonstrated gate bite: temporarily rename a hit field, show `payload_contracts` fails, revert.
- Confirm no other golden has `"result": true`.

**Out of scope:** other `#[schemars(with = "serde_json::Value")]` embedded fields in other payloads (ADR-011 accepted trade-off; separate follow-up if wanted). Mirror sync in srs-vscode is filed after merge (ADR-048 rule 4).

## Phases

### Phase 1: Mirrors, goldens, tests

**Goal:** both goldens carry full schemas and the gate bites.

**Agent:** CLI Worker

#### Tasks

- [x] Add mirror structs and `From` impls in `payload.rs`
- [x] Switch `FindPayload`/`NeighboursPayload` and handlers
- [x] Add `crates/srs-cli/tests/payload_mirror_fidelity.rs`: fixtures exercise every Option Some/None, facets empty/non-empty, `notes`/`other` zero/non-zero, flatten, both directions, f32 score; compare as `serde_json::Value`; plus one run of real `find`/`neighbours` service output on a small repo
- [x] Run `generate-schemas`, commit goldens
- [x] Validate gate bites (manual, not a deliverable): rename a hit field in the mirror, `payload_contracts` fails; add a field to `DiscoveryHit`, compile fails; revert all

#### Acceptance Criteria

- [x] `find.json`, `relation-neighbours.json` list hit/facet/neighbour fields with types and required sets
- [x] No golden contains `"result": true`
- [x] CLI integration tests for find/neighbours unchanged and pass
- [x] Renaming a mirror hit field fails `payload_contracts`

#### Testing

```bash
cargo test -p srs-cli --test payload_contracts
cargo test -p srs-cli
```

#### Milestone gate

Per TEMPLATE.md: tick criteria and checkboxes, criteria checked, tests pass, `cargo clippy --workspace --all-targets -- -D warnings`, commit.

## Final Acceptance

- [x] `cargo test --workspace` passes
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] CLI output format unchanged
- [x] `cargo test --test payload_contracts` passes
- [x] `grep -rnE '"result": (true|\{\})' crates/srs-cli/schemas/payload` is empty; goldens contain `hits` and `neighbours`
- [ ] srs-vscode mirror-sync issue filed after merge, linked under muDemocracy.org#261

## Assumptions

- schemars 0.8 renders `#[serde(flatten)]` on `FieldFacet.counts` correctly (verify by spike on first generate).
