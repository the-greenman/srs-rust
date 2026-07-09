# Plan: declaredExtensions Conformance Report (#237)

## Summary

`manifest.extra.declaredExtensions[]` is a string array in every SRS repository's manifest,
but it is never validated against what the implementation actually supports or what the
repository's content actually requires. A repo can declare an unsupported extension (typo,
future extension, removed extension) or use extension features (lifecycle states, relations,
type inheritance, etc.) without declaring the corresponding extension ID — either silently.

This plan adds a single service function returning a typed `DeclaredExtensionsReport` struct
(declared set, supported set, `declared-but-unsupported`, `used-but-undeclared`), exposes it
as `srs repo extensions conformance` via a new CLI subcommand with a golden payload schema,
and defines the implementation's supported-extension set as a named constant in one place.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — (single-phase; one agent executes all tasks) |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification Agent | — |

## Architecture Decisions

No new ADR is required. This plan implements:

| ADR | Decision |
|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service takes no input (read-only conformance scan), returns a typed struct; all detection logic in `srs-repository`, not in the CLI handler |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New payload struct `RepoExtensionsConformancePayload` in `payload.rs`; golden schema regenerated via `cargo run --bin generate-schemas` |
| [ADR-005](../docs/adr/005-extension-definitions-are-tier2-records.md) | Supported-extension set is a Rust constant (`SUPPORTED_EXTENSIONS`), not resolved from `meta.extension` records; conformance is an implementation-layer check, not a spec-record lookup |

---

## Contracts

### CLI output contract (ADR-011)

New subcommand `srs repo extensions conformance` is added. Payload struct
`RepoExtensionsConformancePayload` must be added to `crates/srs-cli/src/payload.rs`.
After adding, run `cargo run --bin generate-schemas` and commit the new golden file
`crates/srs-cli/schemas/payload/RepoExtensionsConformancePayload.json`.

`cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No entity schemas under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- New public constant `SUPPORTED_EXTENSIONS: &[&str]` in `crates/srs-repository/src/manifest_service.rs` — single source of truth for which extension IDs this implementation actively handles.
- New service function `declared_extensions_conformance(store) -> Result<DeclaredExtensionsReport, RepositoryError>` in `manifest_service.rs`.
- New result type `DeclaredExtensionsReport` in `manifest_service.rs`.
- New CLI subcommand `RepoExtensionsCommand::Conformance` in `crates/srs-cli/src/commands/mod.rs`.
- New handler `cmd_repo_extensions_conformance` in `crates/srs-cli/src/commands/repo.rs`.
- New payload struct `RepoExtensionsConformancePayload` in `crates/srs-cli/src/payload.rs`.
- New golden schema file `crates/srs-cli/schemas/payload/RepoExtensionsConformancePayload.json`.
- Unit tests for the service function using `MemoryStore`.

**Out of scope:**
- Changes to `srs/docs/schema/2.0/` (no spec model changes).
- WASM binding for this service (deferred; file follow-up issue under #231).
- Auto-repair or CLI flags to add missing declarations (deferred; follow-up issue under #231).
- Detecting usage of `ext:discovery` or `ext:repository` from repository content — both are
  structural/always-available; neither has a detectable absence signal.

---

## Phases

### Phase 1: Service + CLI + Schema

**Goal:** `srs repo extensions conformance` returns a JSON conformance report containing
`declared`, `supported`, `declaredButUnsupported`, and `usedButUndeclared`.

**Agent:** Repository Service Worker + CLI Worker

#### Tasks

- [ ] In `crates/srs-repository/src/manifest_service.rs`:
  - [ ] Add `pub const SUPPORTED_EXTENSIONS: &[&str]` listing the 7 actively-implemented extension IDs: `ext:lifecycle`, `ext:relations`, `ext:type-inheritance`, `ext:field-groups`, `ext:discovery`, `ext:addressability`, `ext:repository`.
  - [ ] Add `pub struct DeclaredExtensionsReport { pub declared: Vec<String>, pub supported: Vec<String>, pub declared_but_unsupported: Vec<String>, pub used_but_undeclared: Vec<String> }` with `#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]`.
  - [ ] Add `pub fn declared_extensions_conformance(store: &dyn RepositoryStore) -> Result<DeclaredExtensionsReport, RepositoryError>` implementing:
    1. `declared = list_declared_extensions(store)?`
    2. `supported = SUPPORTED_EXTENSIONS.iter().map(|s| s.to_string()).collect()`
    3. `declared_but_unsupported = declared.iter().filter(|id| !supported.contains(id)).cloned().collect()`
    4. `used_extensions = detect_used_extensions(store)?`
    5. `used_but_undeclared = used_extensions.into_iter().filter(|id| !declared.contains(id)).collect()`
  - [ ] Add `fn detect_used_extensions(store: &dyn RepositoryStore) -> Result<Vec<String>, RepositoryError>` (private) that returns the subset of `SUPPORTED_EXTENSIONS` actively in use by the repo:
    - `ext:lifecycle` → any record in manifest instance_index has `lifecycle_state.is_some()` (load records on demand from the index)
    - `ext:relations` → `relation_service::load_relations(store)?` is non-empty
    - `ext:type-inheritance` → `store.load_package()` has any type with `extends_type_id.is_some()`
    - `ext:field-groups` → `store.load_package()` has any type with `field_groups.is_some() && !field_groups.is_empty()`
    - `ext:addressability` → `list_files_recursive("records")` contains any path ending in `.revisions.json`
    - `ext:repository` and `ext:discovery` → not detectable from content; always omit from `used_but_undeclared` (they are not signalled by repository data)
    - Return only extension IDs from `SUPPORTED_EXTENSIONS` that have detectable usage (unsupported ones cannot have detected usage by definition)

- [ ] In `crates/srs-cli/src/commands/mod.rs`:
  - [ ] Add variant to `RepoExtensionsCommand`:
    ```rust
    /// Show declared vs supported vs used extension conformance
    Conformance,
    ```

- [ ] In `crates/srs-cli/src/commands/repo.rs`:
  - [ ] Add arm to `cmd_repo_extensions_dispatch`:
    ```rust
    RepoExtensionsCommand::Conformance => cmd_repo_extensions_conformance(ctx),
    ```
  - [ ] Add handler:
    ```rust
    fn cmd_repo_extensions_conformance(ctx: CliContext) -> Result<String> {
        let report = with_store(&ctx, |store| Ok(declared_extensions_conformance(store)?))?;
        output::serialize(
            "repo extensions conformance",
            RepoExtensionsConformancePayload {
                declared: report.declared,
                supported: report.supported,
                declared_but_unsupported: report.declared_but_unsupported,
                used_but_undeclared: report.used_but_undeclared,
            },
        )
    }
    ```
  - [ ] Add import: `use srs_repository::manifest_service::declared_extensions_conformance;` (alongside the existing manifest_service imports).
  - [ ] Add `RepoExtensionsConformancePayload` to the payload import list.

- [ ] In `crates/srs-cli/src/payload.rs`:
  - [ ] Add struct (after `RepoExtensionsMutatePayload`):
    ```rust
    #[derive(Debug, Serialize, JsonSchema)]
    #[serde(rename_all = "camelCase")]
    pub struct RepoExtensionsConformancePayload {
        pub declared: Vec<String>,
        pub supported: Vec<String>,
        pub declared_but_unsupported: Vec<String>,
        pub used_but_undeclared: Vec<String>,
    }
    ```

- [ ] Run `cargo run --bin generate-schemas` and verify `crates/srs-cli/schemas/payload/RepoExtensionsConformancePayload.json` is created.

- [ ] Write unit tests in `manifest_service.rs` `#[cfg(test)]` module:
  - `conformance_empty_repo_reports_nothing_used_or_declared` — MemoryStore with no extensions, empty manifest; assert `declared` empty, `supported` has 7 entries, both diff lists empty.
  - `conformance_declared_but_unsupported_extension_is_flagged` — MemoryStore with `declaredExtensions: ["ext:nonexistent"]`; assert `declared_but_unsupported = ["ext:nonexistent"]`.
  - `conformance_supported_declared_extension_not_flagged` — MemoryStore with `declaredExtensions: ["ext:lifecycle"]`; assert `declared_but_unsupported` empty.
  - `conformance_lifecycle_state_detected_as_used` — MemoryStore with a record that has `lifecycle_state = Some("active")` in instance_index but no `declaredExtensions`; assert `used_but_undeclared` contains `"ext:lifecycle"`.
  - `conformance_declared_lifecycle_not_in_undeclared` — MemoryStore with `declaredExtensions: ["ext:lifecycle"]` and a record with lifecycle_state; assert `used_but_undeclared` empty.

  Note: Tests for relation/type-inheritance/field-groups/addressability usage detection are
  deferred to a follow-up (the MemoryStore test doubles for those paths are complex to set up
  without duplication with existing service tests). File a follow-up issue.

#### Acceptance Criteria

- [ ] `srs repo extensions conformance --repo <repo>` returns valid JSON with the four fields.
- [ ] `declared_but_unsupported` contains extension IDs in the manifest that are not in `SUPPORTED_EXTENSIONS`.
- [ ] `used_but_undeclared` contains extension IDs detected from repo content that are not in the manifest.
- [ ] `supported` always equals the canonical `SUPPORTED_EXTENSIONS` constant (7 entries).
- [ ] `cargo test --test payload_contracts` passes (golden schema matches struct).
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] All five new unit tests pass.

#### Testing

```bash
cargo test -p srs-repository conformance
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

Specific tests to write or verify:
- `conformance_empty_repo_reports_nothing_used_or_declared` — verifies base case
- `conformance_declared_but_unsupported_extension_is_flagged` — verifies declared-but-unsupported set
- `conformance_supported_declared_extension_not_flagged` — verifies no false positives
- `conformance_lifecycle_state_detected_as_used` — verifies `ext:lifecycle` usage detection
- `conformance_declared_lifecycle_not_in_undeclared` — verifies no false positives in used-but-undeclared

#### Milestone gate

1. All five acceptance criteria above are checked.
2. All five named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo test -p srs-cli
   cargo test --test payload_contracts
   cargo clippy -- -D warnings
   ```
4. Update plan checkboxes.
5. Commit: `feat(repository): add declared extensions conformance service (#237)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs repo extensions conformance` command works end-to-end on a test repo
- [ ] Golden schema file `crates/srs-cli/schemas/payload/RepoExtensionsConformancePayload.json` committed
- [ ] `SUPPORTED_EXTENSIONS` constant is the sole definition of the supported-extension set

## Coordination Rules

- Single agent executes all phases; no coordination needed.
- Agents keep to write scopes: service logic in `srs-repository`, CLI in `srs-cli`.
- At end of phase: verify all acceptance criteria, confirm tests pass, update checkboxes, commit.

## Assumptions

- `list_declared_extensions(store: &dyn RepositoryStore) -> Result<Vec<String>, RepositoryError>` **already exists** in `crates/srs-repository/src/manifest_service.rs` (line 8). The new service calls it directly; no new implementation needed.
- `MemoryStore` provides sufficient test coverage for the service layer; `FileStore` integration is covered by the dogfood run.
- `relation_service::load_relations` is `pub(crate)` and importable from `manifest_service.rs` since both are in `srs-repository`.
- The `store.list_files_recursive("records")` method is available on `RepositoryStore` (confirmed in `store.rs:304`).
- Loading all records from the index to check `lifecycle_state` is acceptable for conformance checks (not a hot path; repositories are small).
- All 5 detectable extensions (`ext:lifecycle`, `ext:relations`, `ext:type-inheritance`, `ext:field-groups`, `ext:addressability`) are implemented in `detect_used_extensions()`. Tests for relations/type-inheritance/field-groups/addressability are deferred to a follow-up issue; only `ext:lifecycle` detection has dedicated unit tests in this plan.
- The CLI handler return type is `Result<String>` using `output::serialize()` — this matches the existing `cmd_repo_extensions_list` pattern in `commands/repo.rs`, not the `output::ok()` shown in CLAUDE.md examples.
