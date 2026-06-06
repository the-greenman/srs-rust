# Plan: WebAssembly Bindings via `srs-bindings`

## Summary

A web application needs to load and process an SRS repository entirely in-browser — no server, no CLI process. The `srs-bindings` crate (currently a stub) is the designated home for this surface per ADR-001. The work has two logical parts: (1) add a `JsonStore::from_srsj()` constructor to `srs-repository` so a `.srsj` blob can be loaded without touching the filesystem, and (2) implement a `wasm-bindgen` binding surface in `srs-bindings` that wraps the existing service functions. No business logic moves — this follows the library-first contract exactly.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | Phase A |
| Bindings Worker | Phase B |
| Verification Agent | Phase C |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library-first; `srs-bindings` is the designated WASM consumer | accepted — this plan implements it |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `.srsj` as the Wasm bundle format; `JsonStore::from_srsj()` as the no-filesystem entry point; `srs-bindings` as the sole `wasm-bindgen` surface | proposed — written at end of Phase A, must be accepted before Phase B begins |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `cargo test --test payload_contracts` must pass unchanged as a regression guard — if it fails, an unintended change was made.

### Entity schema sync

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- Add `JsonStore::from_str(content: &str) -> Result<Self, RepositoryError>` constructor to `srs-repository`
- Add `wasm-bindgen` / `cdylib` configuration to `srs-bindings/Cargo.toml`
- Implement `SrsRepository` Wasm type in `srs-bindings/src/lib.rs` with an initial read-only surface (load, validate, list_records, get_record, list_notes)
- Add `uuid = { workspace = true, features = ["js"] }` to `srs-bindings/Cargo.toml` to enable UUID v4 generation on Wasm (Cargo feature unification propagates this to `srs-core`)
- Write ADR-013 at end of Phase A
- Add `#[cfg(not(target_arch = "wasm32"))]` guards to `FileStore` impls and `detect.rs` — these are expected given their `std::fs` usage; attempt compilation first to confirm which guards are needed

**Out of scope:**
- Write operations (create/update/delete) via Wasm — read-only first
- OPFS-backed `FileStore` for in-browser filesystem access
- Python/Node.js native bindings
- `render_view` / document view rendering — deferred until basic query surface is stable
- Async service surface

---

## Phases

### Phase A: `JsonStore::from_srsj()` + ADR-013

**Goal:** A `.srsj` string can be loaded into a `JsonStore` with zero filesystem access, ADR-013 is written, and all existing tests still pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] `crates/srs-repository/src/json_store.rs`: Add `pub fn from_str(content: &str) -> Result<Self, RepositoryError>`. Extract everything in `open()` that comes after `read_to_string()` into this constructor: the `serde_json::from_str::<JsonStoreFile>()`, version check, manifest parse via `serde_json::from_value()`, and `JsonStoreState` construction. Use `PathBuf::from("<memory>")` for `file_path` and `PathBuf::from(".")` for `manifest.root`. Refactor `open()` to call `read_to_string()` then delegate to `from_str()`.

  Note: `manifest.root = PathBuf::from(".")` is a known limitation of in-memory stores. It means package-ref paths that are resolved relative to `manifest.root` will resolve relative to the process CWD. This is acceptable for the initial read-only scope because the `.srsj` format embeds all package definitions inline — no external path resolution occurs.

- [ ] `crates/srs-repository/src/json_store.rs`: Add unit tests:
  - `from_str_roundtrip` — serialize a minimal `JsonStoreFile { srsj: "1", manifest: {...}, data: {...} }` to JSON, call `JsonStore::from_srsj()`, call `store.load_manifest()`, assert instance index is correct.
  - `from_str_bad_version` — call `from_str()` with `"srsj": "2"`, assert `RepositoryError::InvalidSnapshotData`.
  - `open_delegates_to_from_str` — write a `.srsj` file to a `TempDir`, call `open()`, verify it loads the same manifest as `from_str()` on the same content string.

- [ ] `docs/adr/013-wasm-binding-strategy.md`: Write ADR-013.

#### Acceptance Criteria

- [ ] `JsonStore::from_str(srsj_json)` succeeds with a valid `.srsj` payload and returns a usable store
- [ ] `JsonStore::from_str` with `"srsj": "2"` returns `RepositoryError::InvalidSnapshotData`
- [ ] `open()` behaviour is unchanged — it reads the file and delegates to `from_str()`
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` clean
- [ ] ADR-013 committed to `docs/adr/`

---

### Phase B: Wasm binding surface in `srs-bindings`

**Goal:** `wasm-pack build crates/srs-bindings --target web` produces a valid `.wasm` + JS/TS package, and a smoke test confirms `SrsRepository.load()` can query records from a `.srsj` string.

**Agent:** Bindings Worker

**Prerequisite:** Phase A milestone gate complete. ADR-013 accepted.

#### Tasks

- [ ] `crates/srs-bindings/Cargo.toml`: Set `[lib] crate-type = ["cdylib", "rlib"]`.
- [ ] `crates/srs-bindings/src/lib.rs`: Implement `SrsRepository` wasm type.
- [ ] Attempt `cargo build --target wasm32-unknown-unknown -p srs-bindings`; add cfg guards if needed.
- [ ] Create smoke test fixture `crates/srs-bindings/tests/fixtures/smoke.srsj`.

#### Acceptance Criteria

- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` exits 0
- [ ] `wasm-pack build crates/srs-bindings --target web --dev` produces `pkg/`
- [ ] `cargo test` (native target, all crates) still passes
- [ ] No `std::fs` in `crates/srs-bindings/src/`

---

## Final Acceptance

- [ ] `cargo test` passes with no new failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` exits 0
- [ ] `wasm-pack build crates/srs-bindings --target web` produces valid `pkg/`
- [ ] ADR-013 committed to `docs/adr/`
