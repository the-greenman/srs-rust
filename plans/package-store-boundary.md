# Plan: Package Loading Store Boundary (Issue #22)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`package.rs` contains a public `load_package(repo_root: &Path)` function that reads package definitions directly from the filesystem, bypassing the `RepositoryStore` abstraction established in ADR-009. While `RepositoryStore::load_package()` is already the correct abstraction and used correctly by all service code, the standalone function is an invisible seam: future code could call `package::load_package()` directly and silently bypass any non-filesystem backend. Additionally, the loading implementation is duplicated: `package.rs` and `store.rs` each contain their own `load_package_from_dir`, `PackageMetadata`, `FieldJson`, `TypeJson`, `FieldAssignmentJson`, `FieldGroupJson`, `FieldAssignmentOverrideJson`, and `parse_value_type`. This plan removes the public standalone function and all its helpers from `package.rs`, eliminates the duplication, and migrates the affected tests to call `FileStore::new(root).load_package()`. After this change, `RepositoryStore::load_package()` is unambiguously the only entry point.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | `RepositoryStore::load_package()` is the sole abstraction for package loading; no service or caller may reach `package::load_package()` directly | accepted |

No new ADRs are needed — this plan completes the implementation of ADR-009.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. No payload structs are touched. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- Remove `pub fn load_package(repo_root: &Path)` from `crates/srs-repository/src/package.rs`.
- Remove all helpers used only by that function from `package.rs`: `fn load_package_from_dir`, `struct PackageMetadata`, `struct FieldJson`, `struct TypeJson`, `struct FieldAssignmentJson`, `struct FieldGroupJson`, `struct FieldAssignmentOverrideJson`, `fn parse_value_type`.
- Migrate the load-related tests in `package.rs` (those that call `load_package(root)`) to use `FileStore::new(root).load_package()`.
- Keep in `package.rs`: `pub struct Package`, `pub struct DependencyRef`, `impl Package { ... }` (all resolve/find/effective_fields methods), and the `effective_fields` and `validate_record_uses_effective_fields` tests (they are pure in-memory and need no loading).

**Out of scope:**
- Deduplicating the helper structs between `store.rs` and `package.rs` beyond what is needed to remove the standalone function. (`store.rs` retains its own `PackageMetadata`, `FieldJson`, etc.)
- Any changes to CLI commands, payload structs, or JSON Schema files.
- Changes to `MemoryStore`, `JsonStore`, or any other store implementation.

---

## Phases

### Phase 1: Remove standalone `load_package` and migrate tests

**Goal:** `package.rs` contains only `Package`, `DependencyRef`, and their `impl` methods; the public API surface of `srs_repository::package` no longer includes any I/O functions; all tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package.rs`:
  - [ ] Delete `pub fn load_package(repo_root: &Path) -> Result<Package, RepositoryError>` — locate by signature, not by line number.
  - [ ] Delete `fn load_package_from_dir(package_dir: &Path, rt_by_type: &mut HashMap<String, (RelationTypeDefinition, PathBuf)>)` — locate by signature.
  - [ ] Delete `struct PackageMetadata` — locate by name.
  - [ ] Delete `struct FieldJson` — locate by name.
  - [ ] Delete `struct TypeJson` — locate by name.
  - [ ] Delete `struct FieldAssignmentJson` — locate by name.
  - [ ] Delete `struct FieldGroupJson` — locate by name.
  - [ ] Delete `struct FieldAssignmentOverrideJson` — locate by name.
  - [ ] Delete `fn parse_value_type(s: &str, path: &std::path::Path) -> Result<ValueType, RepositoryError>` — locate by signature.
  - [ ] Remove the following imports that are used only by the deleted functions (verify each with grep before removing):
    - `use crate::manifest::load_manifest` — only called from deleted `load_package`; remove.
    - `use srs_core::validation::relation_type_definition::validate_relation_type_definition` — only called from deleted `load_package_from_dir`; remove.
    - `use srs_core::validation::theme::validate_theme` — only called from deleted `load_package_from_dir`; remove.
    - `use srs_core::validation::view::{validate_document_view, validate_view}` — only called from deleted `load_package_from_dir`; remove.
    - `use srs_core::types::field::ValueType` — only used in deleted `parse_value_type`; remove.
  - [ ] Keep the following imports (they are still needed by `impl Package` methods or tests):
    - `use crate::error::RepositoryError` — used in `effective_fields` return type.
    - `use std::collections::HashMap` — used in `effective_fields` (local `HashSet` and `HashMap`).
    - `use std::path::{Path, PathBuf}` — used in `Package::root: PathBuf` field and test helpers.
    - `use srs_core::types::record_type::{FieldAssignment, FieldAssignmentOverride, ...}` — used in `effective_fields`.
    - All other `use srs_core::types::*` that are fields of `Package` or used by its methods.
  - [ ] Run `cargo build -p srs-repository` after import cleanup to confirm no compilation errors; fix any remaining unused-import warnings.

- [ ] In `crates/srs-repository/src/package.rs` `#[cfg(test)]` section:
  - [ ] Update every test that calls `load_package(root)` to call `crate::store::FileStore::new(root).load_package()` instead. The affected tests are:
    - `load_package_preserves_extends_type_id`
    - `load_package_from_live_repo`
    - `resolve_type_by_name_finds_known_type`
    - `resolve_type_by_name_returns_none_for_unknown`
    - `find_field_by_name_finds_status`
    - `resolve_field_returns_none_for_unknown`
    - `load_package_loads_relation_types`
    - `load_package_loads_document_views`
    - `resolve_document_view_finds_srs_spec_view`
    - `resolve_document_view_returns_none_for_unknown`
    - `load_package_loads_themes`
    - `resolve_theme_finds_known_theme`
    - `resolve_theme_returns_none_for_unknown`
    - `load_package_without_themes_key_loads_without_error`
    - `load_package_theme_validation_fails_on_empty_targets`
    - `resolve_canonical_relation_type_precedes`
    - `deprecated_relation_types_loaded_with_correct_status`
    - `load_package_errors_on_missing_package_ref`
    - `load_package_detects_conflicting_field_definitions`
    - `load_package_coalesces_identical_field_definitions`
  - [ ] The following tests use `make_package_with_types` / direct `Package` construction and do **not** call `load_package`; leave them unchanged:
    - `effective_fields_non_inheriting_returns_sorted_own_fields`
    - `effective_fields_single_level_inheritance`
    - `effective_fields_two_level_chain`
    - `effective_fields_detects_cycle`
    - `effective_fields_field_order_reorders`
    - `effective_fields_field_order_incomplete_errors`
    - `effective_fields_field_order_duplicate_entry_errors`
    - `effective_fields_field_order_unknown_id_errors`
    - `effective_fields_override_targets_unknown_field_errors`
    - `effective_fields_detects_duplicate_field`
    - `effective_fields_override_relaxes_required_errors`
    - `effective_fields_override_tightens_required_ok`
    - `effective_fields_override_targets_own_field_errors`
    - `validate_record_uses_effective_fields`
  - [ ] The helper functions `srs_spec_repo()`, `create_minimal_repo()`, `write_package_json()`, `write_field_json()`, `add_package_ref_to_manifest()` are needed by the load tests; keep them.
  - [ ] The helper functions `make_package_with_types()`, `fa()`, `make_type()`, `make_child_type()` are needed by the effective_fields tests; keep them.

- [ ] Before starting deletions, verify no other file in `crates/` calls the standalone `package::load_package`:
  ```bash
  grep -rn "package::load_package\|crate::package::load_package" crates/ | grep -v "target/" | grep -v "package.rs"
  ```
  This must return no results. (Confirms the assumption — if it does return results, stop and report.)

- [ ] Verify that `FileStore::load_package()` behaviorally equivalent proof: after migrating the 20 tests to `FileStore::new(root).load_package()`, all tests must pass. The test suite passing after migration IS the equivalence proof — if any test fails, the implementations diverge and the plan must halt for investigation.

#### Acceptance Criteria

- [ ] `grep -rn "^pub fn load_package" crates/srs-repository/src/package.rs` returns no results (function is gone).
- [ ] `grep -rn "package::load_package\|crate::package::load_package" crates/ | grep -v target/` returns no results (no external callers remain).
- [ ] `cargo build -p srs-repository` succeeds with no errors.
- [ ] `cargo test -p srs-repository` passes with no failures.
- [ ] The tests listed as "migrated" above now use `FileStore::new(root).load_package()`.
- [ ] `grep -n "load_package(root\|load_package(&" crates/srs-repository/src/package.rs` returns no standalone calls (only the `.load_package()` method call pattern as part of `FileStore::new(root).load_package()`).
- [ ] The tests listed as "unchanged" above still pass without modification.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify exist and pass after migration (sample):

- `package::tests::load_package_from_live_repo` — proves FileStore loads the live spec repo
- `package::tests::load_package_preserves_extends_type_id` — proves inheritance field loads correctly
- `package::tests::effective_fields_single_level_inheritance` — proves in-memory effective_fields still works
- `package::tests::load_package_coalesces_identical_field_definitions` — proves sub-package merge logic

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update this plan: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:
   ```bash
   git commit -m "refactor(srs-repository): remove public package::load_package, complete ADR-009 (#22)"
   ```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] `grep -rn "^pub fn load_package" crates/srs-repository/src/package.rs` returns empty (function removed)
- [ ] `grep -rn "package::load_package" crates/ | grep -v target/` returns empty (no callers)

## Coordination Rules

- Repository Service Worker owns all changes in `crates/srs-repository/src/package.rs`.
- Lead Integrator reviews the final diff for ADR-009 compliance before committing.
- Agents keep to their write scopes.
- **At the end of Phase 1:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to PR without completing the milestone gate.
- Verification Agent runs after Phase 1 and before final sign-off.

## Assumptions

- No code outside `package.rs` calls `package::load_package` directly (confirmed by grep returning empty before writing this plan; the plan includes a pre-flight grep to re-verify at execution time).
- `FileStore::load_package()` in `store.rs` is functionally equivalent to the removed `package::load_package()` — both traverse the same `package/` directory structure using identical `load_package_from_dir` helper logic and produce the same `Package` value. **This assumption is verified at execution time**: if any of the 20 migrated tests fail after migration to `FileStore::new(root).load_package()`, the implementations diverge and the plan must halt for investigation.
- Tests that currently call `load_package(root)` in `package.rs` are testing `Package` resolution behavior (type resolution, field resolution, sub-package merging, etc.); migrating them to `FileStore::new(root).load_package()` preserves this intent while routing through the store abstraction.
