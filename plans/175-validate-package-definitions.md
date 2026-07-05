# Plan: Validate Package Definitions (Protocols + Blueprints) in `repo validate`

> **Issue:** [srs-rust#175](https://github.com/the-greenman/srs-rust/issues/175)

## Summary

`srs repo validate` skips all validation of protocol and blueprint definition files — a broken
stage graph or malformed blueprint passes `repo validate` today; only the standalone
`srs protocol validate` / `srs blueprint validate` commands catch it. This plan wires
schema-validation (for blueprints) and semantic-validation (for both) into
`validate_repository`, making `repo validate` the authoritative single gate. It also registers
the `blueprint.json` entity schema in the `srs-schema` registry, completing the missing step
from srs-rust#174.

**No spec change required.** Blueprint and protocol schemas and semantics are already specified;
this is purely a Rust implementation change.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-004 | `BLUEPRINT_SCHEMA_ID` registered in `srs-schema` via the `include_schema!` macro; no runtime schema loading | accepted |
| ADR-009 | Blueprint and protocol paths exposed on `PackageBoundary` struct (not reconstructed by path-string computation in service logic) | accepted |
| ADR-010 | Validation logic lives in `srs-repository/src/validation.rs`, not in the CLI handler | accepted |
| ADR-016 | Protocols are package definitions (`package.json` `protocols[]`); iterate boundaries to find them | accepted |

## Scope

- Add `BLUEPRINT_SCHEMA_ID` to `crates/srs-schema/src/lib.rs`, register in `SCHEMA_SOURCES` and `ALL_SCHEMA_IDS`.
- Extend `PackageBoundary` in `crates/srs-repository/src/package_types.rs` with `blueprint_paths: Vec<String>` and `protocol_paths: Vec<String>`.
- Populate those fields in `crates/srs-repository/src/store.rs` inside `file_store_boundary_from_json`.
- Update `MemoryStore::new()` to initialize `blueprint_paths: vec![]` and `protocol_paths: vec![]`.
- In `validate_repository`: wrap new block in `if let Ok(boundaries) = store.list_package_boundaries()` (silent-skip for repos without a package); use `boundary.blueprint_paths`/`boundary.protocol_paths` directly (ADR-009).
- Blueprint validation: schema (`BLUEPRINT_SCHEMA_ID`) + semantic, both Error and Warning severities forwarded.
- Protocol validation: semantic only, both severities forwarded.
- Six new tests: five TempDir/FileStore + one MemoryStore (CLAUDE.md cross-store requirement).

**Out of scope:** `protocol.json` schema, `srs-cli`, `srs-bindings`, `payload.rs`, schema golden files, `srs blueprint validate` / `srs protocol validate` commands.

## Phases

### Phase 1: Register `blueprint.json` in `srs-schema`
File: `crates/srs-schema/src/lib.rs`
- Add `BLUEPRINT_SCHEMA_ID` constant, add to `ALL_SCHEMA_IDS` and `SCHEMA_SOURCES`
- Update count test 21→22
- Add `valid_blueprint_passes` test

### Phase 2: Extend `PackageBoundary` and add validation in `validate_repository`
Files: `crates/srs-repository/src/package_types.rs`, `store.rs`, `validation.rs`
- Add `blueprint_paths`/`protocol_paths` to `PackageBoundary`; populate in `file_store_boundary_from_json`
- Update `MemoryStore::new()` primary boundary initialization
- Add definitions validation block (wrapped in `if let Ok(...)`)
- Six new tests

## Final Acceptance

- `cargo test` passes with no failures
- `cargo clippy -- -D warnings` passes
- `cargo test --test payload_contracts` passes
- `bash scripts/check-schema-sync.sh` exits 0
- `valid_repo_reports_no_errors` passes (no regression for note-only repos)
- Blueprint schema error → ERROR diagnostic; blueprint semantic error → ERROR diagnostic
- Protocol cycle → ERROR diagnostic; valid blueprint/protocol → 0 diagnostics
- `srs repo validate --repo /home/user/srs/srs` exits 0 with 0 errors
- `srs repo validate --repo /home/user/srs/docs/spec/examples/gallery-project-v2` exits 0 with 0 errors
