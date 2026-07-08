# Plan: refactor: introduce LoadedBlueprint wrapper (#223)

## Summary

`Package.blueprints` currently stores bare `Blueprint` values, discarding sub-package provenance during the merge in `load_package`. `LoadedProtocol` already carries `{ protocol, raw, source_package }` for this exact reason (introduced in #176). This plan adds a structural parallel: `LoadedBlueprint { blueprint, raw, source_package }` in `package.rs`, updates both `FileStore` and `JsonStore` loaders to populate it with the same first-boundary-wins merge logic used for protocols, and fixes all downstream call sites that read `Package.blueprints`. A consumer now exists (`list_blueprints_summary` in `blueprint_service.rs` already tracks `source_package` — the `Package.blueprints` path was the remaining asymmetry). No CLI output changes, no WASM binding changes, no spec change.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Package provenance is tracked in the in-memory `Package` struct, not in service or CLI layers | accepted |
| [ADR-016](../docs/adr/016-protocols-are-package-definitions.md) | Blueprint definitions are package definitions parallel to protocols; `LoadedBlueprint` mirrors `LoadedProtocol` | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions remain unchanged — `blueprint_service.rs` bypasses `Package.blueprints` entirely | accepted |

No new ADRs required — this plan implements an existing pattern.

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands are added or changed. `Package.blueprints` is an internal in-memory type used by the repository layer; it is not serialised into any CLI payload directly. **No payload struct changes; no schema regeneration needed.**

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. **No schema sync action required.**

---

## Scope

- Add `LoadedBlueprint { blueprint: Blueprint, raw: serde_json::Value, source_package: Option<String> }` to `crates/srs-repository/src/package.rs`
- Change `Package.blueprints: Vec<Blueprint>` to `Package.blueprints: Vec<LoadedBlueprint>`
- Update `FileStore::load_package` (in `crates/srs-repository/src/store.rs`) to populate `LoadedBlueprint` and set `source_package = Some(rel_path)` for sub-package blueprints
- Update `JsonStore::load_package` (in `crates/srs-repository/src/json_store.rs`) to populate `LoadedBlueprint` and set `source_package = Some(rel_path)` for sub-package blueprints
- Update `crates/srs-repository/src/repository_portability.rs` call site that converts `Package.blueprints → PackageBoundarySnapshot.blueprints` (extract `.blueprint` field)
- Add a test verifying `source_package` is `None` for root-package blueprints and `Some(path)` for sub-package blueprints

**Out of scope:**
- `PackageBoundarySnapshot.blueprints: Vec<Blueprint>` stays as-is (serde-compatible `.srsj` format; each boundary snapshot represents a single boundary and does not need provenance)
- `blueprint_service.rs` — already tracks source_package independently via boundary iteration; no changes
- WASM bindings — no bindings consume `Package.blueprints` directly
- CLI payload structs — unchanged
- srs-web — unchanged

---

## Phases

### Phase 1: Add `LoadedBlueprint` and update `Package`

**Goal:** `package.rs` defines `LoadedBlueprint` and `Package.blueprints` is `Vec<LoadedBlueprint>`; the codebase compiles with all call sites updated.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package.rs`, add `LoadedBlueprint` struct immediately after `LoadedProtocol` (lines 43–48):
  ```rust
  /// A blueprint as loaded from a package, bundling typed struct + verbatim JSON.
  ///
  /// `raw` preserves all fields from the on-disk JSON that are not captured by
  /// the typed `Blueprint` struct. `source_package` is `None` for the root package
  /// and `Some(path)` for blueprints merged from a dependency package.
  #[derive(Debug, Clone)]
  pub struct LoadedBlueprint {
      pub blueprint: Blueprint,
      pub raw: serde_json::Value,
      pub source_package: Option<String>,
  }
  ```
- [ ] Change `Package.blueprints: Vec<Blueprint>` (line 28) to `Package.blueprints: Vec<LoadedBlueprint>`
- [ ] In `crates/srs-repository/src/store.rs`, update `load_package_from_dir`:
  - At lines 755–768: change blueprint loading to capture `raw` first, then parse `blueprint`, then push `LoadedBlueprint { blueprint, raw, source_package: None }`
  - Update the function's return tuple type: change `Vec<Blueprint>` to `Vec<LoadedBlueprint>`
- [ ] In `crates/srs-repository/src/store.rs`, update the sub-package merge loop in `load_package` (lines 968–969 and 1095–1109):
  - Change initial `for bp in &blueprints` to `for lb in &blueprints` with `lb.blueprint.id`
  - Change `for bp in sub_blueprints` to `for mut lb in sub_blueprints`; use `lb.blueprint.id`, `lb.blueprint.name`; set `lb.source_package = Some(rel_path.to_string())` before pushing (exactly parallel to protocol handling at line 1128)
- [ ] In `crates/srs-repository/src/json_store.rs`, update `load_package_from_prefix` (lines 578–588):
  - Capture `raw: serde_json::Value` from `self.data_get(&full)?`, parse `blueprint` from `raw.clone()`, push `LoadedBlueprint { blueprint, raw, source_package: None }`
- [ ] In `crates/srs-repository/src/json_store.rs`, update sub-package merge loop (lines 920–926):
  - Change `for bp in sub_blueprints` to `for mut lb in sub_blueprints`; check `lb.blueprint.id`; set `lb.source_package = Some(rel_path.to_string())` before pushing
  - Fix type annotation on the `.any()` closure: change `|b: &srs_core::types::blueprint::Blueprint|` to `|b: &crate::package::LoadedBlueprint|` and use `b.blueprint.id`
- [ ] In `crates/srs-repository/src/repository_portability.rs` line 339, update:
  ```rust
  blueprints: pkg.blueprints,
  ```
  to:
  ```rust
  blueprints: pkg.blueprints.into_iter().map(|lb| lb.blueprint).collect(),
  ```
- [ ] Verify the crate compiles: `cargo build -p srs-repository`

#### Acceptance Criteria

- [ ] `LoadedBlueprint` exists in `package.rs` with fields `blueprint: Blueprint`, `raw: serde_json::Value`, `source_package: Option<String>`
- [ ] `Package.blueprints` type is `Vec<LoadedBlueprint>`
- [ ] `cargo build -p srs-repository` succeeds with zero errors

#### Testing

```bash
cargo build -p srs-repository
```

Specific tests to write or verify:
- Compilation — if it builds, the structural changes are consistent

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run: `cargo build -p srs-repository`
3. Update this plan's checkboxes.
4. Commit: `git commit` (message referencing #223)

---

### Phase 2: Add provenance test and run full test suite

**Goal:** A new test verifies `source_package` is correctly set for sub-package blueprints; all existing tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package.rs` (in the `#[cfg(test)]` module), add a test `loaded_blueprint_sub_package_sets_source_package`:
  - Create a temp dir with a minimal FileStore repo
  - Add a blueprint to the primary package
  - Create a sub-package directory with another blueprint
  - Register the sub-package in `manifest.json` via `packageRefs`
  - Call `FileStore::new(root).load_package()`
  - Assert the primary blueprint's `source_package` is `None`
  - Assert the sub-package blueprint's `source_package` equals `Some("package/ext")` (or whichever path was used)

#### Acceptance Criteria

- [ ] `test loaded_blueprint_sub_package_sets_source_package` exists in `package.rs` and passes
- [ ] `cargo test -p srs-repository` passes with zero failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify:
- `loaded_blueprint_sub_package_sets_source_package` — proves source_package is set for sub-package blueprints and None for root package blueprints

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run: `cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings`
3. Update this plan's checkboxes.
4. Commit: `git commit` (message referencing #223)

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `LoadedBlueprint` exists in `package.rs` with `blueprint`, `raw`, `source_package` fields
- [ ] `Package.blueprints` is `Vec<LoadedBlueprint>`
- [ ] A test verifies `source_package` is `None` for root-package blueprints and `Some(path)` for sub-package blueprints

## Coordination Rules

- Lead Integrator owns the `Package` field type and the `LoadedBlueprint` struct definition.
- Repository Service Worker implements all loader changes and call-site updates.
- Verification Agent confirms test coverage and checks for any remaining `Vec<Blueprint>` usage that should have been updated.

## Assumptions

- `PackageBoundarySnapshot.blueprints: Vec<Blueprint>` stays as-is (serde format compatibility)
- `blueprint_service.rs` requires no changes — it bypasses `Package.blueprints` entirely
- No WASM binding call sites consume `Package.blueprints` directly (confirmed by grep: no `.blueprints` access in `crates/srs-bindings/`)
