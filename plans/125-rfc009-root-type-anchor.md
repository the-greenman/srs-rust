# Plan: RFC-009 — root-type anchoring for DocumentView and Container (srs-rust)

> Implements accepted spec RFC-009 ([srs#39](https://github.com/the-greenman/srs/issues/39), merged 2026-06-11) in the Rust workspace. Tracking issue: [#125](https://github.com/the-greenman/srs-rust/issues/125).

## Summary

RFC-009 replaces the free-string `containerType` join between a `DocumentView` and a `Container` with a validated, UUID+version typed anchor: `DocumentView.rootTypeRefs: ExactTypeRef[]`. The accepted spec and its JSON Schemas have already merged, and the srs-rust schema mirror was synced in commit `9e7793b`. What remains is the **Rust behaviour**: carry `rootTypeRefs` on the in-memory `DocumentView`, surface the RFC's conformance diagnostics (I-63, I-64) in `repo validate`, add a `document-view list --root-type <uuid>` filter, and expose a `list_document_views` WASM binding. `containers_for_instance` (I-66 / Change D) and the Container `description`/`tags` reconciliation (Change C) are already implemented; this plan does not re-do them.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this pipeline) |
| Core Model Worker | Phase 1 |
| Repository Service Worker | Phase 2 |
| CLI Worker | Phase 3 |
| Bindings Worker | Phase 4 |
| Verification | Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All validation/orchestration lives in `srs-repository` services; CLI handlers are thin | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Every command output is a named payload struct; golden schemas regenerated | accepted |
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Entity schemas embedded at compile time; mirror kept in sync | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM bindings call the same services as the CLI; JSON-first surface | accepted |

**No new ADR.** `ExactTypeRef` is introduced as a core type distinct from the existing blueprint `TypeRef` (which has optional `type_version`); this distinction is mandated and motivated by accepted RFC-009 — it is not a new architectural decision originating here, so it is recorded against the RFC rather than a new ADR.

**Design decision (resolved at plan time):** I-63 unresolved-`rootTypeRefs` diagnostics are emitted at **`Warning`** severity, not `Error`. Both RFC-009 conformance diagnostics (I-63, I-64) are therefore advisory: `repo validate` stays `is_ok() == true` and `errors == 0`. Rationale — the RFC frames an unresolved entry as "MUST NOT be used for Container matching" (the view simply does not match), not as invalidating the repository; this keeps validate-gated CI from breaking on advisory anchor drift and is consistent with I-64 being spec-mandated `Warning`.

---

## Contracts

### CLI output contract (ADR-011)

**Service type embedded as opaque `Value` — no golden-schema diff expected.** `DocumentViewSummary` (gaining `root_type_refs`) and `DocumentView` (gaining `rootTypeRefs`) are embedded in `DocumentViewListPayload`/`DocumentViewPayload` via `#[schemars(with = "Vec<serde_json::Value>")]` / `#[schemars(with = "serde_json::Value")]` (the documented convention at `payload.rs:18`). Per the TEMPLATE.md ADR-011 note, when a service type is embedded this way the golden schema treats it as opaque and **no regeneration changes the schema file**; the field shape is covered by integration tests, not the golden file. We still run `cargo run --bin generate-schemas` after Phase 3 to confirm no diff, and `cargo test --test payload_contracts` must pass. A payload-local JsonSchema mirror is deliberately **not** introduced: `DocumentViewSummary` lives in `srs-repository`, which forbids `schemars` (CLAUDE.md), and a mirror would deviate from the established opaque-summary convention.

### Entity schema sync (check-schema-sync.sh)

**No.** The `2.0/` entity schemas (`document-view.json`, `container.json`, `manifest.json`) were already synced from the accepted RFC-009 spec in commit `9e7793b`. `bash scripts/check-schema-sync.sh` is expected to exit 0 at the start of this plan; Phase 5 re-verifies it.

---

## Scope

- `srs-core`: add `ExactTypeRef { type_id: String, type_version: u32 }` and `DocumentView.root_type_refs: Option<Vec<ExactTypeRef>>` (serde `rootTypeRefs`, skip-if-none). Extend the DocumentView roundtrip test.
- `srs-repository`: in `validate_repository`, emit I-63 (each `rootTypeRefs` entry resolves to a package Type by `typeId`+`typeVersion`) and I-64 (when a Container has `rootInstanceIds` and `containerType`, warn if `containerType` ≠ resolved root Type's bare `name`) diagnostics.
- `srs-repository`/`srs-cli`: introduce `DocumentViewListFilter` in `view_service.rs`; move `document-view list` filtering (namespace, container_type, new `root_type_id`) into `list_document_views_summary`; add `--root-type <uuid>` CLI flag; add `root_type_refs` to `DocumentViewSummary`. No payload golden-schema change expected (opaque summary).
- `srs-bindings`: add `list_document_views(filter_json)` WASM binding calling `view_service::list_document_views_summary` with a parsed filter (mirrors `list_containers`).

**Out of scope:**

- The muSrs `guide-body-view` fixture update (`rootTypeRefs` pointing at the guide type UUID) — that fixture lives in the external `muDemocracy` repo, not srs-rust. Filed as a follow-up; surfaced as a dogfood scenario in Stage 11.
- `srs-web#43` consumer wiring — separate repo, blocked-on then unblocked by this PR.
- Promoting `containers_for_instance` to a dedicated CLI subcommand — already reachable via `container list --member <id>`.

---

## Phases

### Phase 1: Core model — `ExactTypeRef` + `DocumentView.root_type_refs`

**Goal:** `DocumentView` carries `rootTypeRefs` and roundtrips through JSON; `ExactTypeRef` is a public core type.

**Agent:** Core Model Worker

#### Tasks

- [ ] Add `pub struct ExactTypeRef { pub type_id: String, pub type_version: u32 }` to `crates/srs-core/src/types/view.rs` (serde `rename_all = "camelCase"`, derive `Debug, Clone, PartialEq, Serialize, Deserialize`). Doc-comment notes the distinction from blueprint `TypeRef` (required `type_version`), citing RFC-009.
- [ ] Add `#[serde(skip_serializing_if = "Option::is_none")] pub root_type_refs: Option<Vec<ExactTypeRef>>` to `DocumentView`.
- [ ] Update every `DocumentView { .. }` struct literal across the workspace so it still compiles with the new field: the roundtrip test in `crates/srs-core/src/types/view.rs`, the `minimal_document_view` fixture in `crates/srs-repository/src/view_service.rs` (line ~533), and any other `DocumentView { .. }` literal found via `rg "DocumentView \{" crates/`. New literals use `root_type_refs: None` unless the test needs data.
- [ ] Extend `document_view_roundtrips_json` to populate `root_type_refs` and assert it survives the roundtrip.

#### Acceptance Criteria

- [ ] `DocumentView` has `root_type_refs` serializing as `rootTypeRefs`, omitted when `None`.
- [ ] `ExactTypeRef` roundtrips `{ "typeId": ..., "typeVersion": N }`.
- [ ] `cargo test -p srs-core` green.

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

- `document_view_roundtrips_json` — proves `rootTypeRefs` serializes/deserializes and is preserved.

#### Milestone gate

Acceptance criteria checked, tests pass, clippy clean, plan checkboxes updated, commit `feat(core): add ExactTypeRef and DocumentView.rootTypeRefs (#125)`.

---

### Phase 2: Repository validation — I-63 and I-64 diagnostics

**Goal:** `repo validate` reports unresolved `rootTypeRefs` (I-63) and stale `containerType` hints (I-64).

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/validation.rs::validate_repository`, after the package is resolved, iterate `view_service::list_document_views(store)`; for each `DocumentView` with non-empty `root_type_refs`, check each `ExactTypeRef` resolves to a package Type by both `type_id` and `type_version`. Unresolved → push a `Warning` diagnostic (see Architecture Decisions) with the document-view file path and a message naming the unresolved `typeId@typeVersion`. (I-63)
- [ ] Iterate containers via `store.list_container_summaries()` then `store.load_container(&id)` (the same store-method pattern `container_service` uses — **not** a call into `container_service`, and no new path strings, per the storage boundary rule). For each Container with non-empty `root_instance_ids` **and** a `container_type`, resolve the first root instance's Record (via `store.load_instance_json` / existing record-load path) → its `typeId`/`typeVersion` → the package Type's bare `name`. If `container_type != name`, push a `Warning` diagnostic citing I-64 (hint stale; container remains valid). Containers without `root_instance_ids` are skipped. (I-64)
- [ ] **Edge cases (skip the diagnostic, never error):** if the first root instance ID cannot be loaded as a Record, or the Record's `typeId`/`typeVersion` does not resolve to a package Type, skip the I-64 check for that container (it may reference an external/unresolved instance — that is a separate diagnostic, not I-64's concern).
- [ ] Reuse existing package-resolution helpers; do not add new path strings to service logic (storage boundary rule).

#### Acceptance Criteria

- [ ] A repo with a `rootTypeRefs` entry that does not resolve produces an I-63 `Warning`; the repo stays `is_ok()`.
- [ ] A repo where `containerType` ≠ root Type bare `name` produces an I-64 `Warning`; the container is **not** marked invalid.
- [ ] A Container with `containerType` but no `rootInstanceIds` produces no I-64 diagnostic.
- [ ] Valid repos (e.g. `../srs/srs`) gain no new diagnostics.
- [ ] `cargo test -p srs-repository` green.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
srs repo validate --repo ../srs/srs   # still 0 errors
```

- `validate_flags_unresolved_root_type_ref` (MemoryStore) — I-63 `Warning` fires; report stays `is_ok()`.
- `validate_flags_stale_container_type_hint` (MemoryStore) — I-64 warning fires, container stays valid.
- `validate_skips_container_type_without_roots` (MemoryStore) — no false positive.
- `validate_skips_i64_when_root_record_unresolved` (MemoryStore) — root instance not loadable → no I-64 diagnostic.
- `validate_root_type_diagnostics_consistent_across_stores` — **cross-store roundtrip** (MemoryStore → `to_srsj` → JsonStore): the same fixture produces the same I-63/I-64 diagnostics from both store implementations (CLAUDE.md cross-store rule).

#### Milestone gate

As template. Commit `feat(repository): I-63/I-64 root-type anchor diagnostics (#125)`.

---

### Phase 3: Service filter struct + CLI `document-view list --root-type`

**Goal:** `document-view list` filtering moves into a service-level filter struct (ADR-010); `--root-type <uuid>` filters by `rootTypeRefs`; summary exposes `rootTypeRefs`.

**Agent:** CLI Worker (coordinating the `view_service.rs` filter-struct change with the Lead Integrator)

#### Tasks

- [ ] Add `root_type_refs: Option<Vec<ExactTypeRef>>` to `view_service::DocumentViewSummary`; populate it in `list_document_views_summary` from each `DocumentView.root_type_refs`.
- [ ] Add `pub struct DocumentViewListFilter { pub namespace: Option<String>, pub container_type: Option<String>, pub root_type_id: Option<String> }` (with `Default`) to `view_service.rs`. Change `list_document_views_summary(store)` → `list_document_views_summary(store, filter: &DocumentViewListFilter)` and perform **all three** filters inside the service: namespace equals, container_type equals, and `root_type_id` → keep summaries whose `root_type_refs` contains an `ExactTypeRef` with matching `type_id`. This resolves ADR-010's filter-struct rule for the two pre-existing handler-side retains as well.
- [ ] Update `commands/mod.rs`: add `#[arg(long = "root-type")] root_type: Option<String>` to the `DocumentViewCommand::List` variant (alongside existing `namespace`, `container_type`); update the `dispatch` match arm to pass `root_type` through.
- [ ] Update `commands/document_view.rs::cmd_document_view_list`: signature becomes `(ctx, namespace, container_type, root_type)`; map the three flags into a `DocumentViewListFilter` and make **one** service call (no handler-side retains) — handler returns to the thin pattern (ADR-010, CLAUDE.md ~15-line rule).
- [ ] Update the existing `list_document_views_summary(store)` call site in `srs-bindings` (Phase 4) to pass a filter.
- [ ] Run `cargo run --bin generate-schemas`; confirm **no diff** to `crates/srs-cli/schemas/payload/*.json` (the embedded summary is opaque `Value` — see Contracts); stage only if a diff appears.

#### Acceptance Criteria

- [ ] `document-view list --root-type <uuid>` returns only matching views; `--root-type` with no match returns an empty list (still `ok: true`).
- [ ] `document-view list --namespace` / `--type` behave exactly as before (now via the service filter).
- [ ] `cmd_document_view_list` makes a single service call with no in-handler `retain`.
- [ ] `DocumentViewSummary` JSON carries `rootTypeRefs` (omitted when none).
- [ ] `cargo test --test payload_contracts` green (no golden-schema change expected).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

- CLI integration test for `--root-type` filter (matching + non-matching uuid).

#### Milestone gate

As template. Commit `feat(cli): document-view list --root-type filter (#125)` + the regenerated schema files.

---

### Phase 4: Bindings — `list_document_views` WASM

**Goal:** WASM consumers can list document views (parallels `list_containers`).

**Agent:** Bindings Worker

#### Tasks

- [ ] Add `pub fn list_document_views(&self, filter_json: &str) -> Result<JsValue, JsValue>` to `crates/srs-bindings/src/lib.rs`, mirroring the `list_containers` pattern: parse `filter_json` into a binding-local `DocumentViewListBindingFilter { namespace?, container_type?, root_type_id? }`, build a `DocumentViewListFilter`, call `view_service::list_document_views_summary(&self.store, &filter)`, return via `to_js`. Pass `"{}"` for all views.
- [ ] Doc-comment the JS-side shape inline (a JS array of objects `{ id, namespace, name, version, description, containerType?, rootTypeRefs?, sourcePackage? }` and the filter JSON shape) — describe the shape, not the internal Rust struct name (matches `list_containers`/`list_records` doc style).

#### Acceptance Criteria

- [ ] Binding compiles for the wasm target and returns summaries; `filter_json = "{}"` returns all.
- [ ] `root_type_id` filter narrows results to views whose `rootTypeRefs` include that type id.
- [ ] No business logic in the binding (delegates to service).
- [ ] `cargo build -p srs-bindings` green (native check); `cargo clippy -p srs-bindings -- -D warnings`.

#### Testing

```bash
cargo build -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

#### Milestone gate

As template. Commit `feat(bindings): list_document_views WASM binding (#125)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged except the documented `rootTypeRefs` additions (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `srs repo validate --repo ../srs/srs` reports 0 errors (I-63/I-64 add no false positives)
- [ ] All issue #125 acceptance criteria satisfied except the external muSrs fixture (filed as follow-up)

## Coordination Rules

Standard (see template). Phases are sequential; each gate must pass before the next.

## Assumptions

- The schema mirror (`crates/srs-schema/schemas/2.0/`) already matches the accepted RFC-009 spec (commit `9e7793b`); no entity-schema edits are needed in this plan.
- `containers_for_instance` (I-66) and Container `description`/`tags` (Change C) are already implemented and tested; this plan does not touch them.
- The first entry of `rootInstanceIds` is the authoritative root for I-64 resolution (per RFC-009 resolution semantics).
