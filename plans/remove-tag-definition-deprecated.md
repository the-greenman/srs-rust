# Plan: Remove Deprecated TagDefinition and tag_service Write Operations

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

ADR-012 (Vocabulary Substrate) deprecated `TagDefinition` (the Rust struct), its service write functions, `is_tag_definition()`, `upsert_tag_definition_index_entry`, and related error variants when RFC-006 shipped (PR #70, PR #73). This plan performs the final removal. No external consumers exist: `srs-vscode` has zero references to these symbols, and the CLI's `tag create/update/delete` handlers already return descriptive errors. The `get_foundation_signal_tags` function is also removed; its CLI call site in `note.rs` is replaced with a hardcoded `Vec::new()` (vocabulary-based foundation resolution is deferred). The plan is surgical: remove the deprecated surface, keep the non-deprecated API (`list_terms`, `get_term_by_id`, `query_by_tag`, `audit_tags`, `TagQueryHit`, `TagQueryResult`, `AuditFinding`, etc.), and verify `cargo test` and `cargo clippy -D warnings` pass clean.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | `agents.md#core-model-worker` |
| Repository Service Worker | `agents.md#repository-service-worker` |
| CLI Worker | `agents.md#cli-worker` |
| Verification | `agents.md#verification-agent` |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-003](../docs/adr/003-tagdefinition-is-core.md) | TagDefinition as core native type — superseded | superseded by ADR-012 |
| [ADR-012](../docs/adr/012-vocabulary-substrate.md) | Vocabulary Substrate; TagDefinition write ops deprecated at Task 3 ship | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service boundary contract — governs service function shape | proposed |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI output contract — payload structs, golden schemas | accepted |

No new ADRs required. This plan implements the deferred removal committed to in ADR-012.

---

## Contracts

### CLI output contract (ADR-011)

The `tag create`, `tag update`, `tag delete` handlers already return `output::err(...)` — they produce no payload struct. The `tag list` and `tag get` handlers use `TagListPayload` and `TagPayload` (backed by `Term`), which are unchanged. No payload struct changes.

**No action required.** Golden schemas stay as-is; `cargo test --test payload_contracts` must continue to pass.

### Entity schema sync (check-schema-sync.sh)

This plan makes no changes to `srs/docs/schema/2.0/` or the schema mirror files.

**No action required.**

---

## Scope

- Remove `srs_core::types::tag_definition` module (struct, tests)
- Remove `srs_core::validation::tag_definition` module (validator, tests)
- Remove `pub mod tag_definition` from `srs-core/src/types/mod.rs` and `srs-core/src/validation/mod.rs`
- Remove `srs_repository::loader::load_tag_definition` function and its imports
- Remove `srs_repository::writer::write_tag_definition` function and `upsert_tag_definition_index_entry` function and their imports
- Remove all deprecated functions from `srs_repository::tag_service`: `create_tag_definition`, `update_tag_definition`, `delete_tag_definition`, `create_tag_definition_in_context`, `delete_tag_definition_in_context`, `update_tag_definition_validated`, `list_tag_definitions`, `list_tag_definitions_by_role`, `list_tag_definitions_filtered`
- Remove `TagDefinitionSummary`, `GetTagDefinitionResult`, `CreateTagDefinitionResult`, `UpdateTagDefinitionResult`, `DeleteTagDefinitionResult`, `TagListFilter` result/filter types from `tag_service.rs`
- Remove `slugify_tag_key` private helper from `tag_service.rs` (only used by write ops)
- Remove `get_foundation_signal_tags` from `tag_service.rs`; its use in `srs-cli/src/commands/note.rs:169` must change: replace `let signal_tags = with_store(&ctx, |store| Ok(get_foundation_signal_tags(store)?))?;` with `let signal_tags: Vec<String> = Vec::new();` (no store call needed — foundation signal tags no longer discoverable via deprecated index; vocabulary-based resolution is deferred).
- Remove `is_tag_definition()` method from `srs_repository::index::InstanceIndexEntry` and its `#[allow(deprecated)]` usage in `index.rs` tests
- Remove `RepositoryError::TagDefinitionLoad`, `RepositoryError::TagDefinitionValidation`, `RepositoryError::TagDefinitionWrite` variants and their test comparison arms in `error.rs`
- Remove the deprecated CLI commands `TagCommand::Create`, `TagCommand::Update`, `TagCommand::Delete` variants from `crates/srs-cli/src/commands/mod.rs`, their `cmd_tag_write_error` handler dispatch arms in `crates/srs-cli/src/commands/tag.rs`, and `cmd_tag_write_error` function itself
- Remove all `#[allow(deprecated)]` suppressions that exist solely for TagDefinition (not for unrelated container method deprecations)
- Remove tag-definition-specific tests from `tag_service.rs` test module and `index.rs` test module; update `integration_tests.rs` to remove tests that assert the old `tag create/update/delete` error behavior (those commands will be removed)

**Out of scope:**

- The `#[allow(deprecated)]` suppressions in `store.rs` and `relation_service.rs` — those are for container store method deprecations, unrelated to TagDefinition
- Removing `tag list` and `tag get` commands — they now serve `Term`-based vocabulary queries and are current
- Removing `tag_service.rs` itself — the module stays; it hosts vocabulary query functions (`list_terms`, `get_term_by_id`, `query_by_tag`, `audit_tags`)
- Any changes to `vocabulary_service.rs`, `package_types.rs`, or Term-related code
- Any `srs-vscode` changes (no references exist)
- CLI stub for `get_foundation_signal_tags` behavior via vocabulary — that is a follow-up if needed

---

## Phases

### Phase 1: Remove srs-core TagDefinition types and validation

**Goal:** `srs-core` has no `TagDefinition` struct or `validate_tag_definition` function; all dependent imports removed.

**Agent:** Core Model Worker

#### Tasks

- [ ] Delete `crates/srs-core/src/types/tag_definition.rs` entirely
- [ ] Delete `crates/srs-core/src/validation/tag_definition.rs` entirely
- [ ] Edit `crates/srs-core/src/types/mod.rs`: remove line `pub mod tag_definition;`
- [ ] Edit `crates/srs-core/src/validation/mod.rs`: remove line `pub mod tag_definition;`

#### Acceptance Criteria

- [ ] `crates/srs-core/src/types/tag_definition.rs` does not exist
- [ ] `crates/srs-core/src/validation/tag_definition.rs` does not exist
- [ ] `grep -r "TagDefinition\|tag_definition\|validate_tag_definition" crates/srs-core/` returns no results
- [ ] `cargo test -p srs-core` passes with no failures

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

- No new tests needed; existing tests for other types must still pass

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm `cargo test -p srs-core` passes.
3. Run:
```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```
4. Mark task checkboxes `[x]` and commit:
```bash
git commit -m "chore: remove srs-core TagDefinition types and validation (#75)"
```

---

### Phase 2: Remove tag_definition from srs-repository loader, writer, index, and error

**Goal:** `srs-repository` has no `load_tag_definition`, `write_tag_definition`, `upsert_tag_definition_index_entry`, `is_tag_definition`, or TagDefinition error variants.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Edit `crates/srs-repository/src/loader.rs`:
  - Remove `use srs_core::types::tag_definition::TagDefinition;` import
  - Remove `use srs_core::validation::tag_definition::validate_tag_definition;` import
  - Remove entire `load_tag_definition` function (lines 26–42 in current file)
- [ ] Edit `crates/srs-repository/src/writer.rs`:
  - Remove `use srs_core::types::tag_definition::TagDefinition;` import
  - Remove `write_tag_definition` function (lines 110–121 in current file)
  - Remove `upsert_tag_definition_index_entry` function (lines 123–146 in current file)
- [ ] Edit `crates/srs-repository/src/index.rs`:
  - Remove `#[deprecated(...)]` + `is_tag_definition()` method from `InstanceIndexEntry` impl (lines 39–44)
  - Remove `#[allow(deprecated)]` from the `mod tests` block (line 48) and from the `is_tag_definition_for_tier_3` test (line 85 area) — or remove the entire `is_tag_definition_for_tier_3` test
- [ ] Edit `crates/srs-repository/src/error.rs`:
  - Remove `TagDefinitionLoad { path, source: serde_json::Error }` variant
  - Remove `TagDefinitionValidation { path, source: srs_core::error::CoreError }` variant
  - Remove `TagDefinitionWrite { path, source: std::io::Error }` variant
  - Remove the three corresponding match arms in the `PartialEq` impl (lines 368–384 area)

#### Acceptance Criteria

- [ ] `grep -r "load_tag_definition\|write_tag_definition\|upsert_tag_definition_index_entry\|is_tag_definition\|TagDefinitionLoad\|TagDefinitionValidation\|TagDefinitionWrite" crates/srs-repository/src/` returns no results outside of `tag_service.rs` (tag_service cleanup is Phase 3)
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Note: `tag_service.rs` will have compile errors in this phase until Phase 3 cleans it. Build with `cargo check -p srs-repository` is acceptable as a mid-phase check; full test pass requires Phase 3 to complete first. Alternatively, execute Phases 2 and 3 together before running tests.

#### Milestone gate

Run after Phase 3 completes (phases 2+3 form one compile unit):
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 3: Remove deprecated functions and types from tag_service.rs

**Goal:** `tag_service.rs` retains only the non-deprecated vocabulary query API; all TagDefinition CRUD is gone.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Edit `crates/srs-repository/src/tag_service.rs`:
  - Remove `use crate::loader::load_tag_definition;` import
  - Remove `use crate::writer::{new_instance_id, upsert_tag_definition_index_entry, write_manifest};` entirely. `write_manifest` is only used at lines 251, 290, and 462 — all inside deprecated functions being removed. The remaining non-deprecated functions (`list_terms`, `get_term_by_id`, `query_by_tag`, `audit_tags`) do not call `write_manifest`.
  - Remove `use srs_core::types::tag_definition::TagDefinition;` import
  - Remove `use srs_core::validation::tag_definition::validate_tag_definition;` import
  - Remove struct `TagDefinitionSummary`
  - Remove enum `GetTagDefinitionResult`
  - Remove struct `CreateTagDefinitionResult`
  - Remove struct `UpdateTagDefinitionResult`
  - Remove struct `DeleteTagDefinitionResult`
  - Remove struct `TagListFilter`
  - Remove `slugify_tag_key` private function
  - Remove `list_tag_definitions` function
  - Remove `list_tag_definitions_by_role` function
  - Remove `get_tag_definition_by_id` function
  - Remove `get_foundation_signal_tags` function (vocabulary-based resolution is deferred; CLI Phase 4 replaces the call site with a hardcoded empty Vec)
  - Remove `create_tag_definition` function
  - Remove `update_tag_definition` function
  - Remove `list_tag_definitions_filtered` function
  - Remove `create_tag_definition_in_context` function
  - Remove `delete_tag_definition_in_context` function
  - Remove `update_tag_definition_validated` function
  - Remove `delete_tag_definition` function (and its `find_instances_using_tag` private helper)
  - Remove `find_instances_using_tag` private function
  - Remove the entire `#[cfg(test)] #[allow(deprecated)] mod tests { ... }` block at the bottom of `tag_service.rs`

After removal, `tag_service.rs` should retain only: `TagQueryHit`, `TagQueryResult`, `AuditFinding`, `AuditFindingKind`, `AuditTagsFilter`, `AuditTagsResult`, `list_terms`, `get_term_by_id`, `query_by_tag`, `audit_tags`.

#### Acceptance Criteria

- [ ] `grep -n "TagDefinition\|tag_definition\|create_tag\|delete_tag\|update_tag\|list_tag\|get_foundation\|slugify\|allow(deprecated" crates/srs-repository/src/tag_service.rs` returns no results
- [ ] `tag_service.rs` compiles clean after phases 2+3
- [ ] `cargo test -p srs-repository` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate (combined with Phase 2)

1. Verify all Phase 2 and Phase 3 acceptance criteria are met.
2. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
3. Commit:
```bash
git commit -m "chore: remove deprecated TagDefinition loader, writer, index, error, and tag_service CRUD (#75)"
```

---

### Phase 4: Remove deprecated CLI surface

**Goal:** `srs-cli` has no `TagCommand::Create/Update/Delete` variants, no `cmd_tag_write_error`, and no `get_foundation_signal_tags` call.

**Agent:** CLI Worker

#### Tasks

- [ ] Edit `crates/srs-cli/src/commands/mod.rs`:
  - Remove `TagCommand::Create { json: bool }` variant (lines ~612–617)
  - Remove `TagCommand::Update { id, json: bool }` variant (lines ~618–625)
  - Remove `TagCommand::Delete { id, json: bool }` variant (lines ~626–633)
  - Update doc comments on the remaining `TagCommand::Get` variant: (a) change variant doc line ~604 from "Get a tag definition by ID" to "Get a vocabulary term by ID"; (b) change `Get.id` parameter doc line ~606 from "TagDefinition instance ID" to "Term ID"
- [ ] Edit `crates/srs-cli/src/commands/tag.rs`:
  - Remove `TagCommand::Create { .. } => cmd_tag_write_error("tag create")` arm from `dispatch`
  - Remove `TagCommand::Update { .. } => cmd_tag_write_error("tag update")` arm from `dispatch`
  - Remove `TagCommand::Delete { .. } => cmd_tag_write_error("tag delete")` arm from `dispatch`
  - Remove `fn cmd_tag_write_error(command: &str) -> Result<String>` function entirely
- [ ] Edit `crates/srs-cli/src/commands/note.rs`:
  - Remove `use srs_repository::tag_service::get_foundation_signal_tags;` import
  - In `cmd_note_foundations` (line 168), replace line 169 `let signal_tags = with_store(&ctx, |store| Ok(get_foundation_signal_tags(store)?))?;` with `let signal_tags: Vec<String> = Vec::new();` — no store call needed. The next `with_store` call for `collect_foundation_notes` on line 170 is unchanged.

#### Acceptance Criteria

- [ ] `grep -n "TagCommand::Create\|TagCommand::Update\|TagCommand::Delete\|cmd_tag_write_error\|get_foundation_signal_tags\|tag create\|tag update\|tag delete" crates/srs-cli/src/` returns no results
- [ ] `cargo test -p srs-cli` passes
- [ ] `cargo clippy -p srs-cli -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to update/remove:
- `crates/srs-cli/tests/integration_tests.rs` line 2146–2176: Delete entire standalone `#[test] fn tag_update_rewrites_tag_definition()` function and `#[test] fn tag_delete_removes_tag_definition()` function — they test commands that will no longer exist.
- `crates/srs-cli/tests/integration_tests.rs` line 1077–1101: Delete entire `#[test] fn tag_create_and_retrieve_in_temp_repo()` function — it calls `srs tag create` which will no longer exist.
- `crates/srs-cli/tests/integration_tests.rs` lines 812–828: Remove the RFC-006 tag create block (starting `// RFC-006: tag create returns a descriptive error`) through `assert_eq!(tag_created["ok"], false, ...)`. This block is inside a larger workflow test — remove only these 17 lines. The surrounding test continues.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```
3. Run the full workspace:
```bash
cargo test
cargo clippy -- -D warnings
```
4. Commit:
```bash
git commit -m "chore: remove deprecated TagCommand::Create/Update/Delete and get_foundation_signal_tags (#75)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes with no warnings
- [ ] `grep -r "TagDefinition\|tag_definition\|create_tag_definition\|list_tag_definitions\|upsert_tag_definition\|is_tag_definition\|allow(deprecated" --include="*.rs" crates/` finds no TagDefinition-related hits (only allowed: container-method `allow(deprecated)` hits in `store.rs` and `relation_service.rs`)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `srs repo validate --repo ../srs/srs` still passes (0 errors)
- [ ] All integration tests pass including `tag list` and `tag get` working against vocabulary terms

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Phases 2 and 3 may be executed together before running the milestone gate test (they share a compile unit that will not compile mid-way through Phase 2 alone).

## Assumptions

- RFC-006 Task 3 (issue #73) is merged; `vocabulary_service::list_terms` and `get_term_by_id` exist and work.
- `srs-vscode` has no references to the deprecated symbols (confirmed by grep: zero hits).
- No external consumers of the deprecated functions exist outside this monorepo.
- `get_foundation_signal_tags` returning empty (rather than querying vocabulary for "foundation"-role terms) is acceptable for now; vocabulary-based resolution is a follow-up.
