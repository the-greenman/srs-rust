# Plan: Storage-Agnostic Package Boundaries

## Summary

This plan refactors package management so packages are logical definition boundaries rather than filesystem-shaped subdirectories. Packages segment reusable definitions and metadata; services must address them through package selectors and boundary models while FileStore, MemoryStore, and a future SqlStore map those operations to their own storage representation.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Package Boundary Worker | — |
| Package Service Worker | — |
| CLI Worker | — |
| Documentation Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

This plan depends on the repository lifecycle/storage boundary architecture and may require the same ADR to be extended.

| ADR | Decision | Status |
|---|---|---|
| TBD | Services address packages through logical selectors; adapters own package files/tables | proposed |

The Documentation Worker must either update the ADR from `storage-agnostic-repository-lifecycle.md` or explicitly record in `ARCHITECTURE.md` why the architecture rule is sufficient.

---

## Scope

- Add package selectors and package boundary models.
- Split package reads into effective merged package view and per-package boundary view.
- Preserve current file-backed package layout through FileStore adapter translation.
- Update MemoryStore so package tests prove storage independence instead of imitating package paths.
- Add package lifecycle services: create, import local, update metadata, list.
- Add package-aware field/type list, create, update, and delete behavior.
- Add CLI commands for package lifecycle and package-filtered field/type listing.
- Update [ARCHITECTURE.md](../ARCHITECTURE.md) with package boundary rules if not already covered by the repository lifecycle plan.

**Out of scope:**

- Repository creation and full-repository portability.
- Container storage alignment.
- SQL adapter implementation.
- Registry or network-backed package import.
- Repository slice export/import from RFC-003.
- Changing the current file-backed package layout.

---

## Phases

### Phase 1: Architecture Contract

**Goal:** The package boundary model is documented before implementation.

**Agent:** Lead Integrator + Documentation Worker

#### Tasks

- [ ] Update `srs-rust/ARCHITECTURE.md` with package boundary rules.
- [ ] State that package services use selectors and adapters use files/tables.
- [ ] Document packages as definition/meta boundaries.
- [ ] Document that raw `package.json`, package paths, and package file indexes must not appear in service APIs.
- [ ] Decide whether to add or update an ADR under `srs-rust/docs/adr/`.

#### Acceptance Criteria

- [ ] `ARCHITECTURE.md` names packages as logical definition boundaries.
- [ ] `ARCHITECTURE.md` prohibits path-shaped package service APIs.
- [ ] ADR decision is either created/updated or explicitly deferred with rationale.

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

### Phase 2: Package Boundary Types and Store Contract

**Goal:** `RepositoryStore` exposes logical package operations rather than raw package file operations.

**Agent:** Package Boundary Worker

#### Tasks

- [ ] Add package boundary types in `srs-repository`:
  - `PackageSelector`
  - `PackageBoundary`
  - `EffectivePackage`
  - `DefinitionOwner`
- [ ] Add store methods for package operations:
  - `load_effective_package`
  - `list_package_boundaries`
  - `load_package_boundary`
  - `register_package_boundary`
  - `save_package_boundary_metadata`
  - `save_definition`
  - `delete_definition`
  - `resolve_definition_owner`
- [ ] Mark old raw package path methods as transitional:
  - `load_package_json`
  - `save_package_json`
  - `validate_package_ref_path`
  - field/type/view save methods that accept package-relative paths
- [ ] Implement new package methods for MemoryStore using package identity keys, not fake file paths.
- [ ] Implement new package methods for FileStore by translating selectors to the current `package/` and `packageRefs` layout.

#### Acceptance Criteria

- [ ] New package methods are sufficient for package services without paths.
- [ ] MemoryStore package state is keyed by package identity/selector.
- [ ] FileStore preserves current package layout.
- [ ] Trait remains synchronous.
- [ ] All implementers compile.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository store package
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `memory_store_package_boundaries_are_keyed_by_selector` — proves MemoryStore is not path-shaped.
- `file_store_package_boundary_maps_existing_layout` — proves FileStore preserves current layout.
- `load_effective_package_retains_definition_provenance` — merged package keeps source info.
- `resolve_definition_owner_finds_primary_and_imported_package` — owner lookup works across packages.

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

### Phase 3: Package Lifecycle Services

**Goal:** Package lifecycle behavior is implemented through logical package boundaries.

**Agent:** Package Service Worker

#### Tasks

- [ ] Add package lifecycle service functions:
  - `create_package`
  - `import_package_local`
  - `update_package_metadata`
  - `list_packages`
- [ ] Make `create_package` auto-register the new package boundary.
- [ ] Make `import_package_local` validate local source through adapter behavior, then register by package identity.
- [ ] Make `update_package_metadata` metadata/binding-only; it must not sync or overwrite definitions.
- [ ] Package services must not call `std::fs`.
- [ ] Package services must not accept filesystem paths as package selectors.

#### Acceptance Criteria

- [ ] Package lifecycle behavior is callable through `&dyn RepositoryStore`.
- [ ] Package services contain no `std::fs` usage.
- [ ] Package services do not expose raw `package.json` operations.
- [ ] Package import identity/version mismatch returns deterministic errors.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository package_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `create_package_auto_registers_boundary` — package create registers logical package.
- `import_package_local_registers_logical_package` — local import resolves to package identity.
- `update_package_metadata_does_not_rewrite_definitions` — metadata update is not content sync.
- `import_package_rejects_identity_mismatch` — import conflict is deterministic.

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

### Phase 4: Package-Aware Definition Services

**Goal:** Field/type operations can target and report package boundaries without filename lookup.

**Agent:** Package Service Worker

#### Tasks

- [ ] Refactor field/type create to accept optional `PackageSelector`, defaulting to primary package.
- [ ] Refactor field/type update/delete to use `resolve_definition_owner`.
- [ ] Add `sourcePackage` provenance to field/type summaries.
- [ ] Add `list_fields_by_package` and `list_types_by_package`.
- [ ] Preserve existing namespace filtering and merged/global unfiltered lists.

#### Acceptance Criteria

- [ ] Field/type update/delete no longer search filenames.
- [ ] Unfiltered field/type list remains merged/global.
- [ ] Package-filtered field/type list respects package boundary.
- [ ] Field/type summaries include stable package provenance.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository package_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `list_fields_by_package_filters_correctly` — field list respects package selector.
- `list_types_by_package_filters_correctly` — type list respects package selector.
- `list_fields_includes_source_package` — field summaries expose provenance.
- `list_types_includes_source_package` — type summaries expose provenance.
- `update_field_resolves_owner_package` — update routes through definition owner.
- `delete_type_resolves_owner_package` — delete routes through definition owner.

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

**Goal:** Package lifecycle and package-aware definition queries are available through thin CLI handlers.

**Agent:** CLI Worker

#### Tasks

- [ ] Add CLI commands:
  - `srs package create`
  - `srs package import`
  - `srs package update`
  - `srs package list`
  - `srs slice create`
- [ ] Add package filters:
  - `srs field list --package <selector>`
  - `srs type list --package <selector>`
- [ ] Route existing `package enable/disable` behavior through package lifecycle services or explicitly deprecate it.
- [ ] Migrate remaining direct `load_package(&Path)` callers to `store.load_effective_package`.
- [ ] CLI handlers must parse args, construct stores, call services, and format JSON envelopes only.

#### Acceptance Criteria

- [ ] CLI package lifecycle commands use repository services.
- [ ] `srs slice create` behaves identically to package create.
- [ ] Field/type package filters work through package selectors.
- [ ] No production service caller uses `package::load_package(&Path)`.
- [ ] Existing CLI output envelopes remain compatible.

#### Testing

```bash
cd srs-rust
cargo test -p srs --test integration_tests -- package_
cargo test -p srs --test integration_tests -- field_
cargo test -p srs --test integration_tests -- type_
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:

- `package_create_happy_path` — creates and registers package boundary.
- `package_import_local_happy_path` — imports local package source.
- `package_update_metadata_only` — updates metadata without definition sync.
- `slice_create_alias_matches_package_create` — alias parity.
- `field_list_with_package_filter` — package-filtered fields.
- `type_list_with_package_filter` — package-filtered types.

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
- [ ] `ARCHITECTURE.md` documents logical package boundaries.
- [ ] Package services do not expose filesystem paths or raw `package.json` operations.
- [ ] MemoryStore proves package behavior without fake filesystem assumptions.
- [ ] FileStore preserves current package layout.
- [ ] Field/type package provenance and package filtering work.
- [ ] A future SQL adapter can implement package behavior without changing service APIs.

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behavior summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Repository lifecycle and full-repository portability are handled by `storage-agnostic-repository-lifecycle.md`.
- Container storage alignment is handled by `storage-agnostic-container-boundaries.md`.
- The current file-backed package layout remains supported.
- `package import` v1 supports local source only.
- `package update` v1 is metadata/binding only.
- `slice create` is a CLI alias for package creation in this phase.
- SQL implementation is not part of this plan.
