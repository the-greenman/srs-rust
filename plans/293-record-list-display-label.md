# Plan: Surface core `record_display_label` through `record list` payload + `list_records` binding (#293)

## Summary

The governance editor (srs-web) derives each record's display title client-side by re-implementing a `title → field-name → value` join, duplicating semantics that already live once in the Rust core (`record_label::record_display_label`, priority `title → name → label → type_name`). That core function is already consumed by `srs tree` and `resolve_container_view` (#254) but **not** by the record-list path, so `srs record list` and the WASM `list_records` binding return raw `Record` objects with no derived label — which is exactly why srs-web re-derives it and drifts. This plan closes the gap (the follow-on explicitly deferred in `plans/tree-view.md`): add a `srs-repository` service that returns record summaries enriched with the core-resolved `displayLabel`, and surface it identically through the CLI `record list` payload (ADR-011) and the `list_records` WASM binding (ADR-013). Clients then consume `displayLabel` and delete their derivation. Upstream consumer: the-greenman/srs-web#91.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | `agents.md#repository-service-worker` |
| CLI Worker | `agents.md#cli-worker` |
| Bindings Worker | `agents.md#bindings-worker` |
| Verification | `agents.md#verification-agent` |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `list_record_summaries` is a single service: typed filter struct → typed result vec, all logic in `srs-repository`. | accepted (governs) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `record list` payload now carries `RecordSummary` items (Record + `displayLabel`); golden schema regenerated. | accepted (governs) |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `list_records` binding calls the same service and returns the typed summary vec. | accepted (governs) |
| [capability-layering](../docs/architecture/capability-layering.md) | Typed projection (display label) lives in the service; clients render only. | active guidance (governs) |

**ADR check (read every ADR in `docs/adr/`).** This change is a typed projection composing an existing core function (`record_display_label`) under the existing service/payload/binding contracts (ADR-010/011/013, capability-layering). The display-label algorithm itself is unchanged and already shipped — it establishes no new semantic rule, so by analogy to #254 (where the `ResolvedMember { displayLabel, record }` enrichment needed **no** ADR — only the genuinely-new column-source precedence got ADR-018) **no new ADR is required**. The chosen payload shape (below) follows the existing #254 `ResolvedMember` precedent, so it establishes no new convention either.

---

## Design Decision (Stage 2 checkpoint — RESOLVED)

**How is `displayLabel` attached to each record in the `record list` payload and `list_records` binding result?**

**Requester guidance (2026-06-28):** the current srs-web implementations are broken and will be rewritten, so backward-compatibility of the `record list` / `list_records` shape is **not** a constraint; and the resolution strategy should match what `srs tree` does (which reads labels correctly — `build_field_name_index` once + `record_display_label` per node).

**Decision — Option B (nested), mirroring #254 `ResolvedMember`.** With backward-compat off the table, Option B's only downside (forcing existing consumers to rewrite field access) is moot, and nesting is the more consistent choice: `record list` summaries become structurally identical to the container-view member summaries the *same* governance editor already consumes (#254), and they use the exact resolution `tree` uses (`record_display_label`, priority `title → name → label → type_name`).

```rust
RecordSummary {
    instance_id: String,   // mirrors ResolvedMember; convenient list key
    display_label: String, // record_label::record_display_label — same as tree
    record: Record,        // full record; client renders cells/fields
}
// serde(rename_all = "camelCase") → { "instanceId", "displayLabel", "record": { ... } }
```

This is `ResolvedMember` minus `tier` (`record list` returns only Tier-2 Records). Field naming `displayLabel` matches #254; the resolution source matches `tree`. No new ADR.

(Rejected — Option A flatten `{ #[serde(flatten)] record, displayLabel }`: its sole advantage was backward-compat, now moot; it carried a key-collision risk with `Record.extra` and diverged from the #254 shape.)

---

## Contracts

### CLI output contract (ADR-011)

**Existing command payload changed** — `record list`. `RecordListPayload.records` changes from `Vec<Record>` to `Vec<RecordSummary>`. Because `Record` is embedded opaquely via `#[schemars(with = "Vec<serde_json::Value>")]`, the golden schema (`record-list.json`, currently `"items": true`) is unaffected in shape; run `cargo run --bin generate-schemas` and commit whatever (if any) diff results. `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

**No** — this plan adds/modifies no JSON Schema files under `srs/docs/schema/2.0/`. No entity-schema sync needed.

---

## Scope

- A new `srs-repository` service function `list_record_summaries(store, filter: RecordListFilter) -> Result<Vec<RecordSummary>, RepositoryError>` in `crates/srs-repository/src/record_store.rs`, plus the `RecordSummary` result struct. It calls the existing `list_records_filtered` for the records, builds the field-name index once via `record_label::build_field_name_index`, and maps each record to a `RecordSummary` carrying `record_label::record_display_label(&record, &index)`. No re-derivation; `record_display_label` / `build_field_name_index` stay `pub(crate)`.
- CLI: `cmd_record_list` calls `list_record_summaries`; `RecordListPayload.records: Vec<RecordSummary>`.
- WASM: `SrsRepository::list_records` calls `list_record_summaries` and returns the summary vec via `to_js`.
- Tests: service unit tests (label priority + fallback surfaced in the summary, cross-store roundtrip), CLI integration test asserting `displayLabel` on payload items, bindings smoke test asserting `displayLabel` key.

**Out of scope:**

- srs-web / srs-vscode client wiring (consuming `displayLabel`, deleting the client-side title chain) — tracked in srs-web#91, separate repo not in this workspace.
- Any change to the core `Record` data model or `srs/` schemas — `displayLabel` is derived at the summary layer, never stored on `Record`.
- Changing `srs tree` / `resolve_container_view` (already correct).
- Layer-2 acceleration.
- `record show` / `get_record` single-record path — only the list path is in scope (file as a deferred follow-on if reviewers want it).

---

## Phases

### Phase 1: `srs-repository` service + result type

**Goal:** `list_record_summaries` returns each filtered record paired with its core-resolved `displayLabel`, tested across stores.

**Agent:** Repository Service Worker (`agents.md#repository-service-worker`)

#### Tasks

- [x] In `crates/srs-repository/src/record_store.rs`, define `RecordSummary` (Option B, nested — mirrors `ResolvedMember` minus `tier`): `#[derive(Debug, Clone, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub struct RecordSummary { pub instance_id: String, pub display_label: String, pub record: Record }`. (No `PartialEq` — matches `ResolvedMember`/`TreeNode`; cross-store equality asserted via `serde_json::to_value`.)
- [x] Implement `pub fn list_record_summaries(store: &dyn RepositoryStore, filter: RecordListFilter) -> Result<Vec<RecordSummary>, RepositoryError>`: call `list_records_filtered(store, filter)?`; build `index = record_label::build_field_name_index(store)?` **once**; map each record into `RecordSummary { instance_id: record.instance_id.clone(), display_label: record_label::record_display_label(&record, &index), record }`.
- [x] Add a rustdoc comment noting the label comes from `record_label::record_display_label` (priority `title → name → label → type_name`) and is the same resolution `srs tree` uses.
- [x] Keep `list_records_filtered` and `record_label::{build_field_name_index, record_display_label}` as-is (`pub(crate)` reachable from the same crate). Do not duplicate the filter loop.
- [x] Placement: `RecordSummary` + `list_record_summaries` live in `record_store.rs` (alongside `list_records_filtered`), not a new module — unlike #254's `ResolvedMember`/`container_view_service.rs`, this is a thin enrichment projection over the existing filter function, not a new composing service with its own import sub-graph, so a new module is not warranted.

#### Acceptance Criteria

- [x] `list_record_summaries` returns one `RecordSummary` per filtered record, each `display_label` equal to `record_display_label` for that record.
- [x] A record with a `title` field yields that title; a record with only `name` yields the name; a record with neither yields `type_name` (fallback) — proven via the summary, not the raw helper.
- [x] The same `RecordListFilter` semantics (type/tag/container) as `list_records_filtered` apply (it delegates).

#### Testing

```bash
cargo test -p srs-repository list_record_summaries
```

Specific tests (in `record_store.rs` `#[cfg(test)]`, MemoryStore-based, mirroring existing record_store tests):

- `list_record_summaries_attaches_title_label` — record with `title` field → `display_label == title`.
- `list_record_summaries_falls_back_to_type_name` — record with no title/name/label → `display_label == type_name`.
- `list_record_summaries_respects_filter` — type/tag filter narrows results identically to `list_records_filtered`.
- `list_record_summaries_roundtrip_stores` — **mandatory** (CLAUDE.md Storage Boundary Rules): MemoryStore → FileStore (via the `repository_portability` copy/export helper, matching `container_view_service`'s `resolve_container_view_roundtrip_stores`); assert summaries identical across stores via `serde_json::to_value`.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
Mark checkboxes, commit: `feat(repository): list_record_summaries with core display label (#293)`.

---

### Phase 2: CLI payload + handler

**Goal:** `srs record list` emits each record with a `displayLabel`; golden schema in sync.

**Agent:** CLI Worker (`agents.md#cli-worker`)

#### Tasks

- [x] In `crates/srs-cli/src/payload.rs`, change `RecordListPayload.records` from `Vec<Record>` to `Vec<RecordSummary>` (import `record_store::RecordSummary`); keep `#[schemars(with = "Vec<serde_json::Value>")]`.
- [x] In `crates/srs-cli/src/commands/record.rs`, change `cmd_record_list` to call `list_record_summaries` instead of `list_records_filtered` (the only change). Note: the existing handler is ~35 lines because of the `parse_type_filter` arg-translation block — that is CLI-layer arg parsing, not business logic, and is fine under ADR-010. Do not refactor it; just swap the service call.
- [x] Run `cargo run --bin generate-schemas`; stage any diff under `crates/srs-cli/schemas/payload/`.

#### Acceptance Criteria

- [x] `cargo run --bin srs -- record list` payload items each take the shape `{ instanceId, displayLabel, record: { ... } }`.
- [x] Handler contains no business logic: arg parsing → one service call → `output::serialize`. (The `parse_type_filter` block is CLI-layer arg translation and does not count against the handler-size guidance.)
- [x] `cargo test --test payload_contracts` passes.

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

Specific tests:

- A CLI integration/handler test asserting a listed record carries the expected `displayLabel` (matching the existing `commands/record.rs` test style).
- `payload_contracts` golden test for `record-list.json` passes.

#### Milestone gate

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```
Mark checkboxes, commit: `feat(cli): record list carries core display label (#293)`.

---

### Phase 3: WASM binding

**Goal:** `list_records` binding returns summaries with `displayLabel`; srs-web can consume it.

**Agent:** Bindings Worker (`agents.md#bindings-worker`)

#### Tasks

- [x] In `crates/srs-bindings/src/lib.rs`, change `SrsRepository::list_records` to call `record_store::list_record_summaries` (same `RecordListFilter` parse) and `to_js(&summaries)`.
- [x] Update the binding's rustdoc to state it returns a JS array of records each carrying `displayLabel`.

#### Acceptance Criteria

- [x] Binding compiles for the workspace target; output deserializes to JSON whose items carry `displayLabel`.
- [x] Binding calls the service — no duplicated label/derivation logic.

#### Testing

```bash
cargo test -p srs-bindings
```

Specific tests:

- A bindings smoke test asserting `list_records` output items contain a `displayLabel` key (matching the existing bindings smoke-test style).

#### Milestone gate

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
Mark checkboxes, commit: `feat(bindings): list_records returns core display label (#293)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (golden file committed if changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — no-op)
- [ ] `record list` (CLI) and `list_records` (binding) both carry a `displayLabel` equal to what `srs tree` resolves for the same records (issue acceptance)
- [ ] No title-selection semantics added anywhere; the label comes solely from `record_display_label`
- [ ] Cross-store roundtrip test passes

## Coordination Rules

- Write scopes: Repository → `srs-repository`, CLI → `srs-cli`, Bindings → `srs-bindings`.
- Phase order is strict: service first (defines `RecordSummary` the other two crates consume), then CLI, then bindings.
- Lead Integrator owns final names (`list_record_summaries`, `RecordSummary`, `displayLabel`).
- Each phase ends with its milestone gate before the next begins.

## Assumptions

- `record_label::{build_field_name_index, record_display_label}` are `pub(crate)` and reachable from `record_store.rs` (same crate) — confirmed at `record_label.rs:9,19`.
- `list_records_filtered(store, RecordListFilter) -> Result<Vec<Record>, _>` exists at `record_store.rs:489` and is the single filter implementation to delegate to.
- `RecordListPayload` embeds records opaquely via `#[schemars(with = "Vec<serde_json::Value>")]` (`payload.rs:291`), so the golden schema is shape-stable across this change.
- `RecordSummary` (nested `record: Record`) round-trips through both `serde_json` and the binding's `to_js` (serde_json::to_string → JSON.parse) path — no `serde(flatten)` involved, so `Record.extra`'s own flatten is unaffected.
