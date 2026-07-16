# Plan: Governance seed identity record must be com.semanticops.core/purpose (issue #568)

## Summary

`scaffold_governance_repo` creates the identity record (the one whose ID lands in `manifest.container.identityInstanceId`) as a `governance/article` Tier-2 record. RFC-018 I-81 requires that `identityInstanceId` resolve to a `com.semanticops.core/purpose` record. Validation emits a Warning diagnostic for every newly-created governance document, blocking closure of the-greenman/srs#95. The fix: change the identity record creation in `scaffold_governance_repo` to use `com.semanticops.core/purpose` with its canonical fields (`statement`, `title`).

The `com.semanticops.core/purpose` type and its fields (`statement-3b000001`, `title-3b000002`) are already present in both seed files (`crates/srs-gov/assets/governance-seed.srsj` and `crates/srs-repository/tests/fixtures/governance-seed.srsj`) via the ADR-025 implicit core package merge, so no seed changes are needed.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | Claude (this session) |
| Verification | Architecture Reviewer + Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-025](../docs/adr/025-implicit-core-package-merge.md) | Core package is implicitly merged — `com.semanticops.core/purpose` type is available via `load_package()` in every repo | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All field-resolution logic lives in `srs-repository` services, not clients | accepted |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | Seeds are deterministic byte-copy artifacts — no hand-editing | accepted |

**No new ADRs.** This is a localized correctness fix to an existing service. Using `com.semanticops.core/purpose` as the identity type is already the architectural requirement (RFC-018/ADR-025); this plan brings the scaffold service into conformance with the existing decision.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `CreateGovernanceRepositoryResult` / `ScaffoldGovernanceRepoResult` structs are unchanged — `identity_record_id` is still a UUID string. Golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. No sync required.

---

## Scope

- Change the identity record created in `scaffold_governance_repo` from `governance/article` → `com.semanticops.core/purpose`.
- Map the `title` input parameter to the `com.semanticops.core/title` field (optional; `fieldId: 3b000002-0000-4000-a000-000000000002`).
- Map the `purpose` input parameter to the `com.semanticops.core/statement` field (required; `fieldId: 3b000001-0000-4000-a000-000000000001`).
- Keep the decision-log root record creation unchanged (`governance/decision_log` type, `governance/title` field).
- Add a test proving that `create_governance_repository` produces zero RFC-018 I-81 diagnostics.
- Update the docstring in `crates/srs-bindings/src/lib.rs` that says "governance/article identity record".

**Out of scope:**

- Any change to the seed files (`governance-seed.srsj`) — the core types are already present via ADR-025.
- The broader RFC-018 migration tooling (#426) — that migrates existing repos; this plan fixes new repo creation.
- Any change to `governance/article` type or its usage elsewhere.

---

## Phases

### Phase 1: Change identity record type in scaffold service + add regression test

**Goal:** `scaffold_governance_repo` creates a `com.semanticops.core/purpose` identity record, and a test proves the result validates with zero I-81 warnings.

**Agent:** Repository Service Worker (Claude)

#### Tasks

- [ ] In `crates/srs-repository/src/governance_scaffold_service.rs`, at the start of `scaffold_governance_repo`:
  - Remove the `article_text_field_id` lookup (`package.find_field("governance", "article_text")`).
  - Rename the `title_field_id` variable to `dl_title_field_id` (used for decision-log root only).
  - Add lookups:
    ```rust
    let statement_field_id = package
        .find_field("com.semanticops.core", "statement")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "com.semanticops.core/statement field not found in package".to_string(),
        })?
        .id
        .clone();
    let core_title_field_id = package
        .find_field("com.semanticops.core", "title")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "com.semanticops.core/title field not found in package".to_string(),
        })?
        .id
        .clone();
    ```
  - Change the identity record creation at step 1:
    - Type: `"com.semanticops.core/purpose"` (was `"governance/article"`)
    - Fields: `statement_field_id` → purpose text, `core_title_field_id` → title
  - Update step 2 (decision-log root): replace `title_field_id` with `dl_title_field_id`.

- [ ] In `crates/srs-bindings/src/lib.rs`, update the docstring on `scaffold_new_repository` (line ~718): change "governance/article identity record" to "com.semanticops.core/purpose identity record".

- [ ] Add a test `create_governance_repository_validates_with_zero_i81_warnings` in `governance_scaffold_service.rs` tests:
  - Calls `create_governance_repository` with a test namespace + title.
  - Exports to srsj, reloads, runs `validate_repository`.
  - Asserts zero diagnostics whose `message.contains("RFC-018 I-81")`.

#### Acceptance Criteria

- [ ] `scaffold_governance_repo` creates a `com.semanticops.core/purpose` record as the identity.
- [ ] The identity record has `statement` field = purpose text and `title` field = repo title.
- [ ] `create_governance_repository_validates_with_zero_i81_warnings` passes.
- [ ] Existing scaffold tests pass without modification (the result struct shape is unchanged).
- [ ] `cargo test -p srs-repository` passes.
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository scaffold
cargo test -p srs-repository governance
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `create_governance_repository_validates_with_zero_i81_warnings` — validates fresh repo has zero I-81 warnings.
- All existing `scaffold_*` tests — regression guard that result struct and container wiring are unchanged.

#### Milestone gate

1. All acceptance criteria above are met.
2. `cargo test -p srs-repository` passes.
3. `cargo clippy -p srs-repository -- -D warnings` clean.
4. Commit: `fix(scaffold): identity record uses com.semanticops.core/purpose, not governance/article (#568)`.

---

## Final Acceptance

- [ ] `cargo test` — workspace green.
- [ ] `cargo clippy -- -D warnings` — clean.
- [ ] `cargo test -p srs-gov --test flow` — all pass.
- [ ] New test proves zero RFC-018 I-81 warnings on a freshly created governance repo.
- [ ] Existing `scaffold_*` tests all pass.
- [ ] `cargo test --test payload_contracts` — passes (no payload structs changed).
- [ ] `bash scripts/check-schema-sync.sh` — exits 0 (no entity schemas changed).

## Testability

The fix is provable two ways: (1) the new validation test that runs on the full scaffold output and checks for zero I-81 warnings; (2) the existing flow tests (srs-gov) that exercise the full end-to-end create path and would surface any type-resolution errors.

## Assumptions

- Both seed files already contain `com.semanticops.core/purpose` and its fields via ADR-025 implicit core merge. Confirmed: both seeds contain 3 occurrences of `com.semanticops.core` (statement field, title field, purpose type).
- `create_record_in_context` correctly handles `"com.semanticops.core/purpose"` via `splitn(2, '/')`. Confirmed: the split produces namespace=`com.semanticops.core`, name=`purpose`.
- The seed divergences noted in the issue (missing `requiresRelation`, `abandoned`, `external_links`) were already resolved in commit 2103ad5 (re-vendor with RFC-022 lifecycle states). Both seeds now contain identical lifecycle definitions with `requiresRelation` on the `superseded` state, `abandoned` state, and `external_links` field.
