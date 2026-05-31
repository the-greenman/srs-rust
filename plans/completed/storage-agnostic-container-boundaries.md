# Plan: Storage-Agnostic Container Boundaries

## Summary

This plan aligns container storage with the repository's storage-agnostic boundary model. Containers segment content instances through `containerId`, `rootInstanceIds`, and `memberInstanceIds`; service code should operate on those logical identifiers while FileStore owns the current `containers/*.json` layout and MemoryStore stores containers by ID.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Container Boundary Worker | — |
| Container Service Worker | — |
| CLI Worker | — |
| Documentation Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

This plan depends on the repository lifecycle/storage boundary architecture and may require the same ADR to be extended.

| ADR | Decision | Status |
|---|---|---|
| TBD | Services address containers through stable container IDs; adapters own storage indexes | proposed |

The Documentation Worker must either update the ADR from `storage-agnostic-repository-lifecycle.md` or explicitly record in `ARCHITECTURE.md` why the architecture rule is sufficient.

---

## Scope

- Move container path/index handling behind `RepositoryStore`.
- Keep container domain semantics unchanged.
- Preserve current file-backed `containers/*.json` layout through FileStore.
- Update MemoryStore so container tests prove storage independence instead of imitating container file paths.
- Keep existing container CLI behavior stable.
- Update [ARCHITECTURE.md](../ARCHITECTURE.md) with container boundary rules if not already covered by the repository lifecycle plan.

**Out of scope:**

- Package boundary work.
- Repository lifecycle and full-repository portability.
- Changing container schema or membership semantics.
- SQL adapter implementation.
- Async store traits.

---

## Phases

### Phase 1: Architecture Contract

**Goal:** The container boundary model is documented before implementation.

**Agent:** Lead Integrator + Documentation Worker

#### Tasks

- [x] Update `srs-rust/ARCHITECTURE.md` with container boundary rules.
- [x] State that container services use `containerId` and instance IDs.
- [x] State that FileStore paths and container indexes are adapter details.
- [x] Document containers as content-instance boundaries, distinct from package definition boundaries.
- [x] Decide whether to add or update an ADR under `srs-rust/docs/adr/`: no separate ADR — same adapter-owns-storage principle as ADR-009, documented in ARCHITECTURE.md.

#### Acceptance Criteria

- [x] `ARCHITECTURE.md` names containers as logical content boundaries.
- [x] `ARCHITECTURE.md` prohibits path-shaped container service APIs.
- [x] ADR decision is either created/updated or explicitly deferred with rationale.

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

### Phase 2: Container Store Contract

**Goal:** `RepositoryStore` exposes logical container operations rather than raw container file operations.

**Agent:** Container Boundary Worker

#### Tasks

- [x] Add `ContainerSelector` or equivalent container ID wrapper if useful for API consistency.
- [x] Add store methods for container operations:
  - `load_container`
  - `save_container`
  - `delete_container`
  - `list_container_summaries`
- [x] Move `ContainerIndexEntry.path` lookup behind FileStore.
- [x] Mark old raw container path methods as transitional:
  - `load_container_json`
  - `save_container_json`
  - `delete_container_file`
  - `ensure_containers_dir`
- [x] Implement new container methods for MemoryStore using `container_id` keys.
- [x] Implement new container methods for FileStore by preserving the current `containers/*.json` layout.

#### Acceptance Criteria

- [x] New container methods are sufficient for container services without paths.
- [x] MemoryStore stores containers by `container_id`.
- [x] FileStore preserves current container layout.
- [x] Trait remains synchronous.
- [x] All implementers compile.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository store container
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `memory_store_container_operations_are_keyed_by_id` — proves container storage is logical.
- `file_store_container_adapter_preserves_existing_layout` — proves FileStore compatibility remains.
- `container_store_list_summaries_uses_logical_ids` — summaries do not expose paths.

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

### Phase 3: Container Service Refactor

**Goal:** Container service functions operate only on logical container IDs and instance IDs.

**Agent:** Container Service Worker

#### Tasks

- [x] Refactor container service functions to use store-level container operations by `container_id`.
- [x] Remove service-owned path lookup from `ContainerIndexEntry`.
- [x] Keep roots and members as instance IDs.
- [x] Keep validation semantics unchanged.
- [x] Keep CLI-visible behavior unchanged.

#### Acceptance Criteria

- [x] Container service APIs do not expose file paths.
- [x] Container CRUD and membership operations use `container_id` at the service boundary.
- [x] Container membership/root behavior is unchanged.
- [x] No production container service code calls raw container path methods.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository container_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `create_container_uses_logical_id_boundary` — service creates through container ID.
- `update_container_does_not_require_path_lookup_in_service` — path lookup is adapter-owned.
- `container_membership_unchanged` — member/root behavior is preserved.
- `validate_container_invariants_unchanged` — existing validation remains intact.

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

### Phase 4: CLI Regression

**Goal:** Existing container CLI behavior remains stable while service internals become storage-agnostic.

**Agent:** CLI Worker + Verification

#### Tasks

- [x] Verify existing container CLI handlers remain thin wrappers.
- [x] Update handlers only if service signatures change.
- [x] Preserve existing JSON envelope conventions.
- [x] Add or update integration tests for container CRUD and membership.

#### Acceptance Criteria

- [x] Container CLI output format remains compatible.
- [x] Container CLI handlers do not perform direct file/index manipulation.
- [x] Existing container workflows still pass.

#### Testing

```bash
cd srs-rust
cargo test -p srs --test integration_tests -- container_
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:

- `container_cli_create_get_update_delete` — CRUD behavior remains stable.
- `container_cli_membership_regression` — member/root commands remain stable.

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

- [x] `cargo test` passes with no failures.
- [x] `cargo clippy -- -D warnings` passes.
- [x] `ARCHITECTURE.md` documents logical container boundaries.
- [x] Container services do not expose storage paths.
- [x] MemoryStore stores containers by ID.
- [x] FileStore preserves current `containers/*.json` layout.
- [x] Container CLI behavior remains compatible.
- [x] A future SQL adapter can implement container behavior without changing service APIs.

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behavior summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Repository lifecycle and full-repository portability are handled by `storage-agnostic-repository-lifecycle.md`.
- Package boundary work is handled by `storage-agnostic-package-management.md`.
- Container schema and membership semantics do not change.
- SQL implementation is not part of this plan.
- Async storage traits remain out of scope.
