# Plan: Fix governance scaffold title-field ambiguity and re-vendor seed

## Summary

`Package::find_field_by_name("title")` in `governance_scaffold_service` was ambiguous once the
implicit-core merge (ADR-025) started surfacing `com.semanticops.core/title` alongside
`governance/title`. PR #505 already resolved the Rust service by switching to the
namespace-qualified `Package::find_field("governance", "title")`, but a secondary artefact drifted:
the canonical governance seed (`srs/packages/com.mudemocracy.governance/1.0.0/seed/`) and its
vendored copies in `srs-rust` were generated with a binary that pre-dates RFC-022 (`requiresRelation`
on lifecycle states, `abandoned` state). This plan re-vendors the seed and updates the one test
that hard-coded the old excluded-state list.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude |
| Repository Service Worker | claude |
| Verification | claude |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan re-vendors an artefact under ADR-017 (deterministic SRSJ
serialisation) and corrects a seed build script bug. The namespace-qualified field lookup is already
established by PR #505 / ADR-025.

| ADR | Decision | Status |
|---|---|---|
| [ADR-017](../docs/adr/017-deterministic-srsj-serialisation.md) | Vendored seeds are byte-copy artefacts; changes need an explicit re-vendor commit | accepted |
| [ADR-025](../docs/adr/025-implicit-core-package-merge.md) | Core-package fields merge into every `load_package()`; callers must use namespace-qualified lookups | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands — no action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No entity schemas changed — no action required.

---

## Scope

- Fix `build-governance-seed.mjs` (srs/) to respect the `SRS_BIN` environment variable instead of
  hard-coding `srs`.
- Regenerate the canonical governance seed in `srs/packages/com.mudemocracy.governance/1.0.0/seed/`
  using the current CLI binary.
- Update the vendored seed in `srs-rust/crates/srs-gov/assets/governance-seed.srsj`.
- Update the test fixture in `srs-rust/crates/srs-repository/tests/fixtures/governance-seed.srsj`.
- Update the unit test in `crates/srs-gov/src/main.rs` that asserted the old excluded-state list
  `["superseded", "closed"]` — correct value is now `["superseded", "closed", "abandoned"]`.

**Out of scope:**

- Changes to `governance_scaffold_service` — the namespace-qualified field lookup is already correct.
- Introducing `find_field_in_type_by_name` — unnecessary given the existing `find_field` API.
- Any changes to srs-vscode schema mirrors (pre-existing drift, out of scope for this PR).

---

## Phases

### Phase 1: Seed regeneration (srs/ cross-repo)

**Goal:** The canonical seed in `srs/` is regenerated with the current CLI and the build script
respects `SRS_BIN`.

**Agent:** Repository Service Worker

#### Tasks

- [x] Fix `build-governance-seed.mjs` to use `process.env.SRS_BIN || 'srs'` instead of hardcoded `'srs'`.
- [x] Run `node scripts/build-governance-seed.mjs` to regenerate the canonical seed.
- [x] Run `node scripts/build-governance-seed.mjs --check` to confirm the seed is stable.
- [x] Commit and push in `srs/` on branch `fix/183-regenerate-governance-seed`; open PR #184 (closes #183).

#### Acceptance Criteria

- [x] `build-governance-seed.mjs --check` exits 0.
- [x] The canonical seed includes `requiresRelation` on the `superseded` lifecycle state.
- [x] The canonical seed includes `abandoned` as a lifecycle state.

#### Milestone gate

srs/ PR #184 open and branch pushed.

---

### Phase 2: Re-vendor in srs-rust

**Goal:** The srs-rust vendored seed and test fixture match the canonical seed from Phase 1.

**Agent:** Repository Service Worker

#### Tasks

- [x] Copy new canonical seed to `crates/srs-gov/assets/governance-seed.srsj`.
- [x] Copy new canonical seed to `crates/srs-repository/tests/fixtures/governance-seed.srsj`.
- [x] Update unit test `seed_decision_log_view_is_type_query_with_excludes` in
  `crates/srs-gov/src/main.rs:664` to assert `["superseded", "closed", "abandoned"]`.

#### Acceptance Criteria

- [x] `cargo test -p srs-gov --test flow` — 18/18 pass.
- [x] `cargo test -p srs-repository` — `scaffold_from_raw_seed_produces_valid_repository` passes.
- [x] `cargo test` (full workspace) — 0 failures.
- [x] `cargo clippy -- -D warnings` — clean.
- [x] `cargo test --test payload_contracts` — 104/104 pass.
- [x] `bash scripts/check-schema-drift.sh ../srs` — "No schema drift detected."

#### Testing

```bash
cargo test -p srs-gov --test flow
cargo test -p srs-repository
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
bash scripts/check-schema-drift.sh ../srs
```

#### Milestone gate

All tests pass; commit on `feat/549-scaffold-title-field-ambiguous`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes
- [x] `bash scripts/check-schema-drift.sh ../srs` exits 0

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.

## Assumptions

- The srs/ PR #184 carries the canonical seed regeneration; srs-rust carries only the vendored copy update.
- No new ADRs needed — this is a maintenance re-vendor under ADR-017.
