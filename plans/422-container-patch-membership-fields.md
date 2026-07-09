# Plan: Fix ContainerPatch — add rootInstanceIds / memberInstanceIds + deny_unknown_fields

> **Issue:** srs-rust#422
> **Tracking:** [the-greenman/srs-rust#422](https://github.com/the-greenman/srs-rust/issues/422)

## Summary

`srs container update` accepts a JSON patch on stdin but silently drops `rootInstanceIds` and `memberInstanceIds` because `ContainerPatch` does not declare those fields. Additionally, `ContainerPatch` lacks `#[serde(deny_unknown_fields)]`, so any key not in the struct is dropped during deserialization rather than rejected — the caller has no signal the field was ignored. This plan adds the two missing fields, applies them in `update_container` with full `validate_container` enforcement, and hardens the deserialization to fail loudly on unknown keys. `identity_instance_id` is already present in `ContainerPatch` and applied correctly.

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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All validation in the service; typed input/output structs | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI handler deserialises stdin → service call → output::serialize | accepted |

No new ADRs required — this change implements existing ADRs. The `deny_unknown_fields` attribute is a serde-level correctness fix, not a new architectural decision.

**Out of scope:** Auditing other structs for `deny_unknown_fields` (no other `*Patch` structs exist in the codebase). Changing the `container create` stdin shape. WASM binding changes (the bindings call `update_container` directly and pass typed structs, not JSON patch; no WASM surface changes here).

---

## Contracts

### CLI output contract (ADR-011)

The `container update` command returns `ContainerPayload { container: Container }`. No struct shape changes — the full `Container` is already returned and includes all fields. No payload regeneration required.

### Entity schema sync

No changes to `srs/docs/schema/2.0/` — `rootInstanceIds`, `memberInstanceIds`, and `identityInstanceId` are already defined in the container schema. No sync needed.

---

## Scope

- Add `root_instance_ids: Option<Vec<String>>` and `member_instance_ids: Option<Vec<String>>` to `ContainerPatch` in `crates/srs-repository/src/container_service.rs`.
- Apply those fields in `update_container` before the existing schema + `validate_container` call.
- Add `#[serde(deny_unknown_fields)]` to `ContainerPatch`.
- Add regression tests that prove (a) `rootInstanceIds` and `memberInstanceIds` round-trip correctly through `container update`, and (b) an unknown field in the patch JSON returns a parse error rather than a silent no-op.

**Out of scope:**
- Changes to the CLI handler (it already uses `ContainerPatch` correctly; the fix is purely in the service struct)
- WASM bindings (no JSON patch deserialization surface)
- Other structs (no other `*Patch` structs exist)
- Changing the validation logic in `validate_container` (already correct)

---

## Phases

### Phase 1: Fix ContainerPatch

**Goal:** `ContainerPatch` accepts and applies `rootInstanceIds` and `memberInstanceIds`; unknown fields fail deserialization.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/container_service.rs`, update `ContainerPatch`:
  - Change `#[serde(rename_all = "camelCase")]` to `#[serde(rename_all = "camelCase", deny_unknown_fields)]`
  - Add `pub root_instance_ids: Option<Vec<String>>`
  - Add `pub member_instance_ids: Option<Vec<String>>`
- [ ] In `update_container`, apply the new fields after the existing `identity_instance_id` block:
  ```rust
  if let Some(v) = patch.root_instance_ids {
      container.root_instance_ids = if v.is_empty() { None } else { Some(v) };
  }
  if let Some(v) = patch.member_instance_ids {
      container.member_instance_ids = if v.is_empty() { None } else { Some(v) };
  }
  ```
  These lines go before the schema validation call so `validate_container` sees the updated state.

#### Milestone gate

- [ ] `cargo test -p srs-repository` — all existing tests pass.
- [ ] `cargo clippy -- -D warnings` — clean.

### Phase 2: Tests

**Goal:** Regression tests prove the fix and lock the behaviour.

**Agent:** Repository Service Worker

#### Tasks

Add to the existing `#[cfg(test)]` block in `container_service.rs`:

- [ ] `update_container_patches_root_instance_ids` — create container, patch `root_instance_ids: Some(vec!["aaa..."])`, assert field persists on re-read.
- [ ] `update_container_patches_member_instance_ids` — same for `member_instance_ids`.
- [ ] `update_container_clears_root_instance_ids_with_empty_vec` — patch `root_instance_ids: Some(vec![])`, assert field becomes `None` on re-read.
- [ ] `update_container_unknown_field_in_patch_returns_error` — attempt to deserialise `{"unknownField": "x"}` as `ContainerPatch` via `serde_json::from_str`, assert it returns an `Err`.

#### Milestone gate

- [ ] `cargo test -p srs-repository update_container` — all four new tests pass.
- [ ] `cargo test -p srs-repository` — no regressions.

---

## Final Acceptance

```bash
cargo test
cargo clippy -- -D warnings
# payload structs not changed — payload_contracts test still passes
cargo test --test payload_contracts
```

All must pass with zero warnings and zero test failures.
