# ADR-011: CLI Output Contract

- **Status:** accepted
- **Date:** 2026-05-31
- **Supersedes:** —
- **Superseded by:** —

## Context

ADR-010 established that `srs-repository` service functions have typed input and output structs, making the service boundary explicit and testable. The CLI layer, however, continued to assemble its JSON output via anonymous `json!({...})` literals directly in command handler functions.

This created a second invisible boundary: the shape of `payload` in the CLI output envelope was defined only by the runtime behavior of `json!()` macros scattered across 16+ handler files. No Rust type captured "the note list response has a `notes` key containing objects with `instanceId` and `title`." The VS Code extension maintained hand-written TypeScript interfaces in `src/cli/types.ts` that were manually kept in sync, with no CI check to detect drift.

Additionally, the entity JSON schemas under `srs/docs/schema/2.0/` were mirrored to both `srs-rust/crates/srs-schema/schemas/2.0/` and `srs-vscode/schemas/2.0/`, but `scripts/check-schema-sync.sh` only verified the first copy.

## Decision

### Payload structs as the CLI contract

Every CLI command output is produced by a named Rust struct defined in `crates/srs-cli/src/payload.rs`. No `json!({...})` literals appear in command handler functions. Handlers call `output::serialize(command, TypedPayload { ... })`.

The module has one public struct per logical command output shape. Where a handler previously serialized only a subset of a service struct's fields (e.g., note list emits only `instanceId` and `title` from `NoteSummary`), a dedicated sub-struct (e.g., `NoteListEntry`) is defined rather than re-exporting the full service type.

External service types embedded in payload structs (e.g., `Note`, `Record`, `Relation`) are serialized as `serde_json::Value` in the JSON Schema to avoid coupling `schemars` into `srs-core` or `srs-repository`.

### JSON Schema golden files

`crates/srs-cli/src/payload.rs` derives `schemars::JsonSchema` on all payload structs. A `generate-schemas` binary writes one JSON Schema file per payload type to `crates/srs-cli/schemas/payload/`. These files are committed to the repository.

A golden-file test in `crates/srs-cli/tests/payload_contracts.rs` regenerates each schema at test time and compares it to the committed file. A mismatch fails `cargo test`, making any payload struct change explicit in the PR diff and requiring the developer to run `cargo run --bin generate-schemas` and commit the updated schema.

### Cross-language validation

`srs-vscode/test/suite/payload-contracts.test.ts` uses AJV to validate fixture CLI output payloads (in `test/fixtures/envelopes.ts`) against the golden JSON Schema files from `srs-rust/`. If the repos are not co-located the tests skip with a warning. This catches TypeScript fixture drift against the Rust-defined contract in `srs-vscode` CI without spawning the CLI binary.

### Entity schema sync coverage

`scripts/check-schema-sync.sh` is extended to compare `srs-vscode/schemas/2.0/` against the canonical `srs/docs/schema/2.0/` in addition to `srs-rust/crates/srs-schema/schemas/2.0/`. If the vscode schema directory is absent the script warns and continues; it does not fail for non-monorepo environments.

## Consequences

**Positive:**
- Any CLI payload field rename, addition, or removal is visible as a diff in the committed JSON Schema file, not hidden inside a `json!()` literal.
- `cargo test` enforces the contract without running the CLI binary; CI catches regressions before they reach integration tests.
- TypeScript developers can read `schemas/payload/<command>.json` to understand the exact payload shape rather than reading Rust source.
- The pre-commit hook runs `cargo test --test payload_contracts` to catch golden schema drift before commits.

**Negative / trade-offs:**
- External types embedded in payload structs (e.g., `Note`, `Record`) appear as `{}` (any JSON) in the golden schemas. Changes to those internal fields are not caught by the golden schema test — they remain covered by the existing integration tests that run the binary and parse output.
- Two type aliases (`NoteTagListPayload`, `RepoValidatePayload`) required conversion to proper structs and `From` impls to enable `JsonSchema` derivation. Future type aliases in `payload.rs` must follow this pattern.
- The `generate-schemas` binary must be run and its output committed whenever payload structs change. This is enforced by the golden-file test but requires a two-step workflow (edit struct → run binary → commit schema).

**Neutral:**
- The actual CLI JSON output is unchanged. This ADR formalizes the existing shapes; it does not alter them.
- TypeScript types in `srs-vscode/src/cli/types.ts` remain hand-written. Code generation (json-schema-to-typescript) is deferred; manual types are safe because CI now validates fixtures against the golden schemas.
- `schemars = "0.8"` is added to the workspace and to `srs-cli` only. `srs-core` and `srs-repository` do not gain a `schemars` dependency.
