# ADR-019: Discovery Service, `find` command, and deferred index trait

- **Status:** accepted
- **Date:** 2026-06-28
- **Supersedes:** —
- **Superseded by:** —
- **Amended by:** srs-rust#797 (Tier 0/1 discovery landed — see the note below §Consequences); srs-rust#1228 (first `DiscoveryIndex`, BM25 ranking — see the end); srs-rust#1219 (result facets — see the end)

> Tracking note: epic #212 issue #214 titled this "ADR-018". In this repository
> 018 is already taken (`018-container-view-column-source-precedence.md`), so the
> discovery ADR is **019**. The issue number is stale, not the decision.

## Context

SRS clients need to answer "what matches?" — filter records by type/container/tag/
lifecycle and search their text. Today this is ad-hoc: srs-web hand-rolls a
TypeScript filter, and a recent srs-gov spike added a bespoke
`record_projection_service` in `srs-repository` that re-expressed visible fields,
hidden states, searchable fields, tag filters, and sort order in its own input
struct — bypassing the authored DocumentView/View model and duplicating
already-planned work. That service has been removed.

The portable contract already exists: **RFC-012 (`ext:discovery`)**, committed as
`docs/schema/2.0/discovery.json` (`DiscoveryQuery`, `TextSegment`,
`ConformanceScenario`). It defines the structured filter axes, the deterministic
content-match recall floor, and the `ValueType`-driven Text Projection. What is
missing is the **srs-rust implementation** of that contract.

The substrate to reuse already exists in `srs-repository`:
- `record_store::list_records_filtered` / `RecordListFilter` (type namespace/name,
  container membership, single tag) — the structured-filter pass.
- `record_label::{build_field_name_index, record_display_label}` — field-name
  index and hit labels.
- `package_service::list_fields` → `FieldSummary { id, name, value_type, .. }` —
  enough to drive the searchable/non-searchable `ValueType` split without loading
  full `Field` definitions.
- `srs_core::types::field::ValueType`, `srs_core::types::record::{Record,
  FieldValue, FieldValueEntry, FieldGroupValue}`.

This is a Layer-1 capability per `docs/architecture/capability-layering.md`:
implemented once in the core, consumed identically by CLI, bindings, and web.

## Decision

Introduce a discovery capability in `srs-repository`, conformant to RFC-012.

**1. `project_text` / `TextSegment` (text-projection primitive).**
```rust
pub struct TextSegment { pub field_id: String, pub field_name: String, pub text: String }
pub fn project_text(record: &Record, field_defs: &FieldTextIndex) -> Vec<TextSegment>;
```
- Searchable `ValueType`s: `String | Text | Url | Select | Multiselect`.
  Non-searchable: `Number | Boolean | Date`.
- One segment per searchable scalar value, including repeated `entries` and
  `group_values` field values.
- Append display label (sentinel `field_id`/`field_name` = `label`) and each tag
  (sentinel `tag`) as extra segments, so title search keeps working, generalized
  to all text fields.
- `text` holds the **raw** stored value. Normalization (NFC + Unicode simple
  lowercasing) is applied **at match time**, not at construction — exactly as the
  schema specifies — so the segment stream is implementation-reproducible.
- Deterministic segment order: record `field_values` order → `group_values` →
  label → tags.

**2. `discovery_service::find` (Layer-1 deterministic search).**
```rust
pub fn find(store: &dyn RepositoryStore, query: DiscoveryQuery)
    -> Result<DiscoveryResult, RepositoryError>;
```
`DiscoveryQuery` mirrors `discovery.json` (the canonical contract — **not** the
draft struct in issue #216, which predates the merged schema):
`type_id`, `type_namespace`, `type_name`, `container_id`, `tag: Vec<String>`
(AND-conjunction), `lifecycle_state` (exact include), `tier`, `content_match`.
`DiscoveryResult { hits, total, diagnostics }`,
`DiscoveryHit { instance_id, label, type_namespace, type_name, lifecycle_state,
score: Option<f32>, snippet: Option<String>, matched_fields: Vec<String> }`.

Flow: (1) structured pass via `list_records_filtered` (type ns/name, container,
first tag) then in-service filtering for the remaining tags, `type_id`,
`lifecycle_state`, `tier`; (2) `project_text` per record; (3) case-insensitive
NFC substring match of `content_match` against each segment — the **recall floor**,
`score: None` at Layer 1; (4) build hits, reusing `record_display_label` for
`label` and populating `matched_fields` + `snippet`. Deterministic: same query →
same hit set and order.

**3. `srs find` CLI command** (`#217`): handler = parse flags → one `find` call →
`output::ok`; named `FindPayload` in `crates/srs-cli/src/payload.rs`; committed
golden schema under `crates/srs-cli/schemas/payload/`.

**4. `find` WASM binding** (`#218`): same service, JSON in/out — tracked on the
epic, landed when a client needs it.

**5. Layer-1 first; defer the index.** The deterministic substring implementation
is the permanent correctness floor. A future `DiscoveryIndex` trait is the single
Layer-2 extension point (FTS / vector / semantic); it may add recall and ranking
but MUST NOT drop a Layer-1 match. No `async` is introduced until a real engine
lands (per the storage rules in CLAUDE.md). Ranking and the index trait are
explicitly **out of scope** for Phase 1.

**6. Composition for authored lists (the decision-log list).** `find` is the
runtime query primitive. An authored list (e.g. the decision log) is
`container resolve-view` (authored columns + ordered members + authored defaults)
composed with `find` (runtime content/tag/lifecycle). Authored defaults — which
lifecycle states are hidden by default, which fields are searchable, default sort —
are **defined** in **DocumentView/View metadata** in the package, never authored in
`srs-repository`. Their *extraction* into the resolved view payload (so clients do not
re-parse DocumentView sources or re-derive section precedence) is delegated to the
`container resolve-view` service — see [ADR-020](020-resolve-view-authored-list-defaults.md):
`resolve-view` surfaces the authored `excludeLifecycleStates`, and the client forwards it to
`find`. "Show all" drops the authored lifecycle exclusion. Governance vocabulary
(`decision_statement`, `superseded`/`closed`) stays in package data and the thin
srs-gov adapter.

## Consequences

**Positive:**
- One shared, spec-conformant code path for discovery across CLI, WASM, and web;
  srs-web can retire its bespoke TS filter (#219).
- Search reaches every text field, not just title — the recall gap the old web
  filter and the removed projection service both had.
- Deterministic and store-agnostic: fully testable against `MemoryStore` with a
  memory → json → file roundtrip; conformance fixtures from `discovery.json` are
  reproducible by any implementation.

**Negative / trade-offs:**
- `RecordListFilter` carries a single tag; multi-tag AND and the `lifecycle_state`/
  `tier`/`type_id` predicates are applied in the service after the structured pass
  rather than pushed into the store query (acceptable at Layer 1; an index can
  optimize later).
- ~~Phase 1 composes `list_records_filtered`, which yields Tier-2 Records only.
  Tier 0/1 text projection (note/typed-record sentinels in the schema) is deferred;
  a `tier` of 0 or 1 returns empty with a diagnostic until then.~~ **Resolved by
  srs-rust#797**: `find` now composes Tier 0 (Note) and Tier 1 (TypedRecord)
  alongside Tier 2, per RFC-012 `R1`/`I-113` and `R11`/`I-123` — there is no
  spec-sanctioned "Tier 2 only" phase. `typeId`/`typeNamespace`/`typeName`/
  `lifecycleState` remain Tier-2-only predicates (Tier 0/1 carry none of those
  fields); `tag`, `containerId`, and `tier` apply uniformly. Tier 1 has no typed
  `TypedRecord` struct in `srs-core` yet, so its body is read via the generic-JSON
  `load_instance_json` shim rather than a typed logical-id method.
- Select/Multiselect segments project the stored value token (recall-safe); label
  resolution from `allowed_values` is a later refinement.

**Neutral:**
- `DiscoveryQuery` follows `discovery.json`, so the issue #216 draft struct (`text`,
  `fields`, `limit`/`offset`) is superseded; pagination is a non-goal of RFC-012 and
  is deferred with the index.
- Authored-defaults metadata on DocumentView/View is an additive schema change
  coordinated through RFC #213 and the schema-mirror merge order — separate from
  this service work.

## Amendment (srs-rust#1228): `DiscoveryIndex` and BM25 ranking

Decision 5's reserved extension point now exists: `discovery_index::DiscoveryIndex`
(`score(words, candidates) -> Vec<f32>`) with one implementation, `Bm25Index`.

- It **orders** the Layer-1 all-words hits and fills `score`; it never adds or drops a
  candidate, so the recall floor and RFC-012 [R4] are untouched. No spec change.
- Opt-in through `FindPage.rank` (outside `DiscoveryQuery`, which mirrors the schema).
  Default off for the library, CLI (`--rank`) and WASM; on by default for the MCP `find`
  tool. Conformance fixtures stay on Layer 1.
- Input is the existing `text_projection` segments. BM25 (k1 1.2, b 0.75) with
  segment-kind weights: label/title 4, tag 2, short fields 2, long bodies 1. Term frequency
  is the normalized substring count, document length the word count. No new dependency.
- The index is memoized in a store-scoped slot (`RepositoryStore::discovery_index_cache`)
  that `FileStore` drops with its catalog memos when the write epoch moves. Stores without a
  slot rebuild per query.
- The MCP default (`rank: true`) differs from CLI/WASM (off) on purpose: MCP serves agents,
  who want relevance order; the set of hits is identical on every surface and only the order
  differs, which RFC-012 [R4] leaves free. Weights and the 400-char short/long threshold are
  tunable heuristics, not a contract. Scores are computed in f64 and quantised to 1e-4 so
  native and wasm32 order alike.
- A vector or embedding index would be a second `DiscoveryIndex` implementation; none is
  built (srs#726 open question 6). Measured by the eval harness (srs-rust#1231).

## Amendment (2026-10-04, #1219) — facets over the match set

`DiscoveryResult` is now `{ hits, total, facets, diagnostics }` (the `Neutral` note above that defers pagination predates `FindPage`, #1217). `facets` are counts over the whole Layer-1 match set, before paging, computed in the same pass as `total`: `byType` (`namespace/name`), `notes` (Tier 0, which has no type), `tags`, and `fields` (one entry per closed string field, keyed by `Field.name`, counted only when no package field of that name is open). Each facet keeps the top 20 values by count (ties by value) plus an `other` occurrence count; at most 25 field facets are kept, so a reply stays far below the 128 KB relay limit. `find` with `limit: 0` is the repository map. A Layer-2 index may rank hits but must leave facets unchanged. Result shaping only: `discovery.json` is untouched, no spec change. `byType` keys on the record's `typeNamespace`/`typeName` (the same hints every hit carries; a stale hint is a validate error, not a facet concern). Counts are of values as stored (no case folding). Field facets beyond the 25 largest are dropped without a marker; the cap is a relay-size guard, and a name that is closed-string in one field and anything else in another is never a facet.
