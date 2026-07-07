# Plan: Fix BriefStage (and Brief*) Optional Fields Missing skip_serializing_if

## Summary

`BriefStage` and related `Brief*` structs in `crates/srs-cli/src/payload.rs` have `Option<_>` fields that lack `#[serde(skip_serializing_if = "Option::is_none")]`. When those fields are `None`, they serialize as `"fieldName": null` rather than being absent from the JSON output. `ProtocolStageEntry` (the parallel struct for protocol stages) already has the annotation on all its optional fields and is the reference for the correct pattern. This fix closes the inconsistency, regenerates affected golden schemas, and ensures `cargo test --test payload_contracts` passes.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| CLI Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Payload struct changes require running `cargo run --bin generate-schemas` and committing the updated schema files | accepted |

No new ADRs needed — this plan implements an existing ADR-011 requirement (schema regeneration after payload struct change).

---

## Contracts

### CLI output contract (ADR-011)

**Existing commands with payload structs changed:** `blueprint brief` command uses `BlueprintBriefPayload`, which contains `BriefStage`, `BriefField`, `BriefType`. Adding `skip_serializing_if` changes the serialized JSON shape (None fields become absent rather than null). This is a bug-fix change: the old null-serialization was incorrect.

Action: run `cargo run --bin generate-schemas` after edits; commit the updated `schemas/payload/` files; verify `cargo test --test payload_contracts` passes.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` — this fix is purely in `payload.rs` structs. No sync needed.

---

## Scope

- Add `#[serde(skip_serializing_if = "Option::is_none")]` to all `Option<_>` fields in `BriefStage` that are missing it (identified by grep: `purpose`, `question`, `completion_criteria`, `contributes_to`, `ai_guidance`)
- Fix the same annotation gap in `BriefField.ai_guidance`, `BriefType.ai_guidance`, `BlueprintBriefPayload.ai_guidance`, `BlueprintBriefPayload.protocol` — same pattern, same command surface
- Regenerate golden schemas: `cargo run --bin generate-schemas`
- Ensure `cargo test --test payload_contracts` passes

**Out of scope:**
- Any non-Brief* structs in payload.rs (separate audit if needed)
- Changes to service logic, CLI handlers, or srs-repository

---

## Phases

### Phase 1: Annotate Option fields and regenerate schemas

**Goal:** All `Option<_>` fields in `BriefStage`, `BriefField`, `BriefType`, and `BlueprintBriefPayload` carry `#[serde(skip_serializing_if = "Option::is_none")]`; golden schemas are regenerated and committed.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add `#[serde(skip_serializing_if = "Option::is_none")]` to `BriefStage.purpose`, `BriefStage.question`, `BriefStage.completion_criteria`, `BriefStage.contributes_to`, `BriefStage.ai_guidance` (5 fields, lines ~844–851)
- [ ] Same fix for `BriefField.ai_guidance` (line ~802), `BriefType.ai_guidance` (line ~812), `BlueprintBriefPayload.ai_guidance` (line ~875), `BlueprintBriefPayload.protocol` (line ~880)
- [ ] Run `cargo run --bin generate-schemas` to regenerate golden schemas
- [ ] Stage and commit updated `crates/srs-cli/schemas/payload/` files

#### Acceptance Criteria

- [ ] `BriefStage` matches `ProtocolStageEntry`: every `Option<_>` field has `#[serde(skip_serializing_if = "Option::is_none")]`
- [ ] `BriefField`, `BriefType`, `BlueprintBriefPayload` have the annotation on all their `Option<_>` fields
- [ ] `cargo test --test payload_contracts` passes (golden schemas match structs)
- [ ] `cargo clippy -p srs-cli -- -D warnings` passes

#### Testing

```bash
cargo test --test payload_contracts
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:
- `payload_contracts` integration test — proves golden schema files match structs
- New unit test `brief_stage_none_fields_absent_from_json` in `crates/srs-cli/src/payload.rs` (or a `#[cfg(test)]` module at end of file): construct a `BriefStage` with all optional fields `None`, serialize with `serde_json::to_string`, assert the JSON string does not contain `"purpose"`, `"question"`, `"completionCriteria"`, `"contributesTo"`, `"aiGuidance"`, or `"null"`. Proves the runtime fix.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm `cargo test --test payload_contracts` passes.
3. Run:
```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```
4. Update plan checkboxes `[x]`.
5. Commit: `fix(payload): add skip_serializing_if to BriefStage and Brief* Option fields (#352)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] All `Option<_>` fields in `BriefStage`, `BriefField`, `BriefType`, `BlueprintBriefPayload` have `#[serde(skip_serializing_if = "Option::is_none")]`
- [ ] Golden schemas confirmed unchanged (schemars already treats Option<T> as non-required)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- **Golden schemas will NOT change.** `schemars` treats `Option<T>` fields as non-required regardless of `skip_serializing_if`. Verified by inspecting `blueprint-brief.json`: `BriefStage.required` is already `['dependsOn', 'name', 'order', 'stageId']`. Running `cargo run --bin generate-schemas` produces identical output. The fix is purely runtime behavior (None fields absent rather than null).
- `BriefRelationSpec` already carries `skip_serializing_if` on its optional fields (lines 822–825 in payload.rs) — fixed in a prior PR; intentionally out of scope here.
- No other Brief* structs beyond the four identified (`BriefStage`, `BriefField`, `BriefType`, `BlueprintBriefPayload`) are affected — verified by grep pass during implementation.
