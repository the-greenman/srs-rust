# ADR-019: Discovery Service, `find` command, and deferred index trait

- **Status:** proposed
- **Date:** 2026-06-28
- **Supersedes:** —
- **Superseded by:** —

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
live in **DocumentView/View metadata** in the package, never in `srs-repository`.
"Show all" drops the authored lifecycle exclusion. Governance vocabulary
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
- Phase 1 composes `list_records_filtered`, which yields Tier-2 Records only.
  Tier 0/1 text projection (note/typed-record sentinels in the schema) is deferred;
  a `tier` of 0 or 1 returns empty with a diagnostic until then.
- Select/Multiselect segments project the stored value token (recall-safe); label
  resolution from `allowed_values` is a later refinement.

**Neutral:**
- `DiscoveryQuery` follows `discovery.json`, so the issue #216 draft struct (`text`,
  `fields`, `limit`/`offset`) is superseded; pagination is a non-goal of RFC-012 and
  is deferred with the index.
- Authored-defaults metadata on DocumentView/View is an additive schema change
  coordinated through RFC #213 and the schema-mirror merge order — separate from
  this service work.
