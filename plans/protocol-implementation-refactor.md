# Plan: Protocol Implementation Refactor (ext:protocol)

## Summary

The initial `ext:protocol` implementation has several correctness defects that make protocol records invalid from an SRS perspective: field IDs are stored as human-readable strings instead of the UUID field IDs defined in the spec package; records are not indexed in `manifest.json`; and `create_record` silently fails in repos that lack the `meta.protocol` type declaration, causing bespoke fallback code paths to proliferate. This plan corrects all three root causes, removes the fallback code, adds missing `update` and `delete` commands, and ensures all protocol operations go through the canonical record infrastructure.

**PR strategy (from review):** Ship Phase A as a dedicated bugfix PR. Phases B and C follow as separate PRs. This keeps the critical correctness fixes fast-path mergeable.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Phase A Worker | — |
| Phase B Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. This plan corrects the implementation to comply with existing ADR-006.

| ADR | Decision | Status |
|---|---|---|
| [ADR-006](../docs/adr/006-protocol-definitions-are-tier2-records.md) | Protocols stored as generic Tier 2 Records; field IDs from spec package | accepted |

---

## Root Cause Analysis

### Why `create_record` fails in repos without the type declaration

`create_record` calls `package.resolve_type(type_id, type_version)` which looks up the type in `package.json`. Any repo that has not declared the `meta.protocol` type — including all test repos created by `create_temp_repo()` — will get `TypeNotFound`. The fix is **not** to auto-heal this: `import` must fail with an actionable error message instructing the user to declare the `com.semanticops.srs/meta.protocol@1` type in their package (see error messaging task in Phase A). This is the correct SRS behavior — the package is the contract.

### Why field IDs are wrong

`import_protocol` builds `FieldValue` objects with `field_id: "protocol-id"` etc. — human-readable names. The spec type's field assignments use UUID field IDs (e.g. `"6c66d06c-3f95-4d17-8ecf-e1046a6f2ec1"` for `protocol-id`). Records written by `srs protocol import` are therefore invalid when validated against the real spec type.

### Why storage path and manifest indexing are wrong

ADR-006 specifies `records/protocols/<slug>.json`. The implementation uses `package/records/`. Because `create_record` failed, records were never written via the canonical path — only via direct file writes in tests, bypassing `manifest.json` entirely.

### Why `is_protocol_type` is wrong

It checks `type_namespace == "meta"` but the actual namespace is `com.semanticops.srs`. This matches too broadly.

---

## Resolved Design Questions

**Q: Should `protocol import` auto-heal a missing `meta.protocol` type declaration?**
**A: No.** The SRS contract is that a repo's package declares what types it supports. `import` must fail with a clear error: `"Repository package does not declare type 'com.semanticops.srs/meta.protocol@1'. Add it to your package before importing protocols."` Auto-healing would silently create type definitions that may not match the user's package version intentions.

**Q: Is `protocolId` mutable in `protocol update`?**
**A: No.** `protocolId` is identity metadata (analogous to a namespace/name/version triple). It must be immutable on update — `update_protocol` must preserve the stored value of `FIELD_PROTOCOL_ID` and ignore any `protocolId` in the input. Same rule applies to `protocolNamespace`, `protocolName`, `protocolVersion`, and `protocolCreatedAt`. Only `protocolDescription`, `protocolTargetType`, `protocolStages`, and `protocolTags` are mutable.

---

## Contracts

### CLI output contract (ADR-011)

New commands added: `protocol update` and `protocol delete`.

- `protocol update` → reuse existing `ProtocolPayload { protocol: serde_json::Value }` (same as `get`)
- `protocol delete` → add `ProtocolDeletePayload { instance_id: String }` to `payload.rs`

After adding structs: `cargo run --bin generate-schemas` from `srs-rust/` and commit the new `schemas/payload/protocol-update.json` and `schemas/payload/protocol-delete.json` files.

Existing payloads (`protocol-list.json`, `protocol-get.json`, `protocol-stages.json`, `protocol-validate.json`) do not change shape — no schema regeneration needed for those.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync

No changes to `srs/docs/schema/2.0/`. No action required.

---

## Scope

**Phase A (bugfix PR):**
- Remove `list_records_by_type_fallback`, `get_record_by_id_fallback`, and `parse_record_compat` from `protocol_service.rs`
- Fix `import_protocol` to use UUID field IDs; add actionable error when type is undeclared
- Fix `record_to_protocol` to read by UUID field ID
- Fix storage directory to `records/protocols`; fix `is_protocol_type` namespace
- Add validation for required fields, stage shape, and `createdAt` format in `import_protocol`
- Fix integration test setup: `create_temp_repo_with_protocol_type()` helper
- Fix round-trip: `protocol export` output compatible with `protocol import` (Phase C merged into A)

**Phase B (follow-up PR):**
- Add `protocol update` command (immutable fields: id/namespace/name/version/createdAt)
- Add `protocol delete` command

**Out of scope:**
- Protocol execution / stage advancement (deferred by ADR-006)
- `ext:protocol` manifest declaration enforcement
- Multi-package awareness for protocols (defer to after Blueprint lands)

---

## Field ID Constants

The spec type `srs/srs/package/types/meta.protocol.json` (UUID `48a03f5d-4f27-42f4-b791-999f6c22f8d2`) maps field names to IDs as follows. These are the authoritative field IDs for all read and write operations:

| Field name | Mutable on update | UUID field ID |
|---|---|---|
| `protocol-id` | no | `6c66d06c-3f95-4d17-8ecf-e1046a6f2ec1` |
| `protocol-namespace` | no | `8d0f55f9-80e3-4dd6-a05c-10c4b6b6cc87` |
| `protocol-name` | no | `09c5e389-cf6c-4f72-aad6-8cf26bce0b78` |
| `protocol-version` | no | `f7d28d9d-f90c-4a01-a3eb-2ff4cad54ff6` |
| `protocol-description` | yes | `7d1d2f86-b5b6-4f95-82c9-dd8f820b1d04` |
| `protocol-target-type` | yes | `4939a29b-7f70-481f-bf6b-bf693f8bd67f` |
| `protocol-stages` | yes | `0f1232c6-0db5-4383-b91d-64d81195f1c4` |
| `protocol-tags` | yes | `0eafae91-91a8-4115-a95f-fde3d22a87af` |
| `protocol-created-at` | no | `b953f716-383a-4218-bebf-96e93c4747a4` |

---

## Input Validation Rules

These rules apply in `import_protocol` and in `update_protocol` (for mutable fields only).

**Required fields (import):** `protocolId`, `protocolNamespace`, `protocolName`, `protocolVersion`, `protocolTargetType`, `protocolStages`, `protocolCreatedAt`. Missing any → `RepositoryError::InvalidRepositoryInitialization`.

**`protocolStages` shape:** Must be a JSON array. Each element must be an object with:
- `stageId`: non-empty string
- `name`: non-empty string
- `order`: non-negative integer
- `dependsOn`: array of strings (may be empty; all referenced IDs must exist in the same stages array)

Validation failure for any stage → `RepositoryError::InvalidRepositoryInitialization` with a message naming the offending stage and field.

**`protocolCreatedAt` format:** Must be a valid RFC 3339 / ISO 8601 datetime string. Reject malformed values at import time. Use `chrono::DateTime::parse_from_rfc3339` to validate.

**`protocolVersion`:** Must be a positive integer (`>= 1`). Reject `0` or negative values.

---

## Phases

### Phase A: Fix storage, field IDs, validation, and remove fallback paths

**Goal:** `import_protocol` uses UUID field IDs, writes to `records/protocols/`, updates `manifest.json`, fails with an actionable error when the type is undeclared, validates all field shapes at import time, and the fallback scanners are deleted. Export/import round-trip is also fixed here.

**Agent:** Phase A Worker

#### Tasks

- [ ] Add field ID constants and storage constants to `crates/srs-repository/src/protocol_service.rs`:

  ```rust
  const FIELD_PROTOCOL_ID:          &str = "6c66d06c-3f95-4d17-8ecf-e1046a6f2ec1";
  const FIELD_PROTOCOL_NAMESPACE:   &str = "8d0f55f9-80e3-4dd6-a05c-10c4b6b6cc87";
  const FIELD_PROTOCOL_NAME:        &str = "09c5e389-cf6c-4f72-aad6-8cf26bce0b78";
  const FIELD_PROTOCOL_VERSION:     &str = "f7d28d9d-f90c-4a01-a3eb-2ff4cad54ff6";
  const FIELD_PROTOCOL_DESCRIPTION: &str = "7d1d2f86-b5b6-4f95-82c9-dd8f820b1d04";
  const FIELD_PROTOCOL_TARGET_TYPE: &str = "4939a29b-7f70-481f-bf6b-bf693f8bd67f";
  const FIELD_PROTOCOL_STAGES:      &str = "0f1232c6-0db5-4383-b91d-64d81195f1c4";
  const FIELD_PROTOCOL_TAGS:        &str = "0eafae91-91a8-4115-a95f-fde3d22a87af";
  const FIELD_PROTOCOL_CREATED_AT:  &str = "b953f716-383a-4218-bebf-96e93c4747a4";
  const PROTOCOL_TYPE_ID:           &str = "48a03f5d-4f27-42f4-b791-999f6c22f8d2";
  const PROTOCOL_TYPE_VERSION:      u32  = 1;
  const PROTOCOL_STORAGE_DIR:       &str = "records/protocols";
  ```

- [ ] In `import_protocol`, before building `field_values`:
  1. Check all required fields are present in the input JSON; return `RepositoryError::InvalidRepositoryInitialization` with a message listing missing fields if not.
  2. Validate `protocolVersion >= 1`.
  3. Validate `protocolCreatedAt` parses as RFC 3339 via `chrono::DateTime::parse_from_rfc3339`.
  4. Validate `protocolStages` is a JSON array; for each stage element validate `stageId` (non-empty string), `name` (non-empty string), `order` (non-negative integer), `dependsOn` (array of strings). All `dependsOn` IDs must appear as `stageId` values in the same array.

- [ ] Update `import_protocol` to use `FIELD_*` constants for all `FieldValue.field_id` assignments, `PROTOCOL_TYPE_ID`/`PROTOCOL_TYPE_VERSION` in `create_record`, and `PROTOCOL_STORAGE_DIR` as the directory argument. When `create_record` returns `TypeNotFound`, re-wrap as `RepositoryError::InvalidRepositoryInitialization` with the message: `"Repository package does not declare type 'com.semanticops.srs/meta.protocol@1'. Add it to your package before importing protocols."`.

- [ ] Update `find_field_value` call sites in `record_to_protocol` and `record_to_protocol_summary` to use `FIELD_*` UUID constants. Remove the `or_else(|| find_field_value(fv, &key.replace("-", "_")))` fallback from `get_string_field` and `get_i32_field` — UUID matching is unambiguous.

- [ ] Fix `is_protocol_type` to check `type_namespace == "com.semanticops.srs" && type_name == "meta.protocol"`.

- [ ] Update `list_protocols` to call `list_records_by_type(store, "com.semanticops.srs", "meta.protocol")`.

- [ ] Delete: `list_records_by_type_fallback`, `get_record_by_id_fallback`, `parse_record_compat`. Remove the fallback branch in `get_protocol_struct_by_id` — only call `get_record_by_id`.

- [ ] Fix `cmd_protocol_export` in `crates/srs-cli/src/commands/protocol.rs`: serialize the `Protocol` struct (which already has `#[serde(rename_all = "camelCase")]`) directly — do not inject `instanceId`. The export format is the canonical import format.

- [ ] Add `create_temp_repo_with_protocol_type()` helper to `crates/srs-cli/tests/integration_tests.rs`. It must write:
  - A valid `package/package.json` that declares the `meta.protocol` type (UUID `48a03f5d-4f27-42f4-b791-999f6c22f8d2`, version 1) and all 9 field definitions by UUID.
  - The 9 field JSON files under `package/fields/`.
  - The type JSON file under `package/types/`.

  These fixture files must match exactly the field UUIDs in the constants table above.

- [ ] Rewrite `protocol_list_returns_protocols`, `protocol_get_returns_protocol_by_id`, and `protocol_stages_returns_ordered_stages` to use `create_temp_repo_with_protocol_type()` and call `srs protocol import` (via `run_srs_in_dir_with_stdin`) instead of writing files directly.

#### Acceptance Criteria

- [ ] `srs protocol import` succeeds against a repo with `meta.protocol` declared in its package
- [ ] `srs protocol import` against a repo without `meta.protocol` returns `ok: false` with message containing `"Add it to your package before importing protocols"`
- [ ] Records written by `import_protocol` have UUID field IDs — confirmed via `srs record get` showing UUID keys in `fieldValues`
- [ ] `srs repo validate` reports 0 errors for a repo containing a valid protocol record
- [ ] `srs record list` includes the protocol record (it is indexed in `manifest.json`)
- [ ] `import_protocol` rejects: missing required fields, `protocolVersion < 1`, malformed `protocolCreatedAt`, stage with empty `stageId`, stage with `dependsOn` referencing a nonexistent stage
- [ ] `srs protocol export <id> | srs protocol import --repo <other-repo>` succeeds end-to-end
- [ ] Export output does not contain `instanceId`
- [ ] `grep -n "list_records_by_type_fallback\|get_record_by_id_fallback\|parse_record_compat" crates/srs-repository/src/protocol_service.rs` returns empty
- [ ] All 3 rewritten protocol integration tests pass

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository
cargo test --test integration_tests -- protocol_
cargo clippy -p srs-repository -p srs-cli -- -D warnings
```

Specific tests to write or verify:
- `protocol_list_returns_protocols` — import via CLI, then list
- `protocol_get_returns_protocol_by_id` — import, then get by instance ID
- `protocol_stages_returns_ordered_stages` — import 3-stage protocol, verify stage order
- `protocol_import_fails_without_type_declaration` — fresh minimal repo, expect `ok: false` with actionable message
- `protocol_import_rejects_missing_required_field` — omit `protocolTargetType`, expect `ok: false`
- `protocol_import_rejects_invalid_version` — `protocolVersion: 0`, expect `ok: false`
- `protocol_import_rejects_malformed_created_at` — `protocolCreatedAt: "not-a-date"`, expect `ok: false`
- `protocol_import_rejects_stage_with_bad_depends_on` — stage `dependsOn: ["nonexistent"]`, expect `ok: false`
- `protocol_export_import_roundtrip` — export, import into fresh repo, assert `protocolId`, `stageCount`, and all stage `stageId`/`name`/`order` values match field-by-field (not just `ok: true`)

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed above exists in the codebase and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo test --test integration_tests -- protocol_
cargo clippy -- -D warnings
```

4. Update plan checkboxes.
5. Commit as `fix: correct ext:protocol field IDs, storage path, manifest indexing, and input validation`.

---

### Phase B: Add `protocol update` and `protocol delete` commands

**Goal:** `ProtocolCommand` has `Update` and `Delete` variants with correct immutability semantics; golden schemas committed; payload contract tests pass.

**Agent:** Phase B Worker

**Prerequisite:** Phase A merged. Records are properly indexed in `manifest.json` (delete requires finding and removing the manifest entry).

#### Tasks

- [ ] Add to `crates/srs-cli/src/payload.rs`:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ProtocolDeletePayload {
      pub instance_id: String,
  }
  ```
  `protocol update` reuses the existing `ProtocolPayload`.

- [ ] Add to `crates/srs-repository/src/protocol_service.rs`:

  ```rust
  pub struct UpdateProtocolInput { pub raw: serde_json::Value }
  pub struct UpdateProtocolResult { pub instance_id: String, pub record: Record }
  pub struct DeleteProtocolResult { pub instance_id: String }
  ```

  `update_protocol(store, id, input) -> Result<UpdateProtocolResult, RepositoryError>`:
  1. Load existing record via `get_record_by_id`; return `NotFound` if absent or not a protocol type.
  2. Extract mutable fields from `input.raw` using the shared extraction helper (same camelCase key logic as `import_protocol`): `protocolDescription`, `protocolTargetType`, `protocolStages`, `protocolTags`.
  3. If `protocolStages` is present in input, validate full stage shape (same rules as import).
  4. Build the updated `field_values: Vec<FieldValue>` by taking the existing record's values as base, then overwriting only the 4 mutable field slots by UUID. Identity fields (`FIELD_PROTOCOL_ID`, `FIELD_PROTOCOL_NAMESPACE`, `FIELD_PROTOCOL_NAME`, `FIELD_PROTOCOL_VERSION`, `FIELD_PROTOCOL_CREATED_AT`) are always sourced from the existing record — ignore any matching keys in `input.raw`.
  5. Call `record_store::update_record(store, &instance_id, updated_field_values)` — its actual signature is `(store: &dyn RepositoryStore, instance_id: &str, field_values: Vec<FieldValue>) -> Result<Record, RepositoryError>`. It looks up the record by ID, validates the updated values against the type, overwrites the file in place, and updates the manifest entry. Do not pass a path or the full Record struct.

  `delete_protocol(store, id) -> Result<DeleteProtocolResult, RepositoryError>`:
  1. Load existing record via `get_record_by_id`; return `NotFound` if absent or not a protocol type.
  2. Call `delete_record(store, id)` from `record_store` (removes file + manifest entry).
  3. Verify: after delete, `get_record_by_id(store, id)` returns `None` and the file does not exist on disk.

- [ ] Add `ProtocolCommand::Update { id: String }` and `ProtocolCommand::Delete { id: String }` variants to `commands/mod.rs` after existing variants.

- [ ] Add `cmd_protocol_update` and `cmd_protocol_delete` handlers to `crates/srs-cli/src/commands/protocol.rs`; wire in `dispatch`.

- [ ] Run `cargo run --bin generate-schemas`; commit `schemas/payload/protocol-update.json` and `schemas/payload/protocol-delete.json`.

- [ ] Add payload contract tests to `crates/srs-cli/tests/payload_contracts.rs` for both new commands.

- [ ] Add integration tests:
  - `protocol_update_modifies_stages` — import protocol with 1 stage; update with 2 stages; verify `protocol stages` returns 2 stages with correct `stageId`/`name`/`order`
  - `protocol_update_preserves_identity_fields` — update with different `protocolId`, `protocolNamespace`, `protocolVersion`, `protocolCreatedAt` in input; verify all four are unchanged in the returned payload
  - `protocol_update_file_and_manifest_consistent` — after update, `srs record get <instanceId>` and `srs protocol get <instanceId>` both show the updated stages; `srs repo validate` reports 0 errors
  - `protocol_delete_removes_record` — import, delete, verify `protocol list` returns 0 records, `srs record list` returns 0 records, `srs repo validate` reports 0 errors
  - `protocol_delete_not_found_returns_error` — delete with nonexistent ID returns `ok: false`

#### Acceptance Criteria

- [ ] `srs protocol update <id>` updates mutable fields; identity fields and `createdAt` are unchanged
- [ ] `srs protocol delete <id>` removes the file and the `manifest.json` entry; `srs repo validate` is clean afterward
- [ ] `protocol delete` on nonexistent ID returns `ok: false`
- [ ] File state and manifest state are consistent after both update and delete (no divergence)
- [ ] `cargo test --test payload_contracts` passes
- [ ] All 5 new integration tests pass

#### Testing

```bash
cd srs-rust
cargo test --test integration_tests -- protocol_
cargo test --test payload_contracts -- protocol
cargo clippy -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm all 5 new integration tests exist and pass.
3. Run:

```bash
cargo build
cargo test
cargo clippy -- -D warnings
bash scripts/check-schema-sync.sh
```

4. Update plan checkboxes.
5. Commit as `feat: add protocol update and delete commands`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `grep -n "list_records_by_type_fallback\|get_record_by_id_fallback\|parse_record_compat" crates/srs-repository/src/protocol_service.rs` returns empty
- [ ] `srs protocol import` writes records with UUID field IDs (verified via `srs record get`)
- [ ] `srs protocol import` against a repo missing `meta.protocol` type returns `ok: false` with actionable message
- [ ] `srs repo validate` reports 0 errors for a repo containing a valid protocol record
- [ ] `srs record list` includes protocol records
- [ ] `srs protocol export <id> | srs protocol import --repo <fresh-repo>` succeeds; field-by-field equality confirmed
- [ ] `srs protocol update <id>` updates only mutable fields; identity fields preserved
- [ ] `srs protocol delete <id>` leaves file system and manifest consistent
- [ ] Schema files committed: `schemas/payload/protocol-update.json`, `schemas/payload/protocol-delete.json`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- The spec package type at `srs/srs/package/types/meta.protocol.json` with UUID `48a03f5d-4f27-42f4-b791-999f6c22f8d2` and field UUIDs as listed above is correct and stable. Do not change these UUIDs.
- These functions exist in `crates/srs-repository/src/record_store.rs` with the following exact signatures (verified on both `main` and `feat/multi-package-awareness`):
  - `create_record(store, type_id: &str, type_version: u32, field_values: Vec<FieldValue>, relative_dir: &str) -> Result<Record, RepositoryError>` — writes file, updates `manifest.json`.
  - `update_record(store, instance_id: &str, field_values: Vec<FieldValue>) -> Result<Record, RepositoryError>` — looks up record by ID, validates against type, overwrites file in place, updates manifest. Does NOT take a path or full Record struct.
  - `delete_record(store, instance_id: &str) -> Result<String, RepositoryError>` — removes file and manifest entry, returns instance_id.
- Test repos that need protocol support must declare the `meta.protocol` type in their `package.json`. A `create_temp_repo_with_protocol_type()` helper in the integration test file is the right place for this fixture.
- Phase C (export/import round-trip fix) is merged into Phase A — it is not a separate phase.
- **Branch compatibility:** This plan is fully compatible with `feat/multi-package-awareness`. `protocol_service.rs` is unchanged on that branch. No shared type, trait, or function signatures conflict. This plan can be implemented on `main` or rebased on that branch once it lands.
