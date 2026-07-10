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

No new architectural decisions — this plan fixes a bug already resolved by the RFC-014 migration work. ADR-010 and ADR-011 govern the CLI/service boundary. No ADR exists specifically for the migration service pattern; that is convention, not a formal decision record.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service owns validation; CLI is thin | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Every CLI output is a named struct in payload.rs | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `srs-gov repo-create` output shape is unchanged. No action required; golden schemas stay as-is.

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/` — the RFC-014 Rev 4 schema sync (commit `d6989c6`) already landed. No action required.

---

## Scope

- Add a regression test in `crates/srs-gov/tests/flow.rs` that specifically names issue #428 and asserts `contentHash` is absent from `upstreamPackage` in the output manifest.
- Verify the existing `repo_create_produces_valid_srsj` test covers schema validation (`srs repo validate` returns 0 errors).
- Verify `cargo test -p srs-gov` passes.
- Open a PR with `Closes #428`.

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

- [ ] Read `crates/srs-gov/tests/flow.rs` to understand the existing test structure around `repo_create_produces_valid_srsj`.
- [ ] Add a new test `repo_create_manifest_no_content_hash_regression_428` in `crates/srs-gov/tests/flow.rs` that:
  - Creates a temp directory.
  - Runs `srs-gov repo-create` (via the binary under test, matching the pattern of the existing flow tests).
  - Reads the output `.srsj` file and deserializes the manifest.
  - Asserts `upstreamPackage` is present at the top level of the manifest.
  - Asserts `upstreamPackage.contentHash` is absent (is `null` or the key does not exist).
  - Asserts schema validation passes: `srs repo validate` returns `errors == 0`.
- [ ] Run `cargo test -p srs-gov` — all tests must pass (expect 18 passing, up from 17).
- [ ] Run `cargo clippy -p srs-gov -- -D warnings` — no warnings.

#### Acceptance Criteria

- [ ] `repo_create_manifest_no_content_hash_regression_428` test exists in `crates/srs-gov/tests/flow.rs`.
- [ ] Test asserts both the absence of `contentHash` and the presence of schema-valid `upstreamPackage`.
- [ ] `cargo test -p srs-gov` passes with 0 failures.
- [ ] `cargo clippy -p srs-gov -- -D warnings` exits 0.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests to write or verify:

- `repo_create_manifest_no_content_hash_regression_428` — proves `contentHash` cannot re-appear in `upstreamPackage` silently
- `repo_create_produces_valid_srsj` — existing test confirming `srs repo validate` passes; must still pass

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm both tests listed in Testing exist and pass.
3. Run:

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit -m "test(srs-gov): add regression test for issue #428 contentHash in upstreamPackage (#428)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `repo_create_manifest_no_content_hash_regression_428` test exists and passes in `crates/srs-gov/tests/flow.rs`
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
