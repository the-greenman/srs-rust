# Plan: render_service — per-record heading falls back to identityFieldId (RFC-020 Rule [N+37])

> **Issue:** srs-rust #453
> **RFC:** srs#144 / srs#148 (RFC-020, accepted)

## Summary

RFC-020 Rule [N+37] (already accepted) states that for any `DocumentSection` that does not declare
`titleFieldId`, per-record heading emission SHOULD use the record's Type's effective `identityFieldId`
— for both the Default Rendering Baseline (`renderViewId` absent) and a dispatched L1 View.
`titleFieldId`, when declared, continues to take precedence. This plan implements Rule [N+37] in
`render_service.rs` — a separate render capability from the record-label resolution (#376 already
shipped) that lives in `record_store.rs` / `tree_service.rs` / etc.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Main session |
| Repository Service Worker | Main session |
| Verification | Main session (cargo test + clippy) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All business logic (heading resolution) stays inside the `srs-repository` service function; no logic migrated to `srs-cli`. Note: the pre-existing deviation where `render_service` combines rendering and projection in one service module (rather than `srs-projection`) is out of scope for this plan. | accepted |
| [ADR-023](../docs/adr/023-columnspec-identity-column-marker.md) | ADR-023 prohibits per-record calls to `effective_identity_field_id` that duplicate an already-computed index. This plan calls it once per record inside `render_service`, where no such index is built and the Package is already loaded. This is not a duplicate — there is no pre-existing index to reuse in the render path. The prohibition in ADR-023 targets `container_view_service` where an explicit index is built and reused for the whole call. We extract a private `resolve_heading_field_id` helper so the Rule [N+37] logic lives at a single named site and is shared between `render_record_at_level` and `project_record_json`. | accepted |

No new ADRs are needed: this is a pure implementation of an already-accepted RFC rule within an existing service function. The existing `Package::effective_identity_field_id` method (added in #376) provides the resolution API. No new public boundary is introduced.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** The only changed functions are `render_record_at_level` (internal
markdown/HTML render) and `project_record_json` (internal JSON projection). The `RenderResult` and
`DocumentViewProjection` payload structs are unchanged. The `record_heading: Option<String>` field
in `ProjectedRecord` was already defined; we're now populating it in more cases. No `payload.rs`
changes, no schema regeneration needed.

Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed. `bash scripts/check-schema-sync.sh` will continue to exit 0.

---

## Scope

- Extract a private `resolve_heading_field_id` helper (before the test module in `render_service.rs`) that encapsulates Rule [N+37]: returns `section.title_field_id` when present, otherwise the Type's `effective_identity_field_id` when the Type is known.
- Implement Rule [N+37] in `render_record_at_level` (Default Rendering Baseline and L1 View dispatch, markdown/HTML format): call `resolve_heading_field_id` instead of inspecting `section.title_field_id` directly.
- Implement Rule [N+37] in `project_record_json` (JSON projection path): call `resolve_heading_field_id` for the `record_heading` computation.
- Add unit tests (MemoryStore-based) covering the new fallback, the precedence rule, the no-op case, and a FileStore-based roundtrip test per CLAUDE.md cross-store requirement.

**Out of scope:**
- `titleFieldId` skipping the field from the body when the fallback fires (structured mode stays tied to explicit `section.title_field_id`; the identity field still appears in the body field list). Deferred — this is not mandated by Rule [N+37].
- Subsection recursion (`structured` flag) for the identity-field fallback path. The subsection recursion is explicitly associated with the explicit `titleFieldId` contract; extending it to the identity-field fallback is a separate decision.
- Any changes to `record_label`, `tree_service`, `discovery_service`, `repository_navigation_service`, `container_view_service`, or `text_projection` (these were #376's scope).

---

## Phases

### Phase 0: API verification

**Goal:** Confirm the exact API signatures before touching production code.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Confirm `Package::effective_identity_field_id` signature: takes `&RecordType`, returns `Result<Option<String>, RepositoryError>` (defined in `crates/srs-repository/src/package.rs`).
- [ ] Confirm `render_record_at_level` already has `rt: Option<Arc<RecordType>>` in scope (or equivalent) at the heading-emit site (~line 1376 in `render_service.rs`).
- [ ] Confirm `project_record_json` already has `rt` and `package` in scope at the `record_heading` computation (~line 429).
- [ ] Confirm `RecordType.identity_field_id: Option<String>` exists in `srs-core`.

#### Milestone gate

No code changes. Confirm findings match the plan's assumptions above. If any assumption is wrong, update the plan before proceeding.

---

### Phase 1: Extract helper + implement in render_record_at_level

**Goal:** Extract `resolve_heading_field_id`, implement Rule [N+37] for markdown/HTML path, add MemoryStore tests and one FileStore roundtrip test.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `resolve_heading_field_id` private helper just before the `#[cfg(test)]` module in `render_service.rs`:

  ```rust
  /// RFC-020 Rule [N+37]: resolve the effective heading field ID for a section/record pair.
  /// Returns `section.title_field_id` when present (takes precedence), otherwise falls back
  /// to the Type's effective `identityFieldId` (when the Type is known).
  fn resolve_heading_field_id(
      section: &DocumentSection,
      rt: Option<&srs_core::types::record_type::RecordType>,
      package: &Package,
  ) -> Option<String> {
      section.title_field_id.clone().or_else(|| {
          rt.and_then(|t| package.effective_identity_field_id(t).ok().flatten())
      })
  }
  ```

- [ ] In `render_record_at_level` (~line 1376), replace the heading-emit block to use `resolve_heading_field_id`:

  ```rust
  if let Some(title_field_id) = resolve_heading_field_id(section, rt.as_deref(), ctx.package) {
      if let Some(title) = record.get_field_value_str(&title_field_id) {
          record_heading_value = title.to_string();
          out.push_str(&format_heading(heading_level, ctx.format, &title));
      }
  }
  ```

  The `structured` variable (`let structured = section.title_field_id.is_some();`) remains unchanged — field-skip in body and subsection recursion are only activated by an explicit `titleFieldId`.

- [ ] Create fixture files for the FileStore roundtrip test:
  - `crates/srs-cli/tests/fixtures/repeatable-fields/package/types/identity-item.json` — new Type with UUID `00000000-0000-4000-8000-000000000904`, namespace `fixture.repeatable`, name `identity-item`, version 1, `"identityFieldId": "00000000-0000-4000-8000-000000000901"` (reuses existing Title field)
  - `crates/srs-cli/tests/fixtures/repeatable-fields/records/identity/main.json` — Record with UUID `00000000-0000-4000-8000-000000000993`, typeId `00000000-0000-4000-8000-000000000904`, typeVersion 1, fieldValues with Title field = `"identity heading value"`
  - `crates/srs-cli/tests/fixtures/repeatable-fields/package/document-views/identity-fallback-view.json` — DocumentView UUID `00000000-0000-4000-8000-000000000985`, no `titleFieldId`, root_type_refs pointing to the identity-item type
  - Update `crates/srs-cli/tests/fixtures/repeatable-fields/manifest.json` to add `00000000-0000-4000-8000-000000000993` to `instanceIndex`

#### Acceptance Criteria

- [ ] When a section has no `titleFieldId` but the record's Type has `identityFieldId`, a per-record heading at the correct heading level is emitted using that field's value.
- [ ] When both `titleFieldId` and `identityFieldId` are present, `titleFieldId` wins.
- [ ] When neither is present, no per-record heading is emitted (regression guard).
- [ ] The body field list for the identity-fallback case still includes the identity field (structured mode not activated by fallback).
- [ ] `cargo test -p srs-repository` passes with no failures.

#### Testing (all in `render_service.rs` test module)

Tests to add:

- `identity_field_id_fallback_emits_heading_markdown` (MemoryStore) — type with `identity_field_id: Some(...)`, doc view with no `titleFieldId`, record with heading field value → rendered markdown contains H3 with that value.
- `title_field_id_takes_precedence_over_identity_field_id` (MemoryStore) — same type with `identityFieldId`, doc view WITH `titleFieldId` pointing to a different field → heading uses `titleFieldId` value, not identity field value.
- `no_identity_field_id_no_heading` (MemoryStore) — type with `identity_field_id: None`, no `titleFieldId` → no H3 heading emitted.
- `identity_field_id_fallback_filestore_roundtrip` (FileStore via `repeatable_fixture_root()`) — loads the new fixture records/identity-item/identity-fallback-view, renders as markdown → heading emitted from identity field.

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm the four new tests exist and pass.
3. Run `cargo test -p srs-repository` and `cargo clippy -p srs-repository -- -D warnings`.
4. Update plan checkboxes.
5. Commit: `feat(render): implement Rule [N+37] — identity field fallback for per-record heading (#453)`.

---

### Phase 2: Implement Rule [N+37] in project_record_json

**Goal:** JSON projection path also populates `record_heading` from `identityFieldId` when `titleFieldId` is absent.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `project_record_json` (~line 429 in `render_service.rs`), replace the `record_heading` computation to use `resolve_heading_field_id`:

  ```rust
  let record_heading = resolve_heading_field_id(section, rt.as_deref(), package)
      .as_deref()
      .and_then(|fid| record.get_field_value_str(fid).map(|v| v.to_string()));
  ```

  Note: `rt` is already defined above as `package.resolve_type(...)` returning an `Option<&RecordType>` (or `Option<Arc<RecordType>>`; use `.as_deref()` accordingly). `package` is already in scope.

#### Acceptance Criteria

- [ ] When a section has no `titleFieldId` but the record's Type has `identityFieldId`, `ProjectedRecord.record_heading` is populated with the identity field's value.
- [ ] When `titleFieldId` is set, it takes precedence over `identityFieldId`.
- [ ] `cargo test -p srs-repository` passes with no failures.

#### Testing (in `render_service.rs` test module)

Test to add:

- `identity_field_id_fallback_record_heading_json` (MemoryStore) — builds a store with a type declaring `identityFieldId`, a doc view with `title_field_id: None`, renders as `format: "json"` → `projection.sections[0].records[0].record_heading` equals the identity field value.

#### Milestone gate

1. Verify all acceptance criteria.
2. Confirm the new JSON test exists and passes.
3. Run `cargo test -p srs-repository` and `cargo clippy -p srs-repository -- -D warnings`.
4. Update plan checkboxes.
5. Commit: `feat(render): apply Rule [N+37] to JSON projection path (#453)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures (full workspace)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [ ] Existing `no_title_field_id_omits_structural_heading` test still passes (regression guard)
- [ ] New heading-from-identityFieldId tests exist and pass for both markdown and JSON paths
- [ ] `titleFieldId` precedence test passes
- [ ] FileStore roundtrip test passes

## Coordination Rules

- Single implementer; no cross-agent coordination needed.
- Verification runs `cargo test` and `cargo clippy` after each phase.
- No payload struct changes → no `generate-schemas` run needed.

## Assumptions

- `Package::effective_identity_field_id` (added in #376) takes `&RecordType` and returns `Result<Option<String>, RepositoryError>` — walks the type inheritance chain.
- `RecordType.identity_field_id: Option<String>` exists in `srs-core` and is already deserialized from JSON.
- `render_record_at_level` and `project_record_json` both have `rt: Option<_>` (Arc or ref) and `package: &Package` in scope at the heading-emit site.
- Existing tests that use types without `identity_field_id` set will continue to pass unmodified.
