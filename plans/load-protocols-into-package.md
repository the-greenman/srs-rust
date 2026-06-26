# Plan: Load Protocol Definitions into the Compiled Package Model

## Summary

Protocol definitions are Package definitions (stored under `package/protocols/`, registered in `package.json` `protocols[]`), exactly parallel to blueprints. Currently, every read-side service call (`list_protocols`, `get_protocol_by_id`, `find_protocol_by_target_type`) re-scans `package.json` and re-reads protocol files from storage on every invocation. The compiled `Package` struct in `crates/srs-repository/src/package.rs` has `blueprints: Vec<Blueprint>` but no `protocols` field, so there is no cached model to reuse. This plan adds `protocols: Vec<LoadedProtocol>` to `Package`, populates it in both the FileStore and JsonStore loaders, and refactors the three read-side service functions to consume the compiled model — eliminating repeated file I/O on every protocol read call. Write paths (create/update/delete) remain unchanged as the source of truth for `package.json` mutation. This is needed now because the Decision Logger v1 flow depends on protocol lookup performance and correctness, and the missing field is a structural gap in the Package model.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-006](../docs/adr/006-protocol-definitions-are-tier2-records.md) | Protocols are package definitions with typed validation; original ADR specified Tier 2 Records but implementation correctly uses Package definitions parallel to blueprints | accepted |
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Services address packages through logical selectors; FileStore and JsonStore/MemoryStore own the path mapping | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions take typed inputs, own all validation, return typed results; no re-reading inside a single logical operation | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI output is produced by named structs in payload.rs | accepted |

No new ADRs are required — this plan implements the Package model pattern established for blueprints in ADR-009 and applies the service contract from ADR-010.

---

## Contracts

### CLI output contract (ADR-011)

No new CLI commands are added. No existing command payload structs change shape. The refactored service functions return the same types as before (`Vec<ProtocolSummary>`, `GetProtocolResult`, `Option<FindProtocolByTargetTypeResult>`). No `cargo run --bin generate-schemas` run is needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. No sync action required.

---

## Scope

What is explicitly in scope:

- Add `pub protocols: Vec<LoadedProtocol>` to `Package` in `crates/srs-repository/src/package.rs`
- Define `pub struct LoadedProtocol { pub protocol: Protocol, pub raw: serde_json::Value, pub source_package: Option<String> }` in `package.rs`
- Add `protocols: Vec<String>` to `PackageMetadata` in `crates/srs-repository/src/store.rs`
- Load protocols in `load_package_from_dir()` in `store.rs` (FileStore path)
- Extend the sub-package merge loop in `FileStore::load_package()` to merge protocols
- Extend `load_package_from_prefix()` in `crates/srs-repository/src/json_store.rs` to load protocols
- Update `JsonStore::load_package()` to populate `Package.protocols` from the loaded data
- Update all Package struct literals with `blueprints: vec![]` to also include `protocols: vec![]` (in `record_store.rs`, `json_store.rs`, and `store.rs` test helpers)
- Refactor `list_protocols()` in `protocol_service.rs` to read from `store.load_package()?.protocols`
- Refactor `get_protocol_by_id()` in `protocol_service.rs` to read from `store.load_package()?.protocols`
- Refactor `find_protocol_by_target_type()` in `protocol_service.rs` to read from `store.load_package()?.protocols`
- Add unit tests for the refactored read-side functions in `protocol_service.rs`
- Add a cross-store roundtrip test (create via JsonStore → load_package → list/get/find)

**Out of scope:**

- Refactoring blueprint_service read functions (parallel work, not required for this issue)
- Protocol write paths (create/update/delete) — stay unchanged
- WASM bindings (`srs-bindings`) — no protocol binding surface changes in this plan
- CLI handler changes in `srs-cli`
- Any srs-core Protocol/ProtocolStage struct changes

---

## Phases

### Phase 1: Extend the Package model and loaders

**Goal:** `Package.protocols` is populated on every `store.load_package()` call for both FileStore and JsonStore, and all existing Package struct literals compile with the new field.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package.rs`:
  - Add `use srs_core::types::protocol::Protocol;` import
  - Add `pub struct LoadedProtocol { pub protocol: Protocol, pub raw: serde_json::Value, pub source_package: Option<String> }` with `#[derive(Debug, Clone)]`
  - Add `pub protocols: Vec<LoadedProtocol>` field to `Package` struct after `blueprints`
- [ ] In `crates/srs-repository/src/store.rs` (private `PackageMetadata` struct at ~line 372):
  - Add `#[serde(default)] protocols: Vec<String>,` field after `blueprints`
- [ ] In `crates/srs-repository/src/store.rs` (`load_package_from_dir()` at ~line 728):
  - After the blueprints loop, add a protocols loading loop: for each `blueprint_path` in `metadata.protocols`, read the file, deserialize as `Protocol` (silently skip on parse error matching blueprint behaviour), store as `LoadedProtocol { protocol, raw: val, source_package: None }` (source_package set by caller)
  - Return type changes: extend the tuple from 6 to 7 elements — add `Vec<LoadedProtocol>` at the end
- [ ] In `crates/srs-repository/src/store.rs` (`FileStore::load_package()` at ~line 852):
  - Update the destructuring of `load_package_from_dir` to capture `mut protocols`
  - Add `let mut protocol_sources: HashMap<String, PathBuf>` tracking, seeded from primary package protocols
  - In the sub-package merge loop (~line 1018), add a protocols merge block after blueprints: first-boundary-wins on duplicate `protocolId`
  - Pass `source_package: None` for primary package protocols; set `source_package: Some(rel_path.to_string())` for sub-package protocols
  - Add `protocols` to the final `Package { ... }` constructor
- [ ] In `crates/srs-repository/src/json_store.rs` (`load_package_from_prefix()` return type at ~line 337):
  - Extend the return tuple from 8 to 9 elements — add `Vec<LoadedProtocol>` at the end
  - After the lifecycles loop (~line 560), add a protocols loading loop: for each `rel_path` in `metadata.protocols`, call `self.data_get(&full)`, parse as `serde_json::Value`, attempt `serde_json::from_value::<Protocol>(val.clone())` (silently skip on parse error), push `LoadedProtocol { protocol, raw: val, source_package: None }`
  - Update all callers of `load_package_from_prefix` to destructure the new 9-element tuple
- [ ] In `crates/srs-repository/src/json_store.rs` (`JsonStore::load_package()` at ~line 684):
  - Capture `protocols` from the `load_package_from_prefix("package", ...)` call
  - In the sub-package merge loop, call `load_package_from_prefix(rel_path, ...)`, capture sub-protocols, merge: for each sub-protocol, first-boundary-wins by `protocol.protocol_id`, set `source_package` to `Some(rel_path.to_string())` on the merged entry
  - Add `protocols` to the `Package { ... }` constructor (replacing `blueprints: vec![]`)
- [ ] In `crates/srs-repository/src/json_store.rs` (Package literal at ~line 844):
  - Change `blueprints: vec![],` to load from data (do not simply remove; see tasks above)
  - Actually: this literal is inside `JsonStore::load_package()` — it should now use the populated `protocols` variable from above
- [ ] In `crates/srs-repository/src/record_store.rs` — update all `Package { ... blueprints: vec![], ... }` literals to add `protocols: vec![]` (there are 3 occurrences: ~line 1244, ~line 1997, ~line 2881)
- [ ] In `crates/srs-repository/src/store.rs` — update the test `memory::MemoryStore`'s `initialize_repository` (`Package { ... blueprints: vec![], ... }` at ~line 2203) to add `protocols: vec![]`
- [ ] In `crates/srs-repository/src/store.rs` — update `package_to_json()` helper (~line 2027) to include `"protocols": []` in the returned JSON (alongside `"blueprints": []`)
- [ ] In `crates/srs-repository/src/store.rs` — add `pub fn with_protocol(protocol_json: serde_json::Value) -> Self` helper to `memory::MemoryStore`:
  - Parses `protocol_json` as `Protocol` (panics on invalid input — test helper only)
  - Computes filename `protocols/<slug>-<id_prefix>.json` using protocol name/id
  - Pushes `LoadedProtocol { protocol, raw: protocol_json.clone(), source_package: None }` to `self.package.protocols`
  - Adds filename to `package/package.json` `protocols[]` in `self.data` (or initializes the array if absent)
  - Stores the protocol file at `package/<filename>` in `self.data`

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles with no errors
- [ ] `cargo test -p srs-repository` passes (all pre-existing tests still pass)
- [ ] `Package.protocols` type is `Vec<LoadedProtocol>` and `LoadedProtocol` is `pub` in `package.rs`
- [ ] `PackageMetadata` in `store.rs` has `protocols: Vec<String>` with `#[serde(default)]`
- [ ] No path strings (`package/`, `protocols/`) appear in `package.rs` or `protocol_service.rs`

#### Testing

```bash
cargo build -p srs-repository
cargo test -p srs-repository
```

Specific tests to verify:

- All existing tests in `record_store.rs`, `package_service.rs`, `blueprint_service.rs` pass — these exercise `load_package()` via `make_package_with_types` and similar helpers, which will need `protocols: vec![]` added

#### Milestone gate

1. Verify all acceptance criteria are met.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update this plan: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit: `feat(package): add protocols field to Package, populate in FileStore and JsonStore loaders (#176)`

---

### Phase 2: Refactor read-side protocol_service functions

**Goal:** `list_protocols`, `get_protocol_by_id`, and `find_protocol_by_target_type` read from `store.load_package()?.protocols` instead of scanning `package.json` and re-reading protocol files on every call. Behavior is identical from the caller's perspective.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/protocol_service.rs`:
  - Add `use crate::package::LoadedProtocol;` import
  - Refactor `list_protocols(store)`:
    - Replace the boundary-scan loop with `let package = store.load_package()?;`
    - Map `package.protocols` to `Vec<ProtocolSummary>`: for each `LoadedProtocol`, build `ProtocolSummary` from `lp.protocol` fields (`protocol_id`, `protocol_namespace`, `protocol_name`, `protocol_version`) and `lp.raw["protocolStages"].as_array().map(|a| a.len()).unwrap_or(0)` for `stage_count`; `source_package: lp.source_package.clone()`
    - Deduplication by `protocol_id` (first-boundary-wins) is now handled by the loader; but retain a `HashSet` guard in case the loader doesn't deduplicate — skip if already seen
  - Refactor `get_protocol_by_id(store, id)`:
    - Replace `find_protocol_path` call with `store.load_package()?.protocols`
    - Find the first `LoadedProtocol` where `lp.protocol.protocol_id == id`
    - If found, return `GetProtocolResult::Found(lp.raw.clone())`; otherwise `GetProtocolResult::NotFound`
  - Refactor `find_protocol_by_target_type(store, target_type_id)`:
    - Replace the boundary-scan loop with `store.load_package()?.protocols`
    - Find the first `LoadedProtocol` where `lp.protocol.protocol_target_type == target_type_id`
    - Build `FindProtocolByTargetTypeResult { protocol_id: lp.protocol.protocol_id, protocol_name: lp.protocol.protocol_name, stages: lp.protocol.protocol_stages.clone(), diagnostics: vec![] }` from the typed `Protocol`
    - Return `Ok(Some(...))` if found, `Ok(None)` if not
  - The private `find_protocol_path` helper, `load_protocol_from_value`, `protocol_from_value`, `check_protocol`, and all write functions remain unchanged
  - Add `#[cfg(test)]` module to `protocol_service.rs` with tests (see Testing section)

#### Acceptance Criteria

- [ ] `list_protocols` no longer calls `store.load_instance_json` directly — it uses `store.load_package()`
- [ ] `get_protocol_by_id` no longer calls `find_protocol_path` or `store.load_instance_json` — it uses `store.load_package()`
- [ ] `find_protocol_by_target_type` no longer calls `store.load_instance_json` directly — it uses `store.load_package()`
- [ ] `create_protocol`, `update_protocol`, `delete_protocol`, `import_protocol`, `list_protocol_stages`, `validate_protocol_definition`, `export_protocol` are unchanged
- [ ] `cargo test -p srs-repository` passes
- [ ] Integration test: `srs protocol list`, `srs protocol get <id>`, `srs protocol stages <id>` produce the same output as before the refactor (verified manually in Phase 3 / dogfooding)

#### Testing

```bash
cargo test -p srs-repository
cargo test protocol -- --nocapture
```

Specific tests to write in `protocol_service.rs` `#[cfg(test)]` block:

Use `crate::store::memory::MemoryStore::with_protocol(protocol_json)` (added in Phase 1) for pre-populated tests.

- `list_protocols_empty_when_no_protocols` — `MemoryStore::empty()`, assert `list_protocols` returns `[]`
- `list_protocols_returns_summary_from_package` — `MemoryStore::with_protocol(valid_json)`, call `list_protocols`, assert `ProtocolSummary` fields match (`protocol_id`, `protocol_name`, `stage_count`)
- `get_protocol_by_id_returns_raw_json` — `MemoryStore::with_protocol(valid_json)`, call `get_protocol_by_id(id)`, assert `GetProtocolResult::Found(val)` where `val["protocolId"]` matches
- `get_protocol_by_id_returns_not_found` — `MemoryStore::empty()`, call `get_protocol_by_id("nonexistent")`, assert `GetProtocolResult::NotFound`
- `find_protocol_by_target_type_finds_match` — `MemoryStore::with_protocol(json_with_target_type)`, call `find_protocol_by_target_type("com.test/Decision")`, assert `Some(result)` with correct `protocol_id` and `stages`
- `find_protocol_by_target_type_returns_none` — `MemoryStore::empty()`, call with unknown target type, assert `None`

#### Milestone gate

1. Verify all acceptance criteria are met.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update this plan: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit: `refactor(protocol-service): consume compiled Package.protocols in read-side functions (#176)`

---

### Phase 3: Cross-store roundtrip test

**Goal:** A single test proves that a protocol created via the JsonStore (in-memory) is immediately visible through `load_package()?.protocols` — confirming the loader and service functions work end-to-end without touching disk.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/protocol_service.rs` (in the `#[cfg(test)]` block):
  - Add test `protocol_roundtrip_create_list_get_find`:
    - Uses `crate::json_store::JsonStore` (the in-memory JSON-backed store) — NOT `memory::MemoryStore` — because `JsonStore::load_package()` dynamically rebuilds the Package from its data on every call, so protocols created via the write path are immediately visible through `load_package()`
    - Setup: initialize a `JsonStore` backed by a temp in-memory `.srsj` file (or use `JsonStore::from_value`)
    1. Call `create_protocol(store, valid_protocol_json, None)` — write via the write path (this updates data in JsonStore)
    2. Call `list_protocols(store)` — uses `store.load_package()?.protocols` — assert the created protocol appears in the summary
    3. Call `get_protocol_by_id(store, &protocol_id)` — assert `GetProtocolResult::Found(val)` with correct `protocolId`
    4. Call `find_protocol_by_target_type(store, &target_type_id)` — assert `Some(result)` with correct fields
    5. Confirm `source_package` is `None` for primary-package protocol

#### Acceptance Criteria

- [ ] Test `protocol_roundtrip_create_list_get_find` exists and passes against `MemoryStore` (not only `FileStore`)
- [ ] `cargo test protocol_roundtrip` passes
- [ ] No `FileStore` or disk I/O required in the roundtrip test

#### Testing

```bash
cargo test -p srs-repository protocol_roundtrip
```

#### Milestone gate

1. Verify all acceptance criteria are met.
2. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Update this plan: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit: `test(protocol-service): add cross-store roundtrip test for Protocol loading (#176)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `Package.protocols: Vec<LoadedProtocol>` populated by both FileStore and JsonStore loaders
- [ ] Read-side service functions (`list_protocols`, `get_protocol_by_id`, `find_protocol_by_target_type`) consume `store.load_package()?.protocols`
- [ ] Cross-store roundtrip test passes against MemoryStore/JsonStore
- [ ] No behavioural regression: `protocol list`, `protocol get`, `protocol stages` output is identical before and after (confirmed in dogfooding)
- [ ] All Package struct literals updated to include `protocols: vec![]`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `ProtocolStage` in `srs-core` captures all documented stage fields (including `completionCriteria`, `contributesTo`, `aiGuidance`, `outputType`). The `LoadedProtocol.raw` field preserves the verbatim JSON for any undocumented extra fields.
- There are two distinct in-memory stores: `crate::store::memory::MemoryStore` (stores a static pre-built `Package` — `load_package()` returns this pre-built struct) and `crate::json_store::JsonStore` (dynamically builds `Package` from its BTreeMap data on every `load_package()` call). The roundtrip test uses `JsonStore` because `create_protocol()` writes to the data store and the next `load_package()` picks it up.
- Unit tests (list/get/find with pre-populated data) use `memory::MemoryStore::with_protocol()` (added in Phase 1) which pre-builds the Package with protocols.
- No `RepositoryStore` trait method changes are needed — the loader code runs inside the store implementations, not as a trait method.
- The `make_package_with_types` helper in `record_store.rs` and similar Package literal constructions will need `protocols: vec![]` added but no other changes.
- `add_definition_to_boundary` with `DefinitionKind::Protocol` already maps to `"protocols"` in package.json (confirmed in `package_types.rs`). No changes to `DefinitionKind` or store boundary logic needed.
