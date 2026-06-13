# Plan: Fix `visible: false` incorrectly excluded from JSON projection (#133)

## Summary

`srs render document-view --view-format json` drops fields marked `visible: false` in a view's `fieldViews[]` from the `projection` output. This is wrong: `visible` is a rendering hint for text/markdown/HTML output, not a data filter. The projection code path calls the same `visible` gate as the markdown render path, conflating presentation with data export. This plan removes the visibility gate from `project_record_json` in `srs-repository` and adds a regression test to prevent recurrence.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan fixes a bug against the spec semantics already defined in `ext:views-l1`. The change lives entirely in `srs-repository` per ADR-010. No CLI output shape changes (the struct already includes all fields); no payload schema regeneration needed per ADR-011.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Bug fix in service layer (`srs-repository`), not CLI | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct changes — existing `ProjectedRecord.fields` shape is unchanged | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. The `ProjectedRecord` payload struct is unchanged — it already includes a `fields` map. The fix causes more fields to appear in that map, which is a correct behavioural fix, not a contract change. No schema regeneration needed. `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No entity schema files modified. `bash scripts/check-schema-sync.sh` must exit 0.

---

## Scope

- Remove the `if fv.visible == Some(false) { continue; }` gate in `project_record_json` in `crates/srs-repository/src/render_service.rs` (line 442–444).
- Add a `visible: false` field view entry to the repeatable-fields fixture: a new View file `hidden-field-view.json` and a new DocumentView `json-hidden-field-view.json` (format: `json`).
- Register both in `crates/srs-cli/tests/fixtures/repeatable-fields/package/package.json`.
- Add a regression test in `crates/srs-cli/tests/integration_tests.rs`: `render_document_view_json_includes_visible_false_fields`.
- Confirm the markdown render path (`render_record_at_level`, line 1263) continues to gate on `visible` — it must NOT be changed.

**Out of scope:**
- Changing the markdown/HTML/text render path.
- New payload structs or CLI commands.
- Any srs-vscode changes.

---

## Phases

### Phase 1: Remove visibility gate from projection and add regression test

**Goal:** `project_record_json` includes all `fieldViews[]` entries regardless of `visible`; a regression test proves the fix; the markdown path is unaffected.

**Agent:** Repository Service Worker (fix in `crates/srs-repository/src/render_service.rs`) and CLI Worker (fixture files and test in `crates/srs-cli/tests/`).

#### Tasks

- [ ] In `crates/srs-repository/src/render_service.rs`, inside `project_record_json`, remove the block at lines 442–444:
  ```rust
  if fv.visible == Some(false) {
      continue;
  }
  ```
  The surrounding loop (lines 438–450) iterates `field_views` and builds `fields_to_render`; after this change it must include all entries regardless of `visible`.

- [ ] Verify that `project_record_json` is only called in the JSON-format branch: inspect `render_document_view` in `crates/srs-repository/src/render_service.rs` and confirm the call is guarded by `if format == "json"` (or equivalent). This confirms removing the `visible` gate only affects JSON projection, not other formats.

- [ ] Confirm the markdown render path in `render_record_at_level` (lines 1263–1271) still has the `visible` gate — do not change it. Also confirm `record_satisfies_view` (line 977) still has its `visible != Some(false)` filter — this is view-compatibility checking, not rendering, and must remain.

- [ ] Add `crates/srs-cli/tests/fixtures/repeatable-fields/package/views/hidden-field-view.json`:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/view.json",
    "id": "00000000-0000-4000-8000-000000000989",
    "namespace": "fixture.repeatable",
    "name": "hidden-field-view",
    "version": 1,
    "description": "View with one visible:false field for projection regression tests.",
    "fieldViews": [
      {
        "fieldId": "00000000-0000-4000-8000-000000000901",
        "order": 0,
        "visible": true
      },
      {
        "fieldId": "00000000-0000-4000-8000-000000000903",
        "order": 1,
        "visible": false
      }
    ],
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```
  (IDs 987 and 988 are taken by `themed-document-view.json` and `missing-theme-view.json`; 989 is next free.)

- [ ] Add `crates/srs-cli/tests/fixtures/repeatable-fields/package/document-views/json-hidden-field-view.json`:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/document-view.json",
    "id": "00000000-0000-4000-8000-000000000992",
    "namespace": "fixture.repeatable",
    "name": "json-hidden-field-view",
    "version": 1,
    "description": "JSON-format document view exercising visible:false projection regression.",
    "format": "json",
    "sections": [
      {
        "sectionId": "items",
        "title": "Items",
        "order": 0,
        "source": {
          "type": "type-query",
          "semanticObjectType": "fixture.repeatable/repeatable-item"
        },
        "renderViewId": "00000000-0000-4000-8000-000000000989"
      }
    ],
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```
  (IDs 990 and 991 are taken by `base-theme.json`/manifest and `records/repeatable/valid.json`; 992 is next free.)

- [ ] Register both new files in `crates/srs-cli/tests/fixtures/repeatable-fields/package/package.json`:
  - Add `"views/hidden-field-view.json"` to the `"views"` array.
  - Add `"document-views/json-hidden-field-view.json"` to the `"documentViews"` array.

- [ ] Add test `render_document_view_json_includes_visible_false_fields` in `crates/srs-cli/tests/integration_tests.rs` (near the other `render_document_view_json_*` tests around line 1107):
  ```rust
  #[test]
  fn render_document_view_json_includes_visible_false_fields() {
      let fixture = repeatable_fields_fixture_dir();
      let result = run_srs_in_dir(
          &fixture,
          &[
              "render",
              "document-view",
              "--view",
              "00000000-0000-4000-8000-000000000992",
              "--view-format",
              "json",
          ],
      );
      assert_eq!(result["ok"], true);
      let sections = result["payload"]["projection"]["sections"]
          .as_array()
          .unwrap();
      let records = sections[0]["records"].as_array().unwrap();
      // The valid record must have both title (visible:true) and body (visible:false) fields.
      let record = records
          .iter()
          .find(|r| r["instanceId"] == "00000000-0000-4000-8000-000000000991")
          .expect("valid record must be present in projection");
      let fields = record["fields"].as_object().expect("fields must be an object");
      assert!(
          fields.contains_key("00000000-0000-4000-8000-000000000901"),
          "title field (visible:true) must appear in JSON projection"
      );
      assert!(
          fields.contains_key("00000000-0000-4000-8000-000000000903"),
          "body field (visible:false) must appear in JSON projection — visible is a render concept only"
      );
  }
  ```

#### Acceptance Criteria

- [ ] `render_document_view_json_includes_visible_false_fields` passes — body field (`00000000-0000-4000-8000-000000000903`, `visible: false`) is present in `projection.sections[0].records[*].fields`.
- [ ] All existing `render_document_view_*` tests still pass — no regression in markdown/text render path.
- [ ] `cargo clippy -p srs-repository -p srs-cli -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-cli render_document_view_json_includes_visible_false_fields
cargo test -p srs-cli render_document_view
cargo test -p srs-repository
cargo clippy -p srs-repository -p srs-cli -- -D warnings
```

Specific tests:
- `render_document_view_json_includes_visible_false_fields` — proves `visible: false` fields appear in JSON projection.
- All existing `render_document_view_json_*` tests — prove no regression in the JSON projection path.
- All existing `render_document_view_markup_*` tests — prove markdown path still hides `visible: false` fields (the positive case for the markdown gate is tested indirectly via the existing labeled-field tests; if the markdown gate were removed, those tests would still pass, so this is a weaker check — but the markdown gate is not touched).

#### Milestone gate

1. `render_document_view_json_includes_visible_false_fields` passes.
2. All `render_document_view_*` tests pass.
3. `cargo test -p srs-repository` passes.
4. `cargo clippy -p srs-repository -p srs-cli -- -D warnings` clean.
5. Plan checkboxes updated, commit made.

```bash
cargo test -p srs-cli render_document_view
cargo test -p srs-repository
cargo clippy -p srs-repository -p srs-cli -- -D warnings
git commit
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `render_document_view_json_includes_visible_false_fields` exists and passes
- [ ] Markdown render path (`render_record_at_level`) still gates on `visible: false` — verified by code inspection

## Coordination Rules

- Repository Service Worker edits only `crates/srs-repository/src/render_service.rs`.
- CLI Worker edits only `crates/srs-cli/tests/` (fixture files and `integration_tests.rs`).
- Lead Integrator owns commit sequencing.
- No other files are modified.

## Assumptions

- The `repeatable-fields` fixture record `00000000-0000-4000-8000-000000000991` has both `title` (`00000000-0000-4000-8000-000000000901`) and `body` (`00000000-0000-4000-8000-000000000903`) field values — confirmed from `records/repeatable/valid.json`.
- `project_record_json` is called only for `--view-format json` — to be verified during implementation by inspecting `crates/srs-cli/src/commands/render.rs` and the call site in `render_document_view` in `crates/srs-repository/src/render_service.rs` (the `format == "json"` guard).
- A grep for `visible.*false\|visible == Some(false)` in `render_service.rs` finds three sites: line 442 (`project_record_json` — JSON projection path, **to be removed**), line 977 (`record_satisfies_view` — view compatibility check, must **not** be changed), and line 1263 (`render_record_at_level` — markdown render path, must **not** be changed). Only line 442 is in the projection path.
