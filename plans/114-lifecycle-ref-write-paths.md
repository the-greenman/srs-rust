# Plan: Fix lifecycle write paths to resolve `lifecycleRef` (#114)

## Summary

`record create` and `record transition` only resolve a Type's lifecycle from the inline `Type.lifecycle` field, ignoring `Type.lifecycleRef`. This means records whose Type binds its lifecycle via `lifecycleRef` (the RFC-006 referenceable form) are created without an initial state and cannot be transitioned. Validation already handles `lifecycleRef` correctly (via `package.resolve_lifecycle`). This plan factors the resolution pattern into a shared helper and fixes both write paths to use it.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan fixes a bug by applying the existing pattern used in `validation.rs` (checking `lifecycle_ref` before falling back to inline `lifecycle`) to the two write paths.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Business logic belongs in srs-repository, not srs-cli | accepted |
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Services address packages through Package methods, not raw paths | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command payload structs are added or changed. The `record create` and `record transition` commands return the same payload shapes as before. No schema regeneration required.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/`. No sync action required.

---

## Scope

- Add `EffectiveLifecycle<'a>` struct and `Package::effective_lifecycle(record_type: &'a RecordType) -> Option<EffectiveLifecycle<'a>>` method to `crates/srs-repository/src/package.rs`, re-exported from `crates/srs-repository/src/lib.rs`.
- Fix `create_record` in `crates/srs-repository/src/record_store.rs` (lines 133-136) to use `effective_lifecycle` for initial state resolution.
- Fix `transition_record_lifecycle` in `crates/srs-repository/src/record_store.rs` (lines 675-681) to use `effective_lifecycle` for lifecycle lookup.
- Add regression tests for both write paths exercising a `lifecycleRef`-bound Type.

**Out of scope:**
- Updating `validation.rs` to use the new helper (it already works correctly; future cleanup may unify, but is a separate concern).
- The `record update` silent-ignore of `lifecycleState` on stdin (separate concern per issue description; tracked in follow-up).
- Extending V4/V5/V9 validation at `create_record` time to also run against `lifecycle_ref`-bound lifecycles. Rationale: the validation block at lines 116-131 runs `validate_type_lifecycle` / `validate_type_lifecycle_v9` against `TypeLifecycle` (inline blocks, which have no separate validation pass). Standalone `Lifecycle` entities referenced via `lifecycleRef` are validated at package load time by the package loader. Re-running those checks at record-create time would be redundant. A follow-up issue will be filed to confirm this invariant is enforced in the package loader and to add a test for it.
- Distinguishing "no lifecycle" from "dangling lifecycleRef" in the `effective_lifecycle` return type. A dangling `lifecycle_ref` UUID would already be caught by package load validation before `create_record` is called. A follow-up issue will be filed to explicitly test this invariant and improve the error surface if needed.

---

## Phases

### Phase 1: Add `Package::effective_lifecycle` helper

**Goal:** `package.effective_lifecycle(record_type)` resolves the lifecycle from either `lifecycle_ref` or the inline `lifecycle` field, returning a unified borrowed view.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package.rs`, add the following struct immediately before the `impl Package` block. Use the imported type names (`LifecycleState`, `LifecycleTransition`) — confirm they are in scope via existing imports at the top of `package.rs` (or add `use srs_core::types::lifecycle::{LifecycleState, LifecycleTransition};` if not already imported):
  ```rust
  /// Unified view of a resolved lifecycle — returned by `Package::effective_lifecycle`.
  /// Borrows from either an inline `TypeLifecycle` or a standalone `Lifecycle`, depending
  /// on which the RecordType uses.
  pub struct EffectiveLifecycle<'a> {
      pub initial_state: &'a str,
      pub states: &'a [LifecycleState],
      pub transitions: &'a [LifecycleTransition],
  }
  ```

- [ ] In the `impl Package` block in `crates/srs-repository/src/package.rs`, add the method after `resolve_lifecycle_by_name` (around line 305). Note: both `'a` lifetimes must be the same so the borrow from `record_type.lifecycle` (inline branch) and from `self.lifecycles` (ref branch) are both covered:
  ```rust
  /// Resolve the effective lifecycle for a RecordType.
  ///
  /// Priority: `lifecycle_ref` (resolved via package's standalone lifecycles) > inline `lifecycle`.
  /// Returns `None` in two cases:
  /// - The type has neither `lifecycle` nor `lifecycle_ref`.
  /// - `lifecycle_ref` is set but the UUID does not resolve in this package (dangling ref —
  ///   this should have been caught at package load time; treat as no lifecycle).
  pub fn effective_lifecycle<'a>(
      &'a self,
      record_type: &'a RecordType,
  ) -> Option<EffectiveLifecycle<'a>> {
      if let Some(ref_id) = &record_type.lifecycle_ref {
          self.resolve_lifecycle(ref_id).map(|lc| EffectiveLifecycle {
              initial_state: &lc.initial_state,
              states: &lc.states,
              transitions: &lc.transitions,
          })
      } else {
          record_type.lifecycle.as_ref().map(|lc| EffectiveLifecycle {
              initial_state: &lc.initial_state,
              states: &lc.states,
              transitions: &lc.transitions,
          })
      }
  }
  ```

- [ ] In `crates/srs-repository/src/lib.rs`, add:
  ```rust
  pub use crate::package::EffectiveLifecycle;
  ```
  This makes the type nameable by callers outside the crate (bindings, future HTTP adapter).

- [ ] Add unit tests in the `#[cfg(test)]` block of `package.rs`:
  - `effective_lifecycle_inline_resolves` — RecordType with inline `lifecycle` (no `lifecycle_ref`); confirms `initial_state`, `states.len()`, `transitions.len()` match.
  - `effective_lifecycle_ref_resolves` — RecordType with `lifecycle_ref = Some("lc-ref-standalone-001")` and a `Lifecycle` in `package.lifecycles` with `id = "lc-ref-standalone-001"`; confirms `initial_state`, `states.len()`, `transitions.len()` match.
  - `effective_lifecycle_none_when_absent` — RecordType with neither; confirms `None`.
  - `effective_lifecycle_ref_wins_over_inline` — RecordType with both set; confirms `lifecycle_ref` branch returns the referenced lifecycle's `initial_state`, not the inline one's.

#### Acceptance Criteria

- [ ] `Package::effective_lifecycle` is accessible as a public method.
- [ ] `EffectiveLifecycle` is re-exported from `srs-repository` crate root.
- [ ] All four tests pass.
- [ ] No change to existing tests in `package.rs`.

#### Testing

```bash
cargo test -p srs-repository effective_lifecycle
```

Specific tests:
- `effective_lifecycle_inline_resolves`
- `effective_lifecycle_ref_resolves`
- `effective_lifecycle_none_when_absent`
- `effective_lifecycle_ref_wins_over_inline`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all four tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes `[x]` and commit:
   ```bash
   git commit -m "feat(srs-repository): add Package::effective_lifecycle helper (#114)"
   ```

---

### Phase 2: Fix `create_record` and `transition_record_lifecycle`

**Goal:** Both write paths use `package.effective_lifecycle(record_type)` so `lifecycleRef`-bound Types behave identically to inline-lifecycle Types.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/record_store.rs`, replace lines 133-136:
  ```rust
  // BEFORE
  let initial_lifecycle_state = record_type
      .lifecycle
      .as_ref()
      .map(|lc| lc.initial_state.clone());
  ```
  with:
  ```rust
  // AFTER
  let initial_lifecycle_state = package
      .effective_lifecycle(record_type)
      .map(|lc| lc.initial_state.to_string());
  ```

- [ ] In `crates/srs-repository/src/record_store.rs`, replace lines 675-681:
  ```rust
  // BEFORE
  let lifecycle =
      record_type
          .lifecycle
          .as_ref()
          .ok_or_else(|| RepositoryError::LifecycleNotDefined {
              id: instance_id.to_string(),
          })?;
  ```
  with:
  ```rust
  // AFTER
  let lifecycle = package
      .effective_lifecycle(record_type)
      .ok_or_else(|| RepositoryError::LifecycleNotDefined {
          id: instance_id.to_string(),
      })?;
  ```
  Note: all subsequent uses of `lifecycle` in `transition_record_lifecycle` access `.transitions`, `.states` as slice fields — these compile unchanged against `EffectiveLifecycle<'_>` since the struct exposes the same field names with compatible slice types.

- [ ] Add test helper function `make_store_with_lifecycle_ref() -> MemoryStore` in the `#[cfg(test)]` block of `record_store.rs`. This mirrors `make_store_with_lifecycle` with two differences:
  1. The `RecordType` has `lifecycle: None` and `lifecycle_ref: Some("lc-ref-standalone-001".to_string())`.
  2. The `Package` has a standalone `Lifecycle` added to `package.lifecycles` with `id = "lc-ref-standalone-001"` and the same `states` / `transitions` / `initial_state` as `make_store_with_lifecycle`.
  The matching UUID `"lc-ref-standalone-001"` must appear in both places.

- [ ] Add a `create_lc_ref_record(store: &MemoryStore) -> Record` helper that calls `create_record` against the `lifecycle_ref` type (type id `"type-lc-ref-001"`, version `1`, a title field value `"test"`).

- [ ] Add test `create_record_with_lifecycle_ref_sets_initial_state` — verifies `record.lifecycle_state == Some("draft")`.

- [ ] Add test `transition_with_lifecycle_ref_succeeds` — verifies `transition_record_lifecycle` promotes the record from `draft` to `active` via `by_transition: Some("promote")`.

- [ ] Add test `transition_with_lifecycle_ref_invalid_transition_fails` — verifies `transition_record_lifecycle` returns `Err(RepositoryError::LifecycleTransitionNotAllowed { .. })` when `to: Some("archived")` is passed from `draft` state (no `draft → archived` transition defined).

#### Acceptance Criteria

- [ ] `record create` for a `lifecycleRef`-bound Type produces `lifecycleState: "draft"` (the initial state).
- [ ] `record transition` for a `lifecycleRef`-bound Type succeeds for a valid transition.
- [ ] `record transition` for a `lifecycleRef`-bound Type returns `LifecycleTransitionNotAllowed` for an invalid hop.
- [ ] All existing lifecycle tests continue to pass (inline lifecycle path unaffected).
- [ ] No business logic added to `srs-cli`.

#### Testing

```bash
cargo test -p srs-repository
```

Specific tests:
- `create_record_with_lifecycle_ref_sets_initial_state`
- `transition_with_lifecycle_ref_succeeds`
- `transition_with_lifecycle_ref_invalid_transition_fails`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all three tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes `[x]` and commit:
   ```bash
   git commit -m "fix(srs-repository): resolve lifecycleRef in record create/transition write paths (#114)"
   ```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `create_record_with_lifecycle_ref_sets_initial_state` test exists and passes
- [ ] `transition_with_lifecycle_ref_succeeds` test exists and passes
- [ ] `transition_with_lifecycle_ref_invalid_transition_fails` test exists and passes
- [ ] Existing lifecycle tests (`create_record_sets_initial_lifecycle_state`, `transition_by_state_name_succeeds`, `transition_by_named_transition_succeeds`) all pass
- [ ] `EffectiveLifecycle` is re-exported from `srs-repository` crate root (`pub use crate::package::EffectiveLifecycle` in `lib.rs`)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Only `crates/srs-repository/` is modified — no changes to `srs-cli` or `srs-core`.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `Package` is already loaded in both `create_record` and `transition_record_lifecycle` before the lifecycle is accessed (confirmed: `let package = store.load_package()` at lines 107 and 667 respectively).
- The `EffectiveLifecycle` helper lives in `srs-repository` (not `srs-core`) because lifecycle resolution requires the Package context, which is a repository-layer concern. `srs-core` is I/O-free and has no `Package`.
- Validation already handles `lifecycleRef` correctly; we deliberately do not change `validation.rs` in this plan to keep the diff minimal.
- Standalone `Lifecycle` entities (referenced via `lifecycleRef`) are validated at package load time. The V4/V5/V9 validation block in `create_record` (lines 116-131) specifically targets inline `TypeLifecycle` blocks which have no separate validation pass; it is not needed for referenced lifecycles. A follow-up issue will confirm this invariant is enforced in the package loader.
- A dangling `lifecycle_ref` UUID (ref set but UUID not in package) would already be caught by package load validation. A follow-up issue will explicitly test this and improve the error surface if needed.
