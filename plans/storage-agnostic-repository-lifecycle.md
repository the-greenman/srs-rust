# Plan: Storage-Agnostic Repository Lifecycle and Portability

## Summary

This plan establishes repository instantiation and full-repository portability as storage-boundary foundations. A new SRS repository must be creatable through `RepositoryStore` without service code writing files directly, and a complete logical repository assembled in MemoryStore must be materializable into FileStore or a future SqlStore. FileStore preserves the current on-disk layout; MemoryStore stores equivalent logical state without imitating paths.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Boundary Worker | — |
| Repository Instantiation Worker | — |
| Portability Worker | — |
| CLI Worker | — |
| Documentation Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

This plan updates the repository architecture rules and requires a new ADR.

| ADR | Decision | Status |
|---|---|---|
| TBD | Repository creation and full-repository copy are logical store operations; adapters own files/tables | proposed |

The Documentation Worker must create the ADR during Phase 1. These decisions extend ADR-008 rather than merely implementing it: repository creation is adapter-owned, full-repository portability is logical rather than filesystem-copy based, and services must not assume file-backed repository layout.

---

## Scope

- Add storage-agnostic repository instantiation through `RepositoryStore`.
- Add repository metadata/input/result types for creation and status checks.
- Preserve the current file-backed layout through FileStore.
- Update MemoryStore so new repositories can exist fully in memory without fake path keys.
- Add full-repository snapshot/copy portability between store implementations.
- Add `srs repo create` through the repository service.
- Add `srs repo copy` as the CLI consumer of full-repository portability.
- Update [ARCHITECTURE.md](../ARCHITECTURE.md) with repository lifecycle and portability rules.

**Out of scope:**

- SQL adapter implementation.
- Async store traits.
- Partial repository export/import.
- RFC-003 repository slices.
- Changing the current file-backed on-disk layout.
- Full project scaffolding beyond the minimal valid SRS repository.

**Ordering constraint:**

- This plan must run before `storage-agnostic-package-management.md` and `storage-agnostic-container-boundaries.md`.
- Package/container plans may depend on the repository lifecycle types and `RepositorySnapshot` shape introduced here.
- This plan may define only the primary package metadata needed for initial repository validity. Rich package selectors, per-package definition ownership, and package lifecycle operations belong to `storage-agnostic-package-management.md`.

---

## Phases

### Phase 1: Architecture Contract

**Goal:** Repository instantiation and full-repository portability are documented as storage-boundary responsibilities.

**Agent:** Lead Integrator + Documentation Worker

#### Tasks

- [ ] Update `srs-rust/ARCHITECTURE.md` with a repository lifecycle section.
- [ ] State that new repository creation is requested by services and implemented by adapters.
- [ ] State that full repository copy uses a logical snapshot, not file copying.
- [ ] Document that FileStore paths and future SQL tables are adapter details.
- [ ] Create an ADR under `srs-rust/docs/adr/` covering repository creation and full-repository portability.
- [ ] Document the ordering constraint: repository lifecycle first, package/container boundary plans after.

#### Acceptance Criteria

- [ ] `ARCHITECTURE.md` documents storage-agnostic repository creation.
- [ ] `ARCHITECTURE.md` documents full logical repository portability.
- [ ] ADR is created and linked from this plan's Architecture Decisions section.
- [ ] Ordering constraint is documented in this plan and architecture notes.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository
```

Specific tests to write or verify:

- None for this documentation phase.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

---

### Phase 2: Store Contract For Repository Lifecycle

**Goal:** `RepositoryStore` can create and inspect a repository without service-level file paths.

**Agent:** Repository Boundary Worker

#### Tasks

- [x] Add repository instantiation input/result types with repository identity metadata and minimal primary package metadata.
- [x] Add store methods:
  - `repository_exists`
  - `initialize_repository`
- [x] Implement the methods for FileStore using the current layout:
  - `.srs/`
  - `manifest.json`
  - `package/package.json`
  - empty package definition arrays
- [x] Implement the methods for MemoryStore using logical manifest and primary package state.
- [x] Add a typed `RepositoryAlreadyExists` error.
- [x] Keep `initialize_repository` intentionally narrow: it creates the repository envelope and initial primary package metadata only. Independent package creation/update behavior belongs to `storage-agnostic-package-management.md`.

#### Acceptance Criteria

- [x] A minimal valid repository can be initialized through `&dyn RepositoryStore`.
- [x] FileStore creates the current file-backed layout.
- [x] MemoryStore initializes without fake file path keys.
- [x] Duplicate creation returns `RepositoryAlreadyExists`.
- [x] `initialize_repository` does not expose package paths and does not implement general package lifecycle behavior.
- [x] Trait remains synchronous.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository repository_creation
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `create_repository_initializes_memory_store` — creates manifest and primary package boundary in memory.
- `create_repository_filestore_writes_minimal_layout` — creates `.srs/`, `manifest.json`, and `package/package.json`.
- `create_repository_rejects_existing_repository` — duplicate create fails deterministically.
- `created_repository_loads_effective_package` — created repo is immediately loadable through package resolution.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

---

### Phase 3: Repository Creation Service

**Goal:** Repository creation is exposed as reusable service logic and not as CLI/file code.

**Agent:** Repository Instantiation Worker

#### Tasks

- [x] Add repository lifecycle service functions:
  - `create_repository`
  - `get_repository_status` or equivalent existence/status check
- [x] `create_repository` must accept logical repository metadata and primary package metadata.
- [x] `create_repository` must call `store.repository_exists()` before `store.initialize_repository(...)`.
- [x] The service must not call `std::fs`, create directories, or write JSON files directly.
- [x] Validate enough metadata for immediate repo validation and package loading.
- [x] Add service-level tests against MemoryStore and FileStore.

#### Acceptance Criteria

- [x] Service code creates repositories only through `RepositoryStore`.
- [x] Existing-repository detection is performed by the store contract, not by service-level path checks.
- [x] Created repositories validate and load their primary package.
- [x] FileStore and MemoryStore produce equivalent logical repository state.
- [x] There is no `srs repo create` implementation that bypasses this service.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository repository_creation
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `create_repository_service_initializes_memory_store` — service works against MemoryStore.
- `create_repository_service_initializes_filestore` — service works against FileStore.
- `create_repository_service_rejects_duplicate` — duplicate create is stable.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

---

### Phase 4: Full Repository Portability

**Goal:** A complete logical repository can be copied from one store implementation into another.

**Agent:** Portability Worker

#### Tasks

- [x] Add `RepositorySnapshot` containing:
  - repository metadata and manifest-level configuration, including declared extensions
  - package boundaries, package metadata, and package definitions
  - containers and their member/root instance IDs
  - records/notes/typed records and relations
- [x] Define `RepositorySnapshot` as a logical DTO: it must contain SRS identities, metadata, package definitions, instances, relations, and containers, but no file paths, package-relative paths, directory names, or backend locators.
- [x] Add repository portability service functions:
  - `export_repository_snapshot(source: &dyn RepositoryStore)`
  - `import_repository_snapshot(target: &dyn RepositoryStore, snapshot: RepositorySnapshot)`
  - `copy_repository(source: &dyn RepositoryStore, target: &dyn RepositoryStore)`
- [x] Use logical store APIs only; do not use file paths or raw directory traversal.
- [x] Add any store enumeration methods needed to export a complete repository logically; do not fall back to `MemoryStore` path-key iteration or FileStore directory walking from service code.
- [x] Preserve repository identity by default when copying a full repository.
- [x] Importing into a non-empty target returns a typed error unless a future explicit replace mode is added.

#### Acceptance Criteria

- [x] A repository created in MemoryStore can be copied into FileStore.
- [x] The copied FileStore repository validates and loads the same effective package.
- [x] Declared extensions survive the copy.
- [x] Package definitions, records, containers, and relations survive the copy.
- [x] No portability service accepts or returns filesystem paths.
- [x] `RepositorySnapshot` is backend-neutral and can be implemented by a future SqlStore without path translation hacks.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository repository_portability
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `copy_memory_repo_to_filestore_preserves_manifest_and_extensions` — declared extensions and repository metadata survive.
- `copy_memory_repo_to_filestore_preserves_packages` — package boundaries and definitions survive.
- `copy_memory_repo_to_filestore_preserves_records_and_containers` — content and segmentation survive.
- `copy_repository_rejects_non_empty_target` — import does not overwrite existing target implicitly.
- `copied_repository_validates` — FileStore target passes repository validation.
- `repository_snapshot_contains_no_paths` — snapshot has no file paths or backend locators.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

---

### Phase 5: CLI

**Goal:** Repository lifecycle is available through CLI without direct file writes in CLI handlers.

**Agent:** CLI Worker

#### Tasks

- [x] Add `srs repo create`.
- [x] Add `srs repo copy` as the CLI surface for full-repository materialization.
- [x] CLI handlers construct stores and call repository lifecycle/portability services.
- [x] CLI handlers must not create directories, write manifest files, or copy files directly.
- [x] Document CLI creation flow:
  - for `srs repo create --repo <target>`, CLI does not call `detect.rs`; it constructs `FileStore` at the explicit target and calls `create_repository`
  - for `srs repo create` without `--repo`, CLI uses the current working directory as the target and calls `create_repository`
  - commands that operate on an existing repo continue using `detect.rs` / `find_repo_root`
- [x] Preserve existing JSON envelope conventions.

#### Acceptance Criteria

- [x] `srs repo create` creates a minimal valid file-backed repository.
- [x] Duplicate repo creation returns a stable error envelope.
- [x] `srs repo copy` uses portability services and performs no direct file copying in CLI code.
- [x] `srs repo create` does not use `detect.rs` to require an already existing repo.
- [x] CLI output format remains compatible.

#### Testing

```bash
cd srs-rust
cargo test -p srs --test integration_tests -- repo_
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:

- `repo_create_happy_path` — creates a minimal valid file-backed repository.
- `repo_create_without_existing_repo_does_not_call_detect` — new repo creation works where no `.srs/` exists.
- `repo_create_existing_repo_errors` — duplicate creation returns stable error envelope.
- `repo_copy_memory_fixture_to_filestore` — full logical repository materializes into file-backed target.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test
cargo clippy -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] `ARCHITECTURE.md` documents storage-agnostic repository lifecycle and portability.
- [ ] ADR for repository lifecycle and portability is created.
- [x] Repository instantiation is implemented through `RepositoryStore`.
- [x] MemoryStore can host a newly created repository without fake filesystem assumptions.
- [x] FileStore preserves current on-disk layout for newly created repositories.
- [x] A repository assembled in MemoryStore can be materialized into FileStore.
- [x] `RepositorySnapshot` is path-free and backend-neutral.
- [ ] A future SQL adapter can implement repository lifecycle and portability without changing service APIs.

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behavior summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- There is currently no implemented `srs repo create`; this plan adds it as the canonical consumer of repository instantiation.
- Full repository portability is not RFC-003 repository slicing; slices remain out of scope.
- This plan runs before package and container boundary plans.
- The current file-backed repository layout remains supported.
- SQL implementation is not part of this plan.
- Async storage traits remain out of scope.
