# Plan: ProtocolStage.outputType — typed TypeRef

> Issue: srs-rust#204

## Summary

`ProtocolStage.output_type` in `srs-core/src/types/protocol.rs` is currently typed as `Option<serde_json::Value>`. The ext:protocol spec defines `outputType` as a `TypeRef` (`{ typeId: UUID, typeVersion?: u32 }`), a shape that already exists in `crates/srs-core/src/types/blueprint.rs`. This plan reuses that existing `TypeRef` struct to give the field a compile-time-verified type, propagating the change through `srs-repository::BriefStageResult` and `srs-cli::payload::ProtocolStageEntry` / `BriefStage`. `ai_guidance` retains `serde_json::Value` because its shape is genuinely unspecified in the spec.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | Phase 1 |
| Repository Service Worker | Phase 1 |
| CLI Worker | Phase 2 |
| Verification Agent | Final |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service output types must be fully typed — `serde_json::Value` for a spec-defined shape violates this | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Payload structs in payload.rs keep `#[schemars(with = "Option<serde_json::Value>")]` on the field so the golden schema does NOT change — no `generate-schemas` run needed | accepted |
| [ADR-016](../docs/adr/016-protocols-are-package-definitions.md) | Partially superseded: the sentence "(the latter two as `serde_json::Value`)" in the Consequences section of ADR-016 was written when both `aiGuidance` and `outputType` were untyped. After this plan, only `aiGuidance` remains `serde_json::Value`. ADR-016 must be updated in the docs pass to reflect this. | accepted (update pending) |

No new ADRs needed — this plan implements existing ADR-010/ADR-011 and partially supersedes ADR-016's consequences description.

---

## Contracts

### CLI output contract (ADR-011)

`ProtocolStageEntry.output_type` and `BriefStage.output_type` in `payload.rs` already carry `#[schemars(with = "Option<serde_json::Value>")]`. We change the Rust field type to `Option<blueprint::TypeRef>` but keep the `schemars` annotation unchanged. Per the template rule: "service type embedded via `#[schemars(with = "serde_json::Value")]` → no schema regeneration needed." Golden schemas stay as-is; `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No `srs/docs/schema/2.0/` files are touched. No action required.

---

## Scope

- Change `ProtocolStage.output_type` in `srs-core/src/types/protocol.rs` from `Option<serde_json::Value>` to `Option<TypeRef>` (reusing `crate::types::blueprint::TypeRef`).
- Change `BriefStageResult.output_type` in `srs-repository/src/blueprint_brief_service.rs` from `Option<serde_json::Value>` to `Option<TypeRef>` (import already present: `use srs_core::types::blueprint::TypeRef`).
- Change `ProtocolStageEntry.output_type` and `BriefStage.output_type` in `srs-cli/src/payload.rs` from `Option<serde_json::Value>` to `Option<blueprint::TypeRef>`, keeping the `#[schemars(with = "Option<serde_json::Value>")]` annotation.
- Update the stale comment on `LoadedProtocol.raw` in `srs-repository/src/package.rs` that claims `output_type` is "not fully captured by the typed Protocol struct".
- Add a deserialization roundtrip test for `ProtocolStage.output_type` in `protocol.rs`.

**Out of scope:**
- Changing `ai_guidance` (legitimately unspecified shape — kept as `serde_json::Value`).
- Updating golden schemas or regenerating JSON Schema files.
- Changing the WASM bindings (`srs-bindings`) — they currently return `serde_json::Value` output and are out of scope for this issue (#205 tracks WASM binding cleanup).

---

## Phases

### Phase 1: Core type + repository propagation

**Goal:** `ProtocolStage.output_type` is `Option<TypeRef>` in srs-core; `BriefStageResult.output_type` matches in srs-repository; `cargo test -p srs-core` and `cargo test -p srs-repository` pass. Full workspace compile is intentionally deferred to Phase 2 (srs-cli handlers will have a type mismatch until payload.rs is updated).

**Agent:** Core Model Worker + Repository Service Worker

#### Tasks

- [x] In `crates/srs-core/src/types/protocol.rs`:
  - Add `use crate::types::blueprint::TypeRef;` at the top.
  - Change `pub output_type: Option<serde_json::Value>` → `pub output_type: Option<TypeRef>`.
- [x] In the same file, add a `test_protocol_stage_output_type_roundtrip` unit test (in the existing `#[cfg(test)]` block) that deserializes a `ProtocolStage` JSON with `"outputType": {"typeId": "abc-123", "typeVersion": 1}` and asserts `stage.output_type == Some(TypeRef { type_id: "abc-123".to_string(), type_version: Some(1) })`. Add a second test `test_protocol_stage_output_type_absent` that serializes a `ProtocolStage` with `output_type: None` and asserts the serialized JSON does not contain `"outputType"`.
- [x] In `crates/srs-repository/src/blueprint_brief_service.rs`:
  - Change `pub output_type: Option<serde_json::Value>` → `pub output_type: Option<TypeRef>` in `BriefStageResult` (line ~70). `TypeRef` is already imported at line 20.

#### Acceptance Criteria

- [x] `ProtocolStage.output_type` is `Option<TypeRef>` — no `serde_json::Value` for this field.
- [x] `BriefStageResult.output_type` is `Option<TypeRef>`.
- [x] `From<ProtocolStage> for BriefStageResult` compiles unchanged (field assignment `output_type: stage.output_type` still holds — same type both sides).
- [x] Two new tests in `protocol.rs` pass: one with `outputType` present, one absent.
- [x] `cargo test -p srs-core` and `cargo test -p srs-repository` pass.

#### Testing

```bash
cargo test -p srs-core
cargo test -p srs-repository
```

Specific tests to write or verify:

- `test_protocol_stage_output_type_roundtrip` — proves TypeRef survives serde round-trip on ProtocolStage
- `test_protocol_stage_output_type_absent` — proves absent `outputType` serialises with `skip_serializing_if`

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

3. Update plan checkboxes.
4. Commit: `feat(srs-core,srs-repository): type ProtocolStage.outputType as TypeRef (#204)`

---

### Phase 2: CLI payload update + stale comment fix

**Goal:** `ProtocolStageEntry` and `BriefStage` in payload.rs use `Option<TypeRef>`; stale comment in package.rs fixed; full workspace compiles; payload contracts pass.

**Agent:** CLI Worker

#### Tasks

- [x] In `crates/srs-cli/src/payload.rs`:
  - Add `blueprint::TypeRef` to the existing `srs_core::types` import block (line 22–33). The field type `blueprint::TypeRef` is imported via `use srs_core::types::blueprint::TypeRef;` added to the block.
  - Change `pub output_type: Option<serde_json::Value>` → `pub output_type: Option<TypeRef>` in `ProtocolStageEntry` (line ~132). Keep the `#[schemars(with = "Option<serde_json::Value>")]` annotation above it unchanged.
  - Change `pub output_type: Option<serde_json::Value>` → `pub output_type: Option<TypeRef>` in `BriefStage` (line ~841). Keep the `#[schemars(with = "Option<serde_json::Value>")]` annotation above it. Add `#[serde(skip_serializing_if = "Option::is_none")]` to `BriefStage.output_type` — it is missing (unlike `ProtocolStageEntry.output_type` which has it), causing `None` to serialize as `"outputType": null` rather than being absent.
- [x] In `crates/srs-repository/src/package.rs`, update the doc comment on `LoadedProtocol.raw` (line ~39–42). Replace the current comment body with: `` `raw` preserves all fields from the on-disk JSON that are not already captured by the typed `Protocol` struct. `source_package` is `None` for the root package and `Some` for protocols merged from a dependency package. `` (removing the `output_type` example that no longer applies after this fix).

#### Acceptance Criteria

- [x] `ProtocolStageEntry.output_type` is `Option<TypeRef>` with `#[schemars(with = "Option<serde_json::Value>")]`.
- [x] `BriefStage.output_type` is `Option<TypeRef>` with `#[schemars(with = "Option<serde_json::Value>")]`.
- [x] No `serde_json::Value` for `output_type` anywhere in the codebase (grep confirms).
- [x] Both CLI command handlers that map `output_type: s.output_type` compile without modification.
- [x] `cargo test -p srs-cli` passes; `cargo test --test payload_contracts` passes (golden schemas unchanged).
- [x] `LoadedProtocol.raw` comment no longer references `output_type` as an example of an untyped field.

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

Specific tests to write or verify:

- `payload_contracts` golden test — confirms schema files are unchanged (no regen triggered).

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

3. Update plan checkboxes.
4. Commit: `feat(srs-cli): use typed TypeRef for output_type in payload structs (#204)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (golden schemas unchanged)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] No `serde_json::Value` for `output_type` anywhere (grep: `rg "output_type\s*:\s*Option<serde_json::Value>" crates/` returns 0 results)
- [ ] Two new tests in `srs-core/src/types/protocol.rs` cover TypeRef roundtrip and absent case

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `blueprint::TypeRef` is the correct reuse target — it has the right shape (`typeId: String, typeVersion: Option<u32>`) and is already imported in `blueprint_brief_service.rs`.
- `ai_guidance: Option<serde_json::Value>` remains as-is in all affected structs; it is a legitimate escape hatch.
- No schema regeneration is needed because both changed payload fields retain `#[schemars(with = "Option<serde_json::Value>")]`.
