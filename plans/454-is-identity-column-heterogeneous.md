# Plan: isIdentityColumn marking for heterogeneous (multi-Type) containers (#454)

## Summary

ADR-023 scoped `ColumnSpec.isIdentityColumn` to containers whose `DocumentView.root_type_refs` has exactly one entry. Containers with multi-entry `root_type_refs` get `isIdentityColumn: false` on every column — even when every Type in the container has a well-defined `identityFieldId`. This is a deliberate scope cut, noted in ADR-023's Consequences as a follow-up candidate.

This plan implements the "common-identity" extension: when all Types listed in a multi-entry `root_type_refs` agree on the same effective `identityFieldId`, `resolve_columns` can still mark that column unambiguously. If any Type lacks an identity field or disagrees with the others, the behavior falls back to `false` on every column (preserving the existing semantics for truly ambiguous cases). A new ADR-027 documents this extension.

The change is contained to `crates/srs-repository/src/container_view_service.rs` — the same file ADR-023 was implemented in. No new parameters, no new crates, no schema or payload changes.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-023](../docs/adr/023-columnspec-identity-column-marker.md) | `isIdentityColumn` scoped to single-entry `root_type_refs` — this plan extends it | accepted |
| [ADR-027](../docs/adr/027-identity-column-multi-type-extension.md) | Common-identity extension: multi-entry `root_type_refs` can mark an identity column when all entries agree | proposed (this plan) |

No new ADRs are needed beyond ADR-027 — this plan adds no new service boundaries, CLI commands, or payload changes.

---

## Contracts

### CLI output contract (ADR-011)

`container resolve-view`'s `ContainerViewPayload.container_view` embeds `ContainerView` via `#[schemars(with = "serde_json::Value")]` (opaque). The change to `is_identity_column` computation changes behavior of an already-existing field on an opaque type — **no golden-schema regeneration required**. `cargo test --test payload_contracts` must still pass unmodified.

### Entity schema sync (check-schema-sync.sh)

No entity schemas change. `bash scripts/check-schema-sync.sh` must pass unmodified.

---

## Scope

- `crates/srs-repository/src/container_view_service.rs` — extend `resolve_columns` to handle multi-entry `root_type_refs` by computing a common identity field ID; update two stale doc comments.
- `docs/adr/027-identity-column-multi-type-extension.md` — pre-authored ADR documenting the extension; flip status from `proposed` to `accepted` when implementation commits.

**Out of scope:**

- Per-member identity field ID on `ResolvedMember` (the "richer per-row structure" mentioned in the issue). This would add a new field to `ResolvedMember`, changing the public API shape; it's deferred to a follow-up issue until a concrete consumer need surfaces (blueprint-backed editors).
- Cases where Types in a multi-entry `root_type_refs` have *different* identity fields — these containers still get `isIdentityColumn: false` on every column (unambiguous semantics).
- Any consumer-side changes in `srs-web` or `srs-bindings` — the WASM binding and CLI already pass through the field correctly.

---

## Phases

### Phase 1: Implement common-identity extension in `resolve_columns`

**Goal:** `resolve_columns` marks the identity column when all Types in a multi-entry `root_type_refs` share the same effective `identityFieldId`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Verify `docs/adr/027-identity-column-multi-type-extension.md` exists and is consistent with the implementation below. It is pre-authored with `Status: proposed`; do not modify its content unless the implementation diverges from its Decision section.

- [ ] Add test fixture infrastructure in `crates/srs-repository/src/container_view_service.rs` (test module):
  - Add `const TYPE_ID_2: &str = "00000000-0000-4000-8000-00000000bbbb";`
  - Add a `record_type_with_identity_v2(identity_field_id: Option<&str>) -> RecordType` helper — same structure as the existing `record_type_with_identity` but uses `TYPE_ID_2` as the type id. This is needed for multi-type tests where two distinct types must both be installed in the store.

- [ ] In `crates/srs-repository/src/container_view_service.rs`, extract the identity-field lookup (currently lines 312-315) into a private helper `common_identity_field`:

  ```rust
  fn common_identity_field<'a>(
      dv: &DocumentView,
      identity_field_index: &'a record_label::IdentityFieldIndex,
  ) -> Option<&'a String> {
      let refs = dv.root_type_refs.as_deref()?;
      if refs.is_empty() {
          return None;
      }
      let first = identity_field_index.get(&(refs[0].type_id.clone(), refs[0].type_version))?;
      let all_agree = refs[1..]
          .iter()
          .all(|r| identity_field_index.get(&(r.type_id.clone(), r.type_version)) == Some(first));
      if all_agree { Some(first) } else { None }
  }
  ```

- [ ] In `resolve_columns`, replace:
  ```rust
  let identity_field_id: Option<&String> = match dv.root_type_refs.as_deref() {
      Some([single]) => identity_field_index.get(&(single.type_id.clone(), single.type_version)),
      _ => None,
  };
  ```
  with:
  ```rust
  let identity_field_id = common_identity_field(dv, identity_field_index);
  ```

- [ ] Update the `ColumnSpec.is_identity_column` field doc (approximately lines 46-50) to: "True when this column's `fieldId` is the effective `identityFieldId` shared by **all** Types in the DocumentView's `root_type_refs` — see ADR-023 (single-entry case) and ADR-027 (common-identity multi-entry extension). `false` whenever that resolution is absent, ambiguous, or any referenced Type disagrees. Never affects column order."

- [ ] Update the `resolve_columns` doc comment (approximately lines 272-281): remove the phrase "must have exactly one entry" and replace with a description of the common-identity rule — when all `root_type_refs` entries agree on the same field ID (via `common_identity_field`), that column is marked `true`; all other cases yield `false`. Cross-reference ADR-023 and ADR-027.

- [ ] Rename the existing test `resolve_container_view_ambiguous_root_type_refs_all_columns_false` to `resolve_container_view_disagreeing_root_type_refs_all_columns_false`. Update its docstring to clarify: the scenario has two `root_type_refs` entries but one is absent from the identity index → still all `false`, no behavior change.

- [ ] Add a new test `resolve_container_view_marks_identity_column_when_all_types_agree`:
  - Two `root_type_refs` entries: `TYPE_ID` (version 1) with `identityFieldId: "f-title"` and `TYPE_ID_2` (version 1) with `identityFieldId: "f-title"`.
  - Both RecordTypes installed in the store via `build_store_with_types`.
  - Assert: `f-title` column has `is_identity_column: true`; `f-status` column has `is_identity_column: false`.

- [ ] Add a new test `resolve_container_view_no_signal_when_types_disagree_on_identity`:
  - Two `root_type_refs` entries: `TYPE_ID` with `identityFieldId: "f-title"`, `TYPE_ID_2` with `identityFieldId: "f-status"`.
  - Both RecordTypes installed in the store.
  - Assert: all columns `is_identity_column: false` (types disagree → no column-level signal).

- [ ] Add a new test `resolve_container_view_no_signal_when_one_type_has_no_identity`:
  - Two `root_type_refs` entries: `TYPE_ID` with `identityFieldId: "f-title"`, `TYPE_ID_2` with `identityFieldId: None`.
  - Both RecordTypes installed in the store.
  - Assert: all columns `is_identity_column: false` (one type has no identity → can't agree).

- [ ] Add a cross-store roundtrip test `resolve_view_roundtrip_marks_identity_column_when_all_types_agree`:
  - Use `build_store_with_types` with two RecordTypes (TYPE_ID and TYPE_ID_2) both declaring `identityFieldId: "f-title"`.
  - Copy repository to a `FileStore` via `copy_repository`.
  - Call `resolve_container_view` on the FileStore.
  - Assert that `is_identity_column: true` on the `f-title` column survives the roundtrip.

#### Acceptance Criteria

- [ ] `resolve_columns` calls `common_identity_field` — no inline `match dv.root_type_refs` remains.
- [ ] `ColumnSpec.is_identity_column` field doc updated to reference ADR-023 and ADR-027; "single Type" / "exactly one entry" language removed.
- [ ] `resolve_columns` doc comment updated to describe the common-identity rule; "exactly one entry" constraint replaced with ADR-027 reference.
- [ ] All existing ADR-023 tests (single-entry, no identity, explicit `view_id`) still pass unmodified.
- [ ] New "all agree" test passes: identity column marked correctly with two-type container.
- [ ] New "disagree" test passes: all columns `false` when types differ.
- [ ] New "one type has no identity" test passes: all columns `false`.
- [ ] Cross-store roundtrip test passes: `is_identity_column: true` survives FileStore roundtrip.
- [ ] ADR-027 status flipped from `proposed` to `accepted`.

#### Testing

```bash
cargo test -p srs-repository container_view_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `resolve_container_view_marks_identity_column_when_all_types_agree` — the primary new case
- `resolve_container_view_no_signal_when_types_disagree_on_identity` — disagreement → all false
- `resolve_container_view_no_signal_when_one_type_has_no_identity` — partial absence → all false
- `resolve_container_view_marks_identity_column_for_single_type_container` — must still pass
- `resolve_container_view_disagreeing_root_type_refs_all_columns_false` — renamed from `ambiguous`
- `resolve_view_roundtrip_marks_identity_column_when_all_types_agree` — FileStore roundtrip

#### Milestone gate

1. Verify all acceptance criteria — check each checkbox.
2. Run:
   ```bash
   cargo test -p srs-repository container_view_service
   cargo clippy -p srs-repository -- -D warnings
   ```
3. Mark completed checkboxes `[x]`.
4. Flip `docs/adr/027-identity-column-multi-type-extension.md` status from `proposed` to `accepted`.
5. Commit: `git commit -m "feat(srs-repository): isIdentityColumn for common-identity multi-type containers (#454)"`

---

### Phase 2: Full workspace verification

**Goal:** Workspace is green; no regressions; payload and schema contracts hold.

**Agent:** Verification Agent

#### Tasks

- [ ] Run full workspace tests and lint.
- [ ] Confirm `cargo test --test payload_contracts` passes with no schema file changes.
- [ ] Confirm `bash scripts/check-schema-sync.sh` exits 0.
- [ ] Confirm `srs-bindings` has no duplicated `common_identity_field`/identity logic — it calls the same `container_view_service` via `resolve_container_view`.
- [ ] File a follow-up issue for per-member `identity_field_id` on `ResolvedMember` (deferred from out-of-scope): title "feat: add identity_field_id to ResolvedMember for per-row identity marking in heterogeneous containers". Link the filed issue to the same parent epic as issue #454.

#### Acceptance Criteria

- [ ] `cargo test` passes workspace-wide.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] `cargo test --test payload_contracts` passes, no schema changes.
- [ ] `check-schema-sync.sh` exits 0.
- [ ] Follow-up issue filed and linked to the parent epic.

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
bash scripts/check-schema-sync.sh
```

#### Milestone gate

Report results to the issue thread. No commit required on a clean pass (this phase produces no code changes).

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no schema regeneration)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `ColumnSpec.is_identity_column: true` is marked when all `root_type_refs` types share the same identity field (new case)
- [ ] `ColumnSpec.is_identity_column` remains `false` for all columns when any type in `root_type_refs` is absent from the index or disagrees (no regression)
- [ ] Column order unchanged in all cases (ADR-023 / ADR-027)
- [ ] ADR-027 status is `accepted`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit.

## Assumptions

- `identity_field_index` is correctly pre-built in `resolve_container_view` before being passed to `resolve_columns` — no second Package load needed in `common_identity_field`.
- The follow-up issue for per-member `identity_field_id` on `ResolvedMember` is filed in Phase 2 and linked to the same parent epic as issue #454.
- PR creation is handled by the `/ship` pipeline (Stage 8), not by the plan phases — this plan covers only the service implementation.
