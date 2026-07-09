# Plan: WASM expose list_relation_types binding (#411)

## Summary

`srs-web#160` needs to derive installed relation types from the loaded package rather than hardcoding them in `DecisionLinkPicker`. The `list_relation_types_filtered` service function already exists in `srs-repository` at `crates/srs-repository/src/package_service.rs:355`. This plan adds a thin `list_relation_types` WASM binding in `crates/srs-bindings/src/lib.rs` following the same pattern as `list_fields` and `list_types`, and adds integration tests in `crates/srs-bindings/tests/definition_browse.rs`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Bindings Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM bindings are thin wrappers over `srs-repository` services; no business logic in `srs-bindings` | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions accept typed input structs, not `serde_json::Value` | accepted |

No new ADRs needed — this plan implements an existing binding pattern (ADR-013) against an already-compliant service function. The binding filter struct mirrors `FieldListBindingFilter` / `TypeListBindingFilter`.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands** — this plan adds only a WASM binding, not a CLI handler. No payload structs change. Golden schemas stay as-is.

Verification: `cargo test --test payload_contracts` must still pass after this change.

### Entity schema sync (check-schema-sync.sh)

**No** — no JSON Schema files under `srs/docs/schema/2.0/` are added or modified.

---

## Scope

- Add `list_relation_types(&self, filter_json: &str) -> Result<JsValue, JsValue>` to `SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- Add `RelationTypeListBindingFilter` private struct in `lib.rs` (near other binding filter structs around line 693).
- Import `list_relation_types_filtered` from `srs_repository::package_service` in `lib.rs`.
- Add integration tests for `list_relation_types_filtered` in `crates/srs-bindings/tests/definition_browse.rs`.
- Update `docs/dogfooding.md` coverage matrix to note `list_relation_types` WASM binding.

**Out of scope:**
- srs-web consumer changes (`srs-client.ts`, `GovernanceShell.svelte`) — tracked in srs-web#160.
- A CLI command equivalent — no CLI surface exists for relation types currently; tracked separately.
- Rebuilding or publishing the WASM package artifact — that is a release step, not part of this PR.

---

## Phases

### Phase 1: Add the WASM binding

**Goal:** `list_relation_types` is callable from the `SrsRepository` WASM struct and compiles for the `wasm32-unknown-unknown` target.

**Agent:** Bindings Worker

#### Tasks

- [ ] In `crates/srs-bindings/src/lib.rs`, add `list_relation_types_filtered` to the `use srs_repository::package_service::{...}` import block (line 10–12).
- [ ] Add the `RelationTypeListBindingFilter` private struct near the other binding filter structs (after `TypeListBindingFilter`, around line 703):
  ```rust
  /// Input shape for `list_relation_types` — parsed from caller-supplied JSON.
  /// `filter_json` is `{}` or `{"status": "active"}` to filter by status.
  #[derive(serde::Deserialize, Default)]
  #[serde(rename_all = "camelCase")]
  struct RelationTypeListBindingFilter {
      #[serde(default)]
      status: Option<String>,
  }
  ```
- [ ] Add the `list_relation_types` method to the `impl SrsRepository` block, immediately after `list_types` (around line 523), following the same doc-comment and thin-wrapper pattern:
  ```rust
  /// List relation type definitions from the compiled package.
  /// `filter_json` is `{}` or `{"status": "active"}` to filter by status.
  /// Returns a JS array of `RelationTypeDefinition` objects.
  pub fn list_relation_types(&self, filter_json: &str) -> Result<JsValue, JsValue> {
      let filter: RelationTypeListBindingFilter = serde_json::from_str(filter_json)
          .map_err(|e| js_err(format!("invalid filter: {e}")))?;
      let relation_types =
          package_service::list_relation_types_filtered(&self.store, filter.status)
              .map_err(js_err)?;
      to_js(&relation_types)
  }
  ```

#### Acceptance Criteria

- [ ] `list_relation_types` compiles without warnings: `cargo build -p srs-bindings`
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds (wasm CI target gate)
- [ ] `cargo clippy -p srs-bindings -- -D warnings` is clean
- [ ] No business logic in the binding — it only deserializes the filter, calls the service, and serializes output

#### Testing

```bash
cargo build -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests are written in Phase 2.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
   ```bash
   cargo build -p srs-bindings
   cargo clippy -p srs-bindings -- -D warnings
   ```
3. Commit: `feat(bindings): expose list_relation_types WASM binding (#411)`

---

### Phase 2: Add integration tests

**Goal:** `list_relation_types_filtered` is exercised against the gallery fixture from `definition_browse.rs`, covering the no-filter and status-filter paths.

**Agent:** Bindings Worker

#### Tasks

- [ ] In `crates/srs-bindings/tests/definition_browse.rs`, add to the `use srs_repository::package_service::{...}` import: `list_relation_types_filtered`.
- [ ] Update the file's opening comment to reference `list_relation_types` and the gallery fixture count of 4 relation types.
- [ ] Add a local mirror struct `TestRelationTypeListBindingFilter` (mirrors `RelationTypeListBindingFilter` from `lib.rs`; the existing `TestListBindingFilter` has `namespace`/`package` fields and is not appropriate here):
  ```rust
  #[derive(Deserialize, Default)]
  #[serde(rename_all = "camelCase")]
  struct TestRelationTypeListBindingFilter {
      #[serde(default)]
      status: Option<String>,
  }
  ```
- [ ] Add the following tests after the `list_packages_returns_primary_package` test:
  - `list_relation_types_returns_all_types`: assert `list_relation_types_filtered(&store, None)` returns 4 items (gallery has 4 relation types).
  - `list_relation_types_status_filter_none_match`: assert `list_relation_types_filtered(&store, Some("active".to_string()))` returns 0 items (gallery relation types have no `status` field, so the serialized status is `""`, which does not match `"active"`).
  - `relation_type_filter_json_maps_to_service`: parse `r#"{"status":"active"}"#` as `TestRelationTypeListBindingFilter`, then call `list_relation_types_filtered(&store, raw.status)` and assert 0 results (following the existing `field_filter_json_namespace_and_package_map_to_service_filter` pattern at line 199 of `definition_browse.rs`).
- [ ] Update the file's opening comment (lines 9–13 of `definition_browse.rs`) to add: `//!   - 4 relation types (namespace "governance", no status set)` and reference the new tests in the module comment.

#### Acceptance Criteria

- [ ] `cargo test -p srs-bindings` passes with the three new tests included
- [ ] Gallery fixture count assertions hold: 4 relation types with no status filter; 0 with status `"active"`
- [ ] `relation_type_filter_json_maps_to_service` calls `list_relation_types_filtered` after deserializing (not deserialization-only)
- [ ] No use of `to_js()` in tests (it panics off-wasm — tests call service directly, as per existing convention in this file)

#### Testing

```bash
cargo test -p srs-bindings
```

Specific tests:
- `list_relation_types_returns_all_types` — proves the no-filter path returns all 4 gallery relation types
- `list_relation_types_status_filter_none_match` — proves the status filter path works (filter against gallery where none have a status)
- `relation_type_filter_json_maps_to_service` — proves the JSON → binding filter struct → service call chain (end-to-end, not deserialization-only)
- [ ] Update `docs/dogfooding.md`: add a note to the `relation type` row in the Coverage matrix: `WASM read binding (\`list_relation_types\`) verified via integration tests in \`crates/srs-bindings/tests/definition_browse.rs\` (#411)`.

#### Milestone gate

1. Verify all three tests pass.
2. Run:
   ```bash
   cargo test -p srs-bindings
   cargo clippy -p srs-bindings -- -D warnings
   ```
3. Commit: `test(bindings): integration tests for list_relation_types (#411)`

---

## Final Acceptance

- [x] `cargo test` passes with no failures (srs-gov pre-existing failures excluded)
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds
- [x] `list_relation_types` method is in `srs-bindings/src/lib.rs` with no business logic
- [x] Integration tests in `definition_browse.rs` cover no-filter and status-filter paths
- [x] `docs/dogfooding.md` coverage matrix updated to note `list_relation_types` WASM binding

## Coordination Rules

- Bindings Worker writes only in `crates/srs-bindings/`.
- Lead Integrator reviews API naming, checks filter struct placement matches file conventions, and owns the doc update.
- Verification Agent confirms `cargo test -p srs-bindings` and `cargo clippy -- -D warnings` pass before sign-off.
- **At the end of each phase:** verify all acceptance criteria, confirm tests pass, update plan checkboxes, commit.

## Assumptions

- `RelationTypeDefinition` implements `Serialize` (confirmed: `crates/srs-core/src/types/relation_type_definition.rs` line 9 has `#[derive(... Serialize, Deserialize)]`).
- The gallery fixture has exactly 4 relation types (verified: `package/package.json` `relationTypes` array length = 4).
- None of the gallery relation types have a `status` field set — they will serialize to `""` when status-filtered, so a filter for `"active"` returns 0 results.
- The wasm32 build is gated in CI by `cargo build --target wasm32-unknown-unknown -p srs-bindings` (per ADR-013).
- The binding filter exposes only `status` (not `namespace`/`package`) because `list_relation_types_filtered` in `srs-repository` only accepts `status: Option<String>`. Extending the service filter is out of scope; if needed, that would require a new service filter struct in `srs-repository`.
