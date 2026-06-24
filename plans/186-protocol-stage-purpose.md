# Plan: Add `purpose` field to ProtocolStage (#186)

## Summary

The SRS spec (`ext:protocol`) defines `ProtocolStage.purpose: string` — the epistemic description of what understanding a stage builds. The Rust implementation uses `name: String` as a display label but never surfaced `purpose`. The owner chose Option 2: add `purpose: Option<String>` alongside `name`, keeping `name` as the short display label and `purpose` as the richer spec-defined description. This is a non-breaking additive change. Both fields coexist; existing protocol JSON without `purpose` deserialises cleanly because the new field is optional.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-006](../docs/adr/006-protocol-definitions-are-tier2-records.md) | `ProtocolStage` is a typed validation struct aligned to the spec; spec-defined `purpose` field belongs in srs-core per the hybrid storage model | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `purpose` propagation happens in srs-repository service layer (not in CLI handlers). Note: the existing `cmd_protocol_stages` handler contains pre-existing inline field projection (`ProtocolStageSummary → ProtocolStageEntry`) in violation of ADR-010; this plan adds `purpose` to that mapping but does not refactor it — tracked in a separate issue. | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Both `ProtocolStageEntry` and `BriefStage` payload structs are updated; `cargo run --bin generate-schemas` must run after | accepted |

---

## Contracts

### CLI output contract (ADR-011)

Two existing command payloads change:

1. **`protocol stages`** → `ProtocolStageEntry` gains `purpose?: string` (optional, omitted when absent)
2. **`blueprint brief`** → `BriefStage` gains `purpose?: string` (optional, omitted when absent)

After updating the structs in `payload.rs`, run:
```bash
cargo run --bin generate-schemas
```
Commit the updated `crates/srs-cli/schemas/payload/protocol-stages.json` and `crates/srs-cli/schemas/payload/blueprint-brief.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. The `ext:protocol` spec record (`srs/srs/records/extensions/ext-protocol.json`) is a spec content record, not a JSON Schema entity schema file. No schema sync needed.

---

## Scope

- Add `purpose: Option<String>` (camelCase: `purpose`) to `ProtocolStage` in `srs-core`
- Add `purpose: Option<String>` to `ProtocolStageSummary` in `srs-core`
- Add `purpose: Option<String>` to `BriefStageResult` in `srs-repository` and propagate in `From<ProtocolStage>` impl
- Propagate `purpose` through `list_protocol_stages` in `srs-repository`
- Add `purpose: Option<String>` to `ProtocolStageEntry` in `srs-cli/payload.rs`
- Add `purpose: Option<String>` to `BriefStage` in `srs-cli/payload.rs`
- Update `map_brief_stage` in `srs-cli/src/commands/blueprint.rs`
- Update `cmd_protocol_stages` mapping in `srs-cli/src/commands/protocol.rs`
- Regenerate `schemas/payload/protocol-stages.json` and `schemas/payload/blueprint-brief.json`

**Out of scope:**
- Making `purpose` required (breaking) — it stays optional so existing protocol JSON deserialises cleanly
- Renaming `name` to anything else — `name` stays as-is in the payload contract
- Migrating existing test fixture protocol JSON files to add `purpose` — fixtures are valid without it
- Changing `validate_protocol_stage` validation logic — `purpose` is optional and unconstrained
- Refactoring the pre-existing ADR-010 violation in `cmd_protocol_stages` (inline field projection) — separate issue
- Spec clarifying edit (`srs/srs/records/extensions/ext-protocol.json`) and `srs-usage.md` update — separate coordinated PR in the `srs` repo; does not block these Rust changes; handled in the documentation pass after Phase 3

**Coordination note:** The spec record update (`srs/srs/records/extensions/ext-protocol.json`) adding `name` alongside `purpose` in the `ProtocolStage` TypeScript block is a separate commit on a branch in the `srs` repo, coordinated with this PR. An agent implementing this plan must not edit the `srs/` repo; that edit is outside this plan's scope.

---

## Phases

### Phase 1: Core type update

**Goal:** `ProtocolStage` and `ProtocolStageSummary` in `srs-core` carry the new optional `purpose` field; all existing tests still pass.

**Agent:** Core Model Worker

#### Tasks

- [ ] In `crates/srs-core/src/types/protocol.rs`, add to `ProtocolStage` after the `name` field (currently line 10, before `order`):
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub purpose: Option<String>,
  ```
- [ ] In the same file, add to `ProtocolStageSummary` after `name` (currently line 103):
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub purpose: Option<String>,
  ```

#### Acceptance Criteria

- [ ] `ProtocolStage` in `crates/srs-core/src/types/protocol.rs` has `pub purpose: Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]`
- [ ] `ProtocolStageSummary` in the same file has `pub purpose: Option<String>` with same serde attr
- [ ] A protocol JSON without a `purpose` key deserialises to `ProtocolStage { purpose: None, ... }`
- [ ] A protocol JSON with `"purpose": "builds shared understanding"` deserialises to `ProtocolStage { purpose: Some("builds shared understanding".to_string()), ... }`
- [ ] `cargo test -p srs-core` passes with zero failures

#### Testing

```bash
cargo test -p srs-core
```

No new test needed: `Option<String>` serde roundtrip is fully determined by derive; the `skip_serializing_if` attribute is standard and tested by serde's own test suite. The acceptance criteria bullets above serve as the agent's manual verification.

#### Milestone gate

1. Verify all acceptance criteria above by reading the source file.
2. Run:
```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```
3. Mark checkboxes `[x]`, commit:
```bash
git commit -m "feat(srs-core): add purpose field to ProtocolStage and ProtocolStageSummary (#186)"
```

---

### Phase 2: Repository service propagation

**Goal:** `BriefStageResult` (blueprint brief path) and `ProtocolStageSummary` (protocol stages path) both carry and propagate `purpose`; both propagation paths have unit tests confirming the field flows through.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/blueprint_brief_service.rs`, add `pub purpose: Option<String>` to `BriefStageResult` (line 60, after `name: String` on line 62).
- [ ] In the same file, update `From<ProtocolStage> for BriefStageResult` (line 351) to add `purpose: stage.purpose` in the `Self { ... }` block.
- [ ] In `crates/srs-repository/src/protocol_service.rs`, update the `list_protocol_stages` mapping (line ~287) where `ProtocolStageSummary { stage_id: s.stage_id, name: s.name, order: s.order, depends_on: s.depends_on }` is constructed: add `purpose: s.purpose`.
- [ ] Add two unit tests in the `#[cfg(test)] mod tests` block of `crates/srs-repository/src/blueprint_brief_service.rs` (block starts at line 397):

  **Test 1** — `brief_stage_from_protocol_stage_maps_purpose`:
  ```rust
  #[test]
  fn brief_stage_from_protocol_stage_maps_purpose() {
      use srs_core::types::protocol::ProtocolStage;
      let v = serde_json::json!({
          "stageId": "s1",
          "name": "Understand",
          "purpose": "builds shared understanding of the problem space",
          "order": 0,
          "dependsOn": []
      });
      let stage: ProtocolStage = serde_json::from_value(v).unwrap();
      let brief = BriefStageResult::from(stage);
      assert_eq!(brief.purpose, Some("builds shared understanding of the problem space".to_string()));
  }
  ```

  **Test 2** — `brief_stage_from_protocol_stage_purpose_absent_when_missing`:
  ```rust
  #[test]
  fn brief_stage_from_protocol_stage_purpose_absent_when_missing() {
      use srs_core::types::protocol::ProtocolStage;
      let v = serde_json::json!({
          "stageId": "s1",
          "name": "Understand",
          "order": 0,
          "dependsOn": []
      });
      let stage: ProtocolStage = serde_json::from_value(v).unwrap();
      let brief = BriefStageResult::from(stage);
      assert_eq!(brief.purpose, None);
  }
  ```

  Place both tests after the existing `test_brief_stage_output_type_*` tests in the module.

#### Acceptance Criteria

- [ ] `BriefStageResult` in `crates/srs-repository/src/blueprint_brief_service.rs` (line 60) has `pub purpose: Option<String>`
- [ ] `From<ProtocolStage> for BriefStageResult` maps `purpose: stage.purpose`
- [ ] `ProtocolStageSummary` in `crates/srs-core/src/types/protocol.rs` has `pub purpose: Option<String>` (from Phase 1)
- [ ] `list_protocol_stages` in `crates/srs-repository/src/protocol_service.rs` constructs `ProtocolStageSummary` with `purpose: s.purpose`
- [ ] `brief_stage_from_protocol_stage_maps_purpose` test exists and passes
- [ ] `brief_stage_from_protocol_stage_purpose_absent_when_missing` test exists and passes
- [ ] `cargo test -p srs-repository` passes with zero failures

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-repository brief_stage_from_protocol_stage
```

Specific tests that must pass:
- `brief_stage_from_protocol_stage_maps_purpose` — proves `purpose` flows from `ProtocolStage` through `From` impl to `BriefStageResult`
- `brief_stage_from_protocol_stage_purpose_absent_when_missing` — proves optional absence is clean

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Commit:
```bash
git commit -m "feat(srs-repository): propagate purpose through BriefStageResult and ProtocolStageSummary (#186)"
```

---

### Phase 3: CLI payload and handler update

**Goal:** Both `protocol stages` and `blueprint brief` CLI commands surface `purpose` in their JSON output; golden schema files are regenerated and committed.

**Prerequisite:** Phase 1 and Phase 2 milestone gates must be complete. `ProtocolStageSummary.purpose` and `BriefStageResult.purpose` must exist before the CLI mapping can reference them.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add to `ProtocolStageEntry` (line 111, after `name: String`):
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub purpose: Option<String>,
  ```
- [ ] In the same file, add to `BriefStage` (line 760, after `name: String`):
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub purpose: Option<String>,
  ```
- [ ] In `crates/srs-cli/src/commands/protocol.rs` in the `cmd_protocol_stages` handler (line ~85), add `purpose: s.purpose` to the `ProtocolStageEntry { ... }` struct literal.
- [ ] In `crates/srs-cli/src/commands/blueprint.rs` in `map_brief_stage` (line 226), add `purpose: s.purpose` to the `BriefStage { ... }` struct literal.
- [ ] Run `cargo run --bin generate-schemas` and stage the updated golden files:
  - `crates/srs-cli/schemas/payload/protocol-stages.json`
  - `crates/srs-cli/schemas/payload/blueprint-brief.json`

#### Acceptance Criteria

- [ ] `ProtocolStageEntry` has `purpose: Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]`
- [ ] `BriefStage` has `purpose: Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]`
- [ ] `purpose` is mapped in both `ProtocolStageEntry` and `BriefStage` struct literal expressions
- [ ] `cargo run --bin generate-schemas` succeeds
- [ ] `crates/srs-cli/schemas/payload/protocol-stages.json` has `purpose` as an optional string property
- [ ] `crates/srs-cli/schemas/payload/blueprint-brief.json` has `purpose` as an optional string property in stage entries
- [ ] `cargo test --test payload_contracts` passes
- [ ] `cargo test -p srs-cli` passes with zero failures

#### Testing

```bash
cargo run --bin generate-schemas
cargo test --test payload_contracts
cargo test -p srs-cli
```

Specific tests to verify pass:
- `protocol_stages` in `crates/srs-cli/tests/payload_contracts.rs` (line ~272) — verifies golden schema matches struct
- `protocol_stages_returns_ordered_stages` in `crates/srs-cli/tests/integration_tests.rs` (line ~3373) — existing fixture may lack `purpose` (all protocol stage fixtures currently omit it); this test exercises the `None` case, which is valid. The `Some` case is covered by Phase 2 unit tests.

#### Milestone gate

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Commit:
```bash
git commit -m "feat(srs-cli): add purpose to ProtocolStageEntry and BriefStage payload (#186)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (golden schemas updated and committed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas unchanged — no action taken)
- [ ] Protocol JSON without `purpose` still deserialises without error
- [ ] `protocol stages` output includes `purpose` when the protocol stage defines it
- [ ] `blueprint brief` output includes `purpose` in stage entries when present
- [ ] Two new unit tests for `BriefStageResult` `From` impl pass

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.
- **Spec edit is out of scope for this plan** — a separate coordinated PR in the `srs` repo handles the `ext-protocol.json` clarification; do not edit `srs/` repo files in this plan's implementation.

## Assumptions

- Existing protocol fixture JSON files in tests do not currently contain a `purpose` field — so no fixture updates are required for tests to pass. The new field is optional and will round-trip cleanly.
- The pre-existing ADR-010 violation in `cmd_protocol_stages` (inline `ProtocolStageSummary → ProtocolStageEntry` field projection) is out of scope; a follow-up issue should track the refactor.
