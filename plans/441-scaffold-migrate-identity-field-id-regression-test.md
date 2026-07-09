# Plan: Regression test for scaffold / migrate-identity purpose-record field-ID parity (#441)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.
>
> Save this file to `plans/<slug>.md` before assigning agents. Agents receive the plan file as their primary brief.

## Summary

Issue #441 reported that `repository_lifecycle.rs` (the `repo create` scaffold path) and `migrate_identity_service.rs` (the `repo migrate-identity` path) defined **divergent** `CORE_STATEMENT_FIELD_ID` / `CORE_TITLE_FIELD_ID` constants (`3b…` vs `fc…`), so purpose records created by one path would carry field IDs unreadable by anything keyed off the other path's constants.

Git archaeology shows this was already fixed as a side effect of commit `07bb4cc` ("refactor(repository): consolidate core/purpose constants into core_purpose module (#432)"), merged after #441 was filed. Both `repository_lifecycle::scaffold_purpose_record` and `migrate_identity_service::migrate_identity` now build purpose records exclusively through the shared `crate::core_purpose::build_purpose_record`, which uses a single set of constants (`STATEMENT_FIELD_ID = "3b000001-…"`, `TITLE_FIELD_ID = "3b000002-…"` — confirmed as the spec-authorised values in `srs/srs/package/core/fields/{statement-3b000001,title-3b000002}.json`). There is no remaining `fc…` reference anywhere in `crates/`; the only surviving `fc…` text is historical prose in `plans/426-migrate-identity.md`, which is not touched by this plan.

What #441 explicitly asked for and does **not** yet exist is a regression test: "Add a test that scaffolded purpose records survive `migrate_identity`'s field-ID validation without error." No existing test builds a repo via the `repo create` scaffold path and then feeds it through `migrate_identity`. This plan adds that test as a guard against the two call sites drifting apart again (e.g. if a future change reintroduces a local constant instead of using `core_purpose`).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan adds test coverage only; the production fix already shipped under the existing `core_purpose` consolidation (commit `07bb4cc`, in service of ADR-008 repository lifecycle/portability). No ADR governs test-only changes and none is warranted here.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands, no payload struct changes. No action required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files touched. No action required.

---

## Scope

- Add one regression test in `crates/srs-repository/src/migrate_identity_service.rs`'s `#[cfg(test)] mod tests` that:
  1. Builds a `MemoryStore` and scaffolds a repository via `crate::repository_lifecycle::create_repository_with_intent` (the same path `repo create` uses), producing a Tier-2 `com.semanticops.core/purpose` identity record.
  2. Asserts the scaffolded record's field values use `core_purpose::STATEMENT_FIELD_ID` (and, when a title is supplied, `core_purpose::TITLE_FIELD_ID`) — i.e. the same constants `migrate_identity` reads.
  3. Calls `migrate_identity_service::migrate_identity` against that store and asserts it returns the expected `RepositoryError::InvalidInput` with message `"already a com.semanticops.core/purpose record; no migration needed"` (the Tier-2 short-circuit branch at `migrate_identity_service.rs:156-163` on `master`). Note: this branch checks only `typeNamespace`/`typeName`, not field IDs — it proves *type* identity is recognized. The field-ID regression guard itself is step 2's direct assertion; step 3 is included because it's the actual `migrate_identity`-facing behaviour #441 asked to be proven safe.
- Add a one-line code comment at the `core_purpose` constant definitions (`crates/srs-repository/src/core_purpose.rs`) noting the regression this test guards against, cross-referencing #441.

**Out of scope:**
- Any change to `CORE_STATEMENT_FIELD_ID` / `CORE_TITLE_FIELD_ID` values themselves — already unified.
- Removing the "Temporary hardcoded UUIDs pending core-type registry (#423)" comment/mechanism in `core_purpose.rs` — that's #423's scope, not this issue's.
- Editing `plans/426-migrate-identity.md` (historical plan prose, not live code).

---

## Phases

### Phase 1: Regression test

**Goal:** A test exists proving a `repo create`-scaffolded purpose record survives `migrate_identity`'s field/type validation without error, and fails loudly (compile or test failure) if the two call sites' field IDs ever diverge again.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/migrate_identity_service.rs`, inside `#[cfg(test)] mod tests`, add `fn migrate_identity_recognizes_scaffolded_purpose_record()`:
  - Import `crate::repository_lifecycle::{create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata}` at the top of the test (module already imports `MemoryStore`).
  - Construct the store with `let store = MemoryStore::uninitialized();` — **not** `MemoryStore::default()` (the convention elsewhere in this file). `MemoryStore::default()` builds an already-initialized store (`repository_initialized: true`, `store.rs:1979`), and `create_repository_with_intent` → `create_repository` errors with `RepositoryError::RepositoryAlreadyExists` whenever `store.repository_exists()` is true (`repository_lifecycle.rs:84-88`). `repository_lifecycle.rs`'s own scaffold tests use `MemoryStore::uninitialized()` for exactly this reason — follow that, not the local file's default.
  - Build an `InitializeRepositoryInput` matching the existing `input()` helper pattern in `repository_lifecycle.rs` tests (repository_id, namespace, srs_version, title `Some("My Repo")`, description `Some("I build SRS.")`).
  - Call `create_repository_with_intent(&store, &input)`, unwrap, capture `identity_instance_id`.
  - Load the record: `store.load_manifest()` → find the instance-index entry whose `instance_id()` matches `identity_instance_id` → `store.load_instance_json(entry.path())` → deserialize into `srs_core::types::record::Record` via `serde_json::from_value` (matches the pattern in `repository_lifecycle.rs::create_repository_with_intent_record_has_correct_type`).
  - Assert `record.field_values.iter().any(|fv| fv.field_id == core_purpose::STATEMENT_FIELD_ID)` and the equivalent for `TITLE_FIELD_ID` (title was supplied). **This assertion is the actual field-ID regression guard** — it fails immediately if `scaffold_purpose_record` ever stops routing through `core_purpose::build_purpose_record`.
  - Call `let err = migrate_identity(&store).unwrap_err();`, then pattern-match: `match err { RepositoryError::InvalidInput { message } => assert!(message.contains("no migration needed"), "unexpected message: {message}"), other => panic!("expected InvalidInput, got {other:?}") }` (mirrors the existing `RepositoryError::InvalidInput { message: a }` destructuring pattern in `crates/srs-repository/src/error.rs:641`).
- [x] Add a short comment above `STATEMENT_FIELD_ID`/`TITLE_FIELD_ID` in `crates/srs-repository/src/core_purpose.rs` noting these constants are shared by both `repository_lifecycle` and `migrate_identity_service` specifically to prevent the divergence in #441 from recurring.

#### Acceptance Criteria

- [x] `migrate_identity_recognizes_scaffolded_purpose_record` exists in `crates/srs-repository/src/migrate_identity_service.rs` and passes.
- [x] The test asserts `record.field_values` contains entries whose `field_id` equals `core_purpose::STATEMENT_FIELD_ID`/`TITLE_FIELD_ID` (not a hardcoded literal) — this is what makes the test fail should the two call sites' field IDs ever diverge again, since it ties the scaffold's actual output to the same constant `migrate_identity` reads.
- [x] No production code changed (fix already landed under commit `07bb4cc`).

#### Testing

```bash
cargo test -p srs-repository migrate_identity_recognizes_scaffolded_purpose_record
cargo test -p srs-repository
```

Specific tests to write or verify:

- `migrate_identity_recognizes_scaffolded_purpose_record` — proves a `repo create`-scaffolded purpose record's field IDs are recognized by `migrate_identity`'s Tier-2 short-circuit, i.e. both call sites agree on field identity.

#### Milestone gate

1. Verify acceptance criteria above.
2. Confirm the test exists and passes.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update this plan file: mark task/acceptance checkboxes `[x]`.
5. Commit:
   ```bash
   git commit -m "test(repository): add regression test for scaffold/migrate-identity field-ID parity (#441)"
   ```

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass) — N/A, no CLI surface touched
- [x] `cargo test --test payload_contracts` passes — N/A, no payload structs changed
- [x] `bash scripts/check-schema-sync.sh` exits 0 — N/A, no entity schemas changed
- [x] `migrate_identity_recognizes_scaffolded_purpose_record` exists and passes
- [x] No `fc000001…`/`fc000002…` references remain anywhere in `crates/` (confirm via `git grep -n "fc000001\|fc000002" -- crates/`)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Branch off `origin/master` (srs-rust's default branch) and open the PR against `master` — per repo convention, not the generic `main` used elsewhere in the ship pipeline template.
- The `core_purpose` module consolidation (commit `07bb4cc`) is present on `master` at the time this plan's branch is cut, so no rebase surprises reintroduce the divergence. Stage 6 of the ship pipeline (sync with main) re-verifies this before final acceptance.
- `RepositoryError::InvalidInput` remains a struct-variant with a `message: String` field, matching its current shape in `crates/srs-repository/src/error.rs`.
