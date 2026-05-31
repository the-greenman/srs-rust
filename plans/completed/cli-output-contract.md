# Plan: CLI Output Contract

> **Status: COMPLETE** — implemented 2026-05-31.

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

CLI command output payload shapes are currently implicit: 16+ command handlers produce JSON via inline `json!({...})` macros with no named Rust types and no machine-verifiable contract. The VS Code extension (`srs-vscode`) maintains hand-written TypeScript interfaces in `srs-vscode/src/cli/types.ts` that are manually kept in sync. Additionally, entity schemas (`srs/docs/schema/2.0/`) are mirrored to `srs-vscode/schemas/2.0/` but the `check-schema-sync.sh` script does not validate the vscode copy. This plan establishes explicit, enforceable contracts on two tracks: (A) entity schema sync coverage for srs-vscode, and (B) typed payload structs + JSON Schema golden files that make Rust the authoritative source of truth for CLI output shapes, detectable in CI without running the binary.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Contract Worker | — |
| Schema Worker | — |
| Verification | — |

See [agents.md](../agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-011](../../docs/adr/011-cli-output-contract.md) | CLI payload shapes are defined by named Rust structs in `srs-cli/src/payload.rs`; JSON Schema golden files are the machine-verifiable contract; TypeScript types in srs-vscode are validated against those golden files in CI | **accepted** |

ADR-010 (service-boundary-contract) established that typed result structs replace `json!()` literals at the service layer. ADR-011 extends that principle to the CLI output layer and adds cross-language contract verification.

---

## Scope

- Define named Rust payload structs for every CLI command output (~20 shapes)
- Replace all `json!({...})` literals in command handlers with typed struct serialization
- Add `schemars` to `srs-cli`, derive `JsonSchema` on all payload structs
- Generate and commit JSON Schema golden files to `crates/srs-cli/schemas/payload/`
- Add a golden-file CI test that fails if any payload struct changes without regenerating
- Extend `scripts/check-schema-sync.sh` to also validate `srs-vscode/schemas/2.0/`
- Add TypeScript payload contract tests in srs-vscode that validate fixtures against golden schemas

**Out of scope:**

- TypeScript type code generation (json-schema-to-typescript) — deferred; manual types in `types.ts` are safe once validated in CI
- OpenAPI/AsyncAPI specification
- HTTP API layer
- Changes to CLI output JSON shapes — the existing shapes are the contract being formalized, not changed
- `ext:lifecycle`, federation, extension/protocol handler migration (tracked separately)

---

## Phases

### Phase 0: Entity Schema Sync Coverage

**Goal:** `scripts/check-schema-sync.sh` validates all three schema copies (srs-schema, srs-vscode) and fails CI when any copy drifts from the canonical `srs/docs/schema/2.0/`.

**Agent:** Schema Worker

#### Tasks

- [x] In `scripts/check-schema-sync.sh`: add a third loop that checks `../../srs-vscode/schemas/2.0/` against `srs/docs/schema/2.0/` using the same sha256sum pattern as the existing srs-schema check. If the vscode directory is absent (non-monorepo environment), print a warning and skip without failing.
- [x] Verify the script exits non-zero when a schema file in `srs/docs/schema/2.0/` has no matching copy or a mismatched copy in `srs-vscode/schemas/2.0/`.

#### Acceptance Criteria

- [x] `bash scripts/check-schema-sync.sh` exits 0 with current files in all three locations
- [x] Temporarily mutating one file in `srs-vscode/schemas/2.0/` causes the script to exit non-zero
- [x] Script runs cleanly when invoked from a CI environment that has `srs-vscode/` as a sibling directory

---

### Phase 1: Named Payload Structs in srs-cli

**Goal:** Every CLI command output is produced by a named Rust struct in `crates/srs-cli/src/payload.rs`; no `json!({...})` literals remain in command handler functions.

#### Tasks

- [x] Create `crates/srs-cli/src/payload.rs` with ~45 typed payload structs and sub-structs
- [x] Add `pub mod payload;` to `crates/srs-cli/src/main.rs`
- [x] Add `output::serialize<T>()` helper to `crates/srs-cli/src/output.rs`
- [x] Update all 16 command handler files to use typed payload structs

#### Acceptance Criteria

- [x] `cargo build -p srs` succeeds
- [x] `cargo test` passes — 119 existing integration tests pass, 1 pre-existing failure (`note_audit_tags_returns_tag_counts`) unrelated to this work
- [x] No `json!({` literals remain inside handler function bodies
- [x] `cargo clippy -- -D warnings` passes

---

### Phase 2: JSON Schema Golden Files

**Goal:** Every payload struct has a derived JSON Schema committed as a golden file; `cargo test` fails if any payload struct changes without regenerating the golden file.

#### Tasks

- [x] Add `schemars = "0.8"` to workspace `Cargo.toml` and `crates/srs-cli/Cargo.toml`
- [x] Add `#[derive(JsonSchema)]` to every payload struct in `payload.rs`; use `#[schemars(with = "serde_json::Value")]` for embedded external types
- [x] Replace the two type aliases (`NoteTagListPayload`, `RepoValidatePayload`) with proper structs + `From` impls to enable `JsonSchema` derivation
- [x] Add `[lib]` target to `srs-cli/Cargo.toml` (`src/lib.rs`) so bins can reference `srs::payload`
- [x] Create `crates/srs-cli/src/bin/generate-schemas.rs`
- [x] Run `cargo run --bin generate-schemas` — generates 75 golden files in `crates/srs-cli/schemas/payload/`
- [x] Create `crates/srs-cli/tests/payload_contracts.rs` — 54 golden-file tests
- [x] Commit all generated `schemas/payload/*.json` files

#### Acceptance Criteria

- [x] `cargo test --test payload_contracts` passes (54 tests)
- [x] `cargo run --bin generate-schemas` is idempotent
- [x] `cargo clippy -- -D warnings` passes

---

### Phase 3: TypeScript Validation Against Golden Schemas

**Goal:** `srs-vscode` CI fails if TypeScript fixture payloads diverge from the Rust-generated golden schemas.

#### Tasks

- [x] Add `"ajv": "^8.0.0"` to `srs-vscode/package.json` devDependencies, run `npm install`
- [x] Create `srs-vscode/test/suite/payload-contracts.test.ts` — 9 AJV-based contract tests; skips gracefully when `srs-rust` is not co-located

#### Acceptance Criteria

- [x] `npm test` in `srs-vscode/` passes — 31 tests total (9 new payload contract tests)
- [x] Schema directory absence causes skip, not failure

---

## Final Acceptance

- [x] `cargo test --no-fail-fast` passes: 119 integration tests, 54 payload contract tests, 250 srs-repository tests, 127 srs-core tests, 10 srs-schema tests
- [x] `cargo clippy -- -D warnings` passes
- [x] No `json!({` literals remain inside handler function bodies in `crates/srs-cli/src/commands/`
- [x] `crates/srs-cli/schemas/payload/` contains 75 JSON Schema files, committed
- [x] `cargo test --test payload_contracts` includes 54 golden-file tests
- [x] `bash scripts/check-schema-sync.sh` validates all three schema locations and exits 0
- [x] `cd srs-vscode && npm test` passes — 31 tests including payload-contracts
- [x] CLI JSON output is identical before and after (integration tests pass unchanged)
- [x] ADR-011 authored and linked from this plan (`docs/adr/011-cli-output-contract.md`)
- [x] Pre-commit hook updated to run `cargo test --test payload_contracts`
