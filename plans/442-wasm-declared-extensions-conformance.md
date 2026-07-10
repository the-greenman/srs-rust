# Plan: WASM binding — expose declared_extensions_conformance

> Issue: srs-rust#442

## Summary

`manifest_service::declared_extensions_conformance` was introduced in #237 and is already exposed
through the CLI (`srs manifest conformance`). Per capability-layering (docs/architecture/
capability-layering.md) every `srs-repository` service must also be reachable through the WASM
binding surface. This plan adds the missing `SrsRepository::declared_extensions_conformance()`
method so srs-web (and any other WASM consumer) can query extension conformance without spawning
a CLI subprocess.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (pipeline runner) |
| Bindings Worker | (pipeline runner) |
| Verification | (pipeline runner) |

## Architecture Decisions

No new ADRs required. This plan implements existing decisions:

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | srs-bindings is a thin consumer of srs-repository; no business logic in the binding layer | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Bindings call exactly one service function; all validation and logic stay in srs-repository | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM bindings call srs-repository service functions via `to_js()`; no business logic in srs-bindings | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands are added or changed. No payload structs touched. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are modified. `check-schema-sync.sh` is not required.

---

## Scope

- Add `pub fn declared_extensions_conformance(&self) -> Result<JsValue, JsValue>` to
  `impl SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- Add `use srs_repository::manifest_service;` to the imports in that file.
- Add one smoke test in `crates/srs-bindings/src/lib.rs` `#[cfg(test)]` block that:
  - Loads a minimal `.srsj` store.
  - Calls `manifest_service::declared_extensions_conformance(&store)`.
  - Asserts the result serialises to a JSON object with the four expected keys:
    `declared`, `supported`, `declaredButUnsupported`, `usedButUndeclared`.

**Out of scope:**
- Exposing `add_declared_extension` / `remove_declared_extension` write bindings (separate
  issue — requires write surface design that adds extension mutation to srs-web).
- CLI changes (none needed — CLI already has `srs manifest conformance` from #237).

---

## Phases

### Phase 1: Add the binding method and smoke test

**Goal:** `SrsRepository::declared_extensions_conformance()` is callable from the WASM surface
and returns a JSON value with the four report keys; a native smoke test proves serialisation.

**Agent:** Bindings Worker

#### Tasks

- [x] In `crates/srs-bindings/src/lib.rs`, add `use srs_repository::manifest_service;` to the
  import block (alphabetically near `manifest` imports).
- [x] Add the following method to `impl SrsRepository`:

```rust
/// Return a conformance report comparing the manifest's `declaredExtensions` against the
/// implementation's supported set and detected content usage.
/// Returns a `DeclaredExtensionsReport` as a JS value with four camelCase keys:
/// `declared`, `supported`, `declaredButUnsupported`, `usedButUndeclared`.
pub fn declared_extensions_conformance(&self) -> Result<JsValue, JsValue> {
    let report =
        manifest_service::declared_extensions_conformance(&self.store).map_err(js_err)?;
    to_js(&report)
}
```

- [x] Add smoke test `declared_extensions_conformance_report_serialises` in the `#[cfg(test)]`
  block using a minimal in-memory srsj that loads correctly and calls the service function
  directly (not through `to_js()` — `js_sys::JSON::parse` inside `to_js()` requires a
  JavaScript runtime and is not available in native `#[cfg(test)]` builds).

#### Acceptance Criteria

- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds; if the
  `wasm32-unknown-unknown` target is not installed locally, the CI `wasm-build` job is the
  enforcement gate per ADR-013 — do not substitute `cargo check`.
- [x] `cargo test -p srs-bindings` passes with the new smoke test green.
- [x] `cargo clippy -p srs-bindings -- -D warnings` reports zero warnings.
- [x] The smoke test asserts all four keys are present in the serialised output:
  `declared`, `supported`, `declaredButUnsupported`, `usedButUndeclared`.

#### Testing

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests to write or verify:
- `declared_extensions_conformance_report_serialises` — proves `DeclaredExtensionsReport`
  serialises to JSON with all four camelCase keys present and correct types.

#### Milestone gate

1. All acceptance criteria above are checked.
2. `declared_extensions_conformance_report_serialises` exists in the test module and passes.
3. Run:
   ```bash
   cargo test -p srs-bindings
   cargo clippy -p srs-bindings -- -D warnings
   ```
4. Mark task checkboxes `[x]` and commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass) — no CLI changes made
- [ ] `cargo test --test payload_contracts` passes — no payload structs changed
- [ ] `bash scripts/check-schema-sync.sh` exits 0 — no entity schemas changed
- [ ] `declared_extensions_conformance_report_serialises` smoke test passes
- [ ] New binding method follows ADR-013 pattern (one service call, `to_js` serialisation)

## Coordination Rules

- Write scope: `crates/srs-bindings/src/lib.rs` only.
- No changes to `srs-repository`, `srs-core`, or `srs-cli`.
- Lead Integrator verifies method placement is consistent with the existing binding API ordering
  (add it in the read-only section, near `validate` or repository-level methods).

## Assumptions

- `DeclaredExtensionsReport` already derives `serde::Serialize` with `#[serde(rename_all = "camelCase")]`
  (confirmed in manifest_service.rs).
- `manifest_service::declared_extensions_conformance` accepts `&dyn RepositoryStore`;
  `&self.store` satisfies this via `JsonStore: RepositoryStore` (confirmed from service signature).
- The `wasm32-unknown-unknown` build target may not be available in the cloud environment;
  `cargo test -p srs-bindings` (native) is sufficient for CI gating alongside the existing
  `wasm-build` CI job that enforces WASM compilation.
