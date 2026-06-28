# Plan: Structured container-view field/column spec for editor member lists (#254)

## Summary

The srs-web editor needs to render a container's members as an interactive, selectable
list whose **columns are driven by a DocumentView's field selection** — not by client-side
type knowledge and not as pre-rendered HTML. `render_document_view` produces markdown/HTML,
which is the wrong shape for an interactive list pane. This plan adds a single read-only
**projection** capability, `resolve_container_view`, in `srs-repository`, exposed identically
through the CLI payload and a WASM binding (per `docs/architecture/capability-layering.md`).
For a `(container, optional documentView)` pair it returns: the container's root record, the
ordered member records (full `Record` + core-resolved display label + tier), and a
**column/field spec** resolved from the DocumentView's section → render View. All semantics
live in the core; clients only render.

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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `resolve_container_view` is a single service: typed input struct → typed result struct, all logic in `srs-repository`. | accepted (governs) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `ContainerViewPayload` named struct in `payload.rs`; regenerated golden schema. | accepted (governs) |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding `resolve_container_view` calls the same service; returns the typed result, not ad-hoc `json!`. | accepted (governs) |
| [capability-layering](../docs/architecture/capability-layering.md) | Typed projection lives in the service; clients add presentation only. | active guidance (governs) |
| [ADR-018](../docs/adr/018-container-view-column-source-precedence.md) | Column-source precedence: container-targeting section wins, else first section by `order` with a `render_view_id`, else empty columns. | accepted (this plan) |

**One new ADR (ADR-018).** The capability is otherwise a typed projection composing existing
entities (DocumentView, View/FieldView, Container, Record) under the existing
service/payload/binding contracts and establishes no other architectural constraint. The
column-source precedence rule, however, is a **reusable semantic contract** — any future
DocumentView+container projection must make the same section choice or clients diverge — so it
is captured in ADR-018 (status `proposed`; flip to `accepted` when this ships). The two design
decisions confirmed with the requester:

1. **Column source** — columns come from the section that *targets this container*
   (`SectionSource::ContainerSubset { container_id }` matching the requested container) and has
   a `render_view_id`; if none matches, fall back to the first section (by `order`) that has a
   `render_view_id`. The referenced View's `field_views` become the columns. If no View resolves,
   `columns` is empty.
2. **Member shape** — each member (and the root) carries the **full `Record`** plus its
   core-resolved `displayLabel` and `tier`. The client reads `fieldValues` against the column
   spec to render cells; no projection of cell values in the service.

---

## Contracts

### CLI output contract (ADR-011)

**New command added** → `container resolve-view <container-id> [--view-id <uuid>]`.
Add `ContainerViewPayload` to `crates/srs-cli/src/payload.rs`, wire the handler to
`output::serialize()`, run `cargo run --bin generate-schemas`, and commit the new
`crates/srs-cli/schemas/payload/container_view_payload.json`. The result types
(`ContainerView`, `ResolvedMember`, `ColumnSpec`) live in `srs-repository`; the payload embeds
`ContainerView` with `#[schemars(with = "serde_json::Value")]` (the same pattern
`RecordPayload`/`DocumentViewsForContainerPayload` use for embedded service/core types).

Verification: `cargo test --test payload_contracts` must pass after the payload change.

### Entity schema sync (check-schema-sync.sh)

**No** — this plan adds no JSON Schema files under `srs/docs/schema/2.0/`. No entity-schema sync needed.

---

## Scope

- A new `srs-repository` service module `container_view_service.rs` with:
  - `pub struct ResolveContainerViewInput { container_id: String, view_id: Option<String> }` — derive `Debug, Clone` only (constructed from CLI args / binding params; never crosses a serde boundary, matching `ContainerListFilter` at `container_service.rs:63`).
  - `pub struct ColumnSpec { field_id: String, field_name: String, display_label: String, order: i32, required: bool }` (`order` is `i32` to match `FieldView.order` at `view.rs:8` — do not narrow to `u32`).
  - `pub struct ResolvedMember { instance_id, tier, display_label, record }` (`record: srs_core::types::record::Record`)
  - `pub struct ContainerView { container_id, document_view_id: Option<String>, root: Option<ResolvedMember>, members: Vec<ResolvedMember>, columns: Vec<ColumnSpec>, diagnostics: Vec<String> }` — `document_view_id` is the UUID of the resolved DocumentView, or `None` when none resolves (columns then empty).
  - Result structs (`ColumnSpec`, `ResolvedMember`, `ContainerView`) derive **`Debug, Clone, Serialize, Deserialize`** with `#[serde(rename_all = "camelCase")]` — **no `PartialEq`** (matches `DocumentViewSummary` `view_service.rs:65`, `ContainerSummary` `container_service.rs:34`, `TreeNode`; none derive `PartialEq`). Cross-store equality in tests is asserted via `serde_json::to_value` comparison, not `==`.
  - `pub fn resolve_container_view(store: &dyn RepositoryStore, input: ResolveContainerViewInput) -> Result<ContainerView, RepositoryError>`
- Reuse existing internals (no re-derivation): `record_label::build_field_name_index` + `record_label::record_display_label`,
  `record_store::get_record_by_id`, `container_service::{list_container_members, list_roots, get_container}`,
  `view_service::{document_views_for_container, get_document_view_by_id, get_view_by_id}`.
- **Tier gating:** load `store.load_manifest()` once and build an `instance_id → tier` lookup from `manifest.instance_index` (`entry.instance_id`, `entry.tier`). `get_record_by_id` does **not** tier-check — `load_record` deserializes the file as a `Record` and **errors** on a Tier-0/1 instance (`record_store.rs:214`). So before loading any member/root, check `tier == 2`; skip non-Tier-2 with a diagnostic (mirrors `tree_service.rs:114`).
- CLI: `container resolve-view` subcommand + `ContainerViewPayload` + golden schema.
- WASM: `SrsRepository::resolve_container_view(container_id, view_id_json)` binding.
- Tests: service unit tests, a cross-store roundtrip test (memory → json → file), CLI/binding smoke coverage.

**Out of scope:**

- Any change to the DocumentView/View/FieldView/Container/Record data model or to `srs/` schemas.
- Layer-2 acceleration (indexes, caching) — Layer-1 deterministic projection only.
- Pre-projecting member cell values in the service (members carry full `Record`; client renders cells).
- Member instances that are not Tier-2 Records: they are **skipped** with a diagnostic (mirrors `tree_service`), not projected. Tier-0/1 support in this view is deferred (see deferred-issues list in Stage 3).
- srs-web / srs-vscode client wiring (consumers of this binding; tracked under epic srs-web#92).

---

## Phases

### Phase 1: `srs-repository` service + result types

**Goal:** `resolve_container_view` returns root + ordered members + columns + diagnostics, fully tested across stores.

**Agent:** Repository Service Worker (`agents.md#repository-service-worker`)

#### Tasks

- [x] Create `crates/srs-repository/src/container_view_service.rs`; declare `pub mod container_view_service;` in `crates/srs-repository/src/lib.rs` (alphabetical position).
- [x] Define `ResolveContainerViewInput` (`#[derive(Debug, Clone)]`) and the three result structs `ColumnSpec`, `ResolvedMember`, `ContainerView` (`#[derive(Debug, Clone, Serialize, Deserialize)]`, `#[serde(rename_all = "camelCase")]`, **no `PartialEq`** — match `DocumentViewSummary` at `view_service.rs:65`). Do **not** add `schemars` to `srs-repository/Cargo.toml`.
- [x] Implement `resolve_container_view`:
  - [x] Load `manifest = store.load_manifest()?` once; build an `instance_id → tier` map from `manifest.instance_index` (`entry.instance_id`, `entry.tier`). Load the container **once** via `container_service::get_container` (not found → return `Err(RepositoryError…)`, do not panic; mirror `document_views_for_container`'s container-missing handling) and reuse it for both root resolution and the `view_id = None` DV matching — do **not** call `document_views_for_container` (it re-fetches the container internally); instead replicate its small matching step over the already-loaded container + `view_service::list_document_views`, or keep a single `get_container` and call `document_views_for_container` accepting one redundant load (acceptable for Layer-1, but prefer the single-load path).
  - [x] Resolve the DocumentView: if `input.view_id` is `Some`, call `view_service::get_document_view_by_id` and match `GetDocumentViewResult::Found(dv) => …` / `GetDocumentViewResult::NotFound =>` leave `document_view_id = None`, `columns = []`, push diagnostic `"resolve-container-view: documentView <id> not found"`. If `input.view_id` is `None`, call `view_service::document_views_for_container` and take the first element if any (else no DV → empty columns). Set `document_view_id = Some(dv.id)` for the resolved DV.
  - [x] Build `field_name_index` once via `record_label::build_field_name_index`.
  - [x] Resolve columns from the chosen DocumentView using the **column-source precedence**: first the section whose `source` is `SectionSource::ContainerSubset { container_id, .. }` equal to `input.container_id` and whose `render_view_id` is `Some`; else the first section, **sorted by `order` ascending (lowest first)**, with `render_view_id = Some`. Load that View via `view_service::get_view_by_id`, matching `GetViewResult::Found(v)` / `GetViewResult::NotFound` (NotFound → empty columns + diagnostic). For each `FieldView` (skip `visible == Some(false)`), sorted by `order` ascending, emit `ColumnSpec { field_id, field_name: field_name_index.get(field_id).cloned().unwrap_or_else(|| field_id.clone()), display_label: fv.display_label.clone().unwrap_or_else(|| field_name), order: fv.order, required: fv.required.unwrap_or(false) }`. Push a diagnostic if a column's `field_id` is absent from the index.
  - [x] Resolve the root: `container_service::list_roots` (or `get_container().root_instance_ids`); if there is no root id, `root = None`. If a root id exists: if its `tier != 2`, `root = None` + diagnostic; else `get_record_by_id` → if `None` (id exists in container but not in index), `root = None` + diagnostic `"resolve-container-view: root instance <id> does not resolve"`; if `Some(record)`, build `ResolvedMember { instance_id, tier: 2, display_label: record_label::record_display_label(&record, &index), record }`.
  - [x] Resolve ordered members: `container_service::list_container_members` (roots-first, deduped order). For each id: if `tier != 2`, skip with diagnostic `"resolve-container-view: instance <id> not a Tier 2 record — skipped"`; else `get_record_by_id` and push a `ResolvedMember` (tier `2`); a `None` from the loader yields a `"… does not resolve"` diagnostic (mirror `tree_service`).
  - [x] Reuse only existing helpers — no re-derivation of display-label or membership logic.
- [x] Add a rustdoc comment on `resolve_container_view` citing ADR-018 for the column-source precedence rule.

#### Acceptance Criteria

- [x] Calling the service on a container with a matching DocumentView returns the expected columns (order, displayLabel override applied, visible:false excluded), the root member, and ordered members.
- [x] `view_id` override selects that DocumentView; an unknown `view_id` yields empty columns + a diagnostic (members/root still returned).
- [x] A container with no resolvable DocumentView returns `columns = []`, `document_view_id = None`, and still returns root + members.
- [x] A non-Tier-2 member is skipped and recorded in `diagnostics`.
- [x] Container-not-found returns `Err`, not a panic.

#### Testing

```bash
cargo test -p srs-repository container_view
```

Specific tests to write (in `container_view_service.rs` `#[cfg(test)]`, MemoryStore-based, mirroring `view_service.rs` test helpers):

- `resolve_container_view_returns_columns_from_matching_section` — ContainerSubset section drives columns; verifies order + displayLabel override + visible:false exclusion.
- `resolve_container_view_falls_back_to_first_section_with_view` — no ContainerSubset match → first section's view used.
- `resolve_container_view_view_id_override` — explicit `view_id` selects a different DocumentView.
- `resolve_container_view_unknown_view_id_empty_columns_with_diagnostic` — unknown id → empty columns + diagnostic.
- `resolve_container_view_no_document_view_returns_members_only` — empty columns, root+members present.
- `resolve_container_view_skips_non_tier2_member_with_diagnostic`.
- `resolve_container_view_root_and_member_labels` — labels come from `record_display_label`.
- `resolve_container_view_container_not_found_errors`.
- `resolve_container_view_roundtrip_stores` — **mandatory** (CLAUDE.md Storage Boundary Rules). Build a MemoryStore, export to srsj/JsonStore (and a temp FileStore if a fixture helper exists, e.g. the pattern in `repository_portability.rs`); assert the `ContainerView` is identical across stores by comparing `serde_json::to_value(&result)` (the result structs do not derive `PartialEq`).

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
Mark checkboxes, commit: `feat(repository): resolve_container_view projection service (#254)`.

---

### Phase 2: CLI payload + handler + command wiring

**Goal:** `srs container resolve-view <id> [--view-id <uuid>]` returns the `ContainerView` envelope; golden schema committed.

**Agent:** CLI Worker (`agents.md#cli-worker`)

#### Tasks

- [x] Add `ContainerViewPayload { #[schemars(with = "serde_json::Value")] container_view: ContainerView }` to `crates/srs-cli/src/payload.rs` (import `container_view_service::ContainerView`); `#[derive(Debug, Serialize, JsonSchema)]`, `#[serde(rename_all = "camelCase")]`.
- [x] Add `ContainerCommand::ResolveView { container_id: String, #[arg(long = "view-id")] view_id: Option<String> }` to `crates/srs-cli/src/commands/mod.rs` with a doc comment.
- [x] Add handler `cmd_container_resolve_view` in `crates/srs-cli/src/commands/container.rs` following the ≤15-line handler pattern: build `ResolveContainerViewInput`, one `with_store` service call, `output::serialize("container resolve-view", ContainerViewPayload { container_view })`; on `Err`, `output::err`.
- [x] Wire the new arm in the container command dispatch `match`.
- [x] Run `cargo run --bin generate-schemas`; stage `crates/srs-cli/schemas/payload/container_view_payload.json`.

#### Acceptance Criteria

- [x] `cargo run --bin srs -- container resolve-view <id>` emits the `{ ok, command, version, payload: { containerView: { ... } } }` envelope.
- [x] `--view-id <uuid>` is honoured.
- [x] Handler body is parse → one service call → output; no business logic.
- [x] `cargo test --test payload_contracts` passes with the new golden file present.

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

Specific tests:

- CLI integration tests (matching the existing `commands/container.rs` / container CLI test style): `container_resolve_view_happy_path` and `container_resolve_view_with_view_id`.
- `payload_contracts` golden test passes for `container_view_payload.json`.

#### Milestone gate

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```
Mark checkboxes, commit: `feat(cli): container resolve-view command + payload (#254)`.

---

### Phase 3: WASM binding

**Goal:** srs-web/srs-vscode can call `resolve_container_view` through the binding and get the typed result.

**Agent:** Bindings Worker (`agents.md#bindings-worker`)

#### Tasks

- [x] Add `SrsRepository::resolve_container_view(&self, container_id: &str, view_id: Option<String>) -> Result<JsValue, JsValue>` to `crates/srs-bindings/src/lib.rs`, mirroring `document_views_for_container`: build `ResolveContainerViewInput`, call `container_view_service::resolve_container_view`, `to_js(&result)` (typed result, not `json!`).
- [x] Use `view_id: Option<String>` as a direct `#[wasm_bindgen]` parameter — the established convention for a single optional string override (`render_document_view` takes `container_id: Option<String>` directly at `bindings/src/lib.rs:237`). Do not use a JSON-string filter (that convention is for multi-field filters like `list_containers`).

#### Acceptance Criteria

- [x] Binding compiles for the workspace target and returns parseable JSON for a known container.
- [x] Binding calls the service — no duplicated resolution/label/membership logic.

#### Testing

```bash
cargo test -p srs-bindings
```

Specific tests:

- A bindings smoke test proving `resolve_container_view` output deserializes to JSON with `containerView`-equivalent keys (matching the existing bindings smoke-test style).

#### Milestone gate

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
Mark checkboxes, commit: `feat(bindings): resolve_container_view WASM binding (#254)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged for existing commands (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (new golden file committed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — should be a no-op)
- [x] A single CLI/binding call returns root + ordered members + column spec + per-member label for a container (issue acceptance)
- [x] Cross-store roundtrip test (memory → json → file) passes

## Coordination Rules

- Agents keep to their write scopes (Repository → `srs-repository`, CLI → `srs-cli`, Bindings → `srs-bindings`).
- Phase order is strict: service first (defines the result types the other two crates consume), then CLI, then bindings.
- Lead Integrator owns final names (`resolve_container_view`, `ContainerView`, `container resolve-view`).
- Each phase ends with its milestone gate before the next begins.

## Assumptions

- `view_service::get_document_view_by_id` returns `Result<GetDocumentViewResult, _>` and `get_view_by_id` returns `Result<GetViewResult, _>`, each `Found(Box<_>)` / `NotFound` (confirmed at `view_service.rs:36-44,199,215`). The plan matches these enums, not `Option`.
- `record_store::get_record_by_id` returns `Result<Option<Record>, _>` but does **not** tier-check — `load_record` deserializes the index entry as a `Record` and errors on Tier-0/1 (confirmed at `record_store.rs:80,214`). The service gates on `manifest.instance_index` `entry.tier == 2` before loading (mirrors `tree_service.rs:114`).
- `record_label::{build_field_name_index, record_display_label}` are `pub(crate)`, reachable from a sibling module in the same crate (confirmed at `record_label.rs:9,19`).
- `container_service` exposes ordered members (`list_container_members`, `container_service.rs:338`), roots (`list_roots`, `container_service.rs:268`), and `get_container`. There is no `list_container_roots`.
