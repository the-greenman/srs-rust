# Plan: Lifecycle allowed-transitions service + CLI payload + WASM binding (#375)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

The transition graph for a record's lifecycle exists only as a hardcoded TypeScript table in `srs-web/lifecycle.ts`. A governance editor (or any consumer) that needs to answer "what states can this record move to next, and is it immutable?" must consult an out-of-process constant instead of querying the Rust core. This plan adds:

1. A `get_allowed_lifecycle_transitions` service in `srs-repository/record_store.rs` that returns the current state, allowed next transitions, and whether the record is in a final (immutable) state.
2. A `srs record allowed-transitions --id <id>` CLI command with a golden schema.
3. A `get_allowed_lifecycle_transitions(instance_id)` WASM binding in `srs-bindings`, which calls `to_js(&result)` directly (the service result types carry `serde::Serialize`).

ADR-022 (governance status = SRS lifecycle state) is accepted; the design decision this depended on is resolved.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (main session) |
| Repository Service Worker | Claude (main session) |
| CLI Worker | Claude (main session) |
| Bindings Worker | Claude (main session) |
| Verification Agent | Claude (main session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service function in `srs-repository`; CLI handler = arg parse + one service call + output | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Named payload structs in `payload.rs`; `cargo run --bin generate-schemas` after any struct change | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding calls same service as CLI; uses `to_js(&result)` directly — no duplicate structs | accepted |
| [ADR-022](../docs/adr/022-governance-status-is-lifecycle-state.md) | Governance status is SRS lifecycle state; `get_allowed_lifecycle_transitions` is the canonical query path | accepted |

No new ADRs needed.

---

## Contracts

### CLI output contract (ADR-011)

New command `srs record allowed-transitions --id <id>` added. Payload struct `RecordAllowedTransitionsPayload` (with nested `AllowedTransitionEntry`) must be added to `crates/srs-cli/src/payload.rs`. After adding: `cargo run --bin generate-schemas` → commit `crates/srs-cli/schemas/payload/record-allowed-transitions.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` — no action required.

---

## Scope

- `get_allowed_lifecycle_transitions` service function in `crates/srs-repository/src/record_store.rs`
- `LifecycleTransitionOption` and `AllowedLifecycleTransitionsResult` result structs in `record_store.rs` (with `serde::Serialize`)
- No `lib.rs` re-export needed — types accessible via `srs_repository::record_store::AllowedLifecycleTransitionsResult` / `LifecycleTransitionOption` through the existing `pub mod record_store`
- `AllowedTransitionEntry`, `RecordAllowedTransitionsPayload`, and their `From` impls in `crates/srs-cli/src/payload.rs`
- `RecordCommand::AllowedTransitions { id: String }` variant in `crates/srs-cli/src/commands/mod.rs`
- `cmd_record_allowed_transitions` handler in `crates/srs-cli/src/commands/record.rs`
- `get_allowed_lifecycle_transitions` WASM binding method in `crates/srs-bindings/src/lib.rs`
- Golden schema file: `crates/srs-cli/schemas/payload/record-allowed-transitions.json`

**Out of scope:**
- Changes to srs-web (that is a consumer of the new WASM binding, tracked in srs-web#135)
- Changes to `set_lifecycle_state` or existing lifecycle transition logic
- Surfacing warnings from `transition_record_lifecycle` in the WASM binding (tracked in srs-rust#367)

---

## Phases

### Phase 1: Service

**Goal:** `get_allowed_lifecycle_transitions` is callable from any store; result types carry `serde::Serialize`; 6 tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `crates/srs-repository/src/record_store.rs` after the `TransitionLifecycleResult` block (~line 794):
  ```rust
  /// One legal next transition for a record in its current lifecycle state.
  #[derive(Debug, Clone, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct LifecycleTransitionOption {
      /// Display name of the transition (e.g. "promote", "archive").
      pub name: String,
      /// Target state key.
      pub to: String,
      /// Whether the target state has `is_final: true`.
      pub to_is_final: bool,
  }

  /// Result of `get_allowed_lifecycle_transitions`.
  #[derive(Debug, Clone, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct AllowedLifecycleTransitionsResult {
      /// The record's current lifecycle state key (empty string if unset).
      pub current_state: String,
      /// Transitions the record is permitted to take from its current state.
      pub transitions: Vec<LifecycleTransitionOption>,
      /// True when the current state has `is_final: true`.
      pub is_immutable: bool,
  }
  ```
- [ ] Add `pub fn get_allowed_lifecycle_transitions` to `record_store.rs` after the `transition_record_lifecycle` function block:
  ```rust
  pub fn get_allowed_lifecycle_transitions(
      store: &dyn RepositoryStore,
      instance_id: &str,
  ) -> Result<AllowedLifecycleTransitionsResult, RepositoryError> {
      let record =
          get_record_by_id(store, instance_id)?.ok_or_else(|| RepositoryError::NotFound {
              path: std::path::PathBuf::from(instance_id),
          })?;
      let package = store.load_package()?;
      let record_type = package
          .resolve_type(&record.type_id, record.type_version)
          .ok_or_else(|| RepositoryError::TypeNotFound {
              type_id: record.type_id.clone(),
              version: record.type_version,
          })?;
      let lifecycle = package.effective_lifecycle(record_type).ok_or_else(|| {
          RepositoryError::LifecycleNotDefined {
              id: instance_id.to_string(),
          }
      })?;

      let current_state = record.lifecycle_state.clone().unwrap_or_default();
      let is_immutable = lifecycle
          .states
          .iter()
          .any(|s| s.key == current_state && s.is_final == Some(true));
      let transitions = lifecycle
          .transitions
          .iter()
          .filter(|t| t.from == current_state)
          .map(|t| {
              let to_is_final = lifecycle
                  .states
                  .iter()
                  .any(|s| s.key == t.to && s.is_final == Some(true));
              LifecycleTransitionOption {
                  name: t.name.clone(),
                  to: t.to.clone(),
                  to_is_final,
              }
          })
          .collect();
      Ok(AllowedLifecycleTransitionsResult {
          current_state,
          transitions,
          is_immutable,
      })
  }
  ```
- [ ] Add tests in the `#[cfg(test)]` block of `record_store.rs`, after the existing lifecycle tests. Use `make_store_with_lifecycle()` and `create_lc_record()` (defined at lines 2236 and 2418).

#### Acceptance Criteria

- [ ] `get_allowed_lifecycle_transitions` exists in `record_store.rs`
- [ ] `LifecycleTransitionOption` and `AllowedLifecycleTransitionsResult` derive `serde::Serialize` with `rename_all = "camelCase"`
- [ ] All 6 tests listed below pass
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository allowed_transitions
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `allowed_transitions_from_draft_returns_correct_options` — creates a record (initial state "draft"), calls `get_allowed_lifecycle_transitions`, asserts `current_state == "draft"`, `transitions.len() == 1`, `transitions[0].name == "promote"`, `transitions[0].to == "active"`, `transitions[0].to_is_final == false`, `is_immutable == false`
- `allowed_transitions_from_active_returns_correct_options` — creates a record, promotes to "active" via `transition_record_lifecycle`, asserts `current_state == "active"`, `transitions.len() == 1`, `transitions[0].name == "archive"`, `transitions[0].to == "archived"`, `transitions[0].to_is_final == true`, `is_immutable == false`
- `allowed_transitions_from_final_state_returns_immutable_empty` — creates a record, promotes to "active" then archives it (two `transition_record_lifecycle` calls), asserts `current_state == "archived"`, `transitions.is_empty()`, `is_immutable == true`
- `allowed_transitions_record_not_found_returns_error` — calls with a made-up instance ID, asserts `Err(RepositoryError::NotFound { .. })`
- `allowed_transitions_with_lifecycle_ref` — uses `make_store_with_lifecycle_ref()` (type "type-lc-ref-001", field "field-title-lcref", initial state "draft" confirmed at line 3235); creates a record via `create_record`, asserts `current_state == "draft"` and `transitions` is non-empty
- `allowed_transitions_roundtrip_file_store` — creates a lifecycle record in `MemoryStore`, exports to `.srsj` via `to_srsj_string`, re-loads into `JsonStore::from_srsj` (or `load_from_srsj`), calls `get_allowed_lifecycle_transitions` against the `JsonStore`, asserts `current_state == "draft"` and results match the `MemoryStore` baseline

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed above exists and passes.
3. Run:
```bash
cargo test -p srs-repository allowed_transitions
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `feat(repository): get_allowed_lifecycle_transitions service (#375)`

---

### Phase 2: CLI

**Goal:** `srs record allowed-transitions --id <id>` outputs a JSON envelope with current state, transitions, and immutability; golden schema committed.

**Agent:** CLI Worker

#### Tasks

- [ ] Add to `crates/srs-cli/src/payload.rs` (after `RecordTransitionPayload`, ~line 340):
  ```rust
  /// One allowed lifecycle transition for `record allowed-transitions`.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct AllowedTransitionEntry {
      pub name: String,
      pub to: String,
      pub to_is_final: bool,
  }

  impl From<record_store::LifecycleTransitionOption> for AllowedTransitionEntry {
      fn from(t: record_store::LifecycleTransitionOption) -> Self {
          Self { name: t.name, to: t.to, to_is_final: t.to_is_final }
      }
  }

  /// Payload for `record allowed-transitions`.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct RecordAllowedTransitionsPayload {
      pub current_state: String,
      pub transitions: Vec<AllowedTransitionEntry>,
      pub is_immutable: bool,
  }

  impl From<record_store::AllowedLifecycleTransitionsResult> for RecordAllowedTransitionsPayload {
      fn from(r: record_store::AllowedLifecycleTransitionsResult) -> Self {
          Self {
              current_state: r.current_state,
              transitions: r.transitions.into_iter().map(AllowedTransitionEntry::from).collect(),
              is_immutable: r.is_immutable,
          }
      }
  }
  ```
  Note: `record_store` must be in scope in `payload.rs`. Check the existing imports — if `record_store` types aren't already imported, add `use srs_repository::record_store;` to the imports.
- [ ] Add `AllowedTransitions` variant to `RecordCommand` in `crates/srs-cli/src/commands/mod.rs` (after `Successor`, ~line 1017):
  ```rust
  /// Query allowed lifecycle transitions for a record (ext:lifecycle)
  AllowedTransitions {
      /// Record instance ID
      #[arg(long)]
      id: String,
  },
  ```
- [ ] Add handler in `crates/srs-cli/src/commands/record.rs` (after `cmd_record_successor`):
  ```rust
  fn cmd_record_allowed_transitions(ctx: CliContext, id: String) -> Result<String> {
      match with_store(&ctx, |store| {
          Ok(record_store::get_allowed_lifecycle_transitions(store, &id)?)
      }) {
          Ok(result) => output::serialize(
              "record allowed-transitions",
              RecordAllowedTransitionsPayload::from(result),
          ),
          Err(e) => Ok(output::err("record allowed-transitions", vec![e.to_string()])),
      }
  }
  ```
- [ ] Wire `RecordCommand::AllowedTransitions { id }` into the `dispatch` match in `record.rs`:
  ```rust
  RecordCommand::AllowedTransitions { id } => cmd_record_allowed_transitions(ctx, id),
  ```
- [ ] Add `get_allowed_lifecycle_transitions`, `AllowedLifecycleTransitionsResult`, `LifecycleTransitionOption` to the `record_store` import in `record.rs`
- [ ] Add `RecordAllowedTransitionsPayload` and `AllowedTransitionEntry` to the `payload` import in `record.rs`
- [ ] Run `cargo run --bin generate-schemas` and commit `crates/srs-cli/schemas/payload/record-allowed-transitions.json`

#### Acceptance Criteria

- [ ] `srs record allowed-transitions --id <id>` compiles and runs
- [ ] `RecordAllowedTransitionsPayload`, `AllowedTransitionEntry`, and their `From` impls exist in `payload.rs`
- [ ] `crates/srs-cli/schemas/payload/record-allowed-transitions.json` exists and is committed
- [ ] `cargo test --test payload_contracts` passes
- [ ] `cargo test -p srs-cli` passes
- [ ] `cargo clippy -p srs-cli -- -D warnings` passes

#### Testing

```bash
cargo build --bin srs
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```
3. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit: `feat(cli): srs record allowed-transitions command (#375)`

---

### Phase 3: WASM binding

**Goal:** `SrsRepository.get_allowed_lifecycle_transitions(instance_id)` is callable from JS; build succeeds.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add to `crates/srs-bindings/src/lib.rs` (after the `set_lifecycle_state` method, ~line 280):
  ```rust
  /// Query the allowed lifecycle transitions for a record in its current state.
  /// Returns `{ "currentState": "...", "transitions": [{ "name": "...", "to": "...", "toIsFinal": bool }], "isImmutable": bool }`.
  pub fn get_allowed_lifecycle_transitions(
      &self,
      instance_id: &str,
  ) -> Result<JsValue, JsValue> {
      let result =
          record_store::get_allowed_lifecycle_transitions(&self.store, instance_id)
              .map_err(js_err)?;
      to_js(&result)
  }
  ```
- [ ] Verify `get_allowed_lifecycle_transitions` is reachable via the existing `use srs_repository::record_store` import in `lib.rs`

#### Acceptance Criteria

- [ ] `get_allowed_lifecycle_transitions` method exists on `SrsRepository` in `lib.rs`
- [ ] `cargo build -p srs-bindings` succeeds
- [ ] `cargo test -p srs-bindings` passes
- [ ] `cargo clippy -p srs-bindings -- -D warnings` passes

#### Testing

```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
3. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit: `feat(bindings): get_allowed_lifecycle_transitions WASM binding (#375)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas unchanged)
- [ ] `srs record allowed-transitions --id <valid-id>` returns a correct JSON envelope against a real repository
- [ ] `get_allowed_lifecycle_transitions` WASM method present in `srs-bindings/src/lib.rs`
- [ ] `crates/srs-cli/schemas/payload/record-allowed-transitions.json` is committed and matches the struct

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- ADR-022 is accepted (confirmed by `docs/adr/022-governance-status-is-lifecycle-state.md`)
- `make_store_with_lifecycle()` exists at `record_store.rs:2236` (draft/active/archived; "archived" is `is_final: true`)
- `make_store_with_lifecycle_ref()` exists at `record_store.rs:3146` (standalone lifecycle, same states, initial state "draft" confirmed at line 3235)
- `create_lc_record()` exists at `record_store.rs:2418` (creates a record against `make_store_with_lifecycle`)
- No `pub use record_store::{ ... }` block exists in `lib.rs` — new types are accessible at `srs_repository::record_store::*` via `pub mod record_store`
- Handler pattern: `match with_store { Ok(r) => output::serialize(...), Err(e) => Ok(output::err(...)) }` — confirmed from adjacent `cmd_record_transition` and `cmd_record_successor`
