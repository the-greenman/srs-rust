# Plan: WASM — expose is_visible_by_default per decision record (#222)

## Summary

`srs-web`'s Decision Log view contains a hardcoded TypeScript constant `HIDDEN_STATUSES = ["superseded", "abandoned"]` that drives its "hide by default" toggle (srs-web#58). The SRS engine already carries the correct authored default via `ContainerView.exclude_lifecycle_states` (ADR-020), but nothing joins this per-container list back to individual member records. This plan adds `is_visible_by_default: bool` to `ResolvedMember` in the `container_view_service`, computed at service time from the same `exclude_lifecycle_states` the container view already resolves. The web client can then delete `HIDDEN_STATUSES` entirely and read this field instead — no spec change, no new service surface.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-020](../docs/adr/020-resolve-view-authored-list-defaults.md) | `resolve_container_view` is the single surface carrying container list defaults; `exclude_lifecycle_states` sourced from the governing DocumentSection | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `srs-bindings` exposes typed service results via `to_js`; no logic duplication | accepted |
| [ADR-018](../docs/adr/018-container-view-column-source-precedence.md) | Governing section selects columns and exclusions through one precedence rule | accepted |

No new ADR required: this plan adds a derived field to an existing struct, following the pattern established by ADR-020. The field is purely a Layer-1 function of data already resolved by `resolve_container_view`.

---

## Contracts

### CLI output contract (ADR-011)

`ContainerView` is embedded in `ContainerViewPayload` as `#[schemars(with = "serde_json::Value")]` (see `crates/srs-cli/src/payload.rs:523`). Adding a field to `ContainerView` (or its nested `ResolvedMember`) does **not** affect the golden schema files — the payload schema treats the view as opaque JSON. No `cargo run --bin generate-schemas` required.

Verification: `cargo test --test payload_contracts` must still pass after this change.

### Entity schema sync (check-schema-sync.sh)

No `srs/docs/schema/2.0/` entity schemas are modified. No schema sync required.

---

## Scope

- Add `is_visible_by_default: bool` to `ResolvedMember` in `crates/srs-repository/src/container_view_service.rs`.
- Compute it in `resolve_container_view`, after `exclude_lifecycle_states` is known: `!exclude_lifecycle_states.contains(record.lifecycle_state)`. When `record.lifecycle_state` is `None`, default to `true` (no lifecycle state → not explicitly hidden).
- Update all tests in `container_view_service.rs` that construct `ResolvedMember` or assert on the struct shape.
- The WASM binding (`crates/srs-bindings/src/lib.rs`) needs no change — `resolve_container_view` already serialises the full `ContainerView` via `to_js`.

**Out of scope:**
- Adding `is_visible_by_default` to `DiscoveryHit` in `discovery_service.rs` — the `find` path already handles exclusion via `exclude_lifecycle_states` in the query; per-hit visibility is a separate concern.
- Modifying `srs-web` to delete `HIDDEN_STATUSES` — that is the downstream consumer change driven by this PR.
- Extending `LifecycleState` with a dedicated `visible_by_default` field (deferred — would require an RFC; the current hardcoded governance set is superseded/abandoned, which the authored `excludeLifecycleStates` in the governance package already captures).

---

## Phases

### Phase 1: Add is_visible_by_default to ResolvedMember

**Goal:** `ResolvedMember` carries `is_visible_by_default: bool`, computed from `exclude_lifecycle_states`, with all tests passing.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/container_view_service.rs`, add `pub is_visible_by_default: bool` to the `ResolvedMember` struct (after `display_label`, before `record`).
- [x] Add `exclude_lifecycle_states: &[String]` as a parameter to the private `resolve_member()` helper (lines ~203–220). Compute `is_visible_by_default` at construction time — do NOT use a post-hoc mutation loop, since that would leave the struct transiently wrong:
  ```rust
  fn resolve_member(
      store: &dyn RepositoryStore,
      id: &str,
      tier_by_id: &HashMap<String, u8>,
      field_name_index: &HashMap<String, String>,
      exclude_lifecycle_states: &[String],
      kind: &str,
      diagnostics: &mut Vec<String>,
  ) -> Result<Option<ResolvedMember>, RepositoryError>
  ```
  At the `ResolvedMember` construction site (currently line ~215–220):
  ```rust
  let is_visible_by_default = record.lifecycle_state.as_deref()
      .is_none_or(|s| !exclude_lifecycle_states.iter().any(|e| e == s));
  Ok(Some(ResolvedMember {
      instance_id: id.to_string(),
      tier: 2,
      display_label,
      is_visible_by_default,
      record,
  }))
  ```
- [x] Update all call sites of `resolve_member` in `resolve_container_view` to pass `&exclude_lifecycle_states`. Note: `exclude_lifecycle_states` is already computed at line ~151 before both the `root` (line ~162) and `members` (line ~175) resolution passes, so the ordering is correct.
- [x] Confirm all existing tests still compile after adding the field. No struct literal updates are required — existing tests call `resolve_container_view(...)` and inspect result fields, not construct `ResolvedMember` directly.
- [x] Add a dedicated test `resolve_view_is_visible_by_default_computed` (MemoryStore) that:
  - Creates a container view with `exclude_lifecycle_states: ["superseded", "abandoned"]` on the TypeQuery section.
  - Includes three members: one with `lifecycle_state: None`, one with `lifecycle_state: Some("active")`, one with `lifecycle_state: Some("superseded")`.
  - Asserts `is_visible_by_default: true` for the first two and `false` for the third.
- [x] Extend the existing roundtrip test `resolve_view_roundtrip_type_query_exclude_states` (or add a new roundtrip variant) to include a member with `lifecycle_state: Some("superseded")` and assert `is_visible_by_default: false` survives the memory → file → memory roundtrip (satisfies CLAUDE.md cross-store roundtrip rule).

#### Acceptance Criteria

- [x] `ResolvedMember.is_visible_by_default` is `false` when `record.lifecycle_state` is in `exclude_lifecycle_states`.
- [x] `ResolvedMember.is_visible_by_default` is `true` when `record.lifecycle_state` is `None` or not in `exclude_lifecycle_states`.
- [x] The root member in `ContainerView.root` also carries the correct `is_visible_by_default` (computed at construction via the new `resolve_member` parameter — no separate mutation step).
- [x] All pre-existing tests in `container_view_service.rs` compile and pass after the field is added.
- [x] New test `resolve_view_is_visible_by_default_computed` passes.
- [x] Roundtrip test covers the `is_visible_by_default: false` case (member with `lifecycle_state: "superseded"`) through memory → file stores.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `resolve_view_is_visible_by_default_computed` — proves the boolean is derived correctly (None→true, non-excluded→true, excluded→false) against MemoryStore.
- Roundtrip extension of `resolve_view_roundtrip_type_query_exclude_states` — proves `is_visible_by_default: false` survives memory → file serialisation.
- All existing `resolve_container_view` tests — proves no regression in column, member, and exclusion-list resolution.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm `resolve_view_is_visible_by_default_computed` exists and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `is_visible_by_default` is present in the serialised WASM output (verified in dogfooding: `resolve_container_view` JSON includes the field for each member)
- [ ] `is_visible_by_default` is `false` for members with `lifecycle_state: "superseded"` or `"abandoned"` when the container's section declares those states in `excludeLifecycleStates`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- The governance package's `DocumentView` section already declares `excludeLifecycleStates: ["superseded", "abandoned"]` on its TypeQuery source. If not, the field defaults correctly to `true` for all members (no exclusions → all visible by default).
- `srs-web` consuming this field (`member.isVisibleByDefault`) is the downstream change tracked in srs-web; it is not part of this PR.
