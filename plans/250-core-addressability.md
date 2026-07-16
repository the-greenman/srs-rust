# Plan: Core: Address + AttentionState + Revision (ext:addressability)

## Summary

The `ext:addressability` extension is fully specified in `srs/srs/records/extensions/ext-addressability.json` — its three types (`Address`, `AttentionState`, `Revision`) are defined in prose and TypeScript pseudocode. The Rust `srs-core` crate has partial stubs: `Address` exists but `Process` and `Conversation` variants are empty units; `Revision` exists but lacks `sourceRefs`; `AttentionState` is absent. This plan completes the `srs-core` data model for ext:addressability so downstream services and bindings can implement the behavioral requirements (context queries, revision chains, attention tracking) on a solid foundation. No CLI, service, or binding changes are in scope — pure core types.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | All types live in `srs-core`; no file I/O | accepted |
| [ADR-005](../docs/adr/005-extension-definitions-are-tier2-records.md) | Extension definition records are generic Tier 2; this plan adds **data model types** (not definition records) | accepted |
| [ADR-028](../docs/adr/028-extension-catalog-types-in-srs-core.md) | Extension data file types go in `srs-core/src/extensions/`; `Address`/`AttentionState`/`Revision` are in-memory model types, so they stay in `srs-core/src/types/` following the Protocol precedent (ADR-016) | accepted |

No new ADRs are needed. The placement of `Address` and `Revision` in `types/` (not `extensions/`) follows the existing pattern established for `Protocol`/`ProtocolStage` (ADR-016): in-memory model types used by service logic belong in `types/`, while external file format types for extension catalogs belong in `extensions/`.

The one cross-cutting structural change: `SourceReference` is currently duplicated in `types/note.rs` and `types/relation.rs`. Since `Revision` also needs it, this plan extracts it to `types/source_reference.rs` and re-imports it into both existing modules. This is a DRY fix, not a behavioural change.

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands are added or changed. No payload structs are added or changed. `cargo test --test payload_contracts` will remain green with no action.

### Entity schema sync (check-schema-sync.sh)

No entity schema files under `srs/docs/schema/2.0/` are added or modified. `bash scripts/check-schema-sync.sh` is unaffected.

---

## Scope

- Complete `Address` enum in `crates/srs-core/src/types/address.rs`: expand `Process` and `Conversation` unit stubs to proper structs with spec-mandated fields.
- Add `AttentionState` struct to `crates/srs-core/src/types/address.rs` (alongside `Address` — same spec section, same conceptual space).
- Extract shared `SourceReference` type family to `crates/srs-core/src/types/source_reference.rs`; update `note.rs` and `relation.rs` to import from there.
- Add `source_refs: Option<Vec<SourceReference>>` to `Revision` in `crates/srs-core/src/types/revision.rs`.
- Register new modules in `crates/srs-core/src/types/mod.rs` (`pub mod source_reference;`). Types are accessible via module path (e.g. `srs_core::types::address::AttentionState`); no `pub use` re-exports are added, following the existing convention in `types/mod.rs`.

**Out of scope:**
- No service functions, CLI handlers, or WASM bindings — those are follow-up issues.
- No Context Query service implementation (the behavioural requirement in the spec) — that requires `srs-repository` work and is deferred.
- No schema file (`revision.json`) for the Revision format — deferred until a schema RFC is filed.
- `RevisionAgent` serialization format (`{"type":"Human"}` vs plain string `"human"`) — pre-existing; not changed here to avoid breaking any stored data.
- Unifying `note::RelationType` and `relation::SourceRelationType` enum names — deferred; Phase 3 only merges the shared `SourceReference` struct and its `SourceType` enum; both calling modules re-export their own type aliases as needed.

---

## Phases

### Phase 1: Complete `Address` variants + add `AttentionState`

**Goal:** `Address::Process` and `Address::Conversation` are fully typed structs; `AttentionState` exists and round-trips through serde.

**Agent:** Core Model Worker

#### Tasks

- [ ] In `crates/srs-core/src/types/address.rs`:
  - Replace unit variant `Process` with `Process(ProcessAddress)` where `ProcessAddress` is:
    ```rust
    pub struct ProcessAddress {
        pub run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub stage_id: Option<String>,
    }
    ```
  - Replace unit variant `Conversation` with `Conversation(ConversationAddress)` where `ConversationAddress` is:
    ```rust
    pub struct ConversationAddress {
        pub session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub chunk_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub annotation_id: Option<String>,
    }
    ```
  - Both new structs: `#[derive(Debug, Clone, Serialize, Deserialize)]`, `#[serde(rename_all = "camelCase")]`.
  - Add `AttentionState` struct after the Address enum:
    ```rust
    pub struct AttentionState {
        pub container_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub record_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub field_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub protocol_run_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub stage_id: Option<String>,
    }
    ```
    Derive: `Debug, Clone, Serialize, Deserialize`, `rename_all = "camelCase"`.
- [ ] Update `Address` serde: keep `#[serde(tag = "space")]` but the new variants are newtype-wrapped structs, which `serde` with `tag = "space"` serialises correctly (the inner struct fields are inlined alongside the tag). Verify the JSON for `Process` is `{"space":"Process","runId":"..."}`.
- [ ] Update existing unit tests for `Process` and `Conversation` in the `tests` module — they currently test unit variants; replace with struct-variant tests:
  - `process_address_with_stage` — `Address::Process(ProcessAddress { run_id, stage_id: Some(...) })` → JSON has `"space":"Process","runId":"...","stageId":"..."`.
  - `process_address_run_only` — `stage_id: None` → no `stageId` in JSON.
  - `conversation_address_full` — all three fields.
  - `conversation_address_session_only` — chunk_id and annotation_id absent.
  - `attention_state_full` — all five fields, JSON round-trip.
  - `attention_state_minimal` — only `containerId`, all others absent.
- [ ] New types are accessible via `srs_core::types::address::AttentionState` (no `pub use` re-exports in `mod.rs` — follows the existing convention where `mod.rs` only declares modules, not re-exports).

#### Acceptance Criteria

- [ ] `Address::Process(ProcessAddress { run_id: "r-1".into(), stage_id: None })` serialises to `{"space":"Process","runId":"r-1"}` (no `stageId` key).
- [ ] `Address::Conversation(ConversationAddress { session_id: "s-1".into(), chunk_id: None, annotation_id: None })` serialises to `{"space":"Conversation","sessionId":"s-1"}`.
- [ ] Deserialisation of the above JSON strings round-trips correctly.
- [ ] `AttentionState` with only `containerId` set serialises to `{"containerId":"c-1"}` (no other keys present).
- [ ] No existing passing tests regress.

#### Testing

```bash
cargo test -p srs-core types::address
cargo clippy -p srs-core -- -D warnings
```

Tests to write or verify in `address.rs`:
- `process_address_run_only` — proves no stageId key when None
- `process_address_with_stage` — proves stageId present when Some
- `conversation_address_session_only` — proves chunk_id and annotation_id absent
- `conversation_address_full` — proves all three fields
- `attention_state_minimal` — proves only containerId emitted
- `attention_state_full` — proves all five fields and round-trip

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed above exists and passes (`cargo test -p srs-core types::address`).
3. Run:
   ```bash
   cargo test -p srs-core
   cargo clippy -p srs-core -- -D warnings
   ```
4. Mark completed task checkboxes `[x]` and acceptance criteria `[x]` in this plan.
5. Commit: `feat(srs-core): complete Address variants, add AttentionState (#250)`.

---

### Phase 2: Extract shared `SourceReference` and add to `Revision`

**Goal:** `SourceReference` lives in one place (`types/source_reference.rs`); both `note.rs` and `relation.rs` import it from there; `Revision` gains `source_refs`.

**Agent:** Core Model Worker

#### Tasks

- [ ] Create `crates/srs-core/src/types/source_reference.rs`:
  - Move the `SourceReference` struct from `note.rs` into this new file. Keep fields identical: `source_type`, `source_id`, `source_standard`, `stream_id`, `relation_type: Option<SourceRelationType>`, `confidence`, `note`.
  - **Do NOT add `#[serde(deny_unknown_fields)]`** to the shared struct. `note.rs`'s `SourceReference` is lenient (no deny); `relation.rs`'s is strict (deny). The shared struct adopts the lenient shape — a safe widening. Callers that need strictness can add it at the container level. This removes the per-source-ref deny_unknown_fields from the relation path, which is acceptable since the outer `Relation` struct already enforces its own schema constraints.
  - Move the `SourceType` enum from `note.rs` into this file (identical in both modules).
  - Move the `SourceRelationType` enum. Both `note.rs` (`RelationType`) and `relation.rs` (`SourceRelationType`) already have identical five variants: `Evidence`, `DerivedFrom`, `QuotedFrom`, `InspiredBy`, `SupersedesContext`. Use the name `SourceRelationType` for the shared version.
    - Final `SourceRelationType` variants: `Evidence`, `DerivedFrom`, `QuotedFrom`, `InspiredBy`, `SupersedesContext`.
  - `SourceReference` derives: `Debug, Clone, PartialEq, Serialize, Deserialize`, `rename_all = "camelCase"`.
  - `SourceType` derives: `Debug, Clone, Copy, PartialEq, Serialize, Deserialize`, `rename_all = "kebab-case"`.
  - `SourceRelationType` derives: `Debug, Clone, Copy, PartialEq, Serialize, Deserialize`, `rename_all = "kebab-case"`.
- [ ] Update `crates/srs-core/src/types/note.rs`:
  - Remove the local `SourceReference`, `SourceType`, and `RelationType` definitions.
  - Add: `use super::source_reference::{SourceReference, SourceType, SourceRelationType};`
  - Add `pub use super::source_reference::SourceRelationType as RelationType;` to preserve the `RelationType` name for any existing callsite that uses `note::RelationType`.
  - Confirm all existing `note.rs` tests still compile and pass.
- [ ] Update `crates/srs-core/src/types/relation.rs`:
  - Remove the local `SourceReference`, `SourceType`, and `SourceRelationType` definitions.
  - Add: `use super::source_reference::{SourceReference, SourceType, SourceRelationType};`
  - Confirm all existing `relation.rs` tests still compile and pass.
- [ ] Update `crates/srs-core/src/types/revision.rs`:
  - Add `use super::source_reference::SourceReference;` at the top.
  - Add `source_refs: Option<Vec<SourceReference>>` field to `Revision` struct (after `provenance`, before `created_at` is fine; add `#[serde(skip_serializing_if = "Option::is_none")]`).
  - Update existing tests to construct `Revision` with `source_refs: None`.
  - Add new test `revision_with_source_refs` — creates a `Revision` with one `SourceReference`, verifies `source_refs[0].sourceId` appears in JSON and round-trips.
- [ ] Register in `crates/srs-core/src/types/mod.rs`:
  - Add `pub mod source_reference;`
- [ ] Fix any downstream crates that import `note::SourceReference`, `note::SourceType`, or `note::RelationType` — update their imports to reference the new location or keep using `note::RelationType` via the re-export.

#### Acceptance Criteria

- [ ] `SourceReference` is defined exactly once in `types/source_reference.rs`.
- [ ] `note.rs` and `relation.rs` compile without their own `SourceReference` definition.
- [ ] `SourceRelationType` has all five variants: Evidence, DerivedFrom, QuotedFrom, InspiredBy, SupersedesContext.
- [ ] `Revision` with `source_refs: Some(vec![...])` serialises `sourceRefs` as a JSON array.
- [ ] `Revision` with `source_refs: None` omits `sourceRefs` from JSON.
- [ ] All tests in `note.rs`, `relation.rs`, and `revision.rs` pass.
- [ ] All downstream crates that reference `SourceReference` compile.

#### Testing

```bash
cargo test -p srs-core
cargo test -p srs-repository
cargo test -p srs-cli
cargo clippy -- -D warnings
```

Tests to write or verify:
- `revision_with_source_refs` in `revision.rs` — proves sourceRefs field serialises and round-trips
- All existing note, relation, revision tests — proves no regressions

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all referenced tests exist and pass.
3. Run:
   ```bash
   cargo test
   cargo clippy -- -D warnings
   ```
4. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `feat(srs-core): extract SourceReference, add sourceRefs to Revision (#250)`.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] `Address::Process` and `Address::Conversation` are proper typed structs per the spec
- [ ] `AttentionState` exists in `srs-core::types::address` with all five spec fields
- [ ] `Revision` has `source_refs: Option<Vec<SourceReference>>`
- [ ] `SourceReference` is defined exactly once

## Coordination Rules

- Core Model Worker owns all changes under `crates/srs-core/src/types/`.
- Do not touch `srs-repository`, `srs-cli`, or `srs-bindings` except to fix compile errors caused by the SourceReference move.
- Agents keep to their write scopes; Lead Integrator owns final naming.
- At the end of each phase: verify acceptance criteria, confirm tests pass, update checkboxes, commit.

## Assumptions

- The `Address` serde tag `#[serde(tag = "space")]` correctly inlines struct-variant fields alongside the `"space"` key. This is standard `serde` behaviour for internally-tagged enums with newtype variants wrapping structs.
- No persisted data currently uses `Address::Process` or `Address::Conversation` (both were unit stubs), so changing them to struct variants is not a breaking serialisation change.
- `RevisionAgent` serialisation (`{"type":"Human"}`) is intentionally different from the spec's string enum and is not changed here.
- Downstream crates (`srs-repository`, `srs-cli`) import `SourceReference` via `note::SourceReference` — this remains accessible via the re-export.
