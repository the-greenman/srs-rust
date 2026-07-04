# Plan: validate FieldRef.typeId in blueprint brief

## Summary

`validate_contributes_to` in `blueprint_brief_service.rs` validates `FieldRef.fieldId` but
silently ignores a non-null `typeId` that doesn't exist in the package. This was explicitly
deferred in #189 (the fieldId fix). A `contributesTo` entry pointing to a ghost typeId renders
as `nonexistent-type/field-id` in the markdown output with no warning. This plan adds the
missing check and a test covering the new behaviour.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification Agent | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All validation belongs in the service; diagnostics are non-fatal | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct changes; `BlueprintBriefResult.diagnostics` already covers this | accepted |

No new ADRs required — this plan implements ADR-010 by adding a missing cross-entity check
inside an existing service function.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `BlueprintBriefResult.diagnostics` already exists and is
serialized unchanged. No payload struct changes; no schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/`. No sync step needed.

---

## Scope

- Extend `validate_contributes_to` in
  `crates/srs-repository/src/blueprint_brief_service.rs` to check `FieldRef.type_id` when
  non-null by calling `get_type_by_id_latest(store, type_id)`.
- Add a non-fatal diagnostic when the typeId is not found: consistent with existing `fieldId`
  diagnostic format.
- Add one test: stage with valid `fieldId` but nonexistent `typeId` → diagnostic emitted,
  stage still present in result.

**Out of scope:**
- No code change needed in `srs-bindings`; the new diagnostic is available there automatically
  because `srs-bindings` calls the same `blueprint_brief` service function (ADR-013).
- Validation of `typeId` in other services or the CLI.
- Checking version-pinned TypeRef (type_version) — the existing code uses
  `get_type_by_id_latest` which ignores version; consistent with all other typeId resolution in
  this file.
- Changing the rendering of unresolved typeId references in `render_brief_markdown` — it
  already renders them as-is (no change needed).

---

## Phases

### Phase 1: Extend validate_contributes_to + add test

**Goal:** `validate_contributes_to` emits a diagnostic for any non-null `typeId` that cannot be
resolved in the package, and one test covers the new behaviour.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `validate_contributes_to` (line 333–351 of
  `crates/srs-repository/src/blueprint_brief_service.rs`), add a check after the `fieldId`
  resolution: for each `field_ref` with a `Some(type_id)`, call
  `get_type_by_id_latest(store, type_id)`. If `GetTypeResult::NotFound`, push:
  `format!("contributesTo type {} not found in package", type_id)` onto `diagnostics`.
- [x] Remove the stale comment `// FieldRef.type_id (optional) is not validated here ...`.
- [x] Add test `brief_unresolved_type_id_in_contributes_to_is_diagnostic` in the existing
  `#[cfg(test)] mod tests` block, following the pattern of the existing
  `brief_unresolved_field_ref_in_contributes_to_is_diagnostic` test:
  - Store contains `field-aaa` (valid) and `type-111` (valid).
  - Import a protocol with a stage that has
    `contributesTo: [{"fieldId": "field-aaa", "typeId": "nonexistent-type"}]`.
  - Call `blueprint_brief`; assert result is Ok.
  - Assert `result.protocol` is Some and stage is present.
  - Assert `result.diagnostics` contains an entry mentioning `"nonexistent-type"`.
  - Assert no diagnostic mentions `"field-aaa"`.
- [x] A second test `brief_valid_type_id_in_contributes_to_no_diagnostic` confirms that a
  valid existing `typeId` produces no diagnostic:
  - Store has `field-aaa`, `field-bbb`, and `type-111` (the `make_article_type()` fixture).
  - Stage has `contributesTo: [{"fieldId": "field-aaa", "typeId": "type-111"}]`.
  - Assert `result.diagnostics` is empty.

#### Acceptance Criteria

- [x] Passing `typeId: "nonexistent-type"` in a stage's `contributesTo` causes a diagnostic
  containing `"nonexistent-type"` in `BlueprintBriefResult.diagnostics`.
- [x] The stage is still present in the result (non-fatal, consistent with fieldId behaviour).
- [x] A valid `typeId` present in the package produces no diagnostic.
- [x] The stale comment is removed.
- [x] All existing tests continue to pass.

#### Testing

```bash
cargo test -p srs-repository blueprint_brief
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `brief_unresolved_type_id_in_contributes_to_is_diagnostic` — proves the diagnostic is emitted
- `brief_valid_type_id_in_contributes_to_no_diagnostic` — proves valid typeId is not flagged
- All pre-existing `test_brief_*` and `brief_stage_*` tests pass (no regression)

A cross-store roundtrip test is not added because the new logic is entirely in the service
layer and calls `get_type_by_id_latest` through the `RepositoryStore` trait with no
storage-layer branching. The identical `fieldId` path has the same MemoryStore-only coverage,
and `get_type_by_id_latest` is already exercised against `FileStore` in `package_service`
tests.

#### Milestone gate

1. All acceptance criteria checked above.
2. Both new tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository blueprint_brief
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark checkboxes `[x]`, commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] Both new tests exist and pass
- [ ] No regression in existing blueprint brief tests

## Coordination Rules

- Repository Service Worker modifies only `crates/srs-repository/src/blueprint_brief_service.rs`.
- No changes to `srs-cli`, `srs-core`, `srs-bindings`, or any schema files.

## Assumptions

- `get_type_by_id_latest` is the right resolution function — it already resolves the latest
  version of a type by ID, consistent with how root types are resolved in `resolve_brief_type`
  when `type_version` is None.
- There is no version-pinned typeId case in `FieldRef` (FieldRef only has `typeId`, no
  `typeVersion`). If a version field is added in future, that is out of scope here.
