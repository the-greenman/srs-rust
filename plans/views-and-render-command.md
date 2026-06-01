# Plan: Views (ext:views-l1 / ext:views-l2) and `srs render document-view`

## Summary

RFC-001 and RFC-002 have been reviewed, corrected, and are ready to apply. Before the Rust implementation can begin, the spec SRS records must reflect the accepted RFC content. Because View semantics are changing, schema parity must be maintained across both schema authorities (`srs/docs/schema/2.0/` and `srs-rust/crates/srs-schema/schemas/2.0/`), and the VS Code extension must ship matching schema/editor behavior. This plan therefore covers five synchronized layers: spec records, canonical schemas, Rust implementation, VS Code extension updates, and the `srs render document-view` command.

## Progress Snapshot (2026-05-31)

- Phase 1 (Spec Record Updates in `srs` repo): mostly complete — RFC-001 and RFC-002 accepted, ext-themes-l1 created, manifest updated; one gap: ext-views-l2.json description stub needs expansion
- Phase 2 (Rust Types): complete
- Phase 3 (Package Loading): complete
- Phase 4 (View Service + Render Service): complete — all 9 render_service tests passing
- Phase 5 (CLI Command): complete
- Phase 6 (RFC-002 Theme Application): in progress — Theme type/validation and package loading are in place; rendering and CLI variant selection still pending

## Implementation Notes

- CLI render override uses `--view-format` (not `--format`) to avoid a Clap flag collision with the global output `--format`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Spec Worker | — |
| Rust Types Worker | — |
| Render Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs — this plan implements the decisions recorded in RFC-001 (Rev 8) and RFC-002 (Rev 5).

---

## Scope

- Accept RFC-001 and RFC-002 into the spec SRS records (status → "accepted", content updated to final revision, ext-views-l2 and new ext-themes-l1 record updated)
- Adopt field-centric View semantics (remove `View.typeId`/`View.typeVersion`, add optional `compatibleTypes`, and change compatibility invariant to field-presence)
- Keep schema parity between:
  - `srs/docs/schema/2.0/`
  - `srs-rust/crates/srs-schema/schemas/2.0/`
- Rust types for `View`, `DocumentView`, `ExportConfig`, `FieldView`, `SectionSource`, `DocumentSection`, `NavigationLink`, `ThemeReference`, `ThemeVariant` in `srs-core`
- Validation rules for those types
- Package loading for `views` (L1) and `documentViews` (L2) in `srs-repository`
- `view_service` and `render_service` in `srs-repository`
- `srs render document-view` CLI command
- `srs-vscode` schema copies and extension behavior aligned with new View semantics
- One spec-compliant `DocumentView` JSON file for the SRS spec document to enable end-to-end testing

**Out of scope:**

- RFC-002 theme application logic (ThemeReference/ThemeVariant types are stubs; no theme rendering)
- `ext:type-inheritance` fieldOrder support (use FieldAssignment.order only)
- `srs view list` / `srs view get` CLI commands (render is the priority; service layer supports them when added)
- `"html"` and `"adoc"` format rendering (markdown and text only in this plan)

> **Note:** `ext:repeatable-fields` was originally out of scope here and covered by a stub diagnostic. It has since been fully implemented (see `plans/completed/repeatable-fields-and-field-groups.md`). The stub is gone; `FieldValue.entries` is rendered correctly by `render_service.rs`.

**Partial-conformance policy:** The renderer is intentionally incomplete with respect to one declared extension. To prevent silent non-conformance, the renderer MUST emit an explicit diagnostic when it encounters conditions it cannot fully handle:
- If a `DocumentView`'s section source resolves a `RecordType` that has a `fieldOrder` property (indicates `ext:type-inheritance` usage): emit diagnostic `"[partial] ext:type-inheritance fieldOrder ignored; using FieldAssignment.order"`

This diagnostic goes into `RenderResult.diagnostics`, not as an error. The render proceeds with best-effort output.

---

## Phases

### Phase 1: Spec Record Updates

**Goal:** RFC-001 and RFC-002 are marked accepted in the SRS records, the ext-views-l2 record reflects all RFC-001 changes, and a new ext-themes-l1 record exists.

**Agent:** Spec Worker

#### Tasks

- [x] Update `srs/srs/records/rfcs/rfc-001-views-l2-rendering.json`:
  - Set `fieldValues[status]` (`5a000002-0000-4000-a000-000000000002`) to `"accepted"`
  - Set `fieldValues[content]` (`1a000002-0000-4000-a000-000000000002`) to the full text from `srs/rfcs/rfc-001.md` (strip the top-level `# RFC-001:` heading and the `> Projection note` callout — keep everything from `**Status**: Draft (Revision 8)` onward, updating the status line to `**Status**: Accepted (Revision 8)`)

- [x] Update `srs/srs/records/rfcs/rfc-002-themes-l1.json`:
  - Set `fieldValues[status]` to `"accepted"`
  - Set `fieldValues[content]` to the full text from `srs/rfcs/rfc-002.md` (same stripping rule, updating status line to `**Status**: Accepted (Revision 5)`)

- [ ] Update `srs/srs/records/extensions/ext-views-l2.json`:
  - Replace the `description` field value (`1a000002-0000-4000-a000-000000000002`) with the updated spec text incorporating all RFC-001 changes (Changes A–D). The updated description must include:
    - The full `SectionSource` type definition (unchanged)
    - The updated `DocumentSection` definition with `titleFieldId` (RFC-001 Change B1)
    - The updated `DocumentView` definition with `depthOffset`, `format` vocabulary, `themeRef`, `themeVariants` (RFC-001 Changes B2, C, D)
    - The `ThemeReference` and `ThemeVariant` type definitions (RFC-001 Change D)
    - All conformance rules [N] through [N+8] verbatim from the RFC, including [N+4b] and [N+6b] which were added in Rev 8
  - **Current state:** description is a 2-sentence stub; does not explicitly contain `titleFieldId`, `depthOffset`, `[N+4b]`, `[N+6b]`

- [x] Create `srs/srs/records/extensions/ext-themes-l1.json` — new extension record:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
    "instanceId": "a3f7c9e1-2b4d-4f8a-9c1e-5d7b3a2f6e0c",
    "typeId": "2a000008-0000-4000-a000-000000000008",
    "typeVersion": 1,
    "typeNamespace": "com.semanticops.spec",
    "typeName": "extension",
    "fieldValues": [
      { "fieldId": "1a000018-0000-4000-a000-000000000018", "value": "ext:themes-l1" },
      { "fieldId": "1a000001-0000-4000-a000-000000000001", "value": "Themes L1" },
      { "fieldId": "1a000019-0000-4000-a000-000000000019", "value": "- ext:views-l2" },
      { "fieldId": "1a000002-0000-4000-a000-000000000002", "value": "<full RFC-002 spec text>" }
    ],
    "createdAt": "2026-05-29T00:00:00Z"
  }
  ```
  The `description` value must contain all RFC-002 type definitions and conformance rules [T-1] through [T-11] at their final Rev 5 state.

- [x] Add `ext-themes-l1` to `srs/srs/manifest.json`:
  - Add to `instanceIndex`: `{ "instanceId": "a3f7c9e1-2b4d-4f8a-9c1e-5d7b3a2f6e0c", "path": "records/extensions/ext-themes-l1.json", "title": "ext:themes-l1" }`
  - Add `"ext:themes-l1"` to `declaredExtensions` array

#### Acceptance Criteria

- [x] Both RFC records have `status: "accepted"`
- [x] RFC-002 content reflects Rev 5 (open question 2 resolved, `{{heading-N}}` scope fixed)
- [ ] `ext-views-l2.json` description contains `titleFieldId`, `depthOffset`, `ThemeReference`, `ThemeVariant`, and all conformance rules [N] through [N+8] — including [N+4b] (depthOffset warning) and [N+6b] (`{{heading-3}}` standalone suppression) introduced in Rev 8
- [x] `ext-themes-l1.json` exists and is listed in the manifest instanceIndex
- [x] `"ext:themes-l1"` appears in `declaredExtensions`
- [ ] `node scripts/validate-all.mjs` passes from `srs/` (if validator is available)

#### Milestone gate

1. Verify all acceptance criteria above.
2. Commit spec record changes.

---

### Phase 1.5: Schema Synchronization (Normative + Runtime)

**Goal:** `view.json` and related schema constraints are synchronized between spec docs and Rust runtime schema bundle, reflecting field-centric View semantics.

**Agent:** Spec Worker + Rust Types Worker

#### Tasks

- [x] Update `srs/docs/schema/2.0/view.json`:
  - Remove required/properties entries for `typeId` and `typeVersion`
  - Add optional `compatibleTypes` (`array<string>`)
  - Keep `fieldViews` as the normative compatibility surface
- [x] Mirror the same changes in `srs-rust/crates/srs-schema/schemas/2.0/view.json`
- [x] Diff both files to verify semantic parity (ordering may differ; constraints must not)
- [x] Validate schema references still resolve for package-level schemas using `view.json`

#### Acceptance Criteria

- [x] `view.json` in both locations has no `typeId`/`typeVersion`
- [x] `compatibleTypes` exists and is optional in both locations
- [x] Both schema trees validate without unresolved refs
- [x] A sample field-centric View instance validates against both schema sets

---

### Phase 2: Rust Types

**Goal:** All View/DocumentView types compile in `srs-core` with serde roundtrip tests passing.

**Agent:** Rust Types Worker

#### Tasks

- [x] Create `srs-rust/crates/srs-core/src/types/view.rs` with all types below
- [x] Register in `srs-rust/crates/srs-core/src/types/mod.rs`: `pub mod view;`
- [x] Add new `CoreError` variants to `srs-rust/crates/srs-core/src/error.rs`
- [x] Create `srs-rust/crates/srs-core/src/validation/view.rs`
- [x] Register in `srs-rust/crates/srs-core/src/validation/mod.rs`: `pub mod view;`

**Types to implement in `view.rs`** (all use `#[serde(rename_all = "camelCase")]`):

```rust
// L1 types
pub struct FieldView {
    pub field_id: String,
    pub order: i32,
    pub required: Option<bool>,
    pub visible: Option<bool>,
    pub display_label: Option<String>,
}

pub struct ExportConfig {
    pub format: Option<String>,
    pub preamble: Option<String>,
    pub field_order: Option<Vec<String>>,
    pub omit_empty_fields: Option<bool>,
}

#[serde(rename_all = "kebab-case")]
pub enum ViewProtection { None, ReadOnly, FillIn }

pub struct View {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub field_views: Vec<FieldView>,
    pub compatible_types: Option<Vec<String>>, // informational hint only
    pub protection: Option<ViewProtection>,
    pub export_config: Option<ExportConfig>,
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// L2 types
// SectionSource uses #[serde(tag = "type", rename_all = "kebab-case")] on the enum.
// Each variant's fields use camelCase (separate #[serde(rename_all = "camelCase")] on each variant).
pub enum SectionSource {
    FixedInstances { instance_ids: Vec<String> },
    TypeQuery {
        semantic_object_type: String,
        lifecycle_state: Option<String>,
        container_ids: Option<Vec<String>>,
    },
    RelationQuery {
        from_instance_id: String,
        relation_type: String,
        direction: Option<RelationDirection>,
    },
    ContainerSubset {
        container_id: String,
        container_type: Option<String>,
    },
}

#[serde(rename_all = "lowercase")]
pub enum RelationDirection { Forward, Inverse }

pub struct SectionOrdering {
    pub field_id: Option<String>,
    pub direction: Option<SortDirection>,
}

#[serde(rename_all = "lowercase")]
pub enum SortDirection { Asc, Desc }

#[serde(rename_all = "camelCase")]
pub enum EmptyBehavior { Hide, ShowPlaceholder }

pub struct DocumentSection {
    pub section_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub order: i32,
    pub source: SectionSource,
    pub render_view_id: Option<String>,
    pub title_field_id: Option<String>,   // RFC-001 Change B
    pub ordering: Option<SectionOrdering>,
    pub required: Option<bool>,
    pub empty_behavior: Option<EmptyBehavior>,
}

pub struct NavigationLink {
    pub from_section_id: String,
    pub to_section_id: String,
    pub label: Option<String>,
    pub bidirectional: Option<bool>,
}

// RFC-001 Change D stubs — types present, no application logic
#[serde(rename_all = "lowercase")]
pub enum ThemeMode { Local, Remote, Bundled }

pub struct ThemeReference {
    pub mode: ThemeMode,
    pub path: Option<String>,
    pub url: Option<String>,
    pub theme_id: Option<String>,
}

pub struct ThemeVariant {
    pub name: String,
    pub description: Option<String>,
    pub theme_ref: ThemeReference,
}

pub struct DocumentView {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub container_type: Option<String>,
    pub sections: Vec<DocumentSection>,
    pub navigation_links: Option<Vec<NavigationLink>>,
    pub preamble: Option<String>,
    pub format: Option<String>,           // RFC-001 Change C
    pub depth_offset: Option<u32>,        // RFC-001 Change B; u32 enforces non-negative
    pub theme_ref: Option<ThemeReference>, // RFC-001 Change D
    pub theme_variants: Option<Vec<ThemeVariant>>, // RFC-001 Change D
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
```

**New `CoreError` variants:**
```rust
EmptyDocumentViewSections,
DuplicateDocumentSectionId { section_id: String },
DuplicateFieldViewId { field_id: String },
EmptyViewFieldViews,
DuplicateThemeVariantName { name: String },
```
Add matching `PartialEq` arms.

**Validation rules in `validation/view.rs`:**

`validate_view(view: &View) -> Result<(), CoreError>`:
- `field_views` non-empty → `EmptyViewFieldViews`
- `field_views` unique `field_id` → `DuplicateFieldViewId`
- tags: no empty strings → `EmptyTag`

`validate_document_view(dv: &DocumentView) -> Result<(), CoreError>`:
- `sections` non-empty → `EmptyDocumentViewSections`
- `section_id` unique across sections → `DuplicateDocumentSectionId`
- `theme_variants` names unique (when present) → new `CoreError::DuplicateThemeVariantName { name: String }` (Rule [N+8], enforced at package validation time per RFC-001 Change D)
- tags: no empty strings → `EmptyTag`
- `depth_offset > 4` is NOT a validation error here — emitted as a diagnostic in render_service

Add `DuplicateThemeVariantName { name: String }` to the CoreError list above.

#### Acceptance Criteria

- [x] `cargo build -p srs-core` compiles clean
- [x] `cargo clippy -p srs-core -- -D warnings` clean
- [x] `DocumentView` roundtrips through `serde_json::to_string` / `from_str` without data loss
- [x] `SectionSource` variants deserialise correctly from `{"type": "type-query", "semanticObjectType": "ns/name"}` etc.
- [x] `validate_document_view` returns `EmptyDocumentViewSections` for a view with no sections
- [x] `validate_document_view` returns `DuplicateDocumentSectionId` for duplicate sectionIds
- [x] `validate_document_view` returns `DuplicateThemeVariantName` for duplicate variant names in `themeVariants`

#### Testing

```bash
cd srs-rust
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Tests to write in `view.rs`:
- `document_view_roundtrips_json` — full DocumentView with all optional fields serialises and deserialises to identical value
- `section_source_type_query_deserialises` — `{"type":"type-query","semanticObjectType":"com.example/decision"}` → `SectionSource::TypeQuery`
- `section_source_fixed_instances_deserialises`
- `section_source_relation_query_defaults_forward` — absent `direction` → `None` (resolved to Forward at render time, not in type)
- `validate_empty_sections_fails`
- `validate_duplicate_section_id_fails`
- `validate_empty_field_views_fails`
- `validate_duplicate_theme_variant_name_fails` — two `ThemeVariant` entries with the same `name` → `DuplicateThemeVariantName` (Rule [N+8] package-validation enforcement)
- `validate_unique_theme_variant_names_passes` — two variants with distinct names → `Ok(())`

#### Milestone gate

1. All tests pass. No clippy warnings. Commit.

---

### Phase 3: Package Loading

**Goal:** `load_package` loads `views` (L1) and `documentViews` (L2) from `package.json`; the SRS spec package loads without error.

**Agent:** Rust Types Worker

#### Tasks

- [x] Add `RepositoryError` variants to `srs-rust/crates/srs-repository/src/error.rs`:
  ```rust
  ViewLoad { path: PathBuf, source: serde_json::Error },
  ViewValidation { path: PathBuf, source: CoreError },
  DocumentViewLoad { path: PathBuf, source: serde_json::Error },
  DocumentViewValidation { path: PathBuf, source: CoreError },
  DocumentViewNotFound { view_id: String },
  ```

- [x] Update `srs-rust/crates/srs-repository/src/package.rs`:
  - Add `views: Vec<String>` and `document_views: Vec<String>` (both `#[serde(default)]`) to `PackageMetadata`
  - Add `pub views: Vec<View>` and `pub document_views: Vec<DocumentView>` to `Package`
  - Add two loading loops to `load_package()` after relation types, following the identical pattern as types:
    - For `views`: read file → `serde_json::from_str::<View>` (error → `ViewLoad`) → `validate_view` (error → `ViewValidation`) → push
    - For `document_views`: same pattern with `DocumentView`, `DocumentViewLoad`, `DocumentViewValidation`
  - Add `pub fn resolve_view(&self, id: &str) -> Option<&View>` to `Package` impl
  - Add `pub fn resolve_document_view(&self, id: &str) -> Option<&DocumentView>` to `Package` impl

- [x] Create spec-compliant DocumentView JSON file at `srs/srs/package/document-views/srs-spec-document-view.json`

- [x] Retire the two pre-spec view files in `srs/srs/package/views/` (deleted)

- [x] Update `srs/srs/package/package.json`: `"views": []`, `"documentViews": ["document-views/srs-spec-document-view.json"]`

#### Acceptance Criteria

- [x] `cargo build -p srs-repository` compiles clean
- [x] `cargo clippy -p srs-repository -- -D warnings` clean
- [x] Existing test `load_package_from_live_repo` passes (still finds fields, types, relation types)
- [x] New test `load_package_loads_document_views` passes — loads the SRS package and finds the document view by ID
- [x] `resolve_document_view("ec34f54b-8636-5c8b-af5b-c9eb3df24fe6")` returns `Some(_)`

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository load_package
cargo clippy -p srs-repository -- -D warnings
```

New tests in `package.rs`:
- `load_package_loads_document_views` — loads srs/srs package, finds `document_views.len() >= 1`
- `resolve_document_view_finds_srs_spec_view` — resolves by UUID, checks `name == "srs-spec-document-view"`
- `resolve_document_view_returns_none_for_unknown` — random UUID returns `None`

#### Milestone gate

1. All existing package tests still pass. New tests pass. No clippy warnings. Commit.

---

### Phase 4: View Service and Render Service

**Goal:** `render_document_view()` produces valid markdown output when called against the SRS spec repo.

**Agent:** Render Worker

#### Tasks

- [ ] Expose relation loading: in `srs-rust/crates/srs-repository/src/relation_service.rs`, change `fn load_relations` to `pub(crate) fn load_relations`

- [x] Create `srs-rust/crates/srs-repository/src/view_service.rs`:
  ```rust
  pub enum GetDocumentViewResult { Found(Box<DocumentView>), NotFound }
  pub enum GetViewResult { Found(Box<View>), NotFound }

  pub fn list_document_views(repo_root: &Path) -> Result<Vec<DocumentView>, RepositoryError>
  pub fn get_document_view_by_id(repo_root: &Path, id: &str) -> Result<GetDocumentViewResult, RepositoryError>
  pub fn list_views(repo_root: &Path) -> Result<Vec<View>, RepositoryError>
  pub fn get_view_by_id(repo_root: &Path, id: &str) -> Result<GetViewResult, RepositoryError>
  ```
  All functions call `load_package(repo_root)?` and delegate to `package.resolve_*`.

- [x] Register in `srs-rust/crates/srs-repository/src/lib.rs`: `pub mod view_service;`

- [x] Create `srs-rust/crates/srs-repository/src/render_service.rs`:

  **Public API:**
  ```rust
  pub struct RenderDocumentViewOptions<'a> {
      pub repo_root: &'a Path,
      pub view_id: &'a str,
      pub format: Option<&'a str>,  // CLI override; supersedes dv.format
  }

  pub struct RenderResult {
      pub rendered: String,
      pub diagnostics: Vec<String>,
  }

  pub fn render_document_view(opts: RenderDocumentViewOptions) -> Result<RenderResult, RepositoryError>
  ```

  **Algorithm for `render_document_view`:**
  1. `load_package(repo_root)?` — get all package data
  2. Find DocumentView by `view_id` → `DocumentViewNotFound` if absent
  3. `load_manifest(repo_root)?` — for container title. Resolution order:
     - If `dv.container_type` is set and the manifest `containerIndex` contains a matching container whose `title` is non-empty: use that title
     - Else if `manifest.meta.title` (or equivalent top-level title field) is non-empty: use it
     - Else: use `manifest.namespace` as a last-resort fallback
     - This fallback chain satisfies Rule [N+3] for the common case while acknowledging that full container-space context is a follow-on. Add a comment in the code explaining this.
  4. `load_relations(repo_root)?` — for RelationQuery source resolution
  5. Effective format: `opts.format` → `dv.format.as_deref()` → `"markdown"` default
  6. If `dv.depth_offset.unwrap_or(0) > 4`: push diagnostic `"[N+4b] depthOffset {n} exceeds 4; heading levels may exceed what standard renderers support"`
  7. Render document opening:
     - If `dv.preamble` is `Some(p)`: emit `substitute_vars(p, &ctx)` + `"\n\n"`
     - Else if format is `"markdown"` or `"adoc"`: emit heading at level `1 + depth_offset` containing the container title (Rule [N+3])
  8. Sort `dv.sections` by `section.order` ascending
  9. For each section: `render_section(section, &dv, &ctx, format)?` → append to buffer
  10. Return `RenderResult { rendered: buffer, diagnostics }`

  **Source resolution (`resolve_section_instances`):**

  Reused functions (no modification needed):
  - `record_store::get_record_by_id(repo_root, id)` — for FixedInstances, RelationQuery
  - `record_store::list_records_by_type(repo_root, namespace, name)` — for TypeQuery
  - `container_service::list_members(repo_root, container_id)` — for ContainerSubset
  - `relation_service::load_relations(repo_root)` — for RelationQuery (now pub(crate))

  Parse `semantic_object_type` as `"namespace/name"` — split on first `/`. If no `/` is present, the value is malformed; emit a diagnostic `"[N] TypeQuery semanticObjectType '{value}' has no namespace separator '/' — expected 'namespace/name' format"` and return an empty instance list for that section (do not error). This surfaces authoring mistakes without aborting the render.

  RelationQuery direction defaults to Forward when `None`.

  **Heading helpers:**
  ```rust
  fn heading_prefix(level: u32, format: &str) -> String {
      match format {
          "markdown" => "#".repeat(level as usize) + " ",
          "adoc"     => "=".repeat(level as usize) + " ",
          _          => String::new(),  // "text" and unknown: no heading markup
      }
  }

  fn depth(base: u32, depth_offset: u32) -> u32 {
      base + depth_offset
  }
  ```

  **Template variable substitution** (`substitute_vars(template, container_title, record, depth_offset, format, context)`):
  - `{{container-title}}` → resolved container title (see step 3 above — containerIndex title → manifest meta title → manifest namespace fallback)
  - `{{date}}` → current date as `YYYY-MM-DD`
  - `{{heading-1}}` → `heading_prefix(depth(1, offset), format)`
  - `{{heading-2}}` → `heading_prefix(depth(2, offset), format)`
  - `{{heading-3}}` → in section context (ExportConfig.preamble): `heading_prefix(depth(3, offset), format)`; in standalone context: `""` (Rule [N+6b])
  - `{{instance-id}}` → `record.instance_id`
  - `{{status}}` → value of field named "status" in record, or `""`
  - `{{namespace}}` → `record.type_namespace`
  - `{{name}}` → `record.type_name`

  **Field rendering (markdown):**
  - Each field: `**{label}**: {value}\n`
  - For text format: `{label}: {value}\n`

  **Record rendering:**
  - Determine field order:
    - If `render_view_id` set: use `view.export_config.field_order` if present; else `view.field_views` sorted by `order`, filter `visible != Some(false)`
    - Else: look up RecordType → sort `FieldAssignment` by `order`
  - Label: `field_view.display_label` → `field_assignment.display_label` → `field.name` from package
  - Value: `record.find_field_value(field_id)` → render as string; null/missing → skip (or placeholder if emptyBehavior)
  - `omit_empty_fields`: skip absent fields when `export_config.omit_empty_fields == Some(true)`
  - Record heading (Rule [N+1]): if `section.title_field_id` is set, emit heading at `depth(3, offset)`; else no structural heading
  - ExportConfig preamble: if `render_view_id` set and `export_config.preamble` is Some, emit substituted preamble before field rows

  **Section rendering:**
  - If instances empty and `emptyBehavior == Some(Hide)` or `required != Some(true)`: return `""`
  - If `section.title` set: emit heading at `depth(2, offset)`
  - If `section.description` set: emit as paragraph
  - For each record: emit record block

- [ ] Register in `lib.rs`: `pub mod render_service;`

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles clean
- [ ] `cargo clippy -p srs-repository -- -D warnings` clean
- [ ] `render_document_view` called against `srs/srs` with view ID `ec34f54b-8636-5c8b-af5b-c9eb3df24fe6` returns `ok: true` and non-empty `rendered` string
- [ ] Rendered output contains `# Specification` or similar document title heading
- [ ] `diagnostics` is empty for default document view (no depthOffset > 4)
- [ ] `DocumentViewNotFound` returned for unknown view ID

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository render
cargo clippy -p srs-repository -- -D warnings
```

Tests in `render_service.rs` (all written and passing):
- ✓ `heading_prefix_markdown` — `heading_prefix(2, "markdown")` == `"## "`
- ✓ `heading_prefix_text_returns_empty` — `heading_prefix(2, "text")` == `""`
- ✓ `render_document_view_produces_output` — integration test against live `srs/srs` repo; checks output is non-empty markdown
- ✓ `render_document_view_unknown_id_returns_error` — `DocumentViewNotFound`
- ✓ `depth_offset_warning_emitted` — DocumentView with `depthOffset: 5` → diagnostic contains "[N+4b]"
- ✓ `title_field_id_emits_record_heading` — section with `titleFieldId` set → rendered output contains an H3 heading
- ✓ `no_title_field_id_omits_structural_heading` — section without `titleFieldId` → no H3 heading between section H2 and field rows (Rule [N+1])
- ✓ `semantic_object_type_missing_slash_emits_diagnostic` — `semanticObjectType: "noslash"` → diagnostic contains "no namespace separator"; section renders empty
- ✓ `repeatable_field_entries_render_all_values` — record with `entries` array → all entry values in output, no `[partial]` diagnostic

Fixture views added to `crates/srs-cli/tests/fixtures/repeatable-fields/package/document-views/`:
- `repeatable-doc-view.json` (id `...0981`) — basic type-query, no titleFieldId
- `deep-offset-view.json` (id `...0982`) — depthOffset 5
- `title-field-view.json` (id `...0983`) — titleFieldId set
- `bad-semantic-type-view.json` (id `...0984`) — semanticObjectType without `/`

#### Milestone gate

1. All tests pass. `cargo run --bin srs -- render document-view --repo ../srs/srs --view ec34f54b-8636-5c8b-af5b-c9eb3df24fe6 --pretty` produces valid JSON with non-empty `payload.rendered`. No clippy warnings. Commit.

---

### Phase 5: CLI Command

**Goal:** `srs render document-view` is available as a fully functional CLI command matching the contract in CLAUDE.md.

**Agent:** Rust Types Worker

#### Tasks

- [x] Create `srs-rust/crates/srs-cli/src/commands/render.rs`

- [x] Update `srs-rust/crates/srs-cli/src/commands/mod.rs`:
  - Add `pub mod render;`
  - Add `RenderCommand` enum with `DocumentView { view, format, output }`
  - Add to `Commands` enum and `dispatch()`

#### Output contract

Response envelope (consistent with all existing commands):
```json
{
  "ok": true,
  "command": "render document-view",
  "version": "0.x.x",
  "payload": {
    "rendered": "# Specification\n\n...",
    "diagnostics": []
  },
  "diagnostics": null
}
```

Rendered content is at `response.payload.rendered`. Render-time diagnostics (warnings) are at `response.payload.diagnostics`. This is consistent with how other commands nest their data in `payload`.

#### Acceptance Criteria

- [x] `cargo build -p srs-cli` compiles clean
- [x] `cargo clippy -p srs-cli -- -D warnings` clean
- [x] `srs render document-view --help` shows expected arguments
- [x] `srs render document-view --repo <path> --view <uuid>` returns JSON with `ok: true`
- [x] `srs render document-view --repo <path> --view <uuid> --format text` returns text-format output
- [x] `srs render document-view --repo <path> --view <unknown-uuid>` returns `ok: false` with error message
- [x] `srs render document-view --repo <path> --view <uuid> --output /tmp/out.md` writes file and returns JSON

#### Testing

```bash
cd srs-rust
cargo build
cargo run --bin srs -- render document-view \
    --repo ../srs/srs \
    --view ec34f54b-8636-5c8b-af5b-c9eb3df24fe6 \
    --pretty
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. All criteria pass. Full `cargo test` passes. Commit.

---

## Final Acceptance

- [x] `cargo test` passes with no failures across all crates
- [x] `cargo clippy -- -D warnings` passes
- [x] `srs render document-view --repo srs/srs --view ec34f54b-8636-5c8b-af5b-c9eb3df24fe6 --pretty` produces non-empty markdown in `payload.rendered`
- [x] All existing CLI commands continue to function (no regression)
- [x] RFC-001 and RFC-002 records have `status: "accepted"` in the SRS spec repo
- [x] `ext-themes-l1` appears in `manifest.json` `declaredExtensions`
- [ ] `ext-views-l2.json` description contains full RFC-001 spec text (Phase 1 gap)
- [ ] RFC-002 theme application implemented in renderer (Phase 6)

---

## Coordination Rules

- Phases 2 and 3 depend on no changes outside `srs-core` and `srs-repository` respectively.
- Phase 4 (render_service) must not start until Phase 3 milestone gate passes (package loading must work first).
- Phase 5 must not start until Phase 4 milestone gate passes.
- Phase 6 sub-phases must be completed in order: 6a → 6b → 6c → 6d. Phase 6d can start as soon as `RenderDocumentViewOptions` signature is finalized in 6c.
- Spec Worker (Phase 1) can run in parallel with Phases 2–3 since they touch different files.
- Workers return changed file paths and a behaviour summary when done.
- Lead Integrator owns final API naming across crate boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit. Do not proceed without completing the milestone gate.

## Assumptions

- `list_records_by_type(repo_root, namespace, name)` is already public in `record_store.rs` (confirmed)
- `list_members(repo_root, container_id)` is already public in `container_service.rs`
- The existing simplified view files in `srs/srs/package/views/` do not need to be migrated to L1 View format in this plan — they are cleared from `package.json`'s `views` array
- Container title resolution uses a fallback chain: containerIndex title → manifest meta title → manifest namespace. Full container-space context (where `DocumentView.containerType` drives container selection) is a follow-on.

---

### Phase 5.5: VS Code Extension Alignment (`srs-vscode`)

**Goal:** VS Code authoring/validation UX reflects the field-centric View model and remains in lockstep with canonical schemas.

**Agent:** VS Code Worker

#### Tasks

- [x] Update `srs-vscode/schemas/2.0/view.json` to match canonical schema changes:
  - remove `typeId`/`typeVersion`
  - add optional `compatibleTypes`
- [x] Ensure schema registration paths in `srs-vscode` still map View documents to updated schema
- [x] Update any editor form/payload assumptions in:
  - `src/webview/forms.ts`
  - `src/webview/EntityEditorPanel.ts`
  - `test/suite/payload-contracts.test.ts`
  so View create/edit flows do not require type binding fields
  - **Implementation note:** no dedicated View create/edit form currently exists in these paths; no code changes required for this migration.
- [x] Add/adjust diagnostics messaging (if present) to report field-compatibility issues rather than type mismatch
- [x] Verify preview/editor roundtrip for a mixed-type, shared-field View

#### Acceptance Criteria

- [x] VS Code View JSON validation passes without `typeId`/`typeVersion`
- [x] View editor UI no longer prompts for bound Type/version
- [x] Payload contract tests pass with new View shape
- [x] A single View can be used across records of different types sharing required fields

---

### Phase 6: RFC-002 Theme Application

**Goal:** Full RFC-002 (`ext:themes-l1`) theme application in the renderer. `srs render document-view` applies element templates from a resolved `Theme` when `DocumentView.themeRef` is set and the theme targets the output format.

**Current state:** `ThemeReference`, `ThemeVariant`, `ThemeMode` types exist and deserialize correctly. `Theme` exists and package loading resolves `themes` entries. The renderer now resolves bundled themes, applies document/section/record/field-row wrappers, honors `themeVariants`, and the CLI forwards `--theme-variant`. HTML/PDF-specific class injection remains a follow-on with the future HTML renderer.

**Out of scope in Phase 6:** `pageTemplates` (paginated formats), stylesheet/typography application, local/remote asset resolution (assets declared, not resolved), HTML format output (full HTML rendering is a separate effort), pdf/docx formats.

---

#### Phase 6a: Theme type and validation (`srs-core`)

**Agent:** Rust Types Worker

##### Tasks

- [x] Create `srs-rust/crates/srs-core/src/types/theme.rs` with:
  - `AssetMode` enum: `Local`, `Remote`, `Inline` (`#[serde(rename_all = "lowercase")]`)
  - `AssetType` enum: `Image`, `Font`, `Stylesheet`, `Data` (`#[serde(rename_all = "lowercase")]`)
  - `AssetDeclaration` struct: `asset_type` (`#[serde(rename = "type")]`), `mode`, `path`, `url`, `data`, `mime_type`
  - `SectionWrapperOverride` struct: `section_id`, `template`
  - `RecordWrapperOverride` struct: `type_id`, `template`
  - `ElementTemplates` struct: `document_wrapper`, `section_wrapper`, `section_wrapper_overrides`, `record_wrapper`, `record_wrapper_overrides`, `field_row`
  - `Theme` struct (`#[serde(rename_all = "camelCase")]`): `id`, `namespace`, `name`, `version: u32`, `description`, `targets: Vec<String>`, `assets: Option<HashMap<String, AssetDeclaration>>`, `css_class_fields: Option<Vec<String>>`, `element_templates: Option<ElementTemplates>`, `created_at: String`; out-of-scope fields (`page_templates`, `stylesheet`, `typography`) typed as `Option<serde_json::Value>` with `#[serde(default)]` so they round-trip without error

- [x] Register `pub mod theme;` in `srs-rust/crates/srs-core/src/types/mod.rs`

- [x] Create `srs-rust/crates/srs-core/src/validation/theme.rs`:
  - `pub fn validate_theme(theme: &Theme) -> Result<(), CoreError>`
  - Rule T-1b: `targets` non-empty → `ThemeTargetsEmpty`
  - Rule T-7a: unique `section_id` in `section_wrapper_overrides` → `DuplicateThemeSectionOverrideId`
  - Rule T-7b: unique `type_id` in `record_wrapper_overrides` → `DuplicateThemeRecordOverrideTypeId`
  - Asset-name uniqueness is structurally enforced by the `assets` object shape; the typed model does not need a dedicated duplicate-asset validator path

- [x] Register `pub mod theme;` in `srs-rust/crates/srs-core/src/validation/mod.rs`

- [x] Add to `srs-rust/crates/srs-core/src/error.rs`:
  ```rust
  ThemeTargetsEmpty,
  DuplicateThemeSectionOverrideId { section_id: String },
  DuplicateThemeRecordOverrideTypeId { type_id: String },
  ```
  Add matching `PartialEq` arms.

##### Tests

In `types/theme.rs`:
- `theme_roundtrips_minimal_json`
- `theme_roundtrips_full_element_templates`
- `theme_deserializes_unknown_top_level_fields_silently` — `pageTemplates` present in JSON does not cause error

In `validation/theme.rs`:
- `validate_theme_empty_targets_fails`
- `validate_theme_single_target_passes`
- `validate_theme_duplicate_section_override_id_fails`
- `validate_theme_unique_section_override_ids_passes`
- `validate_theme_duplicate_record_override_type_id_fails`

##### Acceptance Criteria

- [x] `cargo build -p srs-core` compiles clean
- [x] `cargo clippy -p srs-core -- -D warnings` clean
- [x] `Theme` with no `targets` fails validation with `ThemeTargetsEmpty`
- [x] Minimal valid `Theme` JSON deserializes without error
- [x] Unknown top-level fields in the JSON (e.g. `pageTemplates`) do not cause deserialization errors

##### Milestone gate

`cargo test -p srs-core` green. Commit.

---

#### Phase 6b: Theme loading (`srs-repository/package.rs`)

**Agent:** Rust Types Worker

##### Tasks

- [x] Add `#[serde(default)] themes: Vec<String>` to `PackageMetadata` in `package.rs`

- [x] Add `pub themes: Vec<Theme>` to `Package` struct

- [x] Add to `Package` impl:
  - `pub fn resolve_theme(&self, theme_id: &str) -> Option<&Theme>`
  - `pub fn themes(&self) -> &[Theme]`

- [x] Add theme loading loop to `load_package_from_dir` (mirrors `document_views` loop):
  - read file → `serde_json::from_str::<Theme>` → `validate_theme` → push
  - expand return type to include `Vec<Theme>`; thread through `load_package` merge loop

- [x] Add to `srs-rust/crates/srs-repository/src/error.rs`:
  ```rust
  ThemeLoad { path: PathBuf, source: serde_json::Error },
  ThemeValidation { path: PathBuf, source: CoreError },
  BundledThemeNotFound { theme_id: String },
  ```

- [x] Create fixture `srs-rust/crates/srs-cli/tests/fixtures/themed/` or equivalent temp-repo helpers:
  - `load_package` tests now cover valid, invalid, and no-`themes` cases with temp repositories

##### Tests in `package.rs`

- [x] `load_package_loads_themes`
- [x] `resolve_theme_finds_by_id`
- [x] `resolve_theme_returns_none_for_unknown`
- [x] `load_package_theme_validation_fails_on_empty_targets`

##### Acceptance Criteria

- [x] `cargo build -p srs-repository` compiles clean
- [x] `cargo clippy -p srs-repository -- -D warnings` clean
- [x] `Package.themes` populated from `package.json`'s `themes` array
- [x] `Package::resolve_theme(id)` returns the correct theme
- [x] Package with no `themes` key loads without error

##### Milestone gate

`cargo test -p srs-repository` green. Commit.

---

#### Phase 6c: Theme application (`render_service.rs`)

**Agent:** Render Worker

##### Tasks

- [x] Add `pub theme_variant: Option<&'a str>` to `RenderDocumentViewOptions`

- [x] Add `active_theme: Option<Theme>` to `RenderContext`

- [x] Add all existing `RenderDocumentViewOptions` constructors in tests: add `theme_variant: None` field

- [x] Create private function `resolve_active_theme(dv, package, theme_variant, format, diagnostics) -> Option<Theme>`:
  - Implements RFC-001 Rule [N+8] + Rules T-1, T-2, T-5
  - Variant name given: find in `dv.theme_variants`; if not found emit diagnostic, fall back to `dv.theme_ref`
  - Bundled ref: `package.resolve_theme()`; if missing emit `[T-5]` diagnostic, return None
  - Local/remote refs: emit "not supported in this release" diagnostic, return None
  - Format mismatch (T-2): emit `[T-2]` diagnostic, return None
  - No themeRef at all: return None silently

- [x] Create private function `apply_wrapper(template: &str, vars: &[(&str, &str)]) -> String`:
  - Substitutes `{{name}}` for each `(name, value)` pair
  - Unknown vars pass through literally (Rule T-6)
  - Pre-zeroes `{{heading-1}}` through `{{heading-6}}` to `""` (Rule T-6b — element wrappers only)
  - Post-pass for `{{asset:name}}` pattern: scan and replace via `resolve_asset`

- [x] Create private function `resolve_asset<'a>(theme: &'a Theme, name: &str) -> &'a str`:
  - inline mode: `asset.data.as_deref().unwrap_or("")`
  - local/remote: `""` (Phase 6 scope)

- [x] Extract `format_field_row(format: &str, label: &str, value: &str) -> String` from existing inline logic

- [x] In `render_document_view`: resolve active theme after computing ctx; after all sections assembled, apply `documentWrapper` if present

- [x] In `render_section`: add `theme: Option<&Theme>` param; apply `sectionWrapperOverrides` (by `section_id`) then `sectionWrapper` after section body assembled (Rule T-7 precedence)

- [x] In `render_record_at_level`: add `theme: Option<&Theme>` param; apply `recordWrapperOverrides` (by `type_id`) then `recordWrapper` after record body assembled

- [ ] In `render_record_at_level`: compute and inject CSS classes when `format == "html"` (Rules T-8, T-9: string/text/select fields only)

- [x] In field row loop: apply `fieldRow` template to each row; preamble content NOT wrapped (Rule T-10)

##### Tests (use `themed/` fixture)

- `theme_document_wrapper_wraps_entire_output`
- `theme_section_wrapper_wraps_each_section`
- `theme_section_wrapper_override_takes_precedence`
- `theme_record_wrapper_wraps_each_record`
- `theme_field_row_wraps_each_field`
- `theme_format_mismatch_skips_theme_and_emits_diagnostic`
- `theme_bundled_ref_not_found_emits_diagnostic`
- `theme_no_themeref_renders_without_theme`
- `theme_unknown_template_var_passes_through_literally`
- `theme_variant_selection_uses_matching_variant`
- `theme_variant_not_found_falls_back_to_theme_ref`
- `theme_heading_vars_resolve_empty_in_element_wrappers`
- `theme_field_row_not_applied_to_preamble`

##### Acceptance Criteria

- [x] `cargo build -p srs-repository` compiles clean
- [x] `cargo clippy -p srs-repository -- -D warnings` clean
- [x] Plain renders (no themeRef) produce identical output to before Phase 6
- [x] `documentWrapper` wraps the entire rendered body
- [x] `sectionWrapperOverrides` takes precedence over `sectionWrapper` for matching `sectionId`
- [x] Format mismatch emits `[T-2]` diagnostic; renders without theme
- [x] Unknown template vars pass through as literal text
- [x] `{{heading-N}}` in element wrapper templates resolves to `""`

##### Milestone gate

`cargo test -p srs-repository` green. Manual `srs render document-view` against themed fixture produces visibly wrapped output. No regression on existing integration test. Commit.

---

#### Phase 6d: CLI flag (`srs-cli`)

**Agent:** Rust Types Worker

##### Tasks

- [x] Add `#[arg(long = "theme-variant")] theme_variant: Option<String>` to `RenderCommand::DocumentView` in `commands/mod.rs`

- [x] Destructure and forward `theme_variant.as_deref()` to `RenderDocumentViewOptions` in `commands/render.rs`

##### Tests

- `cli_render_document_view_with_theme_variant_flag_passes_through`
- `cli_render_document_view_without_theme_variant_works_as_before`
- `cli_render_document_view_theme_variant_not_found_produces_diagnostic_not_error`

##### Acceptance Criteria

- [x] `cargo build -p srs-cli` compiles clean
- [x] `cargo clippy -p srs-cli -- -D warnings` clean
- [x] `srs render document-view --help` shows `--theme-variant <NAME>`
- [x] `--theme-variant` value forwarded to `RenderDocumentViewOptions.theme_variant`
- [x] Variant-not-found cases produce a diagnostic in `payload.diagnostics`, not an error exit

##### Milestone gate

`cargo test` green across all crates. Commit.
