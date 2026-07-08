# Plan: srs-bindings: set_lifecycle_state surfaces warnings (#367)

## Summary

`set_lifecycle_state` in `srs-bindings/src/lib.rs` calls `record_store::transition_record_lifecycle` but returns only `result.record`, silently discarding `result.warnings`. After #240 landed, `warnings` became a first-class field of `TransitionLifecycleResult` surfaced to CLI callers via `RecordTransitionPayload`. WASM callers currently receive a structurally inconsistent response — they see the record but never learn when a transition into a final (immutable) state occurred. This plan fixes the inconsistency by making `set_lifecycle_state` return `{ record, warnings }`, matching the CLI payload shape.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (main session) |
| Bindings Worker | Claude (main session) |
| Repository Service Worker | Claude (main session) |
| Verification Agent | Claude (main session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service returns typed result; binding must not drop fields from it | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding calls same service as CLI; uses `to_js(&result)` — no duplicate structs | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | B7 write bindings (lifecycle transitions) call exactly one service function; no business logic in `srs-bindings` | accepted |
| [ADR-022](../docs/adr/022-governance-status-is-lifecycle-state.md) | `set_lifecycle_state` is the only correct write path for governance status; CLI/WASM parity is required | accepted |

No new ADRs needed: the fix restores the structural consistency that ADR-013 requires between the CLI and WASM surfaces. The return shape change (`Record` → `{ record, warnings }`) is a breaking change to the WASM JS API but is tracked as part of this issue (#367), and `warnings` was always present in the service result — this makes it visible.

---

## Contracts

### CLI output contract (ADR-011)

No CLI payload structs are changed. `RecordTransitionPayload` in `crates/srs-cli/src/payload.rs` is untouched. `cargo run --bin generate-schemas` is not needed.

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` — no action required.

---

## Scope

- Add `serde::Serialize` + `#[serde(rename_all = "camelCase")]` to `TransitionLifecycleResult` in `crates/srs-repository/src/record_store.rs`
- Update `set_lifecycle_state` in `crates/srs-bindings/src/lib.rs` to call `to_js(&result)` instead of `to_js(&result.record)`
- Update the doc comment on `set_lifecycle_state` to document the new return shape
- Add a test in `crates/srs-bindings/tests/relation_lifecycle.rs` that verifies the serialized output contains both `record` and `warnings` keys

**Out of scope:**
- Changes to the `srs-web` consumer (tracked separately in srs-web#135 or a follow-up)
- Changes to `TransitionLifecycleInput` or the transition service logic
- Any CLI payload changes

---

## Phases

### Phase 1: Serialize TransitionLifecycleResult

**Goal:** `TransitionLifecycleResult` derives `serde::Serialize` with camelCase names, so `to_js(&result)` in the binding can serialize the whole struct.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/record_store.rs`, update the `TransitionLifecycleResult` struct (at ~line 790) from:
  ```rust
  #[derive(Debug, Clone)]
  pub struct TransitionLifecycleResult {
      pub record: Record,
      pub warnings: Vec<String>,
  }
  ```
  to:
  ```rust
  #[derive(Debug, Clone, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct TransitionLifecycleResult {
      pub record: Record,
      pub warnings: Vec<String>,
  }
  ```

#### Acceptance Criteria

- [ ] `TransitionLifecycleResult` derives `serde::Serialize` and has `#[serde(rename_all = "camelCase")]`
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify (pre-existing, must still pass):
- `set_lifecycle_state_transitions_record` — non-final transition; warnings empty
- `set_lifecycle_state_full_chain_to_final` — LIFECYCLE_FINAL_STATE warning present

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
3. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:
```bash
git commit -m "feat(repository): derive Serialize on TransitionLifecycleResult (#367)"
```

---

### Phase 2: Update WASM binding

**Goal:** `set_lifecycle_state` returns `{ record, warnings }` to JS callers; doc comment updated; test proves the field is present.

**Agent:** Bindings Worker

#### Tasks

- [ ] In `crates/srs-bindings/src/lib.rs`, update the `set_lifecycle_state` method (lines ~267–280):
  - Replace the doc comment block with:
    ```
    /// Transition a record's lifecycle state.
    /// `state` is the target state name (e.g. `"ratified"`).
    /// Returns `{ "record": <Record>, "warnings": ["LIFECYCLE_FINAL_STATE: ..."] }` as a JS value.
    /// `warnings` is empty for non-final transitions; contains a `LIFECYCLE_FINAL_STATE` entry
    /// when the target state has `isFinal: true`.
    ```
  - Replace the final line `to_js(&result.record)` with `to_js(&result)`

- [ ] In `crates/srs-bindings/tests/relation_lifecycle.rs`, add a new test after `set_lifecycle_state_full_chain_to_final`. Note: `serde_json` is already in scope (the `lifecycle_srsj()` fixture helper uses `serde_json::json!`); no new import is needed.
  ```rust
  // ---------------------------------------------------------------------------
  // 4c. set_lifecycle_state serialized output contains both `record` and `warnings`.
  // ---------------------------------------------------------------------------
  #[test]
  fn set_lifecycle_state_result_includes_warnings_field() {
      let store = JsonStore::from_srsj(&lifecycle_srsj()).expect("lifecycle fixture must load");

      // Transition to a final state (active → archived) so warnings is non-empty.
      record_store::transition_record_lifecycle(
          &store,
          "rec-lc-001",
          TransitionLifecycleInput {
              to: Some("active".to_string()),
              by_transition: None,
          },
      )
      .expect("draft→active must succeed");

      let result = record_store::transition_record_lifecycle(
          &store,
          "rec-lc-001",
          TransitionLifecycleInput {
              to: Some("archived".to_string()),
              by_transition: None,
          },
      )
      .expect("active→archived must succeed");

      // Serialize to JSON (mirrors what to_js(&result) does in the WASM binding).
      let json = serde_json::to_value(&result).expect("TransitionLifecycleResult must serialize");
      assert!(
          json.get("record").is_some(),
          "serialized result must contain 'record' key"
      );
      assert!(
          json.get("warnings").is_some(),
          "serialized result must contain 'warnings' key"
      );
      let warnings = json["warnings"].as_array().expect("warnings must be an array");
      assert!(
          warnings.iter().any(|w| w.as_str().map_or(false, |s| s.contains("LIFECYCLE_FINAL_STATE"))),
          "warnings must contain LIFECYCLE_FINAL_STATE entry for final-state transition"
      );
  }
  ```

#### Acceptance Criteria

- [ ] `set_lifecycle_state` calls `to_js(&result)` instead of `to_js(&result.record)`
- [ ] Doc comment reflects the new return shape `{ record, warnings }`
- [ ] `set_lifecycle_state_result_includes_warnings_field` test exists and passes
- [ ] `cargo test -p srs-bindings` passes
- [ ] `cargo clippy -p srs-bindings -- -D warnings` passes
- [ ] `cargo build -p srs-bindings` succeeds

#### Testing

```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific test to write:
- `set_lifecycle_state_result_includes_warnings_field` — serializes `TransitionLifecycleResult` to `serde_json::Value`, asserts both `record` and `warnings` keys are present, and confirms a `LIFECYCLE_FINAL_STATE` warning appears for a final-state transition.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
3. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:
```bash
git commit -m "feat(bindings): set_lifecycle_state returns { record, warnings } (#367)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (CLI payload structs unchanged)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas unchanged)
- [ ] `set_lifecycle_state` in `lib.rs` calls `to_js(&result)` (not `to_js(&result.record)`)
- [ ] `TransitionLifecycleResult` derives `serde::Serialize` in `record_store.rs`
- [ ] `set_lifecycle_state_result_includes_warnings_field` test passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `Record` already implements `serde::Serialize` (confirmed: `CreateRecordSuccessorResult` derives it with `pub record: Record`)
- `to_js` in `srs-bindings/src/lib.rs` calls `serde_json::to_string` internally and converts to `JsValue` — serializing the full result struct works the same way as for `create_record_successor`
- No downstream WASM consumer in `srs-web` currently reads the `set_lifecycle_state` return value as a multi-field object — the call site returns the full JS object and any consumer reading `.record` from the result will still work correctly since the field is still present
- `#[serde(rename_all = "camelCase")]` on `TransitionLifecycleResult` has no effect on `record` or `warnings` (neither contains underscores), but is kept for consistency with neighbouring structs (`LifecycleTransitionOption`, `AllowedLifecycleTransitionsResult`) and to document intent for future fields
