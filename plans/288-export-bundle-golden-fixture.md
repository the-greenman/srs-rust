# Plan: Export Bundle Golden Fixture (#288)

## Summary

`export_record_bundle` was shipped in PR #625 (ADR-035: flat ZIP format). The service itself is tested by unit tests in `export_service.rs` (no-attachment, with-attachment, cross-store roundtrip). The remaining scope of issue #288 is a byte-stable golden fixture test that pins the ZIP layout and asserts the determinism invariant stated in ADR-035 — "same record + attachments → identical bytes across runs."

**Architecture review update (Stage 3/Stage 7):** The initial draft followed `tests/archive_golden.rs` and used `FileStore` in an integration test. The architecture reviewer found a blocking violation of the Storage Boundary Rules ("MemoryStore is the canonical test double — tests that only work against FileStore are testing the adapter, not the service"). The implementation was refactored to use `MemoryStore` inside `#[cfg(test)] mod tests` in `export_service.rs`. `archive_golden.rs` uses `FileStore` because `archive_pack` reads raw file bytes — a different contract. `export_record_bundle` is a service function and should be tested with `MemoryStore`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-035](../docs/adr/035-flat-export-bundle-format.md) | Flat ZIP format; same record + attachments → identical bytes | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | ZIP determinism pattern (DateTime::default, Deflate, sorted entries) — mirrored by export_service | accepted |

**Storage boundary decision (not a new ADR — it follows existing CLAUDE.md rule):** Golden fixture tests for `export_record_bundle` must use `MemoryStore`, not `FileStore`. `FileStore` is the right adapter for `archive_pack` (reads raw bytes from disk) but not for service functions whose contract is store-agnostic.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. No payload struct changes. No `generate-schemas` run needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files added or modified. No schema sync needed.

---

## Scope

- New tests in `crates/srs-repository/src/export_service.rs` (inside `#[cfg(test)] mod tests`):
  - `canonical_golden_store()` — MemoryStore with static-preamble DocumentView (no `{{...}}` variables) and TypeQuery pointing to a non-existent type with `emptyBehavior: hide`
  - `export_bundle_bytes()` — calls `export_record_bundle` on the canonical store
  - `golden_bundle_path()` — path to `tests/fixtures/golden-export-bundle.zip`
  - `test_export_bundle_golden_fixture` — byte-stable golden comparison (REGENERATE_GOLDEN=1 to refresh)
  - `test_export_bundle_determinism` — within-process stability; cross-run guard is the golden fixture
  - `test_export_bundle_zip_contents` — structural check: 1 entry named `decision.md`, starts with `# Golden Export Bundle`
  - `test_export_bundle_determinism_shared_basenames` — covers the collision-resolution branch (same basename → `_<id_prefix>` suffix); asserts byte-identical output (ADR-035 for the attachment collision path)
- Generated golden fixture: `crates/srs-repository/tests/fixtures/golden-export-bundle.zip`

**Out of scope:**
- Changes to production code in `export_service.rs`
- New payload structs or CLI commands
- srs-bindings or srs-cli changes

---

## Phases

### Phase 1: Write tests + generate golden fixture

**Goal:** All new export_bundle tests pass; clippy clean.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add golden-fixture tests to `#[cfg(test)] mod tests` in `export_service.rs` using `MemoryStore`
- [x] `canonical_golden_store()` uses static preamble (`"# Golden Export Bundle"`, no `{{...}}` vars) and TypeQuery with `emptyBehavior: hide` for determinism
- [x] Run `REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture` to generate `tests/fixtures/golden-export-bundle.zip`
- [x] Run `cargo test -p srs-repository -- export_bundle` — all tests pass
- [x] Run `cargo clippy -p srs-repository -- -D warnings` — clean

#### Acceptance Criteria

- [x] `test_export_bundle_golden_fixture` exists in `export_service.rs` and passes
- [x] `test_export_bundle_determinism` exists and passes
- [x] `test_export_bundle_zip_contents` exists and passes
- [x] `test_export_bundle_determinism_shared_basenames` exists and passes (collision-resolution path)
- [x] `tests/fixtures/golden-export-bundle.zip` exists and is committed
- [x] No regression in `cargo test -p srs-repository`
- [x] `cargo clippy -p srs-repository -- -D warnings` passes clean

#### Testing

```bash
REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture
cargo test -p srs-repository -- export_bundle
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `test_export_bundle_golden_fixture` exists with golden byte comparison
- [x] `test_export_bundle_determinism` exists with within-process stability check
- [x] `tests/fixtures/golden-export-bundle.zip` is committed
- [x] All pre-existing export_service unit tests still pass (6 → 7 with new tests)

## Assumptions

- No new architectural decisions needed — this purely adds tests that validate ADR-035's determinism claim.
- The canonical store uses no-attachment to keep the rendered `decision.md` content fully static (TypeQuery → EmptyBehavior::Hide → only preamble in file). If a future change modifies preamble rendering or ZIP format, `REGENERATE_GOLDEN=1` regenerates the fixture and the diff is the explicit change record.
- `test_export_bundle_determinism` guards within-process stability (same process, same SipHash seed). Cross-run determinism is guarded by `test_export_bundle_golden_fixture` (compares against a fixture written by a prior process).
