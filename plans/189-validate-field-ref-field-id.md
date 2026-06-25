# Plan: fix(protocol): validate FieldRef.fieldId values resolve to package fields (#189)

## Summary

`ProtocolStage.contributesTo` carries `FieldRef[]` values whose `fieldId`s are passed through to `BlueprintBriefResult` without being validated against the package. A typo in `contributesTo.fieldId` silently produces a nonsensical label in `blueprint brief` and leaves no diagnostic. This plan adds the missing validation in `blueprint_brief_service.rs`, consistent with the existing type-ref validation pattern and ADR-010 (validation belongs in the service, not the handler).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this pipeline) |
| Repository Service Worker | Phase 1 |
| Verification | Stage 7 |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation lives in the service (`srs-repository`), not the CLI handler. The `contributes_to` FieldRef check belongs in `find_protocol_for_roots`. | accepted |

No new ADRs required. This plan implements existing ADR-010 validation rules for a previously unvalidated field.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. The `BlueprintBriefResult.diagnostics` field already exists and is the established channel for non-fatal advisory messages. No payload struct changes; no golden schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No entity schemas are added or modified. No action required.

---

## Scope

- In `crates/srs-repository/src/blueprint_brief_service.rs`, add validation of `contributes_to` FieldRef values in `find_protocol_for_roots`. For each `FieldRef` in a stage's `contributes_to`, call `get_field_by_id(store, &field_ref.field_id)`; if `GetFieldResult::NotFound`, push a diagnostic `"contributes_to field <id> not found in package"`. Continue rendering; unresolved refs keep their `fieldId` as the label.
- Add one MemoryStore test: `brief_unresolved_field_ref_in_contributes_to_is_diagnostic` — verifies a protocol stage with a bad `fieldId` in `contributesTo` produces a diagnostic and the stage is still included in the result.

**Out of scope:**

- Validating `typeId` within `FieldRef` (not part of this issue; a separate enhancement if needed).
- Removing or filtering out unresolved FieldRefs from the result (the issue specifies continue rendering).
- Changes to `render_brief_markdown` — it already renders bare `fieldId` for unresolved refs.
- Changes to any crate other than `srs-repository`.

---

## Phases

### Phase 1: Add FieldRef validation in `find_protocol_for_roots`

**Goal:** Unresolved `contributes_to` FieldRef entries produce a diagnostic in `BlueprintBriefResult.diagnostics`; the stage and its other fields are still included.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/blueprint_brief_service.rs`, add a private helper:

```rust
fn validate_contributes_to(
    store: &dyn RepositoryStore,
    contributes_to: &Option<Vec<FieldRef>>,
    diagnostics: &mut Vec<String>,
) -> Result<(), RepositoryError> {
    if let Some(refs) = contributes_to {
        for field_ref in refs {
            if let GetFieldResult::NotFound = get_field_by_id(store, &field_ref.field_id)? {
                diagnostics.push(format!(
                    "contributesTo field {} not found in package",
                    field_ref.field_id
                ));
            }
        }
    }
    Ok(())
}
```

- [ ] In `find_protocol_for_roots`, after building each `BriefStageResult` via `BriefStageResult::from(stage)`, call `validate_contributes_to(store, &brief_stage.contributes_to, diagnostics)?`. Do this inside the `into_iter().map(...)` chain or by converting to a `for` loop to allow the `?` operator and `&mut diagnostics` borrow.

  The cleanest form — replace the `.map(BriefStageResult::from).collect()` with a `for` loop, preserving the existing `sort_by_key` call:

```rust
let mut stages: Vec<BriefStageResult> = Vec::new();
for stage in proto_raw.stages {
    let brief_stage = BriefStageResult::from(stage);
    validate_contributes_to(store, &brief_stage.contributes_to, diagnostics)?;
    stages.push(brief_stage);
}
stages.sort_by_key(|s| s.order);
```

- [ ] Add test `brief_unresolved_field_ref_in_contributes_to_is_diagnostic` in the `tests` module using `MemoryStore`:
  - Build a store with one valid field (`field-aaa`) and one type (`type-111`).
  - Create a blueprint with root type `type-111`.
  - Import a protocol that targets `type-111`; one stage has `contributesTo: [{"fieldId": "nonexistent-field-id"}]`.
  - Call `blueprint_brief` and assert: (a) the result is `Ok`, (b) `result.diagnostics` contains a string matching `"nonexistent-field-id"`, (c) `result.protocol` is `Some` and the stage is still present.

#### Acceptance Criteria

- [ ] `blueprint_brief` on a protocol stage with an unresolved `fieldId` in `contributesTo` returns `Ok` with the stage present and a diagnostic containing the offending `fieldId`.
- [ ] Valid `fieldId` values produce no new diagnostics.
- [ ] Stages with no `contributesTo` (or an empty vec) produce no new diagnostics.
- [ ] `cargo test -p srs-repository` passes with no failures.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `brief_unresolved_field_ref_in_contributes_to_is_diagnostic` — proves unresolved FieldRef produces a diagnostic and stage is still in result.
- All existing tests must still pass (no regression).

#### Milestone gate

1. All acceptance criteria above checked.
2. Named test exists and passes.
3. `cargo test -p srs-repository` green; `cargo clippy -p srs-repository -- -D warnings` clean.
4. Plan checkboxes updated.
5. Commit: `fix(protocol): validate contributes_to FieldRef.fieldId in blueprint brief (#189)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `brief_unresolved_field_ref_in_contributes_to_is_diagnostic` exists and passes
- [ ] Existing `test_brief_finds_protocol_for_root_type` still passes (valid refs produce no new diagnostics)

## Coordination Rules

- Repository Service Worker owns `crates/srs-repository/src/blueprint_brief_service.rs`.
- No other crates are modified.
- **At the end of Phase 1:** verify all acceptance criteria, run the milestone gate, update plan checkboxes, then commit. Do not proceed to final acceptance without completing the gate.

## Assumptions

- `get_field_by_id` is already importable in `blueprint_brief_service.rs` (confirmed — it is imported at line 16 from `crate::package_service`).
- `GetFieldResult` is already in scope (confirmed — imported at line 16).
- The `FieldRef` type is already imported (confirmed — line 21).
- No cross-store roundtrip test is added for this change. Waiver rationale: `validate_contributes_to` calls only `get_field_by_id`, which is already exercised by cross-store roundtrip tests in the package_service test suite. The new code path adds no store-specific branching; any cross-store discrepancy in `get_field_by_id` would surface in those existing tests before reaching this service. (CLAUDE.md Storage Boundary Rule: acknowledged and waived on this basis.)
