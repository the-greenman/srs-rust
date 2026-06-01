# Plan: DocumentView JSON Projection (`ext:views-l2`)

## Summary

Implement a first-class JSON projection mode for `srs render document-view` so `DocumentView.format: "json"` (or `--view-format json`) yields deterministic machine-readable output instead of markup text. This closes the current gap in `render_service` where rendering is string-first and heading/theme-centric, and enables downstream structured transforms while preserving existing markdown/adoc/text behavior.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Codex |
| Renderer + Contracts | Codex |
| Verification | Codex |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions - this plan implements existing render-service boundaries and ADR-011 contract requirements.  
(If contract shape decisions trigger policy-level changes, add ADR in `docs/adr/` during Phase 1.)

---

## Contracts

### CLI output contract (ADR-011)

This plan **changes an existing command payload** (`render document-view`) by adding a structured `projection` object for JSON mode.

Required actions:

- Update payload struct in `crates/srs-cli/src/payload.rs`.
- Keep `output::serialize()` path in `crates/srs-cli/src/commands/render.rs`.
- Run `cargo run --bin generate-schemas`.
- Commit updated `crates/srs-cli/schemas/payload/render-document-view.json`.

Verification:

- `cargo test --test payload_contracts` passes.

### Entity schema sync (check-schema-sync.sh)

This plan **adds a JSON Schema** for projected output.

Required actions:

- Add new schema in `srs/docs/schema/2.0/` and mirror to:
  - `crates/srs-schema/schemas/2.0/`
  - `srs-vscode/schemas/2.0/`
- Ensure schema IDs and contents are synchronized.
- Run `bash scripts/check-schema-sync.sh`.

---

## Scope

- Add JSON projection execution path in repository render service.
- Produce normative output object: document metadata + ordered sections + ordered records + typed fields.
- Support `titleFieldId` mapping to `recordHeading`.
- Support L1 `ExportConfig` interactions for JSON mode (`omitEmptyFields`, `fieldOrder` with `orderedFieldKeys`).
- Evaluate `DocumentView.preamble` into document metadata for JSON mode.
- Support `RecordType.fieldGroups` — emit structured `fieldGroups` per record alongside `fields`, with full parity to markup rendering.
- Evaluate and include per-record `export_config.preamble` (with `{{heading-*}}` blanked) as `preamble` on each record object; omit key when no preamble is configured.
- `containerId` in root output is nullable: emit `null` when no stable container ID is resolvable; derive from the first `ContainerSubset` section source when present.
- JSON mode is a pure data-export surface: omit `(empty)` placeholder strings; view-required diagnostics surface only in the top-level `diagnostics` array, not embedded in records.
- Extend CLI render payload with structured `projection`.
- Add and sync `document-view-output` schema.
- Add unit/integration/contract tests for JSON mode.

**Out of scope:**

- New SectionSource query semantics.
- Theme/template rendering in JSON mode (explicitly ignored).
- Non-JSON renderer refactors beyond required extraction for shared logic.

---

## Phases

### Phase 1: Schema + Contract Foundation

**Goal:** JSON projection schema and CLI payload contract are defined and compile cleanly.

**Agent:** Renderer + Contracts

#### Tasks

- [x] Add `document-view-output.json` in `srs/docs/schema/2.0/` with required structure and `$schema` URI, including:
  - `containerId` typed as `string | null` (not required `string`).
  - Record object includes optional `fieldGroups` array: items `{ groupId: string, label?: string, entries: [{ entryId?: string, fields: Record<string, JsonValue> }] }`.
  - Record object includes optional `preamble` string (evaluated, not raw template).
- [x] Mirror schema file into `crates/srs-schema/schemas/2.0/` and `srs-vscode/schemas/2.0/`.
- [x] Register schema constant/include in `crates/srs-schema/src/lib.rs`.
- [x] Update `RenderDocumentViewPayload` in `crates/srs-cli/src/payload.rs` to include optional/nullable `projection` object while preserving `rendered` for markup modes.
- [x] Regenerate CLI payload schemas via `cargo run --bin generate-schemas`.

#### Acceptance Criteria

- [x] New output schema exists in all three schema locations with identical content.
- [x] `render-document-view.json` payload schema reflects `projection` field.
- [x] `cargo test --test payload_contracts` passes.

#### Testing

```bash
cargo run --bin generate-schemas
cargo test --test payload_contracts
bash scripts/check-schema-sync.sh
```

Specific tests to write or verify:

- `payload_contracts::render_document_view` - payload contract updated and stable.
- Schema sync script success - mirrors are in lockstep.

#### Milestone gate

1. Verify all acceptance criteria above are met - check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

### Phase 2: JSON Projection Engine in Render Service

**Goal:** `render_document_view` can produce the normative JSON projection for effective format `json`.

**Agent:** Renderer + Contracts

#### Tasks

- [x] Refactor `crates/srs-repository/src/render_service.rs` to branch by effective format:
  - markup path (existing behavior),
  - JSON projection path (new behavior).
- [x] Implement JSON document assembly:
  - root `$schema`,
  - `documentViewId`, `containerId`, `generatedAt`,
  - metadata (`containerTitle`, optional evaluated `preamble`),
  - sections sorted by `DocumentSection.order`,
  - records resolved from current SectionSource logic.
- [x] Implement JSON record mapping:
  - include `instanceId`, `typeId`, `typeNamespace`, `typeName`,
  - `recordHeading` from `titleFieldId` (if resolvable),
  - `fields` as typed JSON values (scalar/array/null),
  - `orderedFieldKeys` honoring `ExportConfig.fieldOrder` or fallback order (covers top-level `fields` only; group fields are ordered by `FieldGroup.fields[].order`),
  - `fieldGroups`: after the `fields` loop, project `rt.field_groups` sorted by `FieldGroup.order`; for each group find `record.group_values` by `group_id`; emit `{ groupId, label, entries: [{ entryId?, fields: { fieldId: JsonValue } }] }`; use raw `serde_json::Value` (not string-rendered) for group field values; omit key entirely when record has no matching group values; `omitEmptyFields` does NOT apply inside group entries,
  - `preamble`: if section's `render_view_id` resolves a view with `export_config.preamble`, evaluate via `substitute_vars` with `{{heading-*}}` always substituted as `""`; omit key when no preamble configured.
- [x] Apply JSON-mode overrides:
  - ignore heading/depthOffset presentation,
  - ignore theme application,
  - omit missing fields entirely when `omitEmptyFields=true`,
  - never emit `"(empty)"` placeholder strings in any field value (suppress `emptyBehavior: ShowPlaceholder` silently),
  - view-required violations push to top-level `diagnostics` only — not embedded in record objects.
- [x] Ensure preamble variable substitution for JSON mode sets `{{heading-N}}` to empty strings.
- [x] Resolve `containerId` from the first `SectionSource::ContainerSubset` section found; emit `null` when none present; if multiple sections have different `container_id` values, use the first and push a diagnostic.

#### Acceptance Criteria

- [x] JSON mode returns structured projection object with expected hierarchy.
- [x] Theme wrappers and heading markers are absent in JSON mode.
- [x] `titleFieldId` value is mapped to `recordHeading` and field remains in `fields`.
- [x] `omitEmptyFields` and `fieldOrder` behavior matches spec.
- [x] Records with `group_values` produce a non-empty `fieldGroups` array matching `FieldGroup.order`.
- [x] Records without `group_values` omit `fieldGroups` key entirely.
- [x] `export_config.preamble` present → record object has `preamble` key with `{{heading-N}}` replaced by `""`.
- [x] `export_config.preamble` absent → no `preamble` key on record.
- [x] Root `containerId` is `null` when no `ContainerSubset` source is present; non-null string when one is.
- [x] No `"(empty)"` strings appear in any field value in JSON mode.
- [x] View-required violations appear only in top-level `diagnostics`; not embedded in record objects.

#### Testing

```bash
cargo test -p srs-repository render_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- JSON render produces `$schema` + `document.sections.records`.
- `titleFieldId` mapping behavior.
- `omitEmptyFields` key omission behavior.
- `orderedFieldKeys` order preservation.
- JSON preamble evaluation with blank `heading-*`.
- `json_record_with_field_groups_emits_fieldGroups_array` — record with `group_values` produces correct `fieldGroups` structure.
- `json_record_without_group_values_omits_fieldGroups_key` — record with no group data omits `fieldGroups` key.
- `json_record_preamble_substitution_blanks_headings` — `export_config.preamble` with `{{heading-1}}` yields `""` in JSON output.
- `json_record_no_preamble_omits_preamble_key` — section without preamble produces no `preamble` field on record.
- `json_containerId_null_when_no_stable_id` — view with no `ContainerSubset` source emits `"containerId": null`.
- `json_containerId_from_container_subset_source` — view with a `ContainerSubset` section emits the correct container ID.
- `json_no_empty_placeholder_strings` — `emptyBehavior: ShowPlaceholder` does not produce `"(empty)"` strings in JSON output.
- `json_view_required_violation_in_top_level_diagnostics_only` — missing required field appears in top-level `diagnostics`, not on record object.

#### Milestone gate

1. Verify all acceptance criteria above are met - check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

### Phase 3: CLI Integration + End-to-End Verification

**Goal:** CLI exposes JSON projection cleanly for stdout and `--output` with no regressions in existing formats.

**Agent:** Verification

#### Tasks

- [x] Update `crates/srs-cli/src/commands/render.rs` to pass structured projection through command output envelope.
- [x] Ensure `--output` writes JSON document when effective format is `json`, and existing behavior remains for markup formats.
- [x] Add/adjust CLI integration tests for json mode invocation and output file writing.
- [x] Re-run existing render integration tests to confirm markdown/text/adoc behavior unchanged.

#### Acceptance Criteria

- [x] `srs render document-view --view-format json` returns payload containing `projection`.
- [x] Output file generated in JSON mode is valid JSON and conforms to expected shape.
- [x] Existing non-json render tests remain green.

#### Testing

```bash
cargo test -p srs-cli --test integration_tests
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:

- `render_document_view_json_returns_projection_payload`
- `render_document_view_json_writes_output_file`
- Existing tests around theme variants/text override unchanged and passing.

#### Milestone gate

1. Verify all acceptance criteria above are met - check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged for non-json modes (integration tests pass)
- [x] `cargo test --test payload_contracts` passes
- [x] JSON projection schema is present and synced across docs/schema crates
- [x] `render document-view` json mode emits deterministic structured output with `orderedFieldKeys`
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (srs-vscode mirror still pending if that repo is managed separately)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Effective format resolution is unchanged: CLI override first, else `DocumentView.format`, else default.
- JSON mode is represented by effective format string `json`.
- `containerId` source is available from current render context inputs; if absent in current data model, use a consistent null/empty policy and document it in tests.
