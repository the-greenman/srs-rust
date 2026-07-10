# Plan: Close #428 — srs-gov repo-create manifest contentHash regression

## Summary

Issue #428 was filed when `srs-gov repo-create` produced a manifest with `contentHash` inside `upstreamPackage`, which was rejected by the updated schema (RFC-014 Rev 4 added `additionalProperties: false` to `UpstreamPackage`). Investigation shows the root fix was applied as a side-effect of PR #222: commit `d6989c6` synced the schema mirror and commit `c67a06a` updated `migrate_rfc014` to strip `contentHash` before promoting `upstreamPackage` to the top-level manifest position. Neither commit referenced `Closes #428`, leaving the issue open despite the fix being in `main`. This plan adds a targeted regression test to prevent silent re-introduction, verifies the full pipeline, and opens a PR to formally close the issue.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude |
| Repository Worker | claude |
| Verification | claude |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. ADR-013 has a stale description of the RFC-014 migration that this plan corrects in-place (the stale text says "adds `contentHash`"; the implementation strips it). ADR-010 and ADR-011 govern the CLI/service boundary.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service owns validation; CLI is thin | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Every CLI output is a named struct in payload.rs | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `.srsj` bundle format; `srsj_migration_service::load_from_srsj` is the WASM entry point | accepted (stale description corrected by this plan) |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `srs-gov repo-create` output shape is unchanged. No action required; golden schemas stay as-is.

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/` — the RFC-014 Rev 4 schema sync (commit `d6989c6`) already landed. No action required.

---

## Scope

- Add a `// Regression for #428` comment on the existing `contentHash` absence assertion in `repo_create_produces_valid_srsj` (`crates/srs-gov/tests/flow.rs` lines 329–332), making the regression traceability explicit without duplicating coverage.
- Correct the stale description in `docs/adr/013-wasm-binding-strategy.md` (§ "No-filesystem entry point"): change "and adds `contentHash`" to "and strips `contentHash` if present (removed from the spec schema)."
- Verify `cargo test -p srs-gov` passes.

Note: PR opening is handled by the delivery pipeline (Stage 8), not a plan phase. Phase 1 is the only implementation phase.

**Out of scope:**
- RFC-018 `identityInstanceId` type warning (intentional migration-period grace; tracked by #426).
- Any changes to the governance scaffold types or `create_governance_repository` logic.
- Any changes to `migrate_rfc014` beyond what already landed.

---

## Phases

### Phase 1: Regression test + verification

**Goal:** A named regression test for #428 exists and passes alongside all existing srs-gov tests.

**Agent:** Repository Worker

#### Tasks

- [x] In `crates/srs-gov/tests/flow.rs` at lines 329–332, add `// Regression for #428: contentHash must be absent from upstreamPackage` above the existing `assert!(content["manifest"]["upstreamPackage"]["contentHash"].is_null(), ...)` assertion.
- [x] In `docs/adr/013-wasm-binding-strategy.md` (§ "No-filesystem entry point"), change the description from "moves `manifest.meta.upstreamPackage` to top-level and adds `contentHash`" to "moves `manifest.meta.upstreamPackage` to top-level and strips `contentHash` if present (removed from the spec schema in RFC-014 Rev 4)."
- [x] Run `cargo test -p srs-gov` — all tests must pass.
- [x] Run `cargo clippy -p srs-gov -- -D warnings` — no warnings.

#### Acceptance Criteria

- [x] `// Regression for #428` comment appears on the `contentHash.is_null()` assertion in `repo_create_produces_valid_srsj`.
- [x] ADR-013 § "No-filesystem entry point" no longer says "adds `contentHash`".
- [x] `cargo test -p srs-gov` passes with 0 failures.
- [x] `cargo clippy -p srs-gov -- -D warnings` exits 0.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests to verify:

- `repo_create_produces_valid_srsj` — existing test asserting `contentHash` absent and `srs repo validate` returns 0 errors; must still pass

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm `repo_create_produces_valid_srsj` passes.
3. Run:

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit -m "fix(srs-gov): annotate regression for #428 and correct stale ADR-013 description (#428)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `// Regression for #428` comment present on the `contentHash.is_null()` assertion in `repo_create_produces_valid_srsj`
- [ ] ADR-013 stale description corrected
- [ ] `srs-gov repo-create` + `srs repo validate` returns 0 errors in dogfood run

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Commits `d6989c6` and `c67a06a` are on `main` and fully address the root cause.
- The RFC-018 validation warning (1 warning from `srs repo validate`) is intentional and not a regression of this issue.
- `srs` binary is available on PATH in test environments for integration tests that call `srs repo validate`.
