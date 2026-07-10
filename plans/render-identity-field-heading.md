# Plan: render_service — per-record heading falls back to identityFieldId (RFC-020 Rule [N+37])

> **Issue:** srs-rust #453

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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All business logic (heading resolution) stays inside the `srs-repository` service function; no logic migrated to `srs-cli` | accepted |

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

- Implement Rule [N+37] in `render_record_at_level` (Default Rendering Baseline and L1 View dispatch, markdown/HTML format): when `section.title_field_id` is `None`, resolve the record type's `effective_identity_field_id` via `ctx.package.effective_identity_field_id(rt)` and emit a per-record heading if that field has a value.
- Implement Rule [N+37] in `project_record_json` (JSON projection path): when `section.title_field_id` is `None`, populate `record_heading` from `effective_identity_field_id`.
- Add unit tests (MemoryStore-based, self-contained) covering the new fallback, the precedence rule, and the no-op case.

**Out of scope:**
- `titleFieldId` skipping the field from the body when the fallback fires (structured mode stays tied to explicit `section.title_field_id`; the identity field still appears in the body field list). Deferred — this is not mandated by Rule [N+37].
- Subsection recursion (`structured` flag) for the identity-field fallback path. The subsection recursion is explicitly associated with the explicit `titleFieldId` contract; extending it to the identity-field fallback is a separate decision.
- Any changes to `record_label`, `tree_service`, `discovery_service`, `repository_navigation_service`, `container_view_service`, or `text_projection` (these were #376's scope).

---

## Phases

### Phase 1: Implement Rule [N+37] in render_record_at_level

**Goal:** Markdown/HTML rendering emits a per-record heading from `identityFieldId` when `titleFieldId` is absent.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `render_record_at_level` (file: `crates/srs-repository/src/render_service.rs`, around line 1376), after the line `let structured = section.title_field_id.is_some();`, compute the effective heading field ID:

  ```rust
  // RFC-020 Rule [N+37]: when titleFieldId is absent, fall back to the Type's
  // effective identityFieldId for per-record heading emission.
  let heading_field_id: Option<String> = if section.title_field_id.is_some() {
      section.title_field_id.clone()
  } else {
      rt.as_ref().and_then(|t| {
          ctx.package.effective_identity_field_id(t).ok().flatten()
      })
  };
  ```

- [ ] Replace the heading-emit block (lines ~1380-1384) to use `heading_field_id` instead of `section.title_field_id`:

  ```rust
  if let Some(title_field_id) = &heading_field_id {
      if let Some(title) = record.get_field_value_str(title_field_id) {
          record_heading_value = title.to_string();
          out.push_str(&format_heading(heading_level, ctx.format, title));
      }
  }
  ```

  The `structured` variable remains unchanged (`section.title_field_id.is_some()`), so the field-skip in body and subsection recursion are only activated by an explicit `titleFieldId`.

#### Acceptance Criteria

- [ ] When a section has no `titleFieldId` but the record's Type has `identityFieldId`, a per-record heading at the correct heading level is emitted using that field's value.
- [ ] When both `titleFieldId` and `identityFieldId` are present, `titleFieldId` wins (heading uses `titleFieldId` value).
- [ ] When neither is present, no per-record heading is emitted (regression guard for existing behavior).
- [ ] The body field list for the identity-fallback case still includes the identity field (structured mode not activated by fallback).
- [ ] `cargo test -p srs-repository` passes with no failures.

#### Testing

```bash
cargo test -p srs-repository render
cargo clippy -p srs-repository -- -D warnings
```

Tests to add (all using `MemoryStore`, defined near the existing `make_hetero_store` helper around line 2952):

- `identity_field_id_fallback_emits_heading_markdown` — a type with `identity_field_id: Some("f-heading".to_string())`, a doc view with no `titleFieldId`, a record with a heading field value → rendered markdown contains H3 with that value.
- `title_field_id_takes_precedence_over_identity_field_id` — same type with `identityFieldId`, doc view WITH `titleFieldId` pointing to a different field → heading uses `titleFieldId` field value, not identity field value.
- `no_identity_field_id_no_heading` — type with `identity_field_id: None`, no `titleFieldId` → no H3 heading emitted (regression guard; the existing `no_title_field_id_omits_structural_heading` file-based test already covers this with the fixture type, but add a MemoryStore variant for clarity).

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm the three new tests exist and pass.
3. Run `cargo test -p srs-repository` and `cargo clippy -p srs-repository -- -D warnings`.
4. Update plan checkboxes.
5. Commit.

---

### Phase 2: Implement Rule [N+37] in project_record_json

**Goal:** JSON projection path also populates `record_heading` from `identityFieldId` when `titleFieldId` is absent.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `project_record_json` (file: `crates/srs-repository/src/render_service.rs`, around line 429), replace the `record_heading` computation:

  ```rust
  // Current:
  let record_heading = section
      .title_field_id
      .as_ref()
      .and_then(|fid| record.get_field_value_str(fid).map(|v| v.to_string()));

  // New:
  let json_heading_field_id: Option<String> = section.title_field_id.clone().or_else(|| {
      // RFC-020 Rule [N+37]: fall back to Type's identityFieldId when titleFieldId absent
      rt.as_ref()
          .and_then(|t| package.effective_identity_field_id(t).ok().flatten())
  });
  let record_heading = json_heading_field_id
      .as_deref()
      .and_then(|fid| record.get_field_value_str(fid).map(|v| v.to_string()));
  ```

  Note: `rt` is already defined above as `package.resolve_type(&record.type_id, record.type_version).cloned()`.

#### Acceptance Criteria

- [ ] When a section has no `titleFieldId` but the record's Type has `identityFieldId`, `ProjectedRecord.record_heading` is populated with the identity field's value.
- [ ] When `titleFieldId` is set, it takes precedence over `identityFieldId`.
- [ ] `cargo test -p srs-repository` passes with no failures.

#### Testing

```bash
cargo test -p srs-repository render
cargo clippy -p srs-repository -- -D warnings
```

Test to add (MemoryStore-based):

- `identity_field_id_fallback_record_heading_json` — builds a MemoryStore with a type declaring `identityFieldId`, a doc view with `title_field_id: None`, renders as `format: "json"` → `projection.sections[0].records[0].record_heading` equals the identity field value.

#### Milestone gate

1. Verify all acceptance criteria.
2. Confirm the new JSON test exists and passes.
3. Run `cargo test -p srs-repository` and `cargo clippy -p srs-repository -- -D warnings`.
4. Update plan checkboxes.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures (full workspace)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [ ] Existing `no_title_field_id_omits_structural_heading` test still passes (regression guard)
- [ ] New heading-from-identityFieldId tests exist and pass for both markdown and JSON paths
- [ ] `titleFieldId` precedence test passes

## Coordination Rules

- Single implementer; no cross-agent coordination needed.
- Verification runs `cargo test` and `cargo clippy` after each phase.
- No payload struct changes → no `generate-schemas` run needed.

## Assumptions

- `Package::effective_identity_field_id` (added in #376) is the correct resolution API — it walks the type inheritance chain and returns the effective field ID.
- `RecordType.identity_field_id` field exists in `srs-core` and is already deserialized from JSON.
- Existing tests that use types without `identity_field_id` set will continue to pass unmodified.
