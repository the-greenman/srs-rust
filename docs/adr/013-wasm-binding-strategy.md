# ADR-013: WebAssembly Binding Strategy

- **Status:** accepted
- **Date:** 2026-06-06
- **Supersedes:** —
- **Superseded by:** —

## Context

A web application needs to load and process an SRS repository entirely in-browser without a server or CLI process. ADR-001 established that `srs-bindings` is the designated consumer for WASM and FFI surfaces, and that business logic must remain in `srs-core` and `srs-repository`. This ADR records the concrete decisions made when implementing the first Wasm surface.

Three questions needed answers:

1. **Bundle format** — how does a web app pass a repository to the Wasm module?
2. **No-filesystem entry point** — how does `srs-repository` load a repository without `std::fs`?
3. **Scope** — read-only or read-write for the initial surface?

## Decision

### Bundle format: `.srsj`

The `.srsj` single-file format is used as the Wasm bundle. A `.srsj` file is a self-contained JSON object containing the full repository state:

```json
{ "srsj": "1", "manifest": { ... }, "data": { "records/foo.json": { ... }, ... } }
```

A web app loads this via `fetch()` and passes the response text directly to `SrsRepository.load(srsj)`.

**Rationale:** The format already exists and is fully supported by `JsonStore`. It requires no external path resolution — all package definitions are embedded inline. No new format is introduced.

**Rejected alternative:** Passing a raw multi-file structure (a map of `{path: content}` pairs) would be more flexible but would require a new serialisation contract and a new store implementation.

### No-filesystem entry point: `JsonStore::from_srsj()`

A new constructor `JsonStore::from_str(content: &str) -> Result<Self, RepositoryError>` is added to `srs-repository`. It parses a `.srsj` string and populates the in-memory `JsonStoreState` without any `std::fs` calls. `manifest.root` is set to `PathBuf::from(".")`.

`open()` is refactored to call `read_to_string()` then delegate to `from_str()`, so both paths share the same deserialization logic.

**Known limitation:** `manifest.root = "."` means any package-ref paths resolved relative to the manifest root will resolve relative to the process CWD. This is acceptable for the initial read-only scope because the `.srsj` format embeds all package definitions inline and no external path resolution occurs during read-only service calls.

### Wasm surface: `srs-bindings` with `wasm-bindgen`, read-only initial scope

`srs-bindings` is the sole crate that depends on `wasm-bindgen`. It exposes a `#[wasm_bindgen] pub struct SrsRepository` with read-only methods (`load`, `validate`, `list_records`, `get_record`, `list_notes`). Each method is a thin wrapper: deserialize JS input → call one service function from `srs-repository` → serialize output to `JsValue`. No business logic lives in `srs-bindings`.

Write operations (create/update/delete) are deferred to a future plan. The `flush()` method on `JsonStore` requires a backing file; calling it from Wasm would fail. Read-only paths never call `flush()`.

### UUID v4 entropy on Wasm: `uuid` `js` feature

`srs-core` generates UUIDs using `uuid` v1 with the `v4` feature. On `wasm32-unknown-unknown`, UUID v4 requires entropy from the browser's `crypto.getRandomValues`. This is enabled by adding `uuid = { workspace = true, features = ["js"] }` for the `wasm32` target in `srs-bindings/Cargo.toml`. Cargo's feature unification propagates the `js` feature to `srs-core` during Wasm compilation.

### `FileStore` and `detect.rs` Wasm guards

`FileStore` and `detect::find_repo_root` use `std::fs` and are not callable from Wasm. These are gated with `#[cfg(not(target_arch = "wasm32"))]` so that `srs-repository` compiles cleanly for the `wasm32-unknown-unknown` target. All service functions accept `&dyn RepositoryStore` and are unaffected by this gating.

## Consequences

**Positive:**
- SRS record querying and validation run entirely client-side — no server needed for read operations.
- Crate boundaries are preserved: zero business logic in `srs-bindings`, all service logic stays in `srs-repository`.
- The `.srsj` format becomes the canonical bundle for browser delivery of SRS repositories.
- `JsonStore::from_srsj()` is a useful primitive for any embedding context that doesn't have filesystem access (not just Wasm).

**Negative / trade-offs:**
- Write operations via Wasm are deferred. A browser application cannot modify an SRS repository through the Wasm surface in this initial cut.
- `manifest.root = "."` means the in-memory store has a different root path than a file-backed store. Services that depend on absolute path resolution (none currently, in the read-only surface) would behave differently.

**Neutral:**
- The `wasm-pack` tool is required to build the Wasm package. It is not part of the standard Cargo build.
- The `pkg/` output directory produced by `wasm-pack` is gitignored.
