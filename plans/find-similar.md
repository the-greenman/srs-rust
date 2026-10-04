# Plan: find similar (more-like-this over the BM25 index)

## Summary

Issue srs-rust#1230. Once a record is found there is no way to ask "what else is about this?". Add `similar`: the source instance's own top-weighted terms become a query against the existing `Bm25Index` (#1228). Hits use the normal `find` hit shape, the source is excluded, structured predicates compose as in `find`. Deterministic, no new dependency. Exposed in core, CLI (`srs find --similar <id>`), WASM (`findSimilar`) and MCP (`similar`, limit default 25). The eval harness (#1231) gains a `similar` row to test whether lexical similarity recovers the vocabulary-mismatch misses. No spec change: implementation-level ranking (RFC-012 [R4]); Stage 1.5 gate passes.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main agent |
| Repository Worker | main agent (discovery_index.rs, discovery_service.rs, eval test) |
| Bindings / CLI / MCP Worker | main agent |
| Verification | haiku reviewer |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-019](../docs/adr/019-discovery-service.md) (amended here) | `similar` is a second use of the one `DiscoveryIndex`/`Bm25Index`; no second scorer ("one way per goal"). Trait gains `top_terms(id, n)`; the BM25 `score` is reused unchanged for ranking | amendment |
| ADR-048 / capability-layering | Capability lives once in `discovery_service::similar`; CLI/WASM/MCP are thin adapters | accepted |
| Decision D1 | Query = top 10 terms of the source by weight x tf x idf (token-level df, length >= 3, ties by term). Candidate set = structured-filter matches with score > 0 (OR of terms, not all-words, since Layer-1 AND would return nothing). Source excluded | decided (charter: deterministic, cell implementation-ranking) |
| Decision D2 | CLI is `srs find --similar <id>` (reuses `FindArgs`, `FindPayload`, no new payload struct); conflicts with `--text`. MCP is a separate `similar` tool (different input: `instanceId`, no `contentMatch`); WASM `findSimilar` | decided, one result shape on every surface |
| Decision D3 | `similar` always returns ranked hits with `score`; `rank` flag does not apply. Total = number of candidates with score > 0 | decided |
| Alignment register | Interop/agent-facing surface; no register entry contradicted (lexical only, embeddings stay srs#726 open question 6) | n/a |

| Decision D4 | `similar` is a separate operation from `find`: it has no `contentMatch` and no Layer-1 recall floor; ADR-019 decision 5 ("index only orders") governs `find` only. The ADR amendment says so. `DiscoveryIndex` is now ranking plus similarity (`top_terms`) | decided |
| Decision D5 | Term selection uses whole-token df (cheap map built at index build); scoring keeps the existing substring df. Two notions, justified: selecting from thousands of distinct tokens by substring scan would be O(tokens x corpus). Both quantised/f64 | decided |
| Decision D6 | Core `similar` rejects `contentMatch` (InvalidInput) so the invalid combination is core validation, not adapter logic; the CLI flag just forwards. `similar` ignores `FindPage.rank` (always ranked). Unknown id and non-instance ids (containers) -> `InstanceNotFound` | decided |

No new ADR: this amends ADR-019 (section appended and "Amended by" line updated). No schema mirror or srs-vscode payload change (FindPayload unchanged).

## Contracts

- CLI: `find --similar` returns the existing `FindPayload`; no payload struct change, no golden-schema change. `payload_contracts` must still pass.
- MCP: new tool `similar` registered next to `find`; input schema derived from a new `SimilarToolInput`.
- No entity schema change.

## Scope

- `DiscoveryIndex::top_terms`; token df map in `Bm25Index`.
- `discovery_service::similar(store, instance_id, query, page)`; shared hit shaping with `find` (refactor, no behaviour change).
- CLI flag, WASM method, MCP tool, unit + integration tests, eval row, docs, dogfooding scenario.

**Out of scope:** embeddings/semantic layer (srs#726); stemming/synonyms; a token inverted index for scoring df (existing ponytail note); per-term explanation output.

## Phases

### Phase 1: core
**Agent:** Repository Worker
#### Tasks
- [ ] `top_terms` on trait and `Bm25Index` (token df map built in `build`)
- [ ] `similar` in discovery_service, refactor shared page shaping
- [ ] unit tests: source excluded, deterministic, predicates compose (type/tier/container), unknown id error, `--text` conflict rejected at adapter
#### Acceptance Criteria
- [ ] Same-topic records outrank unrelated; source never returned; repeat call identical
#### Testing
Tests in discovery_service.rs: `similar_excludes_source_and_ranks_related_first`, `similar_is_deterministic`, `similar_composes_structured_predicates`, `similar_unknown_id_and_contentmatch_rejected`; index test `top_terms_prefers_rare_title_terms`.
`cargo test -p srs-repository discovery` ; `cargo clippy -p srs-repository -- -D warnings`
#### Milestone gate
Criteria checked; named tests exist and pass; clippy clean; checkboxes updated; commit.

### Phase 2: adapters
**Agent:** CLI / Bindings / MCP Worker
#### Tasks
- [ ] CLI `--similar`; WASM `find_similar`; MCP `similar` (limit 25), tool-list test updates
#### Acceptance Criteria
- [ ] All surfaces return the same hits for the same input; tests in srs-cli, srs-bindings, srs-mcp pass

#### Testing
`cargo test -p srs-cli -p srs-bindings -p srs-mcp` ; tests: `find_similar_cli_excludes_source_and_rejects_text` (CLI), `find_similar_matches_core` (bindings/native test), `similar_tool_default_limit_25_and_tool_list` (MCP), `payload_contracts` unchanged.
#### Milestone gate
Criteria checked; named tests exist and pass; `cargo test` and `cargo clippy --workspace --all-targets -- -D warnings` clean; checkboxes updated; commit.

### Phase 3: eval, docs, dogfood
- [ ] test `similar_eval` in discovery_eval.rs: `similar` evaluation in `discovery_eval.rs` (related-to set from questions' expected ids; report the vocabulary-mismatch recovery)
- [ ] ADR-019 amendment, srs-usage comment on srs#881, `docs/dogfooding.md`

## Final Acceptance

- [ ] `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `generate-schemas` yields no diff (ADR-011)
- [ ] `cargo test --test payload_contracts`
- [ ] Eval result posted on #1231 and srs#726

## Coordination Rules

- Agents keep to their write scopes; Lead Integrator owns API naming.
- Phase gate: criteria, tests, lint, plan checkboxes, commit.

## Assumptions

- Top-N = 10 terms is a tunable heuristic, not a contract.
- Decision mode: complicated, Door 1 (implementation-level, reversible).
