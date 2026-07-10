# Plan: Refactor list_relation_types_filtered to accept a typed filter struct (ADR-010)

## Summary

`list_relation_types_filtered` in `crates/srs-repository/src/package_service.rs` accepts a bare `status: Option<String>` parameter. This violates ADR-010 §Filtering: "List functions accept a filter struct rather than exposing multiple service functions for different filter combinations." Peer functions `list_fields_filtered` and `list_types_filtered` already use `FieldListFilter` / `TypeListFilter`. This plan introduces `RelationTypeListFilter` (matching the singular naming convention of the existing structs) and updates all call sites in `srs-cli` and `srs-bindings`. There are no payload, schema, or CLI output shape changes.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude |
| Repository Service Worker | Claude |
| CLI Worker | Claude |
| Bindings Worker | Claude |
| Verification Agent | Claude |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | List functions accept a filter struct — this plan brings `list_relation_types_filtered` into conformance | accepted |

No new ADRs needed — this plan implements the existing ADR-010 §Filtering rule.

**Naming note:** The issue body suggests `RelationTypesListFilter` (plural). This plan uses `RelationTypeListFilter` (singular) to match the established convention of `FieldListFilter` / `TypeListFilter`.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands, no payload struct changes. `RelationTypeListPayload` in `srs-cli` is unchanged. No schema regeneration required.

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are changed. No sync action required.

---

## Scope

- Define `RelationTypeListFilter { status: Option<String> }` in `crates/srs-repository/src/package_service.rs` and make it `pub`.
- Change `list_relation_types_filtered(store, status: Option<String>)` → `list_relation_types_filtered(store, filter: RelationTypeListFilter)` in the same file.
- Update the CLI call site in `crates/srs-cli/src/commands/relation_type.rs:53`.
- Update the WASM binding call site in `crates/srs-bindings/src/lib.rs:549`.
- Update all test call sites in `crates/srs-bindings/tests/definition_browse.rs` (lines 283, 292, 308).

**Out of scope:**
- Adding namespace/package filtering to `RelationTypeListFilter` — the current filter only needs `status`; future expansion is a separate issue.
- Any CLI command or payload shape change.
- Changes to `srs-core`, `srs-projection`, or `srs-vscode`.

---

## Phases

### Phase 1: Introduce RelationTypeListFilter and update all call sites

**Goal:** All four call sites compile and pass tests with the typed filter struct; no bare `Option<String>` remains as the second parameter of `list_relation_types_filtered`.

**Agent:** Repository Service Worker + CLI Worker + Bindings Worker (single integrator)

#### Tasks

- [x] In `crates/srs-repository/src/package_service.rs`, add the following struct immediately after `TypeListFilter` (around line 313):
  ```rust
  /// Filter options for listing relation types
  #[derive(Debug, Clone, Default)]
  pub struct RelationTypeListFilter {
      /// If Some, only return relation types whose serialized status string matches.
      pub status: Option<String>,
  }
  ```
- [x] Update the doc comment on `list_relation_types_filtered` (lines 351-354) to reference `filter.status` instead of the old `status` parameter name:
  ```rust
  /// List relation type definitions, optionally filtered by status.
  ///
  /// If `filter.status` is `None`, all definitions are returned. If `Some`, only definitions
  /// whose serialized status string matches are returned.
  ```
- [x] Change the signature and body of `list_relation_types_filtered` (currently at line 355) from:
  ```rust
  pub fn list_relation_types_filtered(
      store: &dyn RepositoryStore,
      status: Option<String>,
  ) -> Result<Vec<RelationTypeDefinition>, RepositoryError> {
      ...
      Ok(if let Some(ref status_filter) = status {
          ...
          .filter(|rtd| { ... &serialized == status_filter })
          ...
      } else { defs })
  }
  ```
  to:
  ```rust
  pub fn list_relation_types_filtered(
      store: &dyn RepositoryStore,
      filter: RelationTypeListFilter,
  ) -> Result<Vec<RelationTypeDefinition>, RepositoryError> {
      ...
      Ok(if let Some(ref status_filter) = filter.status {
          ...
          .filter(|rtd| { ... &serialized == status_filter })
          ...
      } else { defs })
  }
  ```
- [x] In `crates/srs-cli/src/commands/relation_type.rs`, import `RelationTypeListFilter` alongside `list_relation_types_filtered` (line 7). Update the call at line 53:
  ```rust
  Ok(list_relation_types_filtered(store, RelationTypeListFilter { status: status_filter })?)
  ```
- [x] In `crates/srs-bindings/src/lib.rs`, add `RelationTypeListFilter` to the named import block at line 12-14 (alongside `FieldListFilter`, `GetFieldResult`, `GetTypeResult`, `TypeListFilter`). Update the call at line 549:
  ```rust
  package_service::list_relation_types_filtered(
      &self.store,
      RelationTypeListFilter { status: filter.status },
  )
  ```
- [x] In `crates/srs-bindings/tests/definition_browse.rs`, import `RelationTypeListFilter` in the use statement (line 20). Update the three call sites:
  - Line 283: `list_relation_types_filtered(&store, RelationTypeListFilter::default())`
  - Line 292: `list_relation_types_filtered(&store, RelationTypeListFilter { status: Some("active".to_string()) })`
  - Line 308: `list_relation_types_filtered(&store, RelationTypeListFilter { status: raw.status })`

#### Acceptance Criteria

- [x] `cargo build` succeeds with zero errors.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `cargo test -p srs-repository` passes (existing tests).
- [x] `cargo test -p srs-bindings` passes — specifically `list_relation_types_returns_all_types`, `list_relation_types_status_filter_none_match`, `relation_type_filter_json_maps_to_service`.
- [x] `cargo test --test payload_contracts` passes (no payload regressions).
- [x] No bare `Option<String>` remains as the second argument of `list_relation_types_filtered` in any call site.

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-bindings
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

Specific tests:
- `list_relation_types_returns_all_types` — proves default filter returns all 4 relation types
- `list_relation_types_status_filter_none_match` — proves status filter works via struct
- `relation_type_filter_json_maps_to_service` — proves JSON → struct → service chain intact

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every named test exists in the codebase and passes.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo test -p srs-bindings
   cargo test -p srs-cli
   cargo test --test payload_contracts
   cargo clippy -- -D warnings
   ```
4. Update plan checkboxes `[x]`.
5. Commit:
   ```bash
   git commit -m "refactor: introduce RelationTypeListFilter for list_relation_types_filtered (ADR-010) (#445)"
   ```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] No bare `Option<String>` as second arg of `list_relation_types_filtered` anywhere in the codebase

## Coordination Rules

- Single integrator (Claude) handles all four file changes in sequence.
- Verification after milestone gate before commit.

## Assumptions

- The `status` field serialization logic in the function body (using `serde_json::to_value`) is correct and unchanged — this plan only wraps the parameter.
- No other crates import `list_relation_types_filtered` beyond the four files identified.
- **Pre-existing ADR-011 violation (out of scope):** Five payload structs (`RelationTypeListPayload`, `RelationTypeGetPayload`, `RelationTypeCreatePayload`, `RelationTypeUpdatePayload`, `RelationTypeDeletePayload`) are defined in `crates/srs-cli/src/commands/relation_type.rs` rather than `crates/srs-cli/src/payload.rs`. ADR-011 requires all CLI output structs to live in `payload.rs` with `schemars::JsonSchema` derives. This violation predates this plan and is tracked separately (filed as a follow-up issue during plan review).
