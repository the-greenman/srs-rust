# Plan: WASM query bindings — containers_for_instance, type_schema, list_blueprints, document_views_for_container

> Implements issue #181 (Decision Logger v1, engine layer). Four read-only `srs-repository` services already exist; this plan exposes each as a thin `#[wasm_bindgen]` method on `SrsRepository`, following ADR-013.

## Summary

The thin web client (srs-web T-C1) needs four more read-only queries available in-browser. The underlying services already exist in `srs-repository` (`container_service::containers_for_instance`, `type_schema_service::type_schema`, `blueprint_service::list_blueprints_summary`, `view_service::document_views_for_container`). This plan adds four thin wrapper methods to the existing `SrsRepository` WASM surface in `crates/srs-bindings/src/lib.rs` — each deserialises JS input, calls exactly one service, and serialises the result to a `JsValue`. No business logic moves; no service signatures change. This is a pure additive binding surface under the already-accepted ADR-013.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Bindings Worker | Phase 1 |
| Verification | Phase 1 milestone gate |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library-first; `srs-bindings` is the designated WASM consumer; zero business logic in bindings | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `.srsj` bundle; thin `wasm-bindgen` wrappers; serialise service output directly to `JsValue` | accepted — this plan adds four methods under it |

_No new ADR required — this plan applies the existing ADR-013 strategy to four additional services. No new architectural constraint is established and no prior decision is changed._

**Binding return-shape convention (resolved at design pause, recorded here):**
- Bindings serialise the **service struct directly**, not the CLI's ADR-011 payload mirror. (Consistent with the 16 existing bindings: `list_containers` returns `ContainerSummary`, `list_document_views` returns the service summary, etc.)
- Where the service returns a **non-`Serialize`** result carrying diagnostics, the binding manually destructures it into a `json!({...})` envelope mirroring the existing `blueprint_schema` binding (`lib.rs:200-203`). `TypeSchemaResult` (`#[derive(Debug, Clone)]` only) → `{ schema, diagnostics }`; `BlueprintListResult` (`#[derive(Debug, Clone)]` only) → `{ summaries, diagnostics }`. Do **not** attempt `to_js(&result)` on these two — it will not compile. `ContainerSummary` and `DocumentView` both derive `Serialize` and go through `to_js` directly.

**`containers_for_instance` vs `list_containers` (Architecture Review F5 — kept, with justification):** The new binding is functionally reachable today via `list_containers('{"memberInstanceId":"<uuid>"}')`. It is kept anyway because (a) issue #181 names it as a required binding and srs-web T-C1 consumes it by name, (b) the **service** `container_service::containers_for_instance` already exists as a named convenience wrapper (`container_service.rs:116`), so the binding is a 1:1 thin export — no business logic is duplicated, the filter construction lives in the service, not the binding. The JS-surface overlap is intentional ergonomics, mirroring the service layer's own choice to offer both entry points.

**`type_schema` input signature (Architecture Review F2):** `type_schema(type_id: &str, type_version: Option<u32>)`. wasm-bindgen 0.2 supports `Option<u32>` at the ABI; JS callers pass `undefined` (or omit) for "latest version". This is documented in the method's rustdoc. If the wasm build rejects `Option<u32>`, fall back to a JSON-string input matching `TypeSchemaInput`.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `crates/srs-cli/src/payload.rs` is untouched. `cargo test --test payload_contracts` must pass unchanged as a regression guard.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- `crates/srs-bindings/src/lib.rs`: add four `#[wasm_bindgen]` methods on `impl SrsRepository`:
  - `containers_for_instance(&self, instance_id: &str) -> Result<JsValue, JsValue>` → `container_service::containers_for_instance` → `Vec<ContainerSummary>` (serialised directly).
  - `type_schema(&self, type_id: &str, type_version: Option<u32>) -> Result<JsValue, JsValue>` → `type_schema_service::type_schema` with `TypeSchemaInput { type_id, type_version }` → `json!({ "schema", "diagnostics" })` (mirrors `blueprint_schema`).
  - `list_blueprints(&self) -> Result<JsValue, JsValue>` → `blueprint_service::list_blueprints_summary` → `json!({ "summaries", "diagnostics" })`.
  - `document_views_for_container(&self, container_id: &str) -> Result<JsValue, JsValue>` → `view_service::document_views_for_container` → `Vec<DocumentView>` (serialised directly).
- Each method is rustdoc-commented documenting its JSON return shape, matching the style of the existing bindings.
- Tests (per-feature file convention, matching `containers.rs` / `blueprint_schema.rs`):
  - `containers_for_instance` → extend `crates/srs-bindings/tests/containers.rs`. Gallery has containers with members → real happy path.
  - `type_schema` → new `crates/srs-bindings/tests/type_schema.rs`. Gallery has 3 types (`decision` `1fcad6a2…`, `article` `a1142ac3…`, `role` `e53dce11…`) → real happy path; unknown id → `Err`.
  - `list_blueprints` → new `crates/srs-bindings/tests/blueprints.rs`. **Gallery has `blueprints: []`**, so use an inline `.srsj` fixture carrying a blueprint (same approach as `blueprint_schema.rs`'s `blueprint_srsj()` helper) for the populated case, plus a gallery call asserting the empty `{ summaries: [], diagnostics: [] }` envelope.
  - `document_views_for_container` → new `crates/srs-bindings/tests/document_views.rs`. **Gallery views carry no `rootTypeRefs`**, so the service returns `[]` for every gallery container; use an inline `.srsj` fixture with a DocumentView whose `rootTypeRefs` binds a typed container's root for the populated case, plus a gallery call asserting `[]`.

**Out of scope:**
- Any change to the four service functions or their signatures.
- Write operations via WASM (governed separately by ADR-015).
- `srs-web` consumption (T-C1, separate repo/issue).
- New CLI payloads or commands.

---

## Phases

### Phase 1: Add four query bindings + tests

**Goal:** All four services are callable from the WASM surface, each returns its documented JSON shape, and `srs-bindings` tests cover each via a gallery round-trip; native and wasm builds are clean.

**Agent:** Bindings Worker

#### Tasks

- [x] Add `use` imports: `type_schema_service::{self, TypeSchemaInput}` and `blueprint_service` (for `list_blueprints_summary`). `container_service` (`lib.rs:5`) and `view_service` (`lib.rs:11`) are already imported — call `container_service::containers_for_instance` / `view_service::document_views_for_container` directly.
- [x] Implement `containers_for_instance(instance_id: &str)` binding → `to_js(&Vec<ContainerSummary>)`.
- [x] Implement `type_schema(type_id: &str, type_version: Option<u32>)` binding → manual `json!({ "schema": result.schema, "diagnostics": result.diagnostics })` (`TypeSchemaResult` is not `Serialize`). Rustdoc the `undefined`-for-latest JS convention.
- [x] Implement `list_blueprints()` binding → manual `json!({ "summaries": result.summaries, "diagnostics": result.diagnostics })` (`BlueprintListResult` is not `Serialize`).
- [x] Implement `document_views_for_container(container_id: &str)` binding → `to_js(&Vec<DocumentView>)`.
- [x] Add per-feature tests as listed in Scope: `containers.rs` (extend), `type_schema.rs`, `blueprints.rs`, `document_views.rs`. Each: a populated happy-path case (inline fixture where gallery cannot supply it) + a negative/empty case.

#### Acceptance Criteria

- [x] Each binding compiles under `#[wasm_bindgen]` and returns the documented JSON shape.
- [x] `type_schema` resolves latest version when `type_version` is `None`, a specific version when `Some`.
- [x] `list_blueprints` returns `{ summaries: [...], diagnostics: [...] }`.
- [x] `srs-bindings` tests cover each new binding with a gallery round-trip.
- [x] No business logic added to `srs-bindings` — each method is deserialise → one service call → serialise.

#### Testing

```bash
cargo test -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings   # if the wasm target is installed
```

Specific tests to write or verify:

- `containers_for_instance_lists_owning_containers` — an instance that is a member of a gallery container returns that container summary.
- `containers_for_instance_empty_for_uncontained` — an unknown/uncontained instance id returns `[]`.
- `type_schema_resolves_latest_version` — `type_version = None` for `decision` returns a draft-07 schema object with `diagnostics`.
- `type_schema_resolves_pinned_version` — `type_version = Some(1)` for `decision` resolves the same schema.
- `type_schema_unknown_id_errors` — unknown type id returns `Err`.
- `list_blueprints_returns_summaries` — inline blueprint fixture returns one summary (`id`, `namespace`, `rootTypeCount`, …) plus `diagnostics`.
- `list_blueprints_empty_on_gallery` — gallery returns `{ summaries: [], diagnostics: [] }`.
- `document_views_for_container_returns_matching` — inline fixture with a `rootTypeRefs`-bound view returns it for the matching container.
- `document_views_for_container_empty_when_unbound` — a gallery container (views carry no `rootTypeRefs`) returns `[]`.

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm every listed test exists and passes.
3. Run:

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

4. Mark plan checkboxes `[x]`.
5. Commit referencing #181.

---

## Final Acceptance

- [x] `cargo test` passes with no failures.
- [x] `cargo clippy -- -D warnings` passes.
- [x] CLI output format unchanged (`cargo test --test payload_contracts` passes — no payload structs changed).
- [x] `bash scripts/check-schema-sync.sh` exits 0 (or: no entity schemas changed — N/A).
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` exits 0 (if target installed).
- [x] Four new bindings present, documented, and tested on the gallery fixture.

## Coordination Rules

- Bindings Worker keeps to `crates/srs-bindings/`. No edits to `srs-repository` services.
- Each method: arg → one service call → serialise. No validation or branching logic beyond input parse and the existing `to_js` / `json!` envelope pattern.
- Verification Agent runs the milestone gate before sign-off.

## Assumptions

- The four target services are stable and return the shapes read from source at plan time; no service changes are needed.
- The `gallery.srsj` fixture covers the happy path for `containers_for_instance` (3 containers with members) and `type_schema` (3 types). It does **not** cover `list_blueprints` (`blueprints: []`) or `document_views_for_container` (no view carries `rootTypeRefs`); those two use a small inline `.srsj` fixture per the `blueprint_schema.rs` precedent, with gallery used for the empty-case assertion.
- Tests follow the established native-service pattern (not `#[wasm_bindgen_test]`); the wasm-pack/`cargo build --target wasm32` step proves the `#[wasm_bindgen]` surface compiles.
