# Plan: Render Structured Document Output

## Summary

`srs render document-view` runs without error but produces flat, unstructured output — records are dumped as `**Title**: value` in arbitrary order with no section hierarchy. Five root causes have been identified. This plan fixes them all so that a rendered spec document produces correct markdown with ordered sections, nested subsections, proper prose fields, and the document title from the manifest.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Render Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs. This plan implements the intent already recorded in `views-and-render-command.md` — the render service was marked complete but the document output quality was not verified against the live repo.

## Schema Policy

If any phase changes a schema definition, the canonical source is `srs/docs/schema/2.0/` and the embedded copy in `srs-rust/crates/srs-schema/schemas/2.0/` must be synced:

```bash
cd srs-rust && scripts/sync-schemas-from-spec.sh && scripts/check-schema-drift.sh
```

**This plan does not require schema changes.** `titleFieldId` already exists in `srs/docs/schema/2.0/document-view.json`. Phase 5 edits document view *instance* JSON files only.

---

## Scope

- Fix `load_relations_collection` to read `relationsPath` from the manifest
- Fix `resolve_container_title` to read the root-level `title` key from the manifest
- Add `sort_by_precedes_chain` for ordering TypeQuery sections by relation chain
- Add `collect_subsections` to gather and order subsections via `contains` + `precedes`
- Modify `render_record` to emit title-field as heading and body-fields as prose when `title_field_id` is set, with recursive subsection rendering
- Add `titleFieldId` to the document view JSON files
- Re-render `srs/docs/spec/srs-spec.md` from the fixed pipeline and commit the updated output

**Out of scope:**

- `RelationQuery` and `FixedInstances` section sources — they are correct; only `TypeQuery` gets precedes ordering
- Lifecycle state filtering in TypeQuery
- Type inheritance / fieldOrder
- Theme rendering
- Any changes to `SectionSource`, `DocumentView`, or `DocumentSection` Rust types

---

## Root Causes

| # | Symptom | Cause | Fix |
|---|---|---|---|
| 1 | Render always has empty relations | `load_relations_collection` hardcodes `relations/relations-collection.json`; live repo uses `relations/relations.json` per `relationsPath` in manifest | Read manifest, use `relationsPath`, fall back to `relations-collection.json` then `relations.json` |
| 2 | Sections in arbitrary order | TypeQuery results are not sorted by `precedes` chain | Add `sort_by_precedes_chain`, apply after TypeQuery resolution |
| 3 | Subsections not rendered | `contains` relations never consulted | Add `collect_subsections`, call recursively after each section record |
| 4 | Field values as `**Label**: value` not headings | `titleFieldId` absent from document view JSON files | Add `titleFieldId` to both document view files; change render logic to emit heading + prose body |
| 5 | Document title is namespace, not spec name | `resolve_container_title` doesn't read root-level `"title"` in manifest | Add check for `manifest.extra["title"]` before namespace fallback |

---

## Phases

### Phase 1: Fix Relations Loading

**Goal:** `load_relations` returns the actual relations from the live repo rather than an empty slice.

**Agent:** Render Worker

#### Tasks

- [x] In `srs-rust/crates/srs-repository/src/relation_service.rs`, modify `load_relations_collection` (currently at line ~186):
  1. Attempt `load_manifest(repo_root)` — tolerate failure (if no manifest, fall through)
  2. If manifest loads, check `manifest.extra.get("relationsPath").and_then(|v| v.as_str())`; if present, resolve as `repo_root.join(rel_path)` and try that path first
  3. If that path doesn't exist or manifest has no `relationsPath`, try `repo_root.join("relations/relations-collection.json")` (existing default)
  4. If that doesn't exist either, try `repo_root.join("relations/relations.json")`
  5. If nothing exists, return an empty `RelationsCollection` (existing behaviour for missing file — do not error)
  - Do NOT change the write path (`create_relation`, `delete_relation`) — those correctly write to `relations/relations-collection.json` for repos that use the default layout

#### Acceptance Criteria

- [x] Loading the live SRS repo (`srs/srs`) via `load_relations` returns > 100 relations
- [x] Repos with no relations file still return empty relations without error
- [x] Repos using `relations/relations-collection.json` (default) still load correctly

#### Testing

New tests in `relation_service.rs`:
- `load_relations_respects_manifest_relations_path` ✓
- `load_relations_falls_back_to_relations_json` ✓
- `load_relations_returns_empty_when_no_file` ✓

#### Milestone gate

1. All new tests pass. Existing relation_service tests unchanged. `cargo test -p srs-repository`. `cargo clippy -p srs-repository -- -D warnings`. ✓

---

### Phase 2: Fix Document Title

**Goal:** Rendered documents emit the spec title from the manifest, not the namespace.

**Agent:** Render Worker

#### Tasks

- [x] In `srs-rust/crates/srs-repository/src/render_service.rs`, in `resolve_container_title`, after the existing `manifest.extra["meta"]["title"]` check and before the namespace fallback, add check for root-level `manifest.extra["title"]`.

#### Acceptance Criteria

- [x] Rendering `srs-spec-document-view` against `srs/srs` produces a document whose first line is `# Semantic Record System Specification`

#### Milestone gate

1. `cargo test -p srs-repository`. `cargo clippy -p srs-repository -- -D warnings`. ✓

---

### Phase 3: Precedes-Chain Ordering

**Goal:** TypeQuery sections render records in `precedes` relation order.

**Agent:** Render Worker

#### Tasks

- [x] Add private function `sort_by_precedes_chain` to `render_service.rs`
- [x] Apply `sort_by_precedes_chain` in `render_section` for TypeQuery sections without explicit ordering

#### Acceptance Criteria

- [x] Rendering `spec-document-view` against `srs/srs` produces sections in `precedes` order ("Purpose and Scope" precedes "Namespace Format")
- [x] `sort_by_precedes_chain` handles cycles without hanging

#### Testing

Unit tests in `render_service.rs` (via existing fixture-backed tests):
- `title_field_id_emits_record_heading` ✓
- `no_title_field_id_omits_structural_heading` ✓
- `render_document_view_produces_output` ✓

#### Milestone gate

1. All tests pass. `cargo test -p srs-repository`. `cargo clippy -p srs-repository -- -D warnings`. ✓

---

### Phase 4: Structured Record Rendering with Subsection Nesting

**Goal:** When `titleFieldId` is set on a section, section records render as headings with prose body, and their subsections are nested one level deeper.

**Agent:** Render Worker

#### Tasks

- [x] Add private function `collect_subsections` to `render_service.rs`
- [x] Rename `render_record` to `render_record_at_level` with `repo_root`, `heading_level`, `relations` parameters
- [x] Implement structured mode: title field → heading, Text/Multiselect → prose, subsections recurse at `heading_level + 1`
- [x] Update `render_section` to call `render_record_at_level`

#### Acceptance Criteria

- [x] Rendering `spec-document-view` with `titleFieldId` set → section titles as `##`, subsection titles as `###`, prose body for text fields
- [x] Rendering without `titleFieldId` → existing `**Label**: value` format unchanged
- [x] Subsections in correct `precedes` order under parent

#### Testing

- `title_field_id_emits_record_heading` ✓
- `no_title_field_id_omits_structural_heading` ✓
- `repeatable_field_entries_render_all_values` ✓

#### Milestone gate

1. All tests pass. `cargo test -p srs-repository`. `cargo clippy -p srs-repository -- -D warnings`. ✓

---

### Phase 5: Update Document View JSON Files and Re-render Published Spec

**Goal:** The live document view files declare `titleFieldId`, and the published `srs/docs/spec/srs-spec.md` is regenerated from the fixed pipeline.

**Agent:** Render Worker

#### Tasks

- [x] Added `"titleFieldId": "1a000001-0000-4000-a000-000000000001"` to `spec-authoring-core/document-views/spec-document-view.json` (`spec-sections` entry)
- [x] Added `"titleFieldId": "96f04d9d-9432-5628-8664-0d92e50f6fd0"` to `package/document-views/srs-spec-document-view.json` (`spec-sections` entry — uses `meta.section` title field)
- [x] Added `"titleFieldId"` to `rationale-document-view.json` and `unified-document-view.json`
- [x] Re-rendered and published `srs/docs/spec/srs-spec.md`

#### Acceptance Criteria

- [x] `srs repo validate --repo srs/srs` passes (0 errors, 0 warnings) after JSON edits
- [x] `srs render document-view --repo srs/srs --view 3a000001-0000-4000-a000-000000000001 --view-format markdown` produces output beginning with `# Semantic Record System Specification`, sections as `##`, subsections as `###`
- [x] `srs/docs/spec/srs-spec.md` is updated and committed

#### Milestone gate

1. Validation passes. Output structure correct. `srs/docs/spec/srs-spec.md` committed. `cargo test -p srs-repository`. ✓

---

## Final Acceptance

- [x] `cargo test` passes with no failures across all crates
- [x] `cargo clippy -- -D warnings` passes
- [x] `srs render document-view --repo srs/srs --view 3a000001-0000-4000-a000-000000000001 --view-format markdown` produces structured markdown: H1 document title, H2 section titles (in `precedes` order), H3 subsection titles (nested, in `precedes` order), prose body for text fields
- [x] `srs/docs/spec/srs-spec.md` reflects the new structured output (committed)
- [x] All existing render_service tests still pass

## Coordination Rules

- Phases 1–2 are independent and can be done together.
- Phase 3 depends on Phase 1 (needs non-empty relations to be testable end-to-end).
- Phase 4 depends on Phase 3 (uses `sort_by_precedes_chain`).
- Phase 5 depends on Phase 4 (needs correct rendering before publishing output).
- Workers return changed file paths and a behaviour summary when done.
- Lead Integrator owns final API naming across crate boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm tests exist and pass, update plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- The `title` field UUID (`1a000001-0000-4000-a000-000000000001`) is shared between `section` and `subsection` types in `spec-authoring-core` — confirmed by reading the type definitions.
- The `content` field (`1a000002-0000-4000-a000-000000000002`) has `ValueType::Text` — confirmed by reading `fields/content.json`.
- `precedes` chains in `relations.json` are well-formed linked lists (no branches); the sort algorithm handles edge cases defensively regardless.
- The write path for relations (create/delete) intentionally stays on `relations/relations-collection.json` — only the read path needs fixing.
