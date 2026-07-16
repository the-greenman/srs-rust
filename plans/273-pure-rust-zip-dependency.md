# Plan: Add pure-Rust zip dependency (wasm-clean)

> **Issue:** srs-rust#273  
> **Epic:** srs-rust#271 (Attachments, .srs archive store & srs-gov dogfood)

## Summary

`srs-repository` will need to read and write `.srs` archive files (ZIP-based) in a future phase
(issues #274–#278). Before that implementation begins, a wasm32-clean zip crate must be declared
as a workspace dependency and verified to compile under `wasm32-unknown-unknown`. Without this
gate, a C-backed zip library could silently enter the tree and only break the WASM CI job much
later. This plan adds `zip` (pure-Rust deflate, no C toolchain) to `srs-repository`, creates a
minimal stub module to anchor the import, and confirms both `cargo build` and the wasm32 target
build remain green.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | All `srs-repository` dependencies must compile to `wasm32-unknown-unknown`; the CI `wasm-build` job (which builds `srs-bindings`, which depends on `srs-repository`) is the enforcement gate. | accepted |

No new ADRs are required. The constraint that `srs-repository` must remain wasm32-clean is
already enforced by ADR-013 and the CI `wasm-build` job. The choice of `zip` with `deflate`
(miniz_oxide backend, pure Rust) over alternatives with C backends is a direct application of
that existing constraint, not a new architectural decision.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No payload structs added or modified. `cargo test --test
payload_contracts` is unaffected.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` or any schema mirror. `bash scripts/check-schema-sync.sh`
is unaffected.

---

## Scope

- Add `zip = { version = "2", default-features = false, features = ["deflate"] }` to
  `[workspace.dependencies]` in the root `Cargo.toml`.
- Add `zip = { workspace = true }` to `crates/srs-repository/Cargo.toml`.
- Create `crates/srs-repository/src/archive.rs` as a placeholder module that imports from `zip`,
  anchoring the dependency so the compiler verifies it resolves.
- Declare `pub(crate) mod archive;` in `crates/srs-repository/src/lib.rs`.
- Verify `cargo build --all` passes.
- Verify `cargo build --target wasm32-unknown-unknown -p srs-bindings` passes.

**Out of scope:**
- Any zip read/write implementation (ZipStore, pack, unpack) — tracked in #276.
- Source-document loading or snapshot changes — tracked in #274, #275.
- Any CLI command or WASM binding changes.
- `wasm-pack build` (not needed for wasm32 build verification — see ADR-013 Neutral).

---

## Phases

### Phase 1: Wire zip dependency and stub archive module

**Goal:** `srs-repository` declares `zip` as a workspace-clean, pure-Rust dependency; the
`archive.rs` stub imports it; native and wasm32 builds are green.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `Cargo.toml` (workspace root), add under `[workspace.dependencies]`:
  ```toml
  zip = { version = "2", default-features = false, features = ["deflate"] }
  ```
- [x] In `crates/srs-repository/Cargo.toml`, add under `[dependencies]`:
  ```toml
  zip = { workspace = true }
  ```
- [x] Create `crates/srs-repository/src/archive.rs` with `pub(crate) use zip::ZipArchive;`
  (read-side only; ZipWriter deferred to #276 per architecture review).
- [x] In `crates/srs-repository/src/lib.rs`, add `pub mod archive;` in alphabetical
  order with the existing module declarations (between `pub mod analysis;` and
  `pub mod blueprint_brief_service;`).
- [x] Run `cargo build --all` and confirm it succeeds with no errors.
- [x] Run `cargo build --target wasm32-unknown-unknown -p srs-bindings` and confirm it succeeds.

#### Acceptance Criteria

- [x] `zip = { version = "2", default-features = false, features = ["deflate"] }` is present in
  the workspace `Cargo.toml` `[workspace.dependencies]` section.
- [x] `zip = { workspace = true }` is present in `crates/srs-repository/Cargo.toml`.
- [x] `crates/srs-repository/src/archive.rs` exists and re-exports `ZipArchive` (read-side only).
- [x] `pub mod archive;` is declared in `crates/srs-repository/src/lib.rs`.
- [x] `cargo build --all` passes with zero errors.
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` passes with zero errors.
- [x] `cargo clippy -- -D warnings` passes.
- [x] No C system libraries (`zstd-sys`, `bzip2-sys`, `libz-sys`) appear in `cargo tree -p srs-repository`.

#### Testing

```bash
cargo build --all
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo clippy -- -D warnings
cargo test -p srs-repository
cargo tree -p srs-repository | grep -E "zstd-sys|bzip2-sys|libz-sys"
```

The `cargo tree` grep must produce **no output** (no C-backed compression libs in the
srs-repository dependency tree).

Specific tests to write or verify:

- No new tests are required: the acceptance criterion is a successful wasm32 build, not a
  runtime behaviour change. The CI `wasm-build` job (ADR-013) is the test.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
   ```bash
   cargo build --all
   cargo build --target wasm32-unknown-unknown -p srs-bindings
   cargo clippy -- -D warnings
   cargo test -p srs-repository
   cargo tree -p srs-repository | grep -E "zstd-sys|bzip2-sys|libz-sys"
   ```
   All must pass; `cargo tree` grep must produce no output.
3. Update plan checkboxes `[x]`.
4. Commit referencing the issue.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` passes
- [ ] `cargo tree -p srs-repository | grep -E "zstd-sys|bzip2-sys|libz-sys"` produces no output

## Coordination Rules

- Repository Service Worker owns all file edits.
- Lead Integrator verifies build outputs and signs off.
- Verification Agent confirms no C transitive deps and both build targets pass.

## Assumptions

- The wasm32-unknown-unknown target is available in the CI toolchain (it is: ADR-013 CI job
  already installs it via `dtolnay/rust-toolchain@stable` with `targets: wasm32-unknown-unknown`).
- `zip` v2 with `default-features = false, features = ["deflate"]` pulls only
  `flate2` (with its `miniz_oxide` pure-Rust backend) — verified by `cargo tree` grep.
- No existing `srs-repository` code uses zip; this is a new, currently-unused module stub.
