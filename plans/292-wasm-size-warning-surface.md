# Plan: 4.3 srs-bindings: expose size-warning diagnostics through WASM validate (#292)

## Summary

`validation::validate_repository` already emits `severity: "warning"` diagnostics for RFC-017 I-107 attachment-size violations when an `attachment_policy` (`com.semanticops.base/repo_settings`) record is present. The WASM `validate()` method already serialises the full `RepositoryValidationReport` — including `diagnostics` and `summary.warnings`. The gap is that no test in `srs-bindings` proves this path works, and the `validate()` docstring does not document the warning surface for callers. This plan closes that gap with a native binding-layer test and a docstring update.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude-code |
| Bindings Worker | claude-code |
| Verification | claude-code |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Native tests prove service functions; `wasm32` build gate proves binding layer compiles | accepted |
| [ADR-031](../docs/adr/031-source-document-blob-portability.md) | Test fixture uses `save_binary_file`/`save_text_file` per ADR-031 Amendment (#291); no new decisions needed | reviewed |

No new ADRs needed. The existing pattern (comment + service-level test + wasm32 build gate) governs. No new WASM method is introduced; the existing `validate()` already returns warnings.

Note: `pub mod memory` in `srs-repository` is `#[cfg(test)]`-gated and inaccessible from `srs-bindings`. The test uses `JsonStore::from_srsj()` (consistent with established srs-bindings test style) and adds binary content via the public `RepositoryStore::save_binary_file` trait method.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are added or modified. No action required.

---

## Scope

- Add a native test in `crates/srs-bindings/src/lib.rs` that calls `validation::validate_repository` against a `JsonStore` carrying an attachment policy record and an oversized file; asserts `summary.warnings > 0`, `summary.errors == 0`, `is_ok() == true`, and a `Warning`-severity diagnostic is present.
- Update the `validate()` docstring in `crates/srs-bindings/src/lib.rs` to document that `diagnostics` entries with `severity: "warning"` represent RFC-017 I-107 soft size-limit violations, and `summary.warnings` counts them.

**Out of scope:**
- Adding a new dedicated WASM method for size warnings (the existing `validate()` surface is sufficient; srs-web reads `diagnostics` by severity).
- Any changes to `srs-repository` validation logic (already complete in #284).
- Any CLI changes.

---

## Phases

### Phase 1: Test + docstring

**Goal:** The binding-layer test passes, the docstring accurately describes the warning surface, and all tests and clippy pass.

**Agent:** Bindings Worker

#### Tasks

- [x] In `crates/srs-bindings/src/lib.rs` `#[cfg(test)]` module, add test `validate_size_warning_surfaces_through_report`:
  - Uses `JsonStore::from_srsj()` with a self-contained `.srsj` fixture embedding the synthetic `com.semanticops.base` package (with `maxPerFileBytes` field id `"bb000002-0000-4000-b000-000000000002"`, `repo_settings` type id `"bb000010-0000-4000-b000-000000000010"`) and a policy record with `maxPerFileBytes: 50`. Manifest includes `sourceDocumentIndex` entry.
  - Adds a 200-byte binary content file via `store.save_binary_file(...)` and sidecar via `store.save_text_file(...)`.
  - Calls `validation::validate_repository(&store).expect("validate should not error")`.
  - Asserts `report.summary.errors == 0` (non-blocking).
  - Asserts `report.is_ok() == true` (is_ok checks only errors).
  - Asserts `report.summary.warnings > 0`.
  - Asserts at least one diagnostic has `severity == DiagnosticSeverity::Warning` and message contains `"I-107"`.
- [x] Update the `validate()` docstring in `crates/srs-bindings/src/lib.rs` to explicitly document:
  - Return type is `RepositoryValidationReport` with `diagnostics` (array) and `summary` (`{ checked, errors, warnings }`).
  - `diagnostics` entries with `severity: "warning"` represent RFC-017 I-107 soft size-limit violations from an `attachment_policy` record; they do not affect `is_ok()` or `summary.errors`.
  - `summary.warnings` counts warning-severity diagnostics.
  - Callers should filter `diagnostics` by `severity` to distinguish errors from warnings.

#### Acceptance Criteria

- [x] `cargo test -p srs-bindings validate_size_warning` passes
- [x] `cargo clippy -p srs-bindings -- -D warnings` passes
- [x] `report.summary.errors == 0` and `report.is_ok() == true` asserted in the test (warns are non-blocking)
- [x] `report.summary.warnings > 0` asserted
- [x] At least one `DiagnosticSeverity::Warning` diagnostic with `"I-107"` in message asserted
- [x] `validate()` docstring documents the warning surface (severity, summary.warnings, non-blocking nature)

#### Testing

```bash
cargo test -p srs-bindings validate_size_warning
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests to write or verify:

- `validate_size_warning_surfaces_through_report` — proves size-warning diagnostics reach the caller via `validation::validate_repository` on a `JsonStore`; the wasm32 build gate confirms the `validate()` binding wrapper compiles and links correctly with `to_js(&report)`.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm `validate_size_warning_surfaces_through_report` exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `validate_size_warning_surfaces_through_report` test present and passing
- [ ] `validate()` docstring documents `summary.warnings` and `severity: "warning"` diagnostics

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `JsonStore::from_srsj()` is the correct fixture mechanism for `srs-bindings` tests; `MemoryStore` is `#[cfg(test)]`-gated in `srs-repository` and not accessible cross-crate.
- `RepositoryStore::save_binary_file` and `save_text_file` are public trait methods callable from `srs-bindings`.
- The `wasm32` build gate in CI validates the `to_js(&report)` call in the binding layer; native tests validate the service path only.
