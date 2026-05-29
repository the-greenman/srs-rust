# Plan: Views (ext:views-l1 / ext:views-l2) and `srs render document-view`

## Summary

RFC-001 and RFC-002 have been reviewed, corrected, and are ready to apply. Before the Rust implementation can begin, the spec SRS records must reflect the accepted RFC content, and then the Rust stack needs the new types, package loading, rendering logic, and CLI command. This plan covers all three layers in sequence: spec record updates, Rust type/service implementation, and the `srs render document-view` command.

## Progress Snapshot (2026-05-29)

- Phase 2 (Rust Types): complete
- Phase 3 (Package Loading): complete
- Phase 4 (View Service + Render Service): complete
- Phase 5 (CLI Command): complete
- Phase 1 (Spec Record Updates in `srs` repo): pending

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
- Rust types for `View`, `DocumentView`, `ExportConfig`, `FieldView`, `SectionSource`, `DocumentSection`, `NavigationLink`, `ThemeReference`, `ThemeVariant` in `srs-core`
- Validation rules for those types
- Package loading for `views` (L1) and `documentViews` (L2) in `srs-repository`
- `view_service` and `render_service` in `srs-repository`
- `srs render document-view` CLI command
- One spec-compliant `DocumentView` JSON file for the SRS spec document to enable end-to-end testing

**Out of scope:**

- RFC-002 theme application logic (ThemeReference/ThemeVariant types are stubs; no theme rendering)
- `ext:repeatable-fields` support in the render baseline (fields treated as scalar only)
- `ext:type-inheritance` fieldOrder support (use FieldAssignment.order only)
- `srs view list` / `srs view get` CLI commands (render is the priority; service layer supports them when added)
- `"html"` and `"adoc"` format rendering (markdown and text only in this plan)

**Partial-conformance policy:** The renderer is intentionally incomplete with respect to two declared extensions. To prevent silent non-conformance, the renderer MUST emit explicit diagnostics when it encounters conditions it cannot fully handle:
- If a `FieldValue` with `entries` array is present (indicates `ext:repeatable-fields` usage): emit diagnostic `"[partial] repeatable field {field_id} rendered as first entry only; ext:repeatable-fields not fully supported"`
- If a `DocumentView`'s section source resolves a `RecordType` that has a `fieldOrder` property (indicates `ext:type-inheritance` usage): emit diagnostic `"[partial] ext:type-inheritance fieldOrder ignored; using FieldAssignment.order"`

These diagnostics go into `RenderResult.diagnostics`, not as errors. The render proceeds with best-effort output.

---

## Phases

### Phase 1: Spec Record Updates

**Goal:** RFC-001 and RFC-002 are marked accepted in the SRS records, the ext-views-l2 record reflects all RFC-001 changes, and a new ext-themes-l1 record exists.

**Agent:** Spec Worker

#### Tasks

- [ ] Update `srs/srs/records/rfcs/rfc-001-views-l2-rendering.json`:
  - Set `fieldValues[status]` (`5a000002-0000-4000-a000-000000000002`) to `"accepted"`
  - Set `fieldValues[content]` (`1a000002-0000-4000-a000-000000000002`) to the full text from `srs/rfcs/rfc-001.md` (strip the top-level `# RFC-001:` heading and the `> Projection note` callout — keep everything from `**Status**: Draft (Revision 8)` onward, updating the status line to `**Status**: Accepted (Revision 8)`)

- [ ] Update `srs/srs/records/rfcs/rfc-002-themes-l1.json`:
  - Set `fieldValues[status]` to `"accepted"`
  - Set `fieldValues[content]` to the full text from `srs/rfcs/rfc-002.md` (same stripping rule, updating status line to `**Status**: Accepted (Revision 5)`)

- [ ] Update `srs/srs/records/extensions/ext-views-l2.json`:
  - Replace the `description` field value (`1a000002-0000-4000-a000-000000000002`) with the updated spec text incorporating all RFC-001 changes (Changes A–D). The updated description must include:
    - The full `SectionSource` type definition (unchanged)
    - The updated `DocumentSection` definition with `titleFieldId` (RFC-001 Change B1)
    - The updated `DocumentView` definition with `depthOffset`, `format` vocabulary, `themeRef`, `themeVariants` (RFC-001 Changes B2, C, D)
    - The `ThemeReference` and `ThemeVariant` type definitions (RFC-001 Change D)
    - All conformance rules [N] through [N+8] verbatim from the RFC, including [N+4b] and [N+6b] which were added in Rev 8

- [ ] Create `srs/srs/records/extensions/ext-themes-l1.json` — new extension record:
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

- [ ] Add `ext-themes-l1` to `srs/srs/manifest.json`:
  - Add to `instanceIndex`: `{ "instanceId": "a3f7c9e1-2b4d-4f8a-9c1e-5d7b3a2f6e0c", "path": "records/extensions/ext-themes-l1.json", "title": "ext:themes-l1" }`
  - Add `"ext:themes-l1"` to `declaredExtensions` array

#### Acceptance Criteria

- [ ] Both RFC records have `status: "accepted"`
- [ ] RFC-002 content reflects Rev 5 (open question 2 resolved, `{{heading-N}}` scope fixed)
- [ ] `ext-views-l2.json` description contains `titleFieldId`, `depthOffset`, `ThemeReference`, `ThemeVariant`, and all conformance rules [N] through [N+8] — including [N+4b] (depthOffset warning) and [N+6b] (`{{heading-3}}` standalone suppression) introduced in Rev 8
- [ ] `ext-themes-l1.json` exists and is listed in the manifest instanceIndex
- [ ] `"ext:themes-l1"` appears in `declaredExtensions`
- [ ] `node scripts/validate-all.mjs` passes from `srs/` (if validator is available)

#### Milestone gate

1. Verify all acceptance criteria above.
2. Commit spec record changes.

---

### Phase 2: Rust Types

**Goal:** All View/DocumentView types compile in `srs-core` with serde roundtrip tests passing.

**Agent:** Rust Types Worker

#### Tasks

- [ ] Create `srs-rust/crates/srs-core/src/types/view.rs` with all types below
- [ ] Register in `srs-rust/crates/srs-core/src/types/mod.rs`: `pub mod view;`
- [ ] Add new `CoreError` variants to `srs-rust/crates/srs-core/src/error.rs`
- [ ] Create `srs-rust/crates/srs-core/src/validation/view.rs`
- [ ] Register in `srs-rust/crates/srs-core/src/validation/mod.rs`: `pub mod view;`

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
    pub type_id: String,
    pub type_version: u32,
    pub field_views: Vec<FieldView>,
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

- [ ] `cargo build -p srs-core` compiles clean
- [ ] `cargo clippy -p srs-core -- -D warnings` clean
- [ ] `DocumentView` roundtrips through `serde_json::to_string` / `from_str` without data loss
- [ ] `SectionSource` variants deserialise correctly from `{"type": "type-query", "semanticObjectType": "ns/name"}` etc.
- [ ] `validate_document_view` returns `EmptyDocumentViewSections` for a view with no sections
- [ ] `validate_document_view` returns `DuplicateDocumentSectionId` for duplicate sectionIds
- [ ] `validate_document_view` returns `DuplicateThemeVariantName` for duplicate variant names in `themeVariants`

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

- [ ] Add `RepositoryError` variants to `srs-rust/crates/srs-repository/src/error.rs`:
  ```rust
  ViewLoad { path: PathBuf, source: serde_json::Error },
  ViewValidation { path: PathBuf, source: CoreError },
  DocumentViewLoad { path: PathBuf, source: serde_json::Error },
  DocumentViewValidation { path: PathBuf, source: CoreError },
  DocumentViewNotFound { view_id: String },
  ```

- [ ] Update `srs-rust/crates/srs-repository/src/package.rs`:
  - Add `views: Vec<String>` and `document_views: Vec<String>` (both `#[serde(default)]`) to `PackageMetadata`
  - Add `pub views: Vec<View>` and `pub document_views: Vec<DocumentView>` to `Package`
  - Add two loading loops to `load_package()` after relation types, following the identical pattern as types:
    - For `views`: read file → `serde_json::from_str::<View>` (error → `ViewLoad`) → `validate_view` (error → `ViewValidation`) → push
    - For `document_views`: same pattern with `DocumentView`, `DocumentViewLoad`, `DocumentViewValidation`
  - Add `pub fn resolve_view(&self, id: &str) -> Option<&View>` to `Package` impl
  - Add `pub fn resolve_document_view(&self, id: &str) -> Option<&DocumentView>` to `Package` impl

- [ ] Create spec-compliant DocumentView JSON file:
  - Create directory `srs/srs/package/document-views/`
  - Create `srs/srs/package/document-views/srs-spec-document-view.json`:
    ```json
    {
      "id": "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6",
      "namespace": "com.semanticops.srs",
      "name": "srs-spec-document-view",
      "version": 1,
      "description": "Renders the full SRS specification as a structured markdown document",
      "format": "markdown",
      "sections": [
        {
          "sectionId": "spec-sections",
          "title": "Specification",
          "order": 0,
          "source": {
            "type": "type-query",
            "semanticObjectType": "com.semanticops.srs/meta.section"
          },
          "emptyBehavior": "hide"
        }
      ],
      "createdAt": "2026-05-29T00:00:00Z"
    }
    ```
    The UUID reuses the one from the existing simplified view file so existing references remain valid.

- [ ] Retire the two pre-spec view files in `srs/srs/package/views/`:
  - `srs/srs/package/views/srs-spec-document-view.json` — this was a pre-RFC placeholder using a non-conformant schema (`title`, `outputFormats`, `rootType`). It is superseded by the new spec-compliant `document-views/srs-spec-document-view.json` (which reuses the same UUID). Delete this file.
  - `srs/srs/package/views/extension-card-view.json` — similarly pre-spec placeholder, never loaded by any Rust code. Delete this file.
  - No Rust code references either file or either UUID; confirmed by grepping the srs-rust codebase. This is not a regression — these files were never consumed.

- [ ] Update `srs/srs/package/package.json`:
  - Set `"views": []` — the two files in `package/views/` have been deleted above; no L1 View files exist yet (follow-on work)
  - Add `"documentViews": ["document-views/srs-spec-document-view.json"]`

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles clean
- [ ] `cargo clippy -p srs-repository -- -D warnings` clean
- [ ] Existing test `load_package_from_live_repo` passes (still finds fields, types, relation types)
- [ ] New test `load_package_loads_document_views` passes — loads the SRS package and finds the document view by ID
- [ ] `resolve_document_view("ec34f54b-8636-5c8b-af5b-c9eb3df24fe6")` returns `Some(_)`

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

- [ ] Create `srs-rust/crates/srs-repository/src/view_service.rs`:
  ```rust
  pub enum GetDocumentViewResult { Found(Box<DocumentView>), NotFound }
  pub enum GetViewResult { Found(Box<View>), NotFound }

  pub fn list_document_views(repo_root: &Path) -> Result<Vec<DocumentView>, RepositoryError>
  pub fn get_document_view_by_id(repo_root: &Path, id: &str) -> Result<GetDocumentViewResult, RepositoryError>
  pub fn list_views(repo_root: &Path) -> Result<Vec<View>, RepositoryError>
  pub fn get_view_by_id(repo_root: &Path, id: &str) -> Result<GetViewResult, RepositoryError>
  ```
  All functions call `load_package(repo_root)?` and delegate to `package.resolve_*`.

- [ ] Register in `srs-rust/crates/srs-repository/src/lib.rs`: `pub mod view_service;`

- [ ] Create `srs-rust/crates/srs-repository/src/render_service.rs`:

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

Tests to write in `render_service.rs`:
- `render_document_view_produces_output` — integration test against live `srs/srs` repo; checks output is non-empty markdown
- `render_document_view_unknown_id_returns_error` — `DocumentViewNotFound`
- `depth_offset_warning_emitted` — DocumentView with `depthOffset: 5` → diagnostic contains "[N+4b]"
- `heading_prefix_markdown` — unit test: `heading_prefix(2, "markdown")` == `"## "`
- `heading_prefix_text_returns_empty` — `heading_prefix(2, "text")` == `""`
- `title_field_id_emits_record_heading` — section with `titleFieldId` set → rendered output contains an H3 heading with the field's value
- `no_title_field_id_omits_structural_heading` — section without `titleFieldId` → no H3 heading injected between section H2 and field rows (Rule [N+1])
- `semantic_object_type_missing_slash_emits_diagnostic` — `semanticObjectType: "nodash"` → diagnostic contains "no namespace separator" and section renders empty
- `repeatable_field_partial_diagnostic_emitted` — record with `entries` array in a FieldValue → diagnostic contains "[partial] repeatable field"

#### Milestone gate

1. All tests pass. `cargo run --bin srs -- render document-view --repo ../srs/srs --view ec34f54b-8636-5c8b-af5b-c9eb3df24fe6 --pretty` produces valid JSON with non-empty `payload.rendered`. No clippy warnings. Commit.

---

### Phase 5: CLI Command

**Goal:** `srs render document-view` is available as a fully functional CLI command matching the contract in CLAUDE.md.

**Agent:** Rust Types Worker

#### Tasks

- [ ] Create `srs-rust/crates/srs-cli/src/commands/render.rs`:
  ```rust
  pub fn dispatch(ctx: CliContext, cmd: RenderCommand) -> Result<String>
  // dispatches to cmd_render_document_view

  fn cmd_render_document_view(ctx, view_id, format, output_path) -> Result<String>
  // calls render_document_view(opts)
  // if output_path provided: writes rendered string to file
  // returns output::ok("render document-view", json!({ "rendered": ..., "diagnostics": [...] }))
  // on RepositoryError: returns output::err("render document-view", vec![e.to_string()])
  ```

- [ ] Update `srs-rust/crates/srs-cli/src/commands/mod.rs`:
  - Add `pub mod render;`
  - Add `RenderCommand` enum:
    ```rust
    #[derive(Subcommand)]
    pub enum RenderCommand {
        DocumentView {
            #[arg(long = "view")]  view: String,
            #[arg(long)]           format: Option<String>,
            #[arg(long)]           output: Option<PathBuf>,
        }
    }
    ```
  - Add to `Commands` enum: `Render(#[command(subcommand)] RenderCommand)`
  - Add to `dispatch()`: `Commands::Render(cmd) => render::dispatch(ctx, cmd)`

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

- [ ] `cargo build -p srs-cli` compiles clean
- [ ] `cargo clippy -p srs-cli -- -D warnings` clean
- [ ] `srs render document-view --help` shows expected arguments
- [ ] `srs render document-view --repo <path> --view <uuid>` returns JSON with `ok: true`
- [ ] `srs render document-view --repo <path> --view <uuid> --format text` returns text-format output
- [ ] `srs render document-view --repo <path> --view <unknown-uuid>` returns `ok: false` with error message
- [ ] `srs render document-view --repo <path> --view <uuid> --output /tmp/out.md` writes file and returns JSON

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

- [ ] `cargo test` passes with no failures across all crates
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `srs render document-view --repo srs/srs --view ec34f54b-8636-5c8b-af5b-c9eb3df24fe6 --pretty` produces non-empty markdown in `payload.rendered`
- [ ] RFC-001 and RFC-002 records have `status: "accepted"` in the SRS spec repo
- [ ] `ext-themes-l1` appears in `manifest.json` `declaredExtensions`
- [ ] All existing CLI commands continue to function (no regression)

---

## Coordination Rules

- Phases 2 and 3 depend on no changes outside `srs-core` and `srs-repository` respectively.
- Phase 4 (render_service) must not start until Phase 3 milestone gate passes (package loading must work first).
- Phase 5 must not start until Phase 4 milestone gate passes.
- Spec Worker (Phase 1) can run in parallel with Phases 2–3 since they touch different files.
- Workers return changed file paths and a behaviour summary when done.
- Lead Integrator owns final API naming across crate boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit. Do not proceed without completing the milestone gate.

## Assumptions

- `list_records_by_type(repo_root, namespace, name)` is already public in `record_store.rs` (confirmed)
- `list_members(repo_root, container_id)` is already public in `container_service.rs`
- The existing simplified view files in `srs/srs/package/views/` do not need to be migrated to L1 View format in this plan — they are cleared from `package.json`'s `views` array
- Container title resolution uses a fallback chain: containerIndex title → manifest meta title → manifest namespace. Full container-space context (where `DocumentView.containerType` drives container selection) is a follow-on.
