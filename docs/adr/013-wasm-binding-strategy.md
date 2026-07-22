# ADR-013: WebAssembly Binding Strategy

- **Status:** accepted — see Addendum (Repo-Independent Free Functions, 2026-07-12, #244)
- **Date:** 2026-06-06
- **Supersedes:** —
- **Superseded by:** [ADR-038](038-vfs-tree-primary-model.md) (bundle-format rationale only — the rejected `{path: content}` map model is now the primary operational model; binding-thinness rules and the wasm32 CI gate remain in force)

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

### No-filesystem entry point: `srsj_migration_service::load_from_srsj()`

A constructor `JsonStore::from_srsj(content: &str) -> Result<Self, RepositoryError>` is provided by `srs-repository`. It parses a `.srsj` string and populates the in-memory `JsonStoreState` without any `std::fs` calls. `manifest.root` is set to `PathBuf::from(".")`.

`open()` calls `read_to_string()` then delegates to `from_srsj()`, so both paths share the same deserialization logic.

`SrsRepository::load` (the WASM entry point) calls `srsj_migration_service::load_from_srsj`, which applies the RFC-014 manifest migration (moves `manifest.meta.upstreamPackage` to top-level and strips `contentHash` if present, as it was removed from the spec schema in RFC-014 Rev 4) before delegating to `from_srsj`. This is idempotent on already-migrated bundles. Callers that need migration applied automatically should use `load_from_srsj`; `from_srsj` remains available as a lower-level primitive for contexts where the input is guaranteed to be already migrated.

**Known limitation:** `manifest.root = "."` means any package-ref paths resolved relative to the manifest root will resolve relative to the process CWD. This is acceptable for the initial read-only scope because the `.srsj` format embeds all package definitions inline and no external path resolution occurs during read-only service calls.

### Wasm surface: `srs-bindings` with `wasm-bindgen`, read-only initial scope

`srs-bindings` is the sole crate that depends on `wasm-bindgen`. It exposes a `#[wasm_bindgen] pub struct SrsRepository` whose public methods are the canonical WASM surface (see `crates/srs-bindings/src/lib.rs` for the full list). The initial read-only set was `load`, `validate`, `list_records`, `get_record`, `list_notes`; the surface has since grown to include write operations (ADR-015) and additional read methods such as `declared_extensions_conformance` (extension conformance reporting, #442). Each method is a thin wrapper: deserialize JS input → call one service function from `srs-repository` → serialize output to `JsValue`. No business logic lives in `srs-bindings`.

Write operations (create/update/delete) are deferred to a future plan. The `flush()` method on `JsonStore` requires a backing file; calling it from Wasm would fail. Read-only paths never call `flush()`.

### UUID v4 entropy on Wasm: `uuid` `js` feature

`srs-core` generates UUIDs using `uuid` v1 with the `v4` feature. On `wasm32-unknown-unknown`, UUID v4 requires entropy from the browser's `crypto.getRandomValues`. This is enabled by adding `uuid = { workspace = true, features = ["js"] }` for the `wasm32` target in `srs-bindings/Cargo.toml`. Cargo's feature unification propagates the `js` feature to `srs-core` during Wasm compilation.

### `FileStore` and `detect.rs` on Wasm

*(Superseded by ADR-038.)* Originally, `FileStore` used `std::fs` directly and was not
callable from Wasm. Since ADR-038, `FileStore` runs over the `Vfs` seam and — backed by
`MemVfs` — is the **primary WASM session store**. What remains true: `DiskVfs` and
`detect::find_repo_root` use `std::fs` and are unreachable from binding paths (dead-stripped
from the `.wasm`); all service functions accept `&dyn RepositoryStore` and are unaffected.

## Consequences

**Positive:**
- SRS record querying and validation run entirely client-side — no server needed for read operations.
- Crate boundaries are preserved: zero business logic in `srs-bindings`, all service logic stays in `srs-repository`.
- The `.srsj` format becomes the canonical bundle for browser delivery of SRS repositories.
- `JsonStore::from_srsj()` is a useful primitive for any embedding context that doesn't have filesystem access (not just Wasm). `srsj_migration_service::load_from_srsj()` is the recommended entry point when RFC-014 migration must also be applied on load (e.g. the WASM binding).

**Negative / trade-offs:**
- Write operations via Wasm are deferred. A browser application cannot modify an SRS repository through the Wasm surface in this initial cut.
- `manifest.root = "."` means the in-memory store has a different root path than a file-backed store. Services that depend on absolute path resolution (none currently, in the read-only surface) would behave differently.

**Neutral:**
- The `wasm-pack` tool is required to build the Wasm package. It is not part of the standard Cargo build.
- The `pkg/` output directory produced by `wasm-pack` is gitignored.
- **CI enforcement:** CI type-safety verification for `srs-bindings` uses `cargo build --target wasm32-unknown-unknown -p srs-bindings` rather than `wasm-pack build` or `cargo check`. Rationale: (1) `wasm-pack` is not in the standard Rust toolchain and requires a separate install step; (2) `cargo check` is insufficient because it does not perform codegen or linking — link-time errors from missing `__wbindgen_*` descriptor symbols are only caught by `cargo build`; (3) JS binding and package generation are distribution concerns, not CI type-safety concerns. The `wasm-build` job must be listed as a Required Status Check on the `master` branch protection rule to enforce this gate on every PR.

---

## Addendum — Repo-Independent Free Functions (2026-07-12, issue #244)

### Context

Registry catalog parsing (the `ext:registry` extension) is a repo-independent operation: a caller supplies a raw JSON string or file path, not a loaded SRS repository. Loading a full `SrsRepository` just to parse a registry file would be wasteful and architecturally misleading — the `SrsRepository` struct is specifically for operations over a loaded repository.

### Decision

Repo-independent operations may be exposed as **free `#[wasm_bindgen]` functions** rather than methods on `SrsRepository`. The canonical `SrsRepository`-methods surface defined above continues to govern all operations that require a loaded repository.

Registry parsing is the first application: `parse_registry(catalog_json: &str)` and `list_registry_entries(catalog_json: &str, filter_json: &str)` are free functions in `srs-bindings`. They call the same lower-level primitives (`parse_registry_json`, `filter_registry_entries`) from `srs-repository` that the CLI's `list_registry` service function calls internally — no business logic is duplicated. (The CLI-facing service also accepts a `PathBuf` and performs file I/O directly, since there is no `RepositoryStore` to delegate to for repo-independent operations; the WASM caller supplies content as a string and uses the lower-level primitives instead.)

### Criteria for a free function (not a `SrsRepository` method)

An operation qualifies as a free function when **all** of the following hold:
- It does not require a loaded SRS repository (`JsonStore`/`RepositoryStore`).
- It operates on a caller-supplied payload (raw JSON string, file path, or primitive type).
- It would also make sense in a context where no repository has been opened (e.g. a standalone registry browser).

If any of these conditions fails, the binding belongs on `SrsRepository`.
