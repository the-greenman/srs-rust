# Plan: ADR-016 — Protocols Are Package Definitions, Not Tier 2 Records

## Summary

ADR-006 ("Protocol Definitions Are Generic Tier 2 Records With Typed Validation") is factually stale: the implementation shipped in srs-rust#170 stores protocols as package definitions under `package/protocols/`, parallel to blueprints — not as Tier 2 Records bound to a `com.semanticops.srs/protocol@1` type. Any agent reading ADR-006 receives guidance that contradicts actual CLI behaviour. This plan files ADR-016 documenting the real storage model and marks ADR-006 as "Superseded by ADR-016".

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this pipeline) |
| Verification | Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | Generic Tier 2 Record pattern — the pattern ADR-006 applied and ADR-016 departs from | accepted (not affected) |
| [ADR-006](../docs/adr/006-protocol-definitions-are-tier2-records.md) | Protocol definitions are Tier 2 Records (superseded by ADR-016) | superseded |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI commands use named payload structs — partially followed by protocol commands | accepted (not affected) |
| [ADR-016](../docs/adr/016-protocols-are-package-definitions.md) | Protocol definitions are package definitions (parallel to Blueprints) | accepted |

No new architectural decisions beyond formalising the existing implementation.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. This is a documentation-only change. `payload_contracts` test is unchanged.

### Entity schema sync (check-schema-sync.sh)

No schema files under `srs/docs/schema/2.0/` are modified. `check-schema-sync.sh` is not applicable.

---

## Scope

- Create `docs/adr/016-protocols-are-package-definitions.md` documenting the package-definition storage model.
- Update `docs/adr/006-protocol-definitions-are-tier2-records.md` status to "Superseded by ADR-016" and add a "Superseded by" link.

**Out of scope:**

- Any changes to Rust source, tests, or CLI behaviour.
- JSON Schema files for protocol definitions (tracked in srs-rust#174).
- `repo validate` coverage of protocol/blueprint definitions (tracked in srs-rust#175).

---

## Phases

### Phase 1: File ADR-016 and update ADR-006

**Goal:** ADR-016 exists and accurately documents the package-definition storage model; ADR-006 status is updated to "Superseded by ADR-016".

**Agent:** Lead Integrator

#### Tasks

- [ ] Create `docs/adr/016-protocols-are-package-definitions.md` using `ADR-TEMPLATE.md`. Cover:
  - Context: original Tier-2-Record design (ADR-006) and what changed in srs-rust#170.
  - Why the original design was abandoned: parity with blueprints; no prerequisite gate on `com.semanticops.srs/protocol@1` type; package definitions are owned by the package maintainer, not the repo instance layer.
  - Decision: protocols stored as `package/protocols/<name>-<uuid>.json`, registered in `package.json → protocols[]`. Parallel to `package/blueprints/`.
  - Rust model: `Protocol` and `ProtocolStage` structs in `srs-core` serve as both the deserialization target and the validation model. Raw `serde_json::Value` is used for storage/retrieval where extra stage fields beyond the struct are preserved verbatim.
  - No golden schema in `schemas/payload/` for the inner shape (value-centric storage).
  - Status: **accepted**.
- [ ] Update `docs/adr/006-protocol-definitions-are-tier2-records.md`:
  - Change `**Status:** accepted` to `**Status:** superseded`.
  - Change `**Superseded by:** —` to `**Superseded by:** [ADR-016](016-protocols-are-package-definitions.md)`.

#### Acceptance Criteria

- [ ] `docs/adr/016-protocols-are-package-definitions.md` exists with Status `accepted`.
- [ ] `docs/adr/006-protocol-definitions-are-tier2-records.md` has Status `superseded` and links ADR-016.
- [ ] `cargo test` passes with no failures (documentation only — tests unaffected).
- [ ] `cargo test --test payload_contracts` passes.
- [ ] `cargo clippy -- -D warnings` passes (no Rust changes).
- [ ] Bug filed for gallery protocol `contributesTo` bare-string format mismatch (see Assumptions).

#### Milestone gate

1. All acceptance criteria above pass.
2. Commit: `docs(adr): file ADR-016 superseding ADR-006 — protocols are package definitions (#183)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo test --test payload_contracts` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `docs/adr/016-protocols-are-package-definitions.md` exists with Status `accepted`
- [ ] `docs/adr/006-protocol-definitions-are-tier2-records.md` has Status `superseded`
- [ ] Bug filed for gallery protocol `contributesTo` bare-string vs FieldRef mismatch

## Coordination Rules

No multi-agent coordination needed. Lead Integrator handles the single phase directly.

## Assumptions

- The `Protocol` and `ProtocolStage` struct shapes in `srs-core/src/types/protocol.rs` accurately reflect the package-definition storage model as shipped in srs-rust#170 and are the authoritative source for ADR-016's "Rust model" section.
- `package/protocols/<name>-<uuid>.json` is the canonical storage path (confirmed by gallery example at `srs/docs/spec/examples/gallery-project-v2/package/protocols/decision-7a088176.json`).
- The gallery protocol file's `contributesTo` field contains bare UUID strings (e.g. `"9889052c-..."`), but the spec (`ext:protocol`, `ProtocolStage`) and the `ProtocolStage` Rust struct define `contributesTo` as `FieldRef[]` (`{fieldId: UUID, typeId?: UUID}`). This is a bug in the gallery file (confirmed: `srs protocol get` fails with "invalid type: string, expected struct FieldRef"). ADR-016 describes the correct `FieldRef` shape from the spec; the gallery fix is tracked as a separate bug (filed during this plan's execution).
- JSON Schema files for validating protocol definitions are tracked in srs-rust#174 and are out of scope here.
