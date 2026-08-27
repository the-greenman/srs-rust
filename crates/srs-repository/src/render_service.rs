use crate::container_service::list_members;
use crate::error::RepositoryError;
use crate::package::Package;
use crate::record_store::{get_instance_by_id, list_records_by_type, LoadedInstance};
use crate::relation_graph;
use crate::relation_service::load_relations;
use crate::store::RepositoryStore;
use serde_json::json;
use srs_core::types::field::{Datatype, FieldType};
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use srs_core::types::relation::RelationStatus;
use srs_core::types::theme::{AssetMode, Theme};
use srs_core::types::view::{
    ContainerScope, DocumentSection, DocumentView, PresentationDirection, RelationDirection,
    SectionSource, SortDirection, ThemeMode,
};
use std::collections::HashSet;

pub struct RenderDocumentViewOptions<'a> {
    pub store: &'a dyn RepositoryStore,
    pub view_id: &'a str,
    pub format: Option<&'a str>,
    pub theme_variant: Option<&'a str>,
    /// When set, TypeQuery sections are filtered to members of this container.
    /// Takes precedence over any container_ids declared in the view definition.
    pub container_id: Option<&'a str>,
    /// When set, ContainerSubset sections are filtered to the single instance with this ID,
    /// producing a per-record export document. Takes precedence over any instance-level
    /// selection already in the view definition.
    pub instance_id_filter: Option<&'a str>,
}

impl<'a> RenderDocumentViewOptions<'a> {
    pub fn new(store: &'a dyn RepositoryStore, view_id: &'a str) -> Self {
        Self {
            store,
            view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        }
    }
}

// Clone not derived: DocumentViewProjection does not implement Clone.
#[derive(Debug, serde::Serialize)]
pub struct RenderResult {
    pub rendered: String,
    pub diagnostics: Vec<String>,
    pub projection: Option<DocumentViewProjection>,
}

// ── JSON projection output types ──────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedRelationTarget {
    pub instance_id: String,
    pub display_label: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedRelationRow {
    pub label: String,
    pub targets: Vec<ProjectedRelationTarget>,
    /// The relation type key backing this row, used for the `srs-relationtype-*`
    /// identity class of `[FR-037-12]`. Not serialised — the `json` projection is
    /// governed by `document-view-output.json` and RFC-037 changes no schema.
    #[serde(skip)]
    pub relation_type_key: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedRecord {
    pub instance_id: String,
    pub type_id: String,
    pub type_namespace: String,
    pub type_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    /// RFC-039 [R11]: keyed by `Field.name`; a composite value is carried
    /// recursively under its own key, exactly as in the instance.
    pub fields: serde_json::Value,
    /// `Field.name` keys in `FieldAssignment.order`.
    pub ordered_field_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relations: Option<Vec<ProjectedRelationRow>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedSection {
    pub section_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub order: i32,
    pub records: Vec<ProjectedRecord>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewProjection {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub document_view_id: String,
    pub container_id: Option<String>,
    pub generated_at: String,
    pub container_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    pub sections: Vec<ProjectedSection>,
}

#[derive(Clone)]
struct ResolvedFieldRender {
    /// May be empty when the render list came from the record's own keys
    /// (no Type resolved) — `name` is then the only identity.
    field_id: String,
    /// `Field.name` — the RFC-039 carrier key.
    name: String,
    required: bool,
}

struct RenderContext<'a> {
    package: &'a Package,
    container_title: String,
    depth_offset: u32,
    format: &'a str,
    /// `Field.name` of the conventional status field, when the package has one
    /// — the RFC-039 carrier keys by name.
    status_field_name: Option<String>,
    active_theme: Option<Theme>,
    /// RFC-036 — DocumentView.compositeRenderers, the lowest-precedence
    /// composite dispatch site ([CR-036-6]).
    doc_composite_renderers: Option<Vec<srs_core::types::view::CompositeRendererDirective>>,
}

pub fn render_document_view(
    opts: RenderDocumentViewOptions<'_>,
) -> Result<RenderResult, RepositoryError> {
    let package = opts.store.load_package()?;
    let dv = package.resolve_document_view(opts.view_id).ok_or_else(|| {
        RepositoryError::DocumentViewNotFound {
            view_id: opts.view_id.to_string(),
        }
    })?;
    let dv = dv.clone();

    let manifest = opts.store.load_manifest()?;
    let mut diagnostics = Vec::new();
    let container_title = resolve_container_title(opts.store, &dv, &manifest, opts.container_id);
    let relations = load_relations(opts.store)?;
    let format = opts
        .format
        .unwrap_or(dv.format.as_deref().unwrap_or("markdown"));
    let depth_offset = dv.depth_offset.unwrap_or(0);
    if depth_offset > 4 {
        diagnostics.push(format!(
            "[N+4b] depthOffset {depth_offset} exceeds 4; heading levels may exceed what standard renderers support"
        ));
    }
    let active_theme =
        resolve_active_theme(&dv, &package, opts.theme_variant, format, &mut diagnostics);

    if format == "json" {
        let projection = project_document_view_json(
            opts.store,
            &package,
            &dv,
            &manifest,
            &container_title,
            &relations,
            opts.container_id,
            opts.instance_id_filter,
            &mut diagnostics,
        )?;
        return Ok(RenderResult {
            rendered: String::new(),
            diagnostics,
            projection: Some(projection),
        });
    }

    let ctx = RenderContext {
        package: &package,
        container_title,
        depth_offset,
        format,
        status_field_name: package.find_field_by_name("status").map(|f| f.name.clone()),
        active_theme,
        doc_composite_renderers: dv.composite_renderers.clone(),
    };

    let mut rendered = String::new();
    if let Some(preamble) = &dv.preamble {
        rendered.push_str(&substitute_vars(preamble, &ctx, None, false));
        rendered.push_str("\n\n");
    } else {
        rendered.push_str(&format_heading(
            depth(1, ctx.depth_offset),
            format,
            &ctx.container_title,
        ));
    }

    let mut sections = dv.sections.clone();
    sections.sort_by_key(|s| s.order);
    for section in &sections {
        rendered.push_str(&render_section(
            opts.store,
            &ctx,
            section,
            &relations,
            opts.container_id,
            opts.instance_id_filter,
            &mut diagnostics,
        )?);
    }

    if format == "html" {
        if let Some(theme) = ctx.active_theme.as_ref() {
            if let Some(stylesheet) = &theme.stylesheet {
                if let Some(css) = stylesheet.get("content").and_then(|v| v.as_str()) {
                    rendered = format!("<style>\n{css}\n</style>\n{rendered}");
                } else if stylesheet.get("mode").and_then(|v| v.as_str()) == Some("local") {
                    diagnostics.push(
                        "[theme-stylesheet] local stylesheet paths are not yet resolved; stylesheet omitted"
                            .to_string(),
                    );
                }
            }
        }
    }

    let doc_wrapper = ctx
        .active_theme
        .as_ref()
        .and_then(|t| t.element_templates.as_ref())
        .and_then(|et| et.document_wrapper.as_deref());
    if let Some(wrapper) = doc_wrapper {
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        rendered = apply_wrapper(
            wrapper,
            &rendered,
            &[("container-title", &ctx.container_title), ("date", &date)],
            ctx.active_theme.as_ref(),
        );
    } else if format == "html" {
        rendered = format!("<div class=\"srs-document\">{rendered}</div>\n");
    }

    Ok(RenderResult {
        rendered,
        diagnostics,
        projection: None,
    })
}

// ── JSON projection engine ─────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn project_document_view_json(
    store: &dyn RepositoryStore,
    package: &Package,
    dv: &DocumentView,
    manifest: &crate::manifest::Manifest,
    container_title: &str,
    relations: &[Relation],
    cli_container_id: Option<&str>,
    instance_id_filter: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<DocumentViewProjection, RepositoryError> {
    let container_id = resolve_container_id_from_sections(&dv.sections);
    if container_id.is_none() {
        let subset_ids: Vec<String> = dv
            .sections
            .iter()
            .filter_map(|s| {
                if let SectionSource::ContainerSubset { container_id, .. } = &s.source {
                    Some(container_id.clone())
                } else {
                    None
                }
            })
            .collect();
        if subset_ids.len() > 1 {
            diagnostics.push(format!(
                "[json-projection] view {} has multiple ContainerSubset sections with different container IDs; using first ({})",
                dv.id, subset_ids[0]
            ));
        }
    }

    let doc_preamble = dv
        .preamble
        .as_ref()
        .map(|p| substitute_vars_json_blanked(p, container_title, manifest));

    let mut sections = dv.sections.clone();
    sections.sort_by_key(|s| s.order);

    let mut projected_sections = Vec::new();
    for section in &sections {
        let projected = project_section_json(
            store,
            package,
            section,
            relations,
            cli_container_id,
            instance_id_filter,
            diagnostics,
        )?;
        projected_sections.push(projected);
    }

    Ok(DocumentViewProjection {
        schema: "https://srs.semanticops.com/schema/2.0/document-view-output.json".to_string(),
        document_view_id: dv.id.clone(),
        container_id,
        generated_at: chrono::Utc::now().to_rfc3339(),
        container_title: container_title.to_string(),
        preamble: doc_preamble,
        sections: projected_sections,
    })
}

fn resolve_container_id_from_sections(sections: &[DocumentSection]) -> Option<String> {
    sections.iter().find_map(|s| {
        if let SectionSource::ContainerSubset { container_id, .. } = &s.source {
            Some(container_id.clone())
        } else {
            None
        }
    })
}

fn substitute_vars_json_blanked(
    template: &str,
    container_title: &str,
    manifest: &crate::manifest::Manifest,
) -> String {
    let mut out = template.to_string();
    for level in 1..=6 {
        out = out.replace(&format!("{{{{heading-{level}}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-open}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-close}}}}"), "");
    }
    out = out.replace("{{container-title}}", container_title);
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    out = out.replace("{{date}}", &date);
    let namespace = manifest
        .extra
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    out = out.replace("{{container-id}}", namespace);
    out
}

fn project_section_json(
    store: &dyn RepositoryStore,
    package: &Package,
    section: &DocumentSection,
    relations: &[Relation],
    cli_container_id: Option<&str>,
    instance_id_filter: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<ProjectedSection, RepositoryError> {
    let mut records = resolve_section_instances(
        store,
        section,
        relations,
        cli_container_id,
        instance_id_filter,
        diagnostics,
    )?;

    if let Some(ordering) = &section.ordering {
        if let Some(field_id) = &ordering.field_id {
            // `SectionOrdering.field_id` is a Field UUID; the RFC-039 carrier
            // keys values by `Field.name` — bridge via the package.
            let field_name = package
                .resolve_field(field_id)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| field_id.clone());
            records.sort_by(|a, b| {
                let av = a.get_field_value_str(&field_name).unwrap_or("");
                let bv = b.get_field_value_str(&field_name).unwrap_or("");
                av.cmp(bv)
            });
            if matches!(ordering.direction, Some(SortDirection::Desc)) {
                records.reverse();
            }
        }
    } else if !matches!(&section.source, SectionSource::FixedInstances { .. }) {
        // Sort by precedes chain for any source that doesn't have authored ordering.
        // FixedInstances sections declare an explicit instance_ids order that must be
        // preserved — applying precedes-chain sorting would override the author's intent.
        // ContainerSubset, TypeQuery, and RelationQuery all benefit from precedes ordering.
        records = relation_graph::sort_by_precedes_chain(records, relations);
    }

    // RFC-008 typeFilter: applied after sort (same invariant as render_section).
    // Sort sees the full container; filter projects onto the sorted survivor set.
    // Tier-0 notes have no type and never match an explicit typeFilter.
    if let SectionSource::ContainerSubset {
        type_filter: Some(filter),
        ..
    } = &section.source
    {
        if !filter.is_empty() {
            records.retain(|inst| {
                let Some(r) = inst.as_record() else {
                    return false;
                };
                if let Some(rt) = package.resolve_type(&r.type_id, r.type_version) {
                    let key = format!("{}/{}", rt.namespace, rt.name);
                    filter.iter().any(|f| f == &key)
                } else {
                    false
                }
            });
        }
    }

    let mut projected_records = Vec::new();
    for instance in &records {
        match instance {
            LoadedInstance::Record(record) => {
                projected_records.push(project_record_json(
                    store,
                    package,
                    section,
                    record,
                    relations,
                    diagnostics,
                )?);
            }
            LoadedInstance::Note(note) => {
                // The document-view JSON output schema models typed records only;
                // Tier-0 notes are skipped from the projection with a warning (#510).
                diagnostics.push(format!(
                    "[section:{}] tier-0 note {} is not representable in the JSON projection; skipped",
                    section.section_id, note.instance_id
                ));
            }
        }
    }

    Ok(ProjectedSection {
        section_id: section.section_id.clone(),
        title: section.title.clone(),
        order: section.order,
        records: projected_records,
    })
}

fn project_record_json(
    store: &dyn RepositoryStore,
    package: &Package,
    section: &DocumentSection,
    record: &Record,
    relations: &[Relation],
    diagnostics: &mut Vec<String>,
) -> Result<ProjectedRecord, RepositoryError> {
    let rt = package
        .resolve_type(&record.type_id, record.type_version)
        .cloned();

    let record_heading = resolve_heading_field_id(section, rt.as_ref(), package)
        .as_deref()
        .and_then(|fid| package.resolve_field(fid))
        .and_then(|f| record.value_str(&f.name).map(|v| v.to_string()));

    let mut fields_to_render: Vec<ResolvedFieldRender> = Vec::new();
    let mut omit_empty = false;
    let mut record_preamble: Option<String> = None;

    let effective_view_id = resolve_effective_view_id(section, record, package);
    let use_view = if let Some(view_id) = effective_view_id {
        if let Some(view) = package.resolve_view(view_id) {
            let (satisfied, _cached_eff) =
                record_satisfies_view(package, view, rt.as_ref(), diagnostics);
            if satisfied {
                Some(view.clone())
            } else {
                diagnostics.push(format!(
                    "[view-dispatch] dispatched view {} for type {}/{} does not satisfy view; falling back to baseline",
                    view_id,
                    rt.as_ref().map(|t| t.namespace.as_str()).unwrap_or("?"),
                    rt.as_ref().map(|t| t.name.as_str()).unwrap_or("?"),
                ));
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if let Some(view) = use_view {
        if let Some(export_config) = &view.export_config {
            if let Some(preamble_tmpl) = &export_config.preamble {
                record_preamble = Some(substitute_vars_record_json(preamble_tmpl, record));
            }
            omit_empty = export_config.omit_empty_fields == Some(true);
            if let Some(order) = &export_config.field_order {
                fields_to_render = order
                    .iter()
                    .cloned()
                    .map(|field_id| ResolvedFieldRender {
                        name: package
                            .resolve_field(&field_id)
                            .map(|f| f.name.clone())
                            .unwrap_or_default(),
                        field_id,
                        required: false,
                    })
                    .collect();
            }
        }
        if fields_to_render.is_empty() {
            let mut field_views = view.field_views.clone();
            field_views.sort_by_key(|fv| fv.order);
            for fv in field_views {
                // `visible` is a rendering hint for text/markdown output — do not apply it
                // here. The JSON projection exports data; all fields must be included
                // regardless of their display visibility.
                fields_to_render.push(ResolvedFieldRender {
                    name: package
                        .resolve_field(&fv.field_id)
                        .map(|f| f.name.clone())
                        .unwrap_or_default(),
                    field_id: fv.field_id,
                    required: fv.required == Some(true),
                });
            }
        }
    } else if let Some(rt) = &rt {
        // Use effective_fields (resolves type-inheritance) to match the markdown path.
        match package.effective_fields(rt) {
            Ok(assignments) => {
                for fa in assignments {
                    fields_to_render.push(ResolvedFieldRender {
                        name: package
                            .resolve_field(&fa.field_id)
                            .map(|f| f.name.clone())
                            .unwrap_or_default(),
                        field_id: fa.field_id,
                        required: fa.required,
                    });
                }
            }
            Err(e) => {
                diagnostics.push(format!("ext:type-inheritance: {e}"));
            }
        }
    } else {
        for (name, _value) in record.field_values.iter() {
            fields_to_render.push(ResolvedFieldRender {
                field_id: String::new(),
                name: name.clone(),
                required: false,
            });
        }
    }

    // [R11]: keys are Field.name.
    let ordered_field_keys: Vec<String> = fields_to_render.iter().map(|f| f.name.clone()).collect();
    let mut fields_map = serde_json::Map::new();

    for field in &fields_to_render {
        if field.name.is_empty() {
            diagnostics.push(format!(
                "field id {} resolves to no Field definition; skipped in projection",
                field.field_id
            ));
            continue;
        }
        let field_value = record.value(&field.name);
        let field_def = if field.field_id.is_empty() {
            package.find_field_by_name(&field.name)
        } else {
            package.resolve_field(&field.field_id)
        };
        let field_type = field_def.map(|f| &f.field_type);
        // [R11]: a composite value is carried recursively under its own key,
        // exactly as in the instance — the projection inherits Change B's
        // recursion instead of restating it.
        let json_val = field_value.and_then(|v| project_field_value(v, field_type));

        if field.required && json_val.is_none() {
            diagnostics.push(format!(
                "[view-required] view {} record {} is missing required field {} for rendered view",
                effective_view_id.unwrap_or("<no-view-id>"),
                record.instance_id,
                field.name
            ));
        }

        match json_val {
            Some(v) => {
                fields_map.insert(field.name.clone(), v);
            }
            None => {
                if !omit_empty {
                    fields_map.insert(field.name.clone(), serde_json::Value::Null);
                }
            }
        }
    }

    let projected_relations =
        project_relations_json(store, section, record, relations, package, diagnostics)?;

    Ok(ProjectedRecord {
        instance_id: record.instance_id.clone(),
        type_id: record.type_id.clone(),
        type_namespace: record.type_namespace.clone(),
        type_name: record.type_name.clone(),
        record_heading,
        preamble: record_preamble,
        fields: serde_json::Value::Object(fields_map),
        ordered_field_keys,
        relations: projected_relations,
    })
}

fn substitute_vars_record_json(template: &str, record: &Record) -> String {
    let mut out = template.to_string();
    for level in 1..=6 {
        out = out.replace(&format!("{{{{heading-{level}}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-open}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-close}}}}"), "");
    }
    out = out.replace("{{instance-id}}", &record.instance_id);
    out = out.replace("{{type-name}}", &record.type_name);
    out = out.replace("{{type-namespace}}", &record.type_namespace);
    out
}

/// Project one stored value for the JSON projection. Scalars and lists pass
/// through with date-format coercion; an inline-composite value (object, or
/// array of objects) passes through **recursively unchanged** — the instance
/// shape is the projection shape ([R11]). An empty string projects as absent
/// (RFC-001 Step 2 rendering-presence, [R5a]).
fn project_field_value(
    value: &serde_json::Value,
    field_type: Option<&FieldType>,
) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) if s.is_empty() => None,
        _ => Some(coerce_json_value(value, field_type)),
    }
}

fn coerce_json_value(raw: &serde_json::Value, field_type: Option<&FieldType>) -> serde_json::Value {
    match field_type.map(|ft| ft.datatype) {
        Some(Datatype::Number) | Some(Datatype::Integer) => {
            if let Some(n) = raw.as_str().and_then(|s| s.parse::<i64>().ok()) {
                return json!(n);
            }
            if let Some(f) = raw.as_str().and_then(|s| s.parse::<f64>().ok()) {
                return json!(f);
            }
        }
        Some(Datatype::Boolean) => match raw.as_str() {
            Some("true") => return json!(true),
            Some("false") => return json!(false),
            _ => {}
        },
        _ => {}
    }
    raw.clone()
}

fn resolve_active_theme(
    dv: &DocumentView,
    package: &Package,
    theme_variant: Option<&str>,
    format: &str,
    diagnostics: &mut Vec<String>,
) -> Option<Theme> {
    let theme_ref = if let Some(variant_name) = theme_variant {
        match dv
            .theme_variants
            .as_ref()
            .and_then(|variants| variants.iter().find(|variant| variant.name == variant_name))
        {
            Some(variant) => Some(&variant.theme_ref),
            None => {
                diagnostics.push(format!(
                    "[theme-variant] view {} theme variant '{}' not found; falling back to themeRef",
                    dv.id, variant_name
                ));
                dv.theme_ref.as_ref()
            }
        }
    } else {
        dv.theme_ref.as_ref()
    }?;

    let theme = match theme_ref.mode {
        ThemeMode::Bundled => {
            let Some(theme_id) = theme_ref.theme_id.as_deref() else {
                diagnostics.push(format!(
                    "[T-5] view {} bundled theme reference is missing themeId",
                    dv.id
                ));
                return None;
            };
            match package.resolve_theme(theme_id) {
                Some(theme) => theme.clone(),
                None => {
                    diagnostics.push(format!(
                        "[T-5] view {} bundled theme '{}' was not found in the package",
                        dv.id, theme_id
                    ));
                    return None;
                }
            }
        }
        ThemeMode::Local | ThemeMode::Remote => {
            diagnostics.push(format!(
                "[theme] view {} theme reference mode {:?} is not supported in this release",
                dv.id, theme_ref.mode
            ));
            return None;
        }
    };

    if theme.targets.iter().any(|target| target == format) {
        return Some(theme);
    }

    // Default theme doesn't target this format. If no explicit variant was requested,
    // auto-select the first themeVariant whose resolved theme does target this format.
    if theme_variant.is_none() {
        if let Some(variants) = dv.theme_variants.as_ref() {
            let matches: Vec<(String, Theme)> = variants
                .iter()
                .filter_map(|variant| {
                    let tid = variant.theme_ref.theme_id.as_deref()?;
                    let t = package.resolve_theme(tid)?.clone();
                    if t.targets.iter().any(|tgt| tgt == format) {
                        Some((variant.name.clone(), t))
                    } else {
                        None
                    }
                })
                .collect();

            if matches.len() > 1 {
                diagnostics.push(format!(
                    "[T-3] view {}: multiple themeVariants target format {}; using first match '{}'",
                    dv.id, format, matches[0].0
                ));
            }
            if let Some((_, matched_theme)) = matches.into_iter().next() {
                return Some(matched_theme);
            }
        }
    }

    diagnostics.push(format!(
        "[T-2] view {} theme {} does not target format {}; skipping theme",
        dv.id, theme.id, format
    ));
    None
}

fn select_section_wrapper<'a>(theme: &'a Theme, section_id: &str) -> Option<&'a str> {
    let element_templates = theme.element_templates.as_ref()?;
    if let Some(overrides) = element_templates.section_wrapper_overrides.as_ref() {
        if let Some(override_template) = overrides
            .iter()
            .find(|override_entry| override_entry.section_id == section_id)
        {
            return Some(override_template.template.as_str());
        }
    }
    element_templates.section_wrapper.as_deref()
}

fn select_record_wrapper<'a>(theme: &'a Theme, type_id: &str) -> Option<&'a str> {
    let element_templates = theme.element_templates.as_ref()?;
    if let Some(overrides) = element_templates.record_wrapper_overrides.as_ref() {
        if let Some(override_template) = overrides
            .iter()
            .find(|override_entry| override_entry.type_id == type_id)
        {
            return Some(override_template.template.as_str());
        }
    }
    element_templates.record_wrapper.as_deref()
}

fn apply_wrapper(
    template: &str,
    content: &str,
    vars: &[(&str, &str)],
    theme: Option<&Theme>,
) -> String {
    let mut out = template.to_string();
    for level in 1..=6 {
        out = out.replace(&format!("{{{{heading-{level}}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-open}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-close}}}}"), "");
    }
    if let Some(theme) = theme {
        out = replace_asset_placeholders(&out, theme);
    }
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    out.replace("{{content}}", content)
}

fn replace_asset_placeholders(template: &str, theme: &Theme) -> String {
    let mut out = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start) = remaining.find("{{asset:") {
        out.push_str(&remaining[..start]);
        let after = &remaining[start + "{{asset:".len()..];
        let Some(end) = after.find("}}") else {
            out.push_str(&remaining[start..]);
            return out;
        };
        let name = &after[..end];
        out.push_str(resolve_asset(theme, name));
        remaining = &after[end + 2..];
    }

    out.push_str(remaining);
    out
}

fn resolve_asset<'a>(theme: &'a Theme, name: &str) -> &'a str {
    let Some(assets) = theme.assets.as_ref() else {
        return "";
    };
    let Some(asset) = assets.get(name) else {
        return "";
    };
    match asset.mode {
        AssetMode::Inline => asset.data.as_deref().unwrap_or(""),
        AssetMode::Local | AssetMode::Remote => "",
    }
}

/// RFC-037 — the value carried by a field row.
///
/// `[FR-037-5]`: cardinality, not element count, selects the form. A sequence
/// renders as a block list even when it holds exactly one entry, so the
/// rendered shape stays a property of the Type rather than of instance data.
#[derive(Debug, Clone, PartialEq)]
enum RowValue {
    /// A single value, emitted verbatim after the label (`[FR-037-3]`).
    Scalar(String),
    /// An ordered sequence, emitted as a per-format block list (`[FR-037-5]`).
    Entries(Vec<String>),
    /// The `(empty)` placeholder. Always scalar form regardless of the field's
    /// cardinality, and in `html` the value element additionally carries
    /// `srs-empty-value` (`[FR-037-11]`).
    Placeholder,
}

impl RowValue {
    /// The value substituted into a Theme's `{{field-value}}` template variable.
    ///
    /// RFC-037 governs the emitted row, not the `ext:themes-l1` variable table,
    /// which defines no form for a sequence. Entries are joined by newline so no
    /// punctuation is invented for them.
    fn template_value(&self) -> String {
        match self {
            RowValue::Scalar(value) => value.clone(),
            RowValue::Placeholder => EMPTY_PLACEHOLDER.to_string(),
            RowValue::Entries(entries) => entries.join("\n"),
        }
    }
}

/// RFC-037 `[FR-037-12]` — what the row's identity CSS class is derived from.
///
/// A field row's identity is `Field.name` and never `FieldAssignment.displayLabel`,
/// which is rendering-only and view-owned (RFC-015). A relation row has no
/// `Field.name`, so it carries `srs-relationtype-*` in place of `srs-fieldname-*`.
#[derive(Debug, Clone, Copy)]
enum RowIdentity<'a> {
    FieldName(&'a str),
    RelationTypeKey(&'a str),
}

impl RowIdentity<'_> {
    fn css_class(&self) -> String {
        match self {
            RowIdentity::FieldName(name) => {
                format!("srs-fieldname-{}", normalise_css_class(name))
            }
            RowIdentity::RelationTypeKey(key) => {
                format!("srs-relationtype-{}", normalise_css_class(key))
            }
        }
    }
}

/// The separator emitted after a field row, per `[FR-037-7]`.
///
/// In the text formats this is a blank line, and it is not cosmetic: in
/// CommonMark two unseparated rows are a single soft-wrapped paragraph, and a
/// row following a block list without a blank line is a lazy continuation that
/// disappears into the list's final item. In `html` the `div` boundary is the
/// separation, so no separator element is inserted.
fn row_separator(format: &str) -> &'static str {
    match format {
        "html" => "\n",
        _ => "\n\n",
    }
}

/// Indent an entry's continuation lines to its content column, per `[FR-037-8]`.
///
/// Two spaces — the width of the `- ` marker — and never more: at four spaces
/// CommonMark reads the continuation as an indented code block. Blank lines stay
/// genuinely blank so the item becomes a loose list item rather than gaining
/// trailing whitespace; the item is not terminated by them.
fn indent_entry_continuation(value: &str) -> String {
    let mut lines = value.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut out = first.to_string();
    for line in lines {
        out.push('\n');
        if !line.is_empty() {
            out.push_str("  ");
            out.push_str(line);
        }
    }
    out
}

/// Attach an entry's subsequent blocks with AsciiDoc list continuations, per
/// `[FR-037-8]`. Indentation is not normative in `adoc`; a `+` line is what
/// actually attaches a following block to the list item.
fn adoc_entry_continuation(value: &str) -> String {
    let mut out = String::new();
    let mut blank_run = false;
    for (idx, line) in value.lines().enumerate() {
        if line.is_empty() {
            blank_run = true;
            continue;
        }
        if idx > 0 {
            if blank_run {
                out.push_str("\n+\n");
            } else {
                out.push('\n');
            }
        }
        out.push_str(line);
        blank_run = false;
    }
    out
}

/// RFC-037 Changes A, A1, B and B1 — the normative emitted form of a field row.
///
/// This is the single row primitive: field rows and relation rows both route
/// through it, which is what satisfies RFC-027 Change C rule 3's requirement
/// that a relation row use "the same label/value markup that implementation
/// emits for a field row in that format" by construction rather than by
/// convention (`[FR-037-15]`).
///
/// Label and value are emitted verbatim in the text formats and HTML-escaped in
/// `html` (`[FR-037-16]`); the baseline performs no markup conversion
/// (`[FR-037-17]`).
fn format_field_row(
    format: &str,
    identity: RowIdentity<'_>,
    label: &str,
    value: &RowValue,
) -> String {
    match format {
        "html" => format_field_row_html(identity, label, value),
        "markdown" => format_field_row_text(format, label, value, "**", "- "),
        "adoc" => format_field_row_text(format, label, value, "*", "* "),
        _ => format_field_row_text(format, label, value, "", "- "),
    }
}

/// The `markdown`, `adoc` and `text` row forms. `emphasis` wraps the label —
/// empty for `text`, which carries no bold requirement because the Heading
/// Hierarchy table's preamble scopes it to `markdown`, `html` and `adoc`, and
/// plain text has no portable emphasis convention.
fn format_field_row_text(
    format: &str,
    label: &str,
    value: &RowValue,
    emphasis: &str,
    marker: &str,
) -> String {
    let label = format!("{emphasis}{label}{emphasis}");
    match value {
        RowValue::Scalar(v) => format!("{label}: {v}"),
        RowValue::Placeholder => format!("{label}: {EMPTY_PLACEHOLDER}"),
        RowValue::Entries(entries) => {
            // `[FR-037-5]`: the label occupies its own line and keeps its
            // trailing colon; the list begins on the next line with no blank
            // line between them.
            let mut out = format!("{label}:");
            for entry in entries {
                let body = if format == "adoc" {
                    adoc_entry_continuation(entry)
                } else {
                    indent_entry_continuation(entry)
                };
                out.push('\n');
                out.push_str(marker);
                out.push_str(&body);
            }
            out
        }
    }
}

/// The `html` row structure of Changes A1 and B1.
///
/// Normative here are the element names and their nesting, their order, the
/// literal colon between `</strong>` and the value element, and the `srs-`
/// prefixed class names. Inter-element whitespace is not normative
/// (`[FR-037-4]`), and the single-line form is emitted so conformance fixtures
/// have a canonical serialisation.
fn format_field_row_html(identity: RowIdentity<'_>, label: &str, value: &RowValue) -> String {
    let id_class = identity.css_class();
    let label = html_escape(label);
    let open = format!(
        "<div class=\"srs-field {id_class}\"><strong class=\"{LABEL_CLASSES}\">{label}</strong>:"
    );
    match value {
        RowValue::Scalar(v) => {
            format!("{open} <span class=\"{VALUE_CLASSES}\">{v}</span></div>")
        }
        RowValue::Placeholder => {
            format!(
                "{open} <span class=\"{VALUE_CLASSES} srs-empty-value\">{EMPTY_PLACEHOLDER}</span></div>"
            )
        }
        RowValue::Entries(entries) => {
            // `[FR-037-6]`: the same enclosing `div` carries the classes `[T-8]`
            // requires of every field row; the `ul` itself carries none.
            let mut out = format!("{open}<ul>");
            for entry in entries {
                out.push_str(&format!("<li class=\"{VALUE_CLASSES}\">{entry}</li>"));
            }
            out.push_str("</ul></div>");
            out
        }
    }
}

/// The portable placeholder of `[FR-037-11]`.
const EMPTY_PLACEHOLDER: &str = "(empty)";

/// `[FR-037-14]` — the #242 cutover fired: only the prefixed names are
/// emitted; the unprefixed compatibility aliases are gone.
const LABEL_CLASSES: &str = "srs-field-label";
const VALUE_CLASSES: &str = "srs-field-value";

fn normalise_css_class(value: &str) -> String {
    let s = value.to_lowercase();
    let s: String = s
        .chars()
        .map(|c| {
            if c == '_' || c == ' ' || c == '.' {
                '-'
            } else {
                c
            }
        })
        .collect();
    let s: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    s.trim_matches('-').to_string()
}

fn css_classes_for_record(
    record: &srs_core::types::record::Record,
    ctx: &RenderContext<'_>,
) -> String {
    let type_class = format!(
        "srs-type-{}-{}",
        normalise_css_class(&record.type_namespace),
        normalise_css_class(&record.type_name)
    );
    let mut classes = format!("srs-record {type_class}");

    if let Some(theme) = ctx.active_theme.as_ref() {
        if let Some(field_ids) = &theme.css_class_fields {
            for field_id in field_ids {
                if let Some(field) = ctx.package.resolve_field(field_id) {
                    // `[T-9]` / ext:themes-l1: only an effective-single, prose
                    // string field contributes a class. Cardinality-only since
                    // the #242 cutover (Change-I condition 4).
                    if !field.field_type.is_theme_css_class_eligible() {
                        continue;
                    }
                    if let Some(value) = record.value(&field.name) {
                        if let Some(raw) = value.as_str() {
                            classes.push(' ');
                            classes.push_str(&format!(
                                "srs-field-{}-{}",
                                normalise_css_class(&field.name),
                                normalise_css_class(raw)
                            ));
                        }
                    }
                }
            }
        }
    }

    classes
}

fn resolve_container_title(
    store: &dyn RepositoryStore,
    dv: &DocumentView,
    manifest: &crate::manifest::Manifest,
    container_id: Option<&str>,
) -> String {
    // When a specific container_id was requested, load it directly (catalog-backed;
    // embed-only RFC-013 roots resolve here too via the shared service helper).
    // RFC-038 Change K retires `manifest.containerIndex`.
    if let Some(cid) = container_id {
        if let Ok(container) = crate::container_service::get_container(store, cid) {
            if !container.title.is_empty() {
                return container.title;
            }
        }
    }

    // Fallback: first container matching the document view's containerType.
    if let Some(container_type) = &dv.container_type {
        if let Ok(summaries) = crate::container_service::list_containers(
            store,
            &crate::container_service::ContainerListFilter {
                container_type: Some(container_type.clone()),
                ..Default::default()
            },
        ) {
            if let Some(summary) = summaries.into_iter().find(|s| !s.title.is_empty()) {
                return summary.title;
            }
        }
    }

    if let Some(title) = manifest
        .extra
        .get("meta")
        .and_then(|m| m.get("title"))
        .and_then(|v| v.as_str())
    {
        if !title.is_empty() {
            return title.to_string();
        }
    }

    if let Some(title) = manifest.extra.get("title").and_then(|v| v.as_str()) {
        if !title.is_empty() {
            return title.to_string();
        }
    }

    manifest
        .extra
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("SRS")
        .to_string()
}

/// Check whether a record's type satisfies a View for rendering dispatch purposes.
///
/// Binding resolution (RFC-001 field-presence model):
/// 1. If `view.compatible_types` is set, the record's type must be listed as
///    `"namespace/name"` — this is the first behavioural use of `compatible_types`;
///    it acts as a constraint only within document-section dispatch, not globally.
/// 2. Otherwise, fall back to field-presence: the record must contain every
///    visible `field_views` field that is marked `required`.
///    If no field_views are required, any record satisfies the view.
///
/// Returns `(satisfied, effective_field_ids)`. When satisfied via the field-presence
/// path, `effective_field_ids` is populated so the caller can reuse it and avoid a
/// second `effective_fields` call for the fallback render path.
fn record_satisfies_view(
    package: &Package,
    view: &srs_core::types::view::View,
    rt: Option<&srs_core::types::record_type::RecordType>,
    diagnostics: &mut Vec<String>,
) -> (bool, Option<HashSet<String>>) {
    if let Some(compatible) = &view.compatible_types {
        let type_key = rt.map(|t| format!("{}/{}", t.namespace, t.name));
        let satisfied = type_key
            .as_deref()
            .is_some_and(|k| compatible.iter().any(|c| c == k));
        return (satisfied, None);
    }
    // Field-presence fallback: every visible required field-view field must exist.
    let required_field_ids: Vec<&str> = view
        .field_views
        .iter()
        .filter(|fv| fv.visible != Some(false) && fv.required == Some(true))
        .map(|fv| fv.field_id.as_str())
        .collect();
    if required_field_ids.is_empty() {
        return (true, None);
    }
    let effective_result = rt.map(|t| package.effective_fields(t));
    let effective_ids: HashSet<String> = match effective_result {
        Some(Ok(fields)) => fields.iter().map(|fa| fa.field_id.clone()).collect(),
        Some(Err(e)) => {
            diagnostics.push(format!(
                "[view-dispatch] ext:type-inheritance error while checking view compatibility: {e}"
            ));
            HashSet::new()
        }
        None => HashSet::new(),
    };
    let satisfied = required_field_ids
        .iter()
        .all(|id| effective_ids.contains(*id));
    (satisfied, Some(effective_ids))
}

/// RFC-008: resolve the effective L1 view UUID for a record in a section.
///
/// Consults `section.type_dispatch` first (keyed by resolved `namespace/name`), then falls
/// back to `section.render_view_id`. Uses the package-resolved type identity so that stale
/// denormalized `type_namespace`/`type_name` hints on the record cannot produce wrong dispatch.
fn resolve_effective_view_id<'a>(
    section: &'a DocumentSection,
    record: &srs_core::types::record::Record,
    package: &Package,
) -> Option<&'a str> {
    if let Some(dispatch) = &section.type_dispatch {
        let key = package
            .resolve_type(&record.type_id, record.type_version)
            .map(|rt| format!("{}/{}", rt.namespace, rt.name));
        if let Some(k) = key {
            if let Some(view_id) = dispatch.get(&k) {
                return Some(view_id.as_str());
            }
        }
    }
    section.render_view_id.as_deref()
}

fn render_section(
    store: &dyn RepositoryStore,
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    relations: &[Relation],
    cli_container_id: Option<&str>,
    instance_id_filter: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let mut records = resolve_section_instances(
        store,
        section,
        relations,
        cli_container_id,
        instance_id_filter,
        diagnostics,
    )?;

    // Apply explicit field-based ordering first if declared.
    if let Some(ordering) = &section.ordering {
        if let Some(field_id) = &ordering.field_id {
            // `SectionOrdering.field_id` is a Field UUID; the RFC-039 carrier
            // keys values by `Field.name` — bridge via the package.
            let field_name = ctx
                .package
                .resolve_field(field_id)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| field_id.clone());
            records.sort_by(|a, b| {
                let av = a.get_field_value_str(&field_name).unwrap_or("");
                let bv = b.get_field_value_str(&field_name).unwrap_or("");
                av.cmp(bv)
            });
            if matches!(ordering.direction, Some(SortDirection::Desc)) {
                records.reverse();
            }
        }
    } else if !matches!(&section.source, SectionSource::FixedInstances { .. }) {
        // Sort by precedes chain for any source that doesn't have authored ordering.
        // FixedInstances sections declare an explicit instance_ids order that must be
        // preserved — applying precedes-chain sorting would override the author's intent.
        // ContainerSubset, TypeQuery, and RelationQuery all benefit from precedes ordering.
        records = relation_graph::sort_by_precedes_chain(records, relations);
    }

    // RFC-008 typeFilter: restrict container-subset members to matching types.
    // Applied AFTER sort so sort_by_precedes_chain sees the full container (including
    // cross-type edges). The filter is a projection step: full ordering established first,
    // non-matching types dropped while preserving the relative order of survivors.
    // Tier-0 notes have no type and never match an explicit typeFilter.
    if let SectionSource::ContainerSubset {
        type_filter: Some(filter),
        ..
    } = &section.source
    {
        if !filter.is_empty() {
            records.retain(|inst| {
                let Some(r) = inst.as_record() else {
                    return false;
                };
                if let Some(rt) = ctx.package.resolve_type(&r.type_id, r.type_version) {
                    let key = format!("{}/{}", rt.namespace, rt.name);
                    filter.iter().any(|f| f == &key)
                } else {
                    false
                }
            });
        }
    }

    if records.is_empty() && section.required != Some(true) {
        return Ok(String::new());
    }

    let mut out = String::new();
    if let Some(title) = &section.title {
        out.push_str(&format_heading(
            depth(2, ctx.depth_offset),
            ctx.format,
            title,
        ));
    }
    if let Some(description) = &section.description {
        out.push_str(description);
        out.push_str("\n\n");
    }

    if records.is_empty() && section.required == Some(true) {
        out.push_str("No records.\n\n");
        return Ok(out);
    }

    let record_heading_level = depth(2, ctx.depth_offset) + 1;
    for instance in &records {
        match instance {
            LoadedInstance::Record(record) => {
                out.push_str(&render_record_at_level(
                    store,
                    ctx,
                    section,
                    record,
                    record_heading_level,
                    relations,
                    diagnostics,
                )?);
            }
            LoadedInstance::Note(note) => {
                // Tier-0 note members render through their note shape: title as the
                // heading, free-text section content as body text (#510).
                out.push_str(&render_note_at_level(ctx, note, record_heading_level));
            }
        }
    }

    let section_wrapper = ctx
        .active_theme
        .as_ref()
        .and_then(|t| select_section_wrapper(t, &section.section_id));
    if let Some(wrapper) = section_wrapper {
        let section_title = section.title.as_deref().unwrap_or("");
        out = apply_wrapper(
            wrapper,
            &out,
            &[
                ("section-title", section_title),
                ("section-id", section.section_id.as_str()),
            ],
            ctx.active_theme.as_ref(),
        );
    } else if ctx.format == "html" {
        let css_id = normalise_css_class(&section.section_id);
        out = format!("<div class=\"srs-section srs-section-{css_id}\">{out}</div>\n");
    }
    Ok(out)
}

/// Resolve container members, degrading per-section on a dangling containerId:
/// a container that does not exist yields an empty member list plus a warning
/// diagnostic instead of failing the whole render (#509). All other errors
/// still propagate.
fn list_members_degraded(
    store: &dyn RepositoryStore,
    container_id: &str,
    section_id: &str,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<String>, RepositoryError> {
    match list_members(store, container_id) {
        Ok(members) => Ok(members),
        Err(RepositoryError::ContainerNotFound { container_id }) => {
            diagnostics.push(format!(
                "[section:{section_id}] container not found: {container_id}; rendering section as empty"
            ));
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

fn resolve_section_instances(
    store: &dyn RepositoryStore,
    section: &DocumentSection,
    relations: &[srs_core::types::relation::Relation],
    cli_container_id: Option<&str>,
    instance_id_filter: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<LoadedInstance>, RepositoryError> {
    match &section.source {
        SectionSource::FixedInstances { instance_ids } => {
            let mut records = Vec::new();
            for id in instance_ids {
                match get_instance_by_id(store, id)? {
                    Some(LoadedInstance::Note(_)) => {
                        diagnostics.push(format!(
                            "[section:{}] FixedInstances: skipping Tier-0 note {}; notes are not rendered in document-view sections",
                            section.section_id, id
                        ));
                    }
                    Some(instance) => records.push(instance),
                    None => {}
                }
            }
            Ok(records)
        }
        SectionSource::TypeQuery {
            semantic_object_type,
            container_ids,
            lifecycle_state,
            lifecycle_states,
            exclude_lifecycle_states,
            container_scope,
        } => {
            let Some((namespace, name)) = semantic_object_type.split_once('/') else {
                diagnostics.push(format!(
                    "[N] TypeQuery semanticObjectType '{}' has no namespace separator '/' — expected 'namespace/name' format",
                    semantic_object_type
                ));
                return Ok(Vec::new());
            };
            let mut records = list_records_by_type(store, namespace, name)?;

            // ── Container scoping (RFC-011 [N+27]) ───────────────────────────────────
            let scope = container_scope
                .as_ref()
                .unwrap_or(&ContainerScope::Explicit);
            match scope {
                ContainerScope::Repository => {
                    // Ignore all container filtering — return all records of the type.
                }
                ContainerScope::Subtree => {
                    // v1: subtree traversal requires RFC-N container hierarchy.
                    // Fall back to explicit scope with a diagnostic.
                    diagnostics.push(
                        "[N+27] containerScope 'subtree' is not yet fully supported (requires RFC-N); \
                         falling back to explicit scope".to_string(),
                    );
                    // cli_container_id takes precedence, matching Explicit scope behaviour.
                    let effective_ids: Option<Vec<String>> = cli_container_id
                        .map(|id| vec![id.to_string()])
                        .or_else(|| container_ids.clone());
                    if let Some(ids) = effective_ids {
                        let mut member_set = HashSet::new();
                        for id in &ids {
                            for m in
                                list_members_degraded(store, id, &section.section_id, diagnostics)?
                            {
                                member_set.insert(m);
                            }
                        }
                        records.retain(|r| member_set.contains(&r.instance_id));
                    } else {
                        diagnostics.push(
                            "[N+27] containerScope 'subtree' with no containerIds — returning empty result".to_string(),
                        );
                        // Return early: the [N+27] message already names the reason;
                        // the generic zero-records diagnostic would be noise.
                        return Ok(Vec::new());
                    }
                }
                ContainerScope::Explicit => {
                    // CLI --container takes precedence; fall back to container_ids declared in the view.
                    let effective_ids: Option<Vec<String>> = cli_container_id
                        .map(|id| vec![id.to_string()])
                        .or_else(|| container_ids.clone());
                    if let Some(ids) = effective_ids {
                        let mut member_set = HashSet::new();
                        for id in &ids {
                            for m in
                                list_members_degraded(store, id, &section.section_id, diagnostics)?
                            {
                                member_set.insert(m);
                            }
                        }
                        records.retain(|r| member_set.contains(&r.instance_id));
                    }
                }
            }

            // ── Lifecycle filtering (RFC-011 [N+25], [N+26]) ─────────────────────────
            // lifecycleStates takes precedence over the back-compat singular lifecycle_state.
            let include_states = lifecycle_states.as_ref().filter(|v| !v.is_empty());
            let has_include = include_states.is_some();
            let backcompat_state = if !has_include {
                lifecycle_state.as_deref()
            } else {
                None
            };

            if has_include || backcompat_state.is_some() {
                // Inclusion filter: only records whose lifecycle_state matches any listed value.
                // Records with no lifecycle_state are excluded.
                records.retain(|r| {
                    r.lifecycle_state
                        .as_deref()
                        .map(|s| {
                            if let Some(inc) = include_states {
                                inc.iter().any(|v| v == s)
                            } else {
                                backcompat_state == Some(s)
                            }
                        })
                        .unwrap_or(false)
                });
            }

            if let Some(exclude) = exclude_lifecycle_states {
                if !exclude.is_empty() {
                    // Exclusion filter applied after inclusion. Records with no lifecycle_state
                    // are NOT excluded by this step.
                    records.retain(|r| {
                        r.lifecycle_state
                            .as_deref()
                            .map(|s| !exclude.iter().any(|ex| ex == s))
                            .unwrap_or(true)
                    });
                }
            }

            let result: Vec<LoadedInstance> =
                records.into_iter().map(LoadedInstance::Record).collect();
            if result.is_empty() {
                diagnostics.push(format!(
                    "[section:{}] type-query '{}' matched 0 records",
                    section.section_id, semantic_object_type
                ));
            }
            Ok(result)
        }
        SectionSource::RelationQuery {
            from_instance_id,
            relation_type,
            direction,
        } => {
            let mut ids = Vec::new();
            let dir = direction.as_ref().unwrap_or(&RelationDirection::Forward);
            for relation in relations {
                if relation.relation_type != *relation_type {
                    continue;
                }
                match dir {
                    RelationDirection::Forward => {
                        if relation.source_instance_id == *from_instance_id {
                            ids.push(relation.target_instance_id.clone());
                        }
                    }
                    RelationDirection::Inverse => {
                        if relation.target_instance_id == *from_instance_id {
                            ids.push(relation.source_instance_id.clone());
                        }
                    }
                }
            }
            let mut records = Vec::new();
            for id in ids {
                match get_instance_by_id(store, &id)? {
                    Some(LoadedInstance::Note(_)) => {
                        diagnostics.push(format!(
                            "[section:{}] RelationQuery: skipping Tier-0 note {}; notes are not rendered in document-view sections",
                            section.section_id, id
                        ));
                    }
                    Some(instance) => records.push(instance),
                    None => {}
                }
            }
            Ok(records)
        }
        SectionSource::ContainerSubset {
            container_id,
            container_type: _,
            type_filter: _,
        } => {
            // CLI --container overrides the view-declared container_id, allowing one
            // ContainerSubset document-view to render any guide by switching at render time.
            let effective_id = cli_container_id.unwrap_or(container_id.as_str());
            let members =
                list_members_degraded(store, effective_id, &section.section_id, diagnostics)?;
            let mut records = Vec::new();
            for id in members {
                if let Some(instance) = get_instance_by_id(store, &id)? {
                    records.push(instance);
                }
            }
            if let Some(filter_id) = instance_id_filter {
                records.retain(|r| r.instance_id() == filter_id);
            }
            Ok(records)
        }
    }
}

/// Render a Tier-0 note as a document-view entry: the note title becomes the
/// heading and each note section's free-text content is emitted as body text
/// (with the section label, when present, as a sub-heading). Notes are legal
/// container roots/members (RFC-013), so ContainerSubset sections must be able
/// to render them (#510).
fn render_note_at_level(
    ctx: &RenderContext<'_>,
    note: &srs_core::types::note::Note,
    heading_level: u32,
) -> String {
    let mut out = String::new();

    if let Some(title) = note.title.as_deref().filter(|t| !t.is_empty()) {
        out.push_str(&format_heading(heading_level, ctx.format, title));
    }

    for note_section in &note.sections {
        if let Some(label) = note_section.label.as_deref().filter(|l| !l.is_empty()) {
            out.push_str(&format_heading(heading_level + 1, ctx.format, label));
        }
        if note_section.content.is_empty() {
            continue;
        }
        match ctx.format {
            "html" => {
                let name_class = normalise_css_class(&note_section.name);
                out.push_str(&format!(
                    "<div class=\"srs-note-section srs-note-section-{name_class}\">{}</div>\n",
                    html_escape(&note_section.content)
                ));
            }
            _ => {
                out.push_str(&note_section.content);
                out.push_str("\n\n");
            }
        }
    }

    if ctx.format == "html" {
        out = format!("<div class=\"srs-record srs-note\">{out}</div>\n");
    }

    out.push('\n');
    out
}

fn render_record_at_level(
    store: &dyn RepositoryStore,
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    record: &Record,
    heading_level: u32,
    relations: &[Relation],
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let rt = ctx
        .package
        .resolve_type(&record.type_id, record.type_version)
        .cloned();

    let structured = section.title_field_id.is_some();
    let mut out = String::new();
    let mut record_heading_value = String::new();

    // The field that actually became the record heading, which is not always the
    // one the section declared: `[N+1]` can make an authored `titleFieldId`
    // ineligible. The body-skip below must key on *this*, not on
    // `section.title_field_id` — skip from the body iff emitted as the heading.
    // Keying the skip on the declaration while the heading is eligibility-filtered
    // drops the field from both, losing its value from the output entirely.
    let mut heading_field_id: Option<String> = None;

    if let Some(title_field_id) = resolve_heading_field_id(section, rt.as_ref(), ctx.package) {
        if let Some(title) = ctx
            .package
            .resolve_field(&title_field_id)
            .and_then(|f| record.value_str(&f.name))
        {
            record_heading_value = title.to_string();
            out.push_str(&format_heading(heading_level, ctx.format, title));
        }
        // Set even when the record carries no value for it, preserving the
        // pre-existing structured-mode behaviour for an eligible titleFieldId.
        heading_field_id = Some(title_field_id);
    }

    let mut fields_to_render: Vec<ResolvedFieldRender> = Vec::new();
    let mut display_labels = std::collections::HashMap::new();
    let mut omit_empty = false;

    let effective_view_id = resolve_effective_view_id(section, record, ctx.package);
    let use_view = if let Some(view_id) = effective_view_id {
        if let Some(view) = ctx.package.resolve_view(view_id) {
            let (satisfied, _cached_eff) =
                record_satisfies_view(ctx.package, view, rt.as_ref(), diagnostics);
            if satisfied {
                Some(view.clone())
            } else {
                diagnostics.push(format!(
                    "[view-dispatch] dispatched view {} for type {}/{} does not satisfy view; falling back to baseline",
                    view_id,
                    rt.as_ref().map(|t| t.namespace.as_str()).unwrap_or("?"),
                    rt.as_ref().map(|t| t.name.as_str()).unwrap_or("?"),
                ));
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // `[FR-037-11]` inherits RFC-001's exclusion of `DocumentSection.emptyBehavior`
    // from the L1 View rendering path, which is the path a resolved View drives.
    let l1_view_path = use_view.is_some();

    if let Some(view) = use_view.as_ref() {
        if let Some(export_config) = &view.export_config {
            if let Some(preamble) = &export_config.preamble {
                out.push_str(&substitute_vars(preamble, ctx, Some(record), true));
                out.push('\n');
            }
            omit_empty = export_config.omit_empty_fields == Some(true);
            if let Some(order) = &export_config.field_order {
                fields_to_render = order
                    .iter()
                    .cloned()
                    .map(|field_id| ResolvedFieldRender {
                        name: ctx
                            .package
                            .resolve_field(&field_id)
                            .map(|f| f.name.clone())
                            .unwrap_or_default(),
                        field_id,
                        required: false,
                    })
                    .collect();
            }
        }
        // Always collect display labels from field_views, regardless of field_order.
        let mut field_views = view.field_views.clone();
        field_views.sort_by_key(|fv| fv.order);
        for fv in &field_views {
            if let Some(label) = &fv.display_label {
                display_labels.insert(fv.field_id.clone(), label.clone());
            }
        }
        if fields_to_render.is_empty() {
            for fv in field_views {
                if fv.visible == Some(false) {
                    continue;
                }
                fields_to_render.push(ResolvedFieldRender {
                    name: ctx
                        .package
                        .resolve_field(&fv.field_id)
                        .map(|f| f.name.clone())
                        .unwrap_or_default(),
                    field_id: fv.field_id,
                    required: fv.required == Some(true),
                });
            }
        }
    } else if let Some(rt) = &rt {
        match ctx.package.effective_fields(rt) {
            Ok(assignments) => {
                for fa in assignments {
                    if let Some(label) = fa.display_label {
                        display_labels.insert(fa.field_id.clone(), label);
                    }
                    fields_to_render.push(ResolvedFieldRender {
                        name: ctx
                            .package
                            .resolve_field(&fa.field_id)
                            .map(|f| f.name.clone())
                            .unwrap_or_default(),
                        field_id: fa.field_id,
                        required: fa.required,
                    });
                }
            }
            Err(e) => {
                diagnostics.push(format!("ext:type-inheritance: {e}"));
            }
        }
    } else {
        for (name, _value) in record.field_values.iter() {
            fields_to_render.push(ResolvedFieldRender {
                field_id: String::new(),
                name: name.clone(),
                required: false,
            });
        }
    }

    for field in fields_to_render {
        let field_id = field.field_id.clone();
        // In structured mode (titleFieldId set), skip the heading field — already
        // emitted above. An ineligible titleFieldId is not the heading field, so it
        // is not skipped and still renders as an ordinary field row.
        if structured {
            if let Some(heading_fid) = &heading_field_id {
                if &field_id == heading_fid {
                    continue;
                }
            }
        }
        if field.name.is_empty() {
            diagnostics.push(format!(
                "field id {field_id} resolves to no Field definition; row skipped"
            ));
            continue;
        }

        let field_value = record.value(&field.name);
        let field_def = if field_id.is_empty() {
            ctx.package.find_field_by_name(&field.name)
        } else {
            ctx.package.resolve_field(&field_id)
        };
        let field_type = field_def.map(|field| &field.field_type);

        // RFC-036/RFC-039: an inline-composite value renders structurally —
        // through a named composite renderer when a view/section/document
        // directive binds one ([CR-036-6]), else the composite baseline. It
        // never falls through to the scalar row path (no raw JSON in output).
        if let Some(ft) = field_type {
            if ft.datatype == Datatype::Ref
                && ft.effective_mode() == srs_core::types::field_type::RefMode::Inline
            {
                if let Some(value) = field_value {
                    out.push_str(&render_composite_field(
                        ctx,
                        record,
                        &field,
                        ft,
                        value,
                        composite_binding_for(&field_id, use_view.as_ref(), section, ctx).as_ref(),
                        diagnostics,
                    ));
                }
                continue;
            }
        }

        let rendered_value =
            field_value.and_then(|v| render_field_value(v, field_type, ctx.format));
        if field.required && rendered_value.is_none() {
            diagnostics.push(format!(
                "[view-required] view {} record {} is missing required field {} for rendered view",
                effective_view_id.unwrap_or("<no-view-id>"),
                record.instance_id,
                field.name
            ));
        }
        if rendered_value.is_none() && omit_empty {
            continue;
        }
        // `[FR-037-10]`/`[FR-037-11]`: an absent field emits no row at all, and
        // never a label with an empty value. The sole exception is
        // `emptyBehavior: "show-placeholder"` on a field the Type marks
        // `required: true` — and it does not reach the L1 View path, where
        // `ExportConfig.omitEmptyFields` governs instead.
        let row_value = match rendered_value {
            Some(value) => value,
            None => {
                let placeholder_applies = !l1_view_path
                    && field.required
                    && matches!(
                        section.empty_behavior,
                        Some(srs_core::types::view::EmptyBehavior::ShowPlaceholder)
                    );
                if !placeholder_applies {
                    continue;
                }
                RowValue::Placeholder
            }
        };

        let field_name = field.name.clone();

        let label = display_labels
            .get(&field_id)
            .cloned()
            .or_else(|| {
                rt.as_ref()
                    .and_then(|t| t.find_field_assignment(&field_id))
                    .and_then(|fa| fa.display_label.clone())
            })
            .unwrap_or_else(|| field_name.clone());

        // `[FR-037-19]`: this form is the content `ElementTemplates.fieldRow`
        // receives as `{{content}}`. A Theme wraps the row; it never replaces it.
        let row_content = format_field_row(
            ctx.format,
            RowIdentity::FieldName(&field_name),
            &label,
            &row_value,
        );
        let value_text = row_value.template_value();
        if let Some(theme) = ctx.active_theme.as_ref() {
            if let Some(element_templates) = &theme.element_templates {
                if let Some(field_row) = &element_templates.field_row {
                    out.push_str(&apply_wrapper(
                        field_row,
                        &row_content,
                        &[
                            ("field-label", &label),
                            ("field-value", &value_text),
                            ("field-name", &field_name),
                        ],
                        Some(theme),
                    ));
                    out.push_str(row_separator(ctx.format));
                    continue;
                }
            }
        }
        out.push_str(&row_content);
        out.push_str(row_separator(ctx.format));
    }
    out.push_str(&render_relations_block(
        store,
        section,
        record,
        relations,
        ctx.package,
        ctx.format,
        diagnostics,
    )?);

    // In structured mode, render subsections nested one heading level deeper.
    if structured {
        let subsections = relation_graph::children_by_relation_type(
            &record.instance_id,
            "contains",
            relations,
            store,
        )?;
        for subsection in &subsections {
            out.push_str(&render_record_at_level(
                store,
                ctx,
                section,
                subsection,
                heading_level + 1,
                relations,
                diagnostics,
            )?);
        }
    }

    let record_wrapper = ctx
        .active_theme
        .as_ref()
        .and_then(|t| select_record_wrapper(t, &record.type_id));
    if let Some(wrapper) = record_wrapper {
        out = apply_wrapper(
            wrapper,
            &out,
            &[
                ("record-heading", &record_heading_value),
                ("type-namespace", &record.type_namespace),
                ("type-name", &record.type_name),
                ("css-classes", &css_classes_for_record(record, ctx)),
            ],
            ctx.active_theme.as_ref(),
        );
    } else if ctx.format == "html" {
        let classes = css_classes_for_record(record, ctx);
        out = format!("<div class=\"{classes}\">{out}</div>\n");
    }

    out.push('\n');
    Ok(out)
}

pub(crate) fn humanize_relation_key(key: &str) -> String {
    let segment = key.rsplit('/').next().unwrap_or(key);
    let spaced: String = segment
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let mut chars = spaced.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn is_active_relation(rel: &Relation) -> bool {
    matches!(rel.status, None | Some(RelationStatus::Active))
}

fn resolve_display_label_for_relation_target(
    store: &dyn RepositoryStore,
    instance_id: &str,
    section: &DocumentSection,
    package: &Package,
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    match crate::record_store::get_instance_by_id(store, instance_id)? {
        Some(crate::record_store::LoadedInstance::Record(target_record)) => {
            if let Some(rt) =
                package.resolve_type(&target_record.type_id, target_record.type_version)
            {
                match package.effective_identity_field_id(rt) {
                    Ok(Some(fid)) => {
                        if let Some(val) = package
                            .resolve_field(&fid)
                            .and_then(|f| target_record.value_str(&f.name))
                        {
                            if !val.is_empty() {
                                return Ok(val.to_string());
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        diagnostics.push(format!(
                            "[relations] error resolving identityFieldId for {instance_id}: {e}"
                        ));
                    }
                }
            }
            // `[N+1]` governs `titleFieldId` wherever it is consumed, not only in
            // record headings. Without this check an ineligible titleFieldId is
            // refused as a heading but still honoured here, which is incoherent.
            // The `instance_id` fallback below already handles "no usable label".
            if let Some(title_fid) = &section.title_field_id {
                // An unresolvable Type is *not* a reason to suppress the label —
                // `title_field_id_is_eligible` already treats an unresolvable
                // reference as eligible and leaves it to referential integrity.
                // Only a resolvable-and-ineligible field is refused here.
                let target_rt =
                    package.resolve_type(&target_record.type_id, target_record.type_version);
                if title_field_id_is_eligible(title_fid, target_rt, package) {
                    if let Some(val) = package
                        .resolve_field(title_fid)
                        .and_then(|f| target_record.value_str(&f.name))
                    {
                        if !val.is_empty() {
                            return Ok(val.to_string());
                        }
                    }
                }
            }
            Ok(instance_id.to_string())
        }
        _ => Ok(instance_id.to_string()),
    }
}

fn collect_relation_rows(
    store: &dyn RepositoryStore,
    section: &DocumentSection,
    record: &Record,
    relations: &[Relation],
    package: &Package,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<ProjectedRelationRow>, RepositoryError> {
    let rp = match &section.relations_presentation {
        Some(rp) => rp,
        None => return Ok(Vec::new()),
    };

    let mut rows: Vec<ProjectedRelationRow> = Vec::new();

    for entry in &rp.include {
        let matching_rtds: Vec<_> = package
            .relation_types()
            .iter()
            .filter(|rt| rt.key == entry.relation_type)
            .collect();

        let rtd = match matching_rtds.len() {
            0 => {
                diagnostics.push(format!(
                    "[I-027-2b] relationsPresentation entry '{}' does not resolve to a known RTD; skipping",
                    entry.relation_type
                ));
                continue;
            }
            1 => matching_rtds[0],
            _ => {
                diagnostics.push(format!(
                    "[I-027-2b] relationsPresentation entry '{}' is conflict-ambiguous (multiple RTDs); skipping",
                    entry.relation_type
                ));
                continue;
            }
        };

        if !rtd.resolves_for_reads() {
            diagnostics.push(format!(
                "[I-027-2b] relationsPresentation entry '{}' resolves to a retired/tombstone RTD; skipping",
                entry.relation_type
            ));
            continue;
        }

        let direction = entry
            .directions
            .as_ref()
            .unwrap_or(&PresentationDirection::Forward);
        let mut seen = std::collections::HashSet::new();
        let mut target_ids: Vec<String> = Vec::new();

        if matches!(
            direction,
            PresentationDirection::Forward | PresentationDirection::Both
        ) {
            for rel in relations {
                if rel.relation_type == entry.relation_type
                    && rel.source_instance_id == record.instance_id
                    && is_active_relation(rel)
                    && seen.insert(rel.target_instance_id.clone())
                {
                    target_ids.push(rel.target_instance_id.clone());
                }
            }
        }
        if matches!(
            direction,
            PresentationDirection::Inverse | PresentationDirection::Both
        ) {
            for rel in relations {
                if rel.relation_type == entry.relation_type
                    && rel.target_instance_id == record.instance_id
                    && is_active_relation(rel)
                    && seen.insert(rel.source_instance_id.clone())
                {
                    target_ids.push(rel.source_instance_id.clone());
                }
            }
        }

        if target_ids.is_empty() {
            continue;
        }

        let row_label = compute_relation_row_label(entry, rtd, direction);

        let mut targets: Vec<ProjectedRelationTarget> = Vec::new();
        for id in &target_ids {
            let display_label = resolve_display_label_for_relation_target(
                store,
                id,
                section,
                package,
                diagnostics,
            )?;
            targets.push(ProjectedRelationTarget {
                instance_id: id.clone(),
                display_label,
            });
        }
        targets.sort_by(|a, b| a.display_label.cmp(&b.display_label));

        rows.push(ProjectedRelationRow {
            label: row_label,
            targets,
            relation_type_key: entry.relation_type.clone(),
        });
    }

    Ok(rows)
}

fn project_relations_json(
    store: &dyn RepositoryStore,
    section: &DocumentSection,
    record: &Record,
    relations: &[Relation],
    package: &Package,
    diagnostics: &mut Vec<String>,
) -> Result<Option<Vec<ProjectedRelationRow>>, RepositoryError> {
    let rows = collect_relation_rows(store, section, record, relations, package, diagnostics)?;
    if rows.is_empty() {
        Ok(None)
    } else {
        Ok(Some(rows))
    }
}

fn compute_relation_row_label(
    entry: &srs_core::types::view::RelationPresentationEntry,
    rtd: &srs_core::types::relation_type_definition::RelationTypeDefinition,
    direction: &PresentationDirection,
) -> String {
    match direction {
        PresentationDirection::Inverse => entry
            .inverse_label
            .clone()
            .or_else(|| {
                rtd.inverse_type
                    .as_ref()
                    .map(|it| humanize_relation_key(it))
            })
            .unwrap_or_else(|| {
                entry
                    .forward_label
                    .clone()
                    .map(|fl| format!("{fl} (incoming)"))
                    .unwrap_or_else(|| {
                        format!("{} (incoming)", humanize_relation_key(&entry.relation_type))
                    })
            }),
        // RFC-027 §B: Both direction produces one combined row under the forward label.
        // inverseLabel is used only for Inverse-direction rows.
        _ => entry
            .forward_label
            .clone()
            .or_else(|| {
                if !rtd.label.is_empty() {
                    Some(rtd.label.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| humanize_relation_key(&entry.relation_type)),
    }
}

fn render_relations_block(
    store: &dyn RepositoryStore,
    section: &DocumentSection,
    record: &Record,
    relations: &[Relation],
    package: &Package,
    format: &str,
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    if section.relations_presentation.is_none() {
        return Ok(String::new());
    }

    let rows = collect_relation_rows(store, section, record, relations, package, diagnostics)?;
    let mut out = String::new();

    for row in &rows {
        // RFC-027 Change C rule 3 requires a relation row to use the same
        // label/value markup as a field row in that format. Routing through the
        // row primitive satisfies that by construction (`[FR-037-15]`), and
        // `[FR-037-12]` substitutes `srs-relationtype-*` for the identity class
        // a relation row cannot carry. RFC-027's comma-join of the related
        // instances is a relation-row rule and is unaffected by `[FR-037-5]`.
        let targets_str = row
            .targets
            .iter()
            .map(|t| t.display_label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let value = if format == "html" {
            RowValue::Scalar(html_escape(&targets_str))
        } else {
            RowValue::Scalar(targets_str)
        };

        out.push_str(&format_field_row(
            format,
            RowIdentity::RelationTypeKey(&row.relation_type_key),
            &row.label,
            &value,
        ));
        out.push_str(row_separator(format));
    }

    Ok(out)
}

/// RFC-036 [CR-036-6] — resolve the composite renderer binding for a field:
/// FieldView.compositeRenderer > DocumentSection.compositeRenderers >
/// DocumentView.compositeRenderers > none (baseline). The `baseline` sentinel
/// cancels lower-precedence sites.
fn composite_binding_for(
    field_id: &str,
    use_view: Option<&srs_core::types::view::View>,
    section: &DocumentSection,
    ctx: &RenderContext<'_>,
) -> Option<srs_core::types::view::CompositeRendererBinding> {
    if let Some(view) = use_view {
        if let Some(fv) = view.field_views.iter().find(|fv| fv.field_id == field_id) {
            if let Some(binding) = &fv.composite_renderer {
                return Some(binding.clone());
            }
        }
    }
    if let Some(directives) = &section.composite_renderers {
        // First in array order wins (document-view.json).
        if let Some(d) = directives.iter().find(|d| d.field_id == field_id) {
            return Some(srs_core::types::view::CompositeRendererBinding {
                renderer: d.renderer.clone(),
                roles: d.roles.clone(),
            });
        }
    }
    if let Some(directives) = &ctx.doc_composite_renderers {
        if let Some(d) = directives.iter().find(|d| d.field_id == field_id) {
            return Some(srs_core::types::view::CompositeRendererBinding {
                renderer: d.renderer.clone(),
                roles: d.roles.clone(),
            });
        }
    }
    None
}

/// Render an inline-composite field value (RFC-039 Change B row 4) through the
/// bound composite renderer, or the composite baseline when none is bound
/// ([CR-036-7] falls back with a diagnostic on an unknown identifier).
#[allow(clippy::too_many_arguments)]
fn render_composite_field(
    ctx: &RenderContext<'_>,
    record: &Record,
    field: &ResolvedFieldRender,
    field_type: &FieldType,
    value: &serde_json::Value,
    binding: Option<&srs_core::types::view::CompositeRendererBinding>,
    diagnostics: &mut Vec<String>,
) -> String {
    // A single composite is rendered as a one-entry sequence; a list is the
    // entry sequence itself ([R16] wraps uniformly).
    let entries: Vec<&serde_json::Map<String, serde_json::Value>> = match value {
        serde_json::Value::Array(items) => items.iter().filter_map(|v| v.as_object()).collect(),
        serde_json::Value::Object(obj) => vec![obj],
        _ => Vec::new(),
    };
    if entries.is_empty() {
        return String::new();
    }

    let range_assignments: Vec<(String, Option<srs_core::types::field::Field>)> = field_type
        .range_type
        .as_ref()
        .and_then(|range| ctx.package.resolve_type(&range.type_id, range.type_version))
        .map(|range_rt| {
            ctx.package
                .effective_fields(range_rt)
                .unwrap_or_default()
                .into_iter()
                .map(|fa| {
                    let f = ctx.package.resolve_field(&fa.field_id).cloned();
                    (f.as_ref().map(|f| f.name.clone()).unwrap_or_default(), f)
                })
                .collect()
        })
        .unwrap_or_default();

    match binding.map(|b| b.renderer.as_str()) {
        Some("table") => {
            let table_config = ctx
                .active_theme
                .as_ref()
                .and_then(|t| t.element_templates.as_ref())
                .and_then(|et| et.composite_renderer_config.as_ref())
                .and_then(|crc| crc.get("table"));
            render_composite_table(
                ctx,
                record,
                field,
                &entries,
                binding.and_then(|b| b.roles.as_ref()),
                table_config,
                diagnostics,
            )
        }
        Some("baseline") | None => {
            render_composite_baseline(ctx, field, &entries, &range_assignments)
        }
        Some(unknown) => {
            // [CR-036-7]: unknown renderer falls back to the composite
            // baseline with a diagnostic.
            diagnostics.push(format!(
                "[CR-036-7] Unrecognised compositeRenderer {:?} on field {:?}; falling back to composite baseline",
                unknown, field.name
            ));
            render_composite_baseline(ctx, field, &entries, &range_assignments)
        }
    }
}

/// The composite **baseline** (RFC-036): each entry renders its range fields as
/// ordinary RFC-037 field rows in `FieldAssignment.order`, entries separated by
/// a blank line. `compositeFieldRowTemplates` ([CR-036-18]) overrides a named
/// field's row.
fn render_composite_baseline(
    ctx: &RenderContext<'_>,
    field: &ResolvedFieldRender,
    entries: &[&serde_json::Map<String, serde_json::Value>],
    range_assignments: &[(String, Option<srs_core::types::field::Field>)],
) -> String {
    let mut out = String::new();

    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        // With no resolvable range Type, fall back to the entry's own key order
        // ([R18] serialises in FieldAssignment.order anyway).
        let names: Vec<(String, Option<&srs_core::types::field::Field>)> =
            if range_assignments.is_empty() {
                entry.keys().map(|k| (k.clone(), None)).collect()
            } else {
                range_assignments
                    .iter()
                    .map(|(n, f)| (n.clone(), f.as_ref()))
                    .collect()
            };
        for (name, field_def) in names {
            let Some(value) = entry.get(&name) else {
                continue;
            };
            let field_type = field_def.map(|f| &f.field_type);
            // Nested composites recurse through the baseline.
            if let Some(obj_entries) = composite_entries(value) {
                let nested = ResolvedFieldRender {
                    field_id: field_def.map(|f| f.id.clone()).unwrap_or_default(),
                    name: name.clone(),
                    required: false,
                };
                let nested_assignments: Vec<(String, Option<srs_core::types::field::Field>)> =
                    field_def
                        .and_then(|f| f.field_type.range_type.as_ref())
                        .and_then(|r| ctx.package.resolve_type(&r.type_id, r.type_version))
                        .map(|range_rt| {
                            ctx.package
                                .effective_fields(range_rt)
                                .unwrap_or_default()
                                .into_iter()
                                .map(|fa| {
                                    let f = ctx.package.resolve_field(&fa.field_id).cloned();
                                    (f.as_ref().map(|f| f.name.clone()).unwrap_or_default(), f)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                out.push_str(&render_composite_baseline(
                    ctx,
                    &nested,
                    &obj_entries,
                    &nested_assignments,
                ));
                continue;
            }
            let Some(row_value) = render_field_value(value, field_type, ctx.format) else {
                continue;
            };
            let label = name.clone();

            let tmpl = ctx
                .active_theme
                .as_ref()
                .and_then(|t| t.element_templates.as_ref())
                .and_then(|et| et.composite_field_row_templates.as_ref())
                .and_then(|gft| gft.get(&name));

            if let Some(tmpl) = tmpl {
                let row = tmpl
                    .replace("{{field-value}}", &row_value.template_value())
                    .replace("{{field-label}}", &label);
                out.push_str(&row);
            } else {
                out.push_str(&format_field_row(
                    ctx.format,
                    RowIdentity::FieldName(&name),
                    &label,
                    &row_value,
                ));
                out.push_str(row_separator(ctx.format));
            }
        }
    }
    let _ = field;
    out
}

/// The object entries behind a nested composite value, or None for scalars.
fn composite_entries(
    value: &serde_json::Value,
) -> Option<Vec<&serde_json::Map<String, serde_json::Value>>> {
    match value {
        serde_json::Value::Object(obj) => Some(vec![obj]),
        serde_json::Value::Array(items) if items.iter().any(|v| v.is_object()) => {
            Some(items.iter().filter_map(|v| v.as_object()).collect())
        }
        _ => None,
    }
}

/// The SRS-defined `table` composite renderer (RFC-036), re-based onto the
/// RFC-039 carrier. Role binding is by `Field.name` ([CR-036-8]), overridable
/// by the binding's UUID-anchored `roles` map:
/// `cells` binds inside each entry object; `columns`, `widths`, `subheading`
/// and `label` bind first among the record's sibling fields, then inside the
/// entry — covering both the spec shape (record-level `columns` + `rows` of
/// `{cells}`) and per-entry table shapes.
// ponytail: role search is record-then-entry by literal name; per-entry roles
// with renderer `roles` overrides across namespaces extend here when muSrs's
// five-field shape lands (unit 3).
fn render_composite_table(
    ctx: &RenderContext<'_>,
    record: &Record,
    field: &ResolvedFieldRender,
    entries: &[&serde_json::Map<String, serde_json::Value>],
    roles: Option<&std::collections::BTreeMap<String, String>>,
    table_config: Option<&serde_json::Value>,
    diagnostics: &mut Vec<String>,
) -> String {
    let mut out = String::new();

    // Resolve a role to its bound Field.name: explicit UUID-anchored override
    // first ([CR-036-8] override), else the role name itself.
    let role_name = |role: &str| -> String {
        roles
            .and_then(|r| r.get(role))
            .and_then(|fid| ctx.package.resolve_field(fid))
            .map(|f| f.name.clone())
            .unwrap_or_else(|| role.to_string())
    };
    let cells_name = role_name("cells");
    let columns_name = role_name("columns");
    let widths_name = role_name("widths");
    let subheading_name = role_name("subheading");
    let label_name = role_name("label");

    // Record-level role values (the spec table shape).
    let record_role = |name: &str| -> Option<&serde_json::Value> { record.value(name) };

    // Read table config keys.
    let table_class = table_config
        .and_then(|c| c.get("tableClass"))
        .and_then(|v| v.as_str())
        .unwrap_or("srs-data-table");
    let wrapper_template = table_config
        .and_then(|c| c.get("wrapperTemplate"))
        .and_then(|v| v.as_str());
    let caption_template = table_config
        .and_then(|c| c.get("captionTemplate"))
        .and_then(|v| v.as_str());

    // The spec shape: every entry is one row of `cells`; columns/widths/label
    // come from record-level siblings. The per-entry shape: an entry carries
    // its own columns/rows — detected by the presence of the columns role in
    // the entry itself.
    let per_entry = entries
        .first()
        .is_some_and(|e| e.contains_key(&columns_name) || !e.contains_key(&cells_name));

    if !per_entry {
        let cols: Vec<String> = record_role(&columns_name)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let widths: Vec<f64> = record_role(&widths_name)
            .and_then(|v| v.as_array().cloned())
            .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();
        let rows: Vec<Vec<String>> = entries
            .iter()
            .filter_map(|e| {
                e.get(&cells_name).and_then(|c| c.as_array()).map(|cells| {
                    cells
                        .iter()
                        .filter_map(|c| c.as_str().map(|s| s.to_string()))
                        .collect()
                })
            })
            .collect();
        if cols.is_empty() && rows.is_empty() {
            diagnostics.push(format!(
                "[CR-036-2] compositeRenderer:table on field {:?} has no columns or rows; skipping",
                field.name
            ));
            return out;
        }
        let table_str = match ctx.format {
            "html" => render_table_html(&cols, &rows, table_class, &widths),
            _ => render_table_markdown(&cols, &rows, &widths),
        };
        let label_text = record_role(&label_name).and_then(|v| v.as_str());
        let subheading = record_role(&subheading_name).and_then(|v| v.as_str());
        out.push_str(&compose_table_entry(
            ctx,
            table_str,
            subheading,
            label_text,
            wrapper_template,
            caption_template,
        ));
        return out;
    }

    // Per-entry tables (each entry carries its own columns/rows roles).
    for (entry_idx, entry) in entries.iter().enumerate() {
        let get = |name: &str| -> Option<&serde_json::Value> { entry.get(name) };
        let columns_json = get(&columns_name).and_then(|v| v.as_array().cloned());
        let rows_json = get("rows").and_then(|v| v.as_array().cloned());

        let has_columns = columns_json
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let has_rows = rows_json.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
        if !has_columns && !has_rows {
            diagnostics.push(format!(
                "[CR-036-2] compositeRenderer:table entry {} on field {:?} has no columns or rows; skipping",
                entry_idx, field.name
            ));
            continue;
        }

        let widths: Vec<f64> = get(&widths_name)
            .and_then(|v| v.as_array().cloned())
            .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();
        let subheading = get(&subheading_name).and_then(|v| v.as_str());
        let label_text = get(&label_name).and_then(|v| v.as_str());

        let cols: Vec<String> = columns_json
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let rows: Vec<Vec<String>> = rows_json
            .unwrap_or_default()
            .iter()
            .filter_map(|row| match row {
                serde_json::Value::Array(cells) => Some(
                    cells
                        .iter()
                        .filter_map(|c| c.as_str().map(|s| s.to_string()))
                        .collect(),
                ),
                serde_json::Value::Object(obj) => obj
                    .get(&cells_name)
                    .and_then(|c| c.as_array())
                    .map(|cells| {
                        cells
                            .iter()
                            .filter_map(|c| c.as_str().map(|s| s.to_string()))
                            .collect()
                    }),
                _ => None,
            })
            .collect();

        let table_str = match ctx.format {
            "html" => render_table_html(&cols, &rows, table_class, &widths),
            _ => render_table_markdown(&cols, &rows, &widths),
        };
        out.push_str(&compose_table_entry(
            ctx,
            table_str,
            subheading,
            label_text,
            wrapper_template,
            caption_template,
        ));
    }

    out
}

/// Shared table-entry envelope: subheading + caption + wrapper.
fn compose_table_entry(
    ctx: &RenderContext<'_>,
    table_str: String,
    subheading: Option<&str>,
    label_text: Option<&str>,
    wrapper_template: Option<&str>,
    caption_template: Option<&str>,
) -> String {
    let subheading_str = match subheading {
        Some(sh) if !sh.is_empty() => match ctx.format {
            "html" => format!(
                "<h{}>{}</h{}>\n",
                depth(4, ctx.depth_offset),
                html_escape(sh),
                depth(4, ctx.depth_offset)
            ),
            _ => format!(
                "{}{sh}\n\n",
                heading_prefix(depth(4, ctx.depth_offset), ctx.format)
            ),
        },
        _ => String::new(),
    };

    let label_str = match label_text {
        Some(lbl) if !lbl.is_empty() => {
            if let Some(tmpl) = caption_template {
                let safe = if ctx.format == "html" {
                    html_escape(lbl)
                } else {
                    lbl.to_owned()
                };
                tmpl.replace("{{field-value}}", &safe)
            } else if ctx.format == "html" {
                format!("<figcaption>{}</figcaption>\n", html_escape(lbl))
            } else {
                format!("*{lbl}*\n\n")
            }
        }
        _ => String::new(),
    };

    if let Some(tmpl) = wrapper_template {
        tmpl.replace("{{subheading}}", &subheading_str)
            .replace("{{label}}", &label_str)
            .replace("{{table}}", &table_str)
    } else if ctx.format == "html" {
        format!("<figure class=\"srs-table\">{subheading_str}{label_str}{table_str}</figure>\n")
    } else {
        format!("{subheading_str}{label_str}{table_str}")
    }
}

fn render_table_markdown(cols: &[String], rows: &[Vec<String>], widths: &[f64]) -> String {
    let mut out = String::new();

    if !cols.is_empty() {
        out.push('|');
        for col in cols {
            out.push(' ');
            out.push_str(&escape_gfm_cell(col));
            out.push_str(" |");
        }
        out.push('\n');

        out.push('|');
        for (i, _) in cols.iter().enumerate() {
            let align = widths.get(i).copied().unwrap_or(0.5);
            let sep = if align <= 0.3 {
                " :--- |"
            } else if align >= 0.7 {
                " ---: |"
            } else {
                " --- |"
            };
            out.push_str(sep);
        }
        out.push('\n');
    }

    for row in rows {
        out.push('|');
        for cell in row {
            out.push(' ');
            out.push_str(&escape_gfm_cell(cell));
            out.push_str(" |");
        }
        out.push('\n');
    }

    out.push('\n');
    out
}

fn render_table_html(
    cols: &[String],
    rows: &[Vec<String>],
    table_class: &str,
    widths: &[f64],
) -> String {
    let mut out = String::new();

    let class_attr = if table_class.is_empty() {
        String::new()
    } else {
        format!(" class=\"{}\"", html_escape(table_class))
    };
    out.push_str(&format!("<table{class_attr}>\n"));

    if !widths.is_empty() {
        out.push_str("<colgroup>\n");
        for w in widths {
            let pct = (w * 100.0).round() as u32;
            out.push_str(&format!("<col style=\"width:{pct}%\">\n"));
        }
        out.push_str("</colgroup>\n");
    }

    if !cols.is_empty() {
        out.push_str("<thead><tr>");
        for col in cols {
            out.push_str(&format!("<th>{}</th>", html_escape(col)));
        }
        out.push_str("</tr></thead>\n");
    }

    if !rows.is_empty() {
        out.push_str("<tbody>\n");
        for row in rows {
            out.push_str("<tr>");
            for cell in row {
                out.push_str(&format!("<td>{}</td>", html_escape(cell)));
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n");
    }

    out.push_str("</table>\n");
    out
}

/// Resolve a field's value to the row form RFC-037 requires of it.
///
/// A field is multi-entry when RFC-001 Step 2 finds it present through an
/// ordered sequence rather than a single scalar. Both mechanisms are covered
/// without preferring either: the RFC-032 `cardinality: "list"` path, whose
/// values are carried as an array in `FieldValue.value`, and the legacy
/// `ext:repeatable-fields` path (`FieldValue.entries`).
///
/// Entries that render to nothing are dropped, and a sequence with no surviving
/// entries is absent — the same outcome an empty string gets (`[FR-037-9]`).
fn render_field_value(
    value: &serde_json::Value,
    field_type: Option<&FieldType>,
    format: &str,
) -> Option<RowValue> {
    if let Some(items) = sequence_items(value, field_type) {
        let texts: Vec<String> = items
            .iter()
            .filter_map(|item| value_to_text_owned(item, format))
            .collect();
        return (!texts.is_empty()).then_some(RowValue::Entries(texts));
    }
    value_to_text_owned(value, format).map(RowValue::Scalar)
}

/// The ordered sequence behind a multi-entry value, or `None` when the value is
/// a scalar. An array value is a sequence whatever its Field declares — that is
/// how a Tier 1 `TypedField` array is recognised (`[FR-037-18]`). The RFC-039
/// carrier stores structure natively, so the pre-cutover JSON-in-a-string
/// coercion branch is deleted, not ported (RFC-039: "no field value is a
/// JSON-bearing string once structure is expressible").
fn sequence_items(
    value: &serde_json::Value,
    _field_type: Option<&FieldType>,
) -> Option<Vec<serde_json::Value>> {
    value.as_array().cloned()
}

fn value_to_text_owned(value: &serde_json::Value, format: &str) -> Option<String> {
    if let Some(s) = value.as_str() {
        // `[FR-037-10]` — an empty string is absent, as RFC-001 Step 2 already
        // declares it to be. Returning `Some("")` here is what made the omit
        // branches unreachable and put 86 label-with-no-value rows into the
        // spec repository's committed exports.
        if s.is_empty() {
            return None;
        }
        let text = if format == "html" {
            html_escape(s)
        } else {
            s.to_string()
        };
        return Some(text);
    }
    if let Some(array) = value.as_array() {
        let parts: Vec<String> = array
            .iter()
            .filter_map(|item| {
                item.as_str().map(|s| {
                    if format == "html" {
                        html_escape(s)
                    } else {
                        s.to_string()
                    }
                })
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join(", "));
        }
    }
    None
}

fn substitute_vars(
    template: &str,
    ctx: &RenderContext<'_>,
    record: Option<&Record>,
    section_context: bool,
) -> String {
    let mut out = template.to_string();
    out = out.replace("{{container-title}}", &ctx.container_title);
    out = out.replace(
        "{{date}}",
        &chrono::Utc::now().format("%Y-%m-%d").to_string(),
    );
    out = out.replace(
        "{{heading-1}}",
        &heading_prefix(depth(1, ctx.depth_offset), ctx.format),
    );
    out = out.replace(
        "{{heading-2}}",
        &heading_prefix(depth(2, ctx.depth_offset), ctx.format),
    );
    let h3 = if section_context {
        heading_prefix(depth(3, ctx.depth_offset), ctx.format)
    } else {
        String::new()
    };
    out = out.replace("{{heading-3}}", &h3);

    for level in 1..=3 {
        out = out.replace(
            &format!("{{{{heading-{level}-open}}}}"),
            &heading_open(depth(level, ctx.depth_offset), ctx.format),
        );
        out = out.replace(
            &format!("{{{{heading-{level}-close}}}}"),
            &heading_close(depth(level, ctx.depth_offset), ctx.format),
        );
    }

    if let Some(record) = record {
        out = out.replace("{{instance-id}}", &record.instance_id);
        out = out.replace("{{namespace}}", &record.type_namespace);
        out = out.replace("{{name}}", &record.type_name);
        let status = ctx
            .status_field_name
            .as_deref()
            .and_then(|name| record.value_str(name))
            .unwrap_or("");
        out = out.replace("{{status}}", status);
    } else {
        out = out.replace("{{instance-id}}", "");
        out = out.replace("{{namespace}}", "");
        out = out.replace("{{name}}", "");
        out = out.replace("{{status}}", "");
    }

    out
}

fn heading_prefix(level: u32, format: &str) -> String {
    match format {
        "markdown" => format!("{} ", "#".repeat(level as usize)),
        "adoc" => format!("{} ", "=".repeat(level as usize)),
        _ => String::new(),
    }
}

fn heading_open(level: u32, format: &str) -> String {
    match format {
        "html" => format!("<h{level}>"),
        "markdown" => format!("{} ", "#".repeat(level as usize)),
        "adoc" => format!("{} ", "=".repeat(level as usize)),
        _ => format!("{} ", "#".repeat(level as usize)),
    }
}

fn heading_close(level: u32, format: &str) -> String {
    match format {
        "html" => format!("</h{level}>\n"),
        _ => "\n\n".to_string(),
    }
}

fn format_heading(level: u32, format: &str, text: &str) -> String {
    match format {
        "html" => format!("<h{level}>{}</h{level}>\n", html_escape(text)),
        _ => format!("{}{text}\n\n", heading_prefix(level, format)),
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_gfm_cell(s: &str) -> String {
    s.replace('|', r"\|").replace('\n', " ")
}

fn depth(base: u32, depth_offset: u32) -> u32 {
    base + depth_offset
}

/// RFC-020 Rule [N+37]: resolve the effective heading field ID for a section/record pair.
/// When `section.title_field_id` is **absent**, falls back to the Type's effective
/// `identityFieldId` (when the Type is known) — RFC-020 [N+37]'s literal scope.
///
/// `[N+1]` / ext:views-l2 governs the eligibility half, and its consequence on failure
/// was settled by owner decision (srs PR #341, 2026-08-02): when an **authored**
/// `titleFieldId` fails eligibility, the heading is **omitted**, not substituted. It
/// does *not* fall through to `identityFieldId` — that reading (RFC-020 [N+37]
/// extended past its literal "does not declare" scope) was raised as an open question
/// and the spec research defeated it before the owner ruled it out explicitly. An
/// ineligible `titleFieldId` is reported separately as a validation diagnostic; see
/// `validate_title_field_id_eligibility`.
fn resolve_heading_field_id(
    section: &DocumentSection,
    rt: Option<&srs_core::types::record_type::RecordType>,
    package: &Package,
) -> Option<String> {
    match &section.title_field_id {
        Some(field_id) => {
            title_field_id_is_eligible(field_id, rt, package).then(|| field_id.clone())
        }
        None => rt.and_then(|t| package.effective_identity_field_id(t).ok().flatten()),
    }
}

/// `[N+1]`: whether `field_id` may serve as a `DocumentSection.titleFieldId`.
///
/// An unresolvable field id is left to referential-integrity validation rather than
/// being silently swallowed here, so it is reported as eligible and fails downstream
/// the same way it did before this rule existed.
///
/// `pub(crate)`: also consumed by [`crate::validation::validate_title_field_id_eligibility`]
/// so the package-validation diagnostic and the render-time behaviour share one predicate
/// (ADR-010 — no restated logic between the two call sites).
pub(crate) fn title_field_id_is_eligible(
    field_id: &str,
    _rt: Option<&srs_core::types::record_type::RecordType>,
    package: &Package,
) -> bool {
    let Some(field) = package.resolve_field(field_id) else {
        return true;
    };
    field.field_type.is_title_field_eligible()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use srs_core::types::record::FieldValues;

    fn srs_spec_repo() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return std::path::PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let vendored = manifest.join("../../tests/fixtures/spec-repo");
        if let Ok(c) = vendored.canonicalize() {
            if c.join(".srs").exists() {
                return c;
            }
        }
        let mut dir = manifest.to_path_buf();
        loop {
            let candidate = dir.join("../srs/srs");
            if let Ok(c) = candidate.canonicalize() {
                if c.join(".srs").exists() {
                    return c;
                }
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        manifest.join("../../../srs/srs")
    }

    #[test]
    fn heading_prefix_markdown() {
        assert_eq!(heading_prefix(2, "markdown"), "## ");
    }

    #[test]
    fn heading_prefix_text_returns_empty() {
        assert_eq!(heading_prefix(2, "text"), "");
    }

    #[test]
    fn heading_open_html() {
        assert_eq!(heading_open(1, "html"), "<h1>");
        assert_eq!(heading_open(2, "html"), "<h2>");
        assert_eq!(heading_open(3, "html"), "<h3>");
    }

    #[test]
    fn heading_open_markdown() {
        assert_eq!(heading_open(1, "markdown"), "# ");
        assert_eq!(heading_open(2, "markdown"), "## ");
    }

    #[test]
    fn heading_open_adoc() {
        assert_eq!(heading_open(1, "adoc"), "= ");
        assert_eq!(heading_open(2, "adoc"), "== ");
    }

    #[test]
    fn heading_open_unknown_falls_back_to_markdown() {
        assert_eq!(heading_open(1, "text"), "# ");
    }

    #[test]
    fn heading_close_html() {
        assert_eq!(heading_close(1, "html"), "</h1>\n");
        assert_eq!(heading_close(3, "html"), "</h3>\n");
    }

    #[test]
    fn heading_close_non_html() {
        assert_eq!(heading_close(1, "markdown"), "\n\n");
        assert_eq!(heading_close(2, "adoc"), "\n\n");
        assert_eq!(heading_close(1, "text"), "\n\n");
    }

    #[test]
    fn format_heading_markdown() {
        assert_eq!(format_heading(1, "markdown", "Title"), "# Title\n\n");
        assert_eq!(format_heading(3, "markdown", "Sub"), "### Sub\n\n");
    }

    #[test]
    fn format_heading_html() {
        assert_eq!(format_heading(1, "html", "Title"), "<h1>Title</h1>\n");
        assert_eq!(format_heading(2, "html", "A & B"), "<h2>A &amp; B</h2>\n");
    }

    #[test]
    fn format_heading_adoc() {
        assert_eq!(format_heading(2, "adoc", "Title"), "== Title\n\n");
    }

    #[test]
    fn format_heading_text() {
        assert_eq!(format_heading(2, "text", "Title"), "Title\n\n");
    }

    #[test]
    fn html_escape_all_chars() {
        assert_eq!(
            html_escape("a & b < c > d \"e\" 'f'"),
            "a &amp; b &lt; c &gt; d &quot;e&quot; &#39;f&#39;"
        );
    }

    #[test]
    fn html_escape_passthrough() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    #[test]
    fn normalise_css_class_basic() {
        assert_eq!(normalise_css_class("com.example.foo"), "com-example-foo");
    }

    #[test]
    fn normalise_css_class_underscores_spaces() {
        assert_eq!(normalise_css_class("hello_world foo"), "hello-world-foo");
    }

    #[test]
    fn normalise_css_class_collapse_hyphens() {
        assert_eq!(normalise_css_class("a--b"), "a-b");
    }

    #[test]
    fn normalise_css_class_uppercase() {
        assert_eq!(normalise_css_class("SomeType"), "sometype");
    }

    #[test]
    fn normalise_css_class_trim_hyphens() {
        assert_eq!(normalise_css_class("-foo-"), "foo");
    }

    // Same disposition as validation.rs's live_srs_repo_validates_cleanly: the
    // vendored corpus has pre-existing defects RFC-038's fatal catalog now
    // rejects on load; the fixture is calibration substrate for
    // tests/catalog.rs and must not be repaired here — upstream srs-repo fix.
    #[test]
    fn render_document_view_produces_output() {
        let repo_root = srs_spec_repo();
        if !repo_root.join("manifest.json").exists() {
            return;
        }
        let store = FileStore::new(repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "3a000004-0000-4000-a000-000000000004",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should work");

        assert!(!result.rendered.trim().is_empty());
        assert!(
            result.rendered.contains("# ")
                || result.rendered.contains("## ")
                || result.rendered.contains("Specification")
        );
    }

    #[test]
    fn render_document_view_unknown_id_returns_error() {
        let repo_root = srs_spec_repo();
        if !repo_root.join("manifest.json").exists() {
            return;
        }
        let store = FileStore::new(repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-0000-0000-000000000000",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        });
        assert!(matches!(
            result,
            Err(RepositoryError::DocumentViewNotFound { .. })
        ));
    }

    fn repeatable_fixture_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../srs-cli/tests/fixtures/repeatable-fields")
    }

    /// Local fixture (single-cardinality heading + a record missing a
    /// view-required field) — the srs-cli repeatable-fields fixture's carrier
    /// migration made its title field list-cardinality, which is the wrong
    /// shape for the [N+37] identity-heading and [view-required] tests.
    fn render_identity_fixture_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/render-identity")
    }

    #[test]
    fn repeatable_field_entries_render_all_values() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000981",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        // The valid record has entries ["first", "second"]; both must appear in output
        assert!(
            result.rendered.contains("first"),
            "expected 'first' in rendered output: {}",
            result.rendered
        );
        assert!(
            result.rendered.contains("second"),
            "expected 'second' in rendered output: {}",
            result.rendered
        );
        // No [partial] repeatable diagnostic — real rendering is in place
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("[partial] repeatable field")),
            "unexpected partial diagnostic: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn depth_offset_warning_emitted() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000982",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            result.diagnostics.iter().any(|d| d.contains("[N+4b]")),
            "expected [N+4b] diagnostic for depthOffset 5, got: {:?}",
            result.diagnostics
        );
    }

    /// `[N+1]` / ext:views-l2 — a repeatable `titleFieldId` is ineligible.
    ///
    /// **Inverted deliberately (srs-rust#790).** This test previously asserted
    /// that a heading *was* emitted from this fixture, whose `titleFieldId`
    /// points at a repeatable field. That locked in behaviour even the
    /// pre-erratum rule forbade, and its assertion
    /// (`contains("### first") || contains("### ")`) was satisfiable by any H3
    /// anywhere in the output, so it did not pin what its name claimed.
    ///
    /// RFC-032 Revision 7 makes `effective-single` an explicit precondition, and
    /// the owner has since settled the ineligibility consequence (srs PR #341):
    /// the heading is **omitted**, not substituted from the Type's identity
    /// field. This fixture's Type has no `identityFieldId` either, so omission
    /// and fall-through happen to coincide here — see
    /// `n1_ineligible_title_field_id_omits_heading_without_identity_fallback`
    /// for the fixture that discriminates the two.
    #[test]
    fn n1_repeatable_title_field_id_emits_no_record_heading() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000983",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            !result.rendered.contains("### first"),
            "a repeatable titleFieldId must not become a record heading, got: {}",
            result.rendered
        );
        // Refusing it as a heading must not also delete it. The body-skip keys on
        // the field actually emitted as the heading, so an ineligible titleFieldId
        // still renders as an ordinary field row — with *every* entry, which the
        // pre-[N+1] heading form could not show.
        for entry in ["first", "second", "a", "b", "c", "d"] {
            assert!(
                result.rendered.contains(entry),
                "entry {entry:?} must survive as a field row, got: {}",
                result.rendered
            );
        }
        assert!(
            result.rendered.contains("**Title**"),
            "the ineligible title field must render as a labelled row, got: {}",
            result.rendered
        );
    }

    #[test]
    fn no_title_field_id_omits_structural_heading() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        // repeatable-doc-view has no titleFieldId — records render without an H3 heading
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000981",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        // Section title "Items" produces an H2; no H3 should appear between it and field rows
        assert!(
            !result.rendered.contains("### "),
            "expected no H3 record heading when titleFieldId is absent, got: {}",
            result.rendered
        );
    }

    #[test]
    fn l1_view_display_label_renders_in_structured_section() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000986",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("**Body Label**: body text"),
            "expected FieldView.displayLabel in rendered output, got: {}",
            result.rendered
        );
    }

    #[test]
    fn missing_required_field_view_emits_soft_diagnostic() {
        let repo_root = render_identity_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000986",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[view-required]")
                    && d.contains("00000000-0000-4000-8000-000000000992")
                    && d.contains("body")),
            "expected missing required FieldView diagnostic, got: {:?}",
            result.diagnostics
        );
        assert!(
            result.rendered.contains("## Items"),
            "expected section to keep rendering, got: {}",
            result.rendered
        );
    }

    #[test]
    fn semantic_object_type_missing_slash_emits_diagnostic() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000984",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed without error");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("no namespace separator")),
            "expected 'no namespace separator' diagnostic, got: {:?}",
            result.diagnostics
        );
        // Section renders empty — no content beyond the document heading
        let lines_with_content: Vec<&str> = result
            .rendered
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect();
        assert!(
            lines_with_content.is_empty(),
            "expected empty section output, got: {:?}",
            lines_with_content
        );
    }

    #[test]
    fn themed_document_view_wraps_content_and_keeps_unknown_vars_literal() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000987",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("DOC{{unknown}}[|"),
            "expected document wrapper to keep unknown vars literal and blank heading vars, got: {}",
            result.rendered
        );
        assert!(
            result.rendered.contains("OVERRIDESECTION[items|"),
            "expected section wrapper, got: {}",
            result.rendered
        );
        // The heading variable is empty because this fixture's `titleFieldId`
        // points at a repeatable field, which `[N+1]` makes ineligible
        // (srs-rust#790) — the same fixture defect that
        // `n1_repeatable_title_field_id_emits_no_record_heading` covers. What
        // this test is about is the wrapper, and the wrapper is still applied.
        assert!(
            result.rendered.contains("OVERRIDERECORD[|"),
            "expected record wrapper, got: {}",
            result.rendered
        );
        assert!(
            result
                .rendered
                .contains("ROW[Body Label=body text|**Body Label**: body text]"),
            "expected fieldRow wrapper, got: {}",
            result.rendered
        );
        assert!(
            result
                .rendered
                .contains("Preamble: Repeatable Fields Fixture"),
            "expected preamble to render outside field rows, got: {}",
            result.rendered
        );
    }

    #[test]
    fn theme_variant_selection_uses_matching_variant() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000987",
            format: None,
            theme_variant: Some("print"),
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("PRINTDOC["),
            "expected print variant wrapper, got: {}",
            result.rendered
        );
        assert!(
            !result.rendered.contains("DOC{{unknown}}["),
            "expected variant theme to replace base theme output, got: {}",
            result.rendered
        );
    }

    #[test]
    fn theme_variant_not_found_falls_back_to_theme_ref() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000987",
            format: None,
            theme_variant: Some("missing"),
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("theme variant 'missing' not found")),
            "expected missing variant diagnostic, got: {:?}",
            result.diagnostics
        );
        assert!(
            result.rendered.contains("DOC{{unknown}}["),
            "expected fallback to base theme, got: {}",
            result.rendered
        );
    }

    #[test]
    fn theme_format_mismatch_skips_theme_and_emits_diagnostic() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000987",
            format: Some("text"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.diagnostics.iter().any(|d| d.contains("[T-2]")),
            "expected [T-2] diagnostic, got: {:?}",
            result.diagnostics
        );
        assert!(
            !result.rendered.contains("DOC{{unknown}}["),
            "expected plain render without theme, got: {}",
            result.rendered
        );
    }

    #[test]
    fn theme_bundled_ref_not_found_emits_diagnostic() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000988",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[T-5]") && d.contains("00000000-0000-4000-8000-000000000999")),
            "expected missing theme diagnostic, got: {:?}",
            result.diagnostics
        );
        assert!(
            !result.rendered.contains("DOC{{unknown}}["),
            "expected plain render without theme, got: {}",
            result.rendered
        );
    }

    fn field_groups_fixture_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../srs-cli/tests/fixtures/field-groups")
    }

    #[test]
    fn json_projection_returns_projection_not_rendered_string() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            result.projection.is_some(),
            "expected projection to be populated"
        );
        assert!(
            result.rendered.is_empty(),
            "expected rendered string to be empty for json mode"
        );
    }

    #[test]
    fn json_projection_schema_and_view_id_fields() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        assert_eq!(
            proj.schema,
            "https://srs.semanticops.com/schema/2.0/document-view-output.json"
        );
        assert_eq!(
            proj.document_view_id,
            "00000000-0000-4000-8000-000000000971"
        );
        assert!(!proj.generated_at.is_empty(), "generatedAt must be set");
    }

    #[test]
    fn json_projection_container_id_is_null_for_type_query_section() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        assert!(
            proj.container_id.is_none(),
            "containerId should be null when no ContainerSubset section: {:?}",
            proj.container_id
        );
    }

    #[test]
    fn json_projection_preamble_blanks_heading_vars() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        let preamble = proj.preamble.expect("preamble should be present");
        assert!(
            preamble.contains("Groups Fixture"),
            "preamble should contain container-title, got: {preamble}"
        );
        assert!(
            !preamble.contains("{{heading-"),
            "heading vars must be blanked, got: {preamble}"
        );
        assert!(
            preamble.contains("exported"),
            "static text after heading var should remain, got: {preamble}"
        );
    }

    #[test]
    fn json_projection_sections_ordered_and_records_present() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        assert_eq!(proj.sections.len(), 1, "expected 1 section");
        let section = &proj.sections[0];
        assert_eq!(section.section_id, "all-groups");
        assert_eq!(section.order, 0);
        assert!(!section.records.is_empty(), "section should have records");
    }

    #[test]
    fn json_projection_record_has_identity_fields() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        let record = proj.sections[0]
            .records
            .iter()
            .find(|r| r.instance_id == "00000000-0000-4000-8000-000000000981")
            .expect("valid record should be present");
        assert_eq!(record.type_id, "00000000-0000-4000-8000-000000000913");
        assert_eq!(record.type_namespace, "fixture.groups");
        assert_eq!(record.type_name, "grouped-item");
    }

    /// RFC-039 [R11]: a composite value is carried recursively under its own
    /// `Field.name` key inside `fields` — no separate `fieldGroups` block.
    #[test]
    fn json_projection_record_composite_carried_under_field_key() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        let record = proj.sections[0]
            .records
            .iter()
            .find(|r| r.instance_id == "00000000-0000-4000-8000-000000000981")
            .expect("valid record should be present");
        let people = record
            .fields
            .get("people")
            .expect("composite 'people' must be carried under its Field.name key");
        let entries = people.as_array().expect("composite list value is an array");
        assert_eq!(entries.len(), 2, "expected 2 entries (alice, bob)");
        assert_eq!(
            entries[0].get("name"),
            Some(&serde_json::Value::String("alice".to_string())),
            "first entry should have name=alice"
        );
    }

    #[test]
    fn json_projection_format_override_uses_json_branch() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000981",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            result.projection.is_some(),
            "projection must be present when format=json overrides view format"
        );
        assert!(
            result.rendered.is_empty(),
            "rendered must be empty in json mode"
        );
    }

    #[test]
    fn theme_no_themeref_renders_without_theme() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000981",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            !result.rendered.contains("DOC{{unknown}}[") && !result.rendered.contains("PRINTDOC["),
            "expected render without theme wrappers, got: {}",
            result.rendered
        );
    }

    // ── Heterogeneous document rendering tests ────────────────────────────────

    /// Build a MemoryStore with two types (text-section and table-section), two
    /// fields (heading + body), a container, and a ContainerSubset document view.
    /// Used by the heterogeneous rendering tests below.
    /// `[T-9]` / ext:themes-l1 — only effective-single prose `string` fields
    /// contribute a CSS class via `Theme.cssClassFields`.
    ///
    /// Entirely constructed: `cssClassFields` has zero first-party use sites, so
    /// nothing in the corpus reaches `css_classes_for_record`'s theme branch at
    /// all (srs-rust#790, CC-33). Before this rule the branch emitted a class
    /// for any field whose stored JSON value happened to be a string — which is
    /// true of `date`, `date-time`, `uri`, `uuid` and `email` alike.
    #[test]
    fn t9_css_class_fields_admits_only_effective_single_prose_strings() {
        use crate::package::Package;
        use srs_core::types::field::Field;
        use srs_core::types::record::Record;
        use srs_core::types::record_type::RecordType;

        fn field(id: &str, name: &str, field_type: serde_json::Value) -> Field {
            serde_json::from_value(serde_json::json!({
                "id": id, "namespace": "com.test", "name": name, "version": 1,
                "description": "d", "fieldType": field_type,
                "createdAt": "2026-01-01T00:00:00Z",
            }))
            .expect("field fixture should deserialize")
        }

        // Each field stores a JSON string, so the pre-rule `as_str()` test
        // admitted every one of them. Only `status` and `note` are eligible.
        let fields = vec![
            field(
                "f-status",
                "status",
                serde_json::json!({ "datatype": "string" }),
            ),
            field(
                "f-note",
                "note",
                serde_json::json!({ "datatype": "string", "format": "markdown" }),
            ),
            field("f-due", "due", serde_json::json!({ "datatype": "date" })),
            field(
                "f-home",
                "home",
                serde_json::json!({ "datatype": "string", "format": "uri" }),
            ),
            field(
                "f-key",
                "key",
                serde_json::json!({ "datatype": "string", "format": "uuid" }),
            ),
            field(
                "f-contact",
                "contact",
                serde_json::json!({ "datatype": "string", "format": "email" }),
            ),
            field(
                "f-labels",
                "labels",
                serde_json::json!({ "datatype": "string", "cardinality": "list" }),
            ),
            // Plain single string — eligible (the retired assignment-level
            // `repeatable` no longer affects eligibility; Change-I condition 4
            // is cardinality-only).
            field(
                "f-alias",
                "alias",
                serde_json::json!({ "datatype": "string" }),
            ),
        ];
        let assignment = |i: usize, id: &str| serde_json::json!({ "fieldId": id, "order": i, "required": false });
        let record_type: RecordType = serde_json::from_value(serde_json::json!({
            "id": "t-task", "namespace": "com.test", "name": "task", "version": 1,
            "description": "d",
            "fields": [
                assignment(0, "f-status"), assignment(1, "f-note"),
                assignment(2, "f-due"), assignment(3, "f-home"),
                assignment(4, "f-key"), assignment(5, "f-contact"),
                assignment(6, "f-labels"), assignment(7, "f-alias"),
            ],
            "createdAt": "2026-01-01T00:00:00Z",
        }))
        .expect("type fixture should deserialize");
        let record: Record = serde_json::from_value(serde_json::json!({
            "instanceId": "i-1", "typeId": "t-task", "typeVersion": 1,
            "typeNamespace": "com.test", "typeName": "task",
            "fieldValues": {
                "status": "open",
                "note": "prose",
                "due": "2026-01-01",
                "home": "https://example.test",
                "key": "00000000-0000-4000-8000-000000000001",
                "contact": "someone@example.test",
                "labels": "alpha",
                "alias": "nickname",
            },
        }))
        .expect("record fixture should deserialize");
        let theme: Theme = serde_json::from_value(serde_json::json!({
            "id": "th-1", "namespace": "com.test", "name": "t9-theme", "version": 1,
            "description": "d", "targets": ["html"],
            "cssClassFields": [
                "f-status", "f-note", "f-due", "f-home",
                "f-key", "f-contact", "f-labels", "f-alias",
            ],
            "createdAt": "2026-01-01T00:00:00Z",
        }))
        .expect("theme fixture should deserialize");

        let package = Package {
            id: "p".to_string(),
            namespace: "com.test".to_string(),
            name: "p".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: vec![record_type],
            relation_type_definitions: Vec::new(),
            views: Vec::new(),
            document_views: Vec::new(),
            themes: vec![theme.clone()],
            blueprints: Vec::new(),
            protocols: Vec::new(),
            root: std::path::PathBuf::from("."),
            dependency_refs: Vec::new(),
            vocabularies: Vec::new(),
            lifecycles: Vec::new(),
        };
        let ctx = RenderContext {
            package: &package,
            container_title: String::new(),
            depth_offset: 0,
            format: "html",
            status_field_name: None,
            active_theme: Some(theme),
            doc_composite_renderers: None,
        };

        let classes = css_classes_for_record(&record, &ctx);
        assert!(
            classes.contains("srs-field-status-open"),
            "an open prose string must yield a class, got: {classes}"
        );
        assert!(
            classes.contains("srs-field-note-prose"),
            "a markdown string must yield a class, got: {classes}"
        );
        assert!(
            classes.contains("srs-field-alias-nickname"),
            "a plain single string stays eligible now the assignment-level \
             repeatable flag is retired (Change-I condition 4), got: {classes}"
        );
        for (label, ineligible) in [
            ("date", "srs-field-due-"),
            ("format: uri", "srs-field-home-"),
            ("format: uuid", "srs-field-key-"),
            ("format: email", "srs-field-contact-"),
            ("cardinality: list", "srs-field-labels-"),
        ] {
            assert!(
                !classes.contains(ineligible),
                "{label} is ineligible under [T-9] but emitted a class: {classes}"
            );
        }
    }

    fn make_hetero_store() -> (crate::store::memory::MemoryStore, String, String, String) {
        use crate::container_service;
        use crate::package::Package;
        use crate::record_store::create_record;
        use crate::relation_service;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::relation::Relation;
        use srs_core::types::view::{
            DocumentSection, DocumentView, EmptyBehavior, FieldView, SectionSource, View,
        };

        let heading_field = Field {
            schema: None,
            id: "f-heading".to_string(),
            namespace: "com.test".to_string(),
            name: "heading".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Heading".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let body_field = Field {
            schema: None,
            id: "f-body".to_string(),
            namespace: "com.test".to_string(),
            name: "body".to_string(),
            version: 1,
            field_type: FieldType::text(),
            description: "Body text".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let caption_field = Field {
            schema: None,
            id: "f-caption".to_string(),
            namespace: "com.test".to_string(),
            name: "caption".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Caption for tables".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let text_type = RecordType {
            id: "t-text".to_string(),
            namespace: "com.test".to_string(),
            name: "section.text".to_string(),
            version: 1,
            description: "Text section".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-heading".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Heading".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "f-body".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Body".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };
        let table_type = RecordType {
            id: "t-table".to_string(),
            namespace: "com.test".to_string(),
            name: "section.table".to_string(),
            version: 1,
            description: "Table section".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-heading".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Heading".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "f-caption".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Caption".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        // View that only matches text sections (has compatible_types constraint)
        let text_only_view = View {
            id: "v-text-only".to_string(),
            namespace: "com.test".to_string(),
            name: "text-view".to_string(),
            version: 1,
            description: "View for text sections only".to_string(),
            field_views: vec![FieldView {
                composite_renderer: None,
                field_id: "f-body".to_string(),
                order: 0,
                required: Some(true),
                visible: None,
                display_label: Some("Content".to_string()),
            }],
            compatible_types: Some(vec!["com.test/section.text".to_string()]),
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        // DocumentView: ContainerSubset section with the text-only view
        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-hetero".to_string(),
            namespace: "com.test".to_string(),
            name: "hetero-view".to_string(),
            version: 1,
            description: "Heterogeneous container document view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "body".to_string(),
                title: Some("Body".to_string()),
                description: None,
                order: 0,
                source: SectionSource::ContainerSubset {
                    container_id: "00000000-0000-4000-8000-000000000c01".to_string(),
                    container_type: None,
                    type_filter: None,
                },
                render_view_id: Some("v-text-only".to_string()),
                type_dispatch: None,
                title_field_id: Some("f-heading".to_string()),
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-hetero".to_string(),
            namespace: "com.test".to_string(),
            name: "hetero-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field, body_field, caption_field],
            record_types: vec![text_type, table_type],
            relation_type_definitions: vec![
                srs_core::types::relation_type_definition::RelationTypeDefinition {
                    schema: None,
                    id: "00000000-0000-4000-8000-000000000rt1".to_string(),
                    namespace: "com.test".to_string(),
                    key: "precedes".to_string(),
                    label: "Precedes".to_string(),
                    description: "Ordering relation".to_string(),
                    category:
                        srs_core::types::relation_type_definition::RelationTypeCategory::Sequence,
                    canonical_direction: None,
                    irreflexive: Some(true),
                    inverse_type: None,
                    version: 1,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    allowed_source_types: None,
                    allowed_target_types: None,
                    require_same_semantic_object_type: None,
                    status: None,
                    updated_at: None,
                    properties: None,
                },
            ],
            views: vec![text_only_view],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        // Create container
        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: "00000000-0000-4000-8000-000000000c01".to_string(),
                title: "Test Guide".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: Some("guide".to_string()),
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        // Create a text record (precedes the table)
        let mut fv_text = srs_core::types::record::FieldValues::new();
        fv_text.insert("heading", serde_json::json!("Introduction"));
        fv_text.insert("body", serde_json::json!("The introduction body."));
        let text_record = create_record(&store, "t-text", 1, fv_text, None, None).unwrap();
        let text_id = text_record.instance_id.clone();

        // Create a table record (follows the text)
        let mut fv_table = srs_core::types::record::FieldValues::new();
        fv_table.insert("heading", serde_json::json!("Summary Table"));
        fv_table.insert("caption", serde_json::json!("Table caption here"));
        let table_record = create_record(&store, "t-table", 1, fv_table, None, None).unwrap();
        let table_id = table_record.instance_id.clone();

        // Add both to container
        container_service::add_member(&store, "00000000-0000-4000-8000-000000000c01", &text_id)
            .unwrap();
        container_service::add_member(&store, "00000000-0000-4000-8000-000000000c01", &table_id)
            .unwrap();

        // Establish precedes: text → table
        relation_service::create_relation_auto(
            &store,
            Relation {
                relation_id: String::new(),
                relation_type: "precedes".to_string(),
                source_instance_id: text_id.clone(),
                target_instance_id: table_id.clone(),
                asserted_by: None,
                confidence: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            },
        )
        .unwrap();

        (store, text_id, table_id, "dv-hetero".to_string())
    }

    #[test]
    fn container_subset_renders_in_precedes_order() {
        let (store, text_id, table_id, view_id) = make_hetero_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let rendered = &result.rendered;
        let text_pos = rendered
            .find("Introduction")
            .expect("text record heading not found");
        let table_pos = rendered
            .find("Summary Table")
            .expect("table record heading not found");
        assert!(
            text_pos < table_pos,
            "text section (precedes) should appear before table section; got:\n{}",
            rendered
        );
        // Suppress unused-variable warnings from make_hetero_store return values
        let _ = (text_id, table_id);
    }

    #[test]
    fn container_subset_instance_id_filter_scopes_to_single_record() {
        let (store, text_id, _table_id, view_id) = make_hetero_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: Some(&text_id),
        })
        .expect("render should succeed");

        let rendered = &result.rendered;
        assert!(
            rendered.contains("Introduction"),
            "filtered instance's heading should still render; got:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("Summary Table"),
            "instance_id_filter should exclude the other ContainerSubset member; got:\n{}",
            rendered
        );
    }

    #[test]
    fn json_projection_container_subset_instance_id_filter_scopes_to_single_record() {
        let (store, text_id, table_id, view_id) = make_hetero_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: Some(&text_id),
        })
        .expect("render should succeed");

        let ids = rfc011_instance_ids_in_result(&result);
        assert!(
            ids.contains(&text_id),
            "expected filtered instance in projection; got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&table_id),
            "instance_id_filter should exclude the other ContainerSubset member; got: {:?}",
            ids
        );
    }

    #[test]
    fn view_dispatch_applies_view_to_matching_type_and_falls_back_for_non_matching() {
        let (store, _text_id, _table_id, view_id) = make_hetero_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let rendered = &result.rendered;
        // Text section gets the view → field label is "Content" (view display_label)
        assert!(
            rendered.contains("Content"),
            "text section should render with view display_label 'Content'; got:\n{}",
            rendered
        );
        // Table section falls back to its own type → field label is "Caption" (type display_label)
        assert!(
            rendered.contains("Caption"),
            "table section should fall back to own type fields and show 'Caption'; got:\n{}",
            rendered
        );
        // Diagnostic emitted for the non-matching table record
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[view-dispatch]")),
            "expected [view-dispatch] diagnostic for non-matching record; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn title_field_id_omitted_silently_for_record_lacking_field() {
        // The ContainerSubset view uses titleFieldId = f-heading. Both record types have
        // f-heading, so headings render for all. This test asserts no crash when rendering
        // a heterogeneous set — the existing l1_view tests cover the heading-omit path.
        let (store, _text_id, _table_id, view_id) = make_hetero_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        });
        assert!(
            result.is_ok(),
            "render should not panic or error on mixed-type container"
        );
    }

    #[test]
    fn fixed_instances_section_preserves_authored_order_via_sort_chain() {
        // Verify that sort_by_precedes_chain, when applied to FixedInstances records
        // with NO precedes relations (simulating the bug scenario), would change their
        // order via created_at sorting — confirming the guard is necessary.
        // This tests the guard logic indirectly by checking sort_by_precedes_chain
        // behaviour on records without precedes edges.
        use srs_core::types::record::Record;
        use std::collections::BTreeMap as StdMap;

        // Create two records with different created_at — "later" has more recent timestamp.
        let make_record = |id: &str, created: &str| Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values: FieldValues::new(),
            lifecycle_state: None,
            tags: None,
            created_at: Some(created.to_string()),
            updated_at: None,
            extra: StdMap::new(),
        };

        // "later" was created first in time (earlier timestamp), "earlier" was created after.
        // Without the guard, sort_by_precedes_chain would sort by created_at (ascending),
        // producing [earlier_ts, later_ts] regardless of the authored order.
        let later_ts = make_record("b-later", "2026-06-01T10:00:00Z");
        let earlier_ts = make_record("a-earlier", "2026-06-01T09:00:00Z");

        // Authored order: [later_ts, earlier_ts] (b first, a second).
        // sort_by_precedes_chain with no precedes relations falls back to created_at,
        // which would produce [earlier_ts, later_ts] (a first, b second) — wrong.
        let authored = vec![later_ts.clone(), earlier_ts.clone()];
        let sorted = crate::relation_graph::sort_by_precedes_chain(authored, &[]);

        // sort_by_precedes_chain DOES reorder (this is what the guard must prevent).
        assert_eq!(
            sorted[0].instance_id, "a-earlier",
            "sort_by_precedes_chain with no relations sorts by created_at ascending"
        );
        assert_eq!(sorted[1].instance_id, "b-later");
        // This confirms that WITHOUT the guard, FixedInstances would be reordered.
        // The guard (matches! FixedInstances) in both render paths prevents this call.
    }

    // ── RFC-007 composite renderer tests ──────────────────────────────────────

    #[test]
    fn render_table_markdown_produces_gfm_pipe_table() {
        let cols = vec!["Question".to_string(), "What to write".to_string()];
        let rows = vec![vec![
            "What was decided?".to_string(),
            "One clear sentence.".to_string(),
        ]];
        let out = render_table_markdown(&cols, &rows, &[]);
        assert!(out.contains("| Question | What to write |"), "got: {out}");
        assert!(out.contains("| --- | --- |"), "got: {out}");
        assert!(
            out.contains("| What was decided? | One clear sentence. |"),
            "got: {out}"
        );
    }

    #[test]
    fn render_table_markdown_widths_alignment() {
        let cols = vec![
            "Left".to_string(),
            "Default".to_string(),
            "Right".to_string(),
        ];
        let rows: Vec<Vec<String>> = vec![];
        let widths = vec![0.25, 0.5, 0.75]; // ≤0.3 left, middle default, ≥0.7 right
        let out = render_table_markdown(&cols, &rows, &widths);
        assert!(out.contains("| :--- |"), "expected left-align, got: {out}");
        assert!(
            out.contains("| --- |"),
            "expected default-align, got: {out}"
        );
        assert!(out.contains("| ---: |"), "expected right-align, got: {out}");
    }

    #[test]
    fn render_table_markdown_boundary_widths_are_deterministic() {
        let cols = vec!["A".to_string(), "B".to_string()];
        let rows: Vec<Vec<String>> = vec![];
        let widths = vec![0.3, 0.7]; // exactly 0.3 → left, exactly 0.7 → right
        let out = render_table_markdown(&cols, &rows, &widths);
        assert!(
            out.contains("| :--- |"),
            "0.3 should be left-align, got: {out}"
        );
        assert!(
            out.contains("| ---: |"),
            "0.7 should be right-align, got: {out}"
        );
    }

    #[test]
    fn render_table_markdown_escapes_pipes_in_cells() {
        let cols = vec!["Type | Status".to_string()];
        let rows = vec![vec!["a | b".to_string()]];
        let out = render_table_markdown(&cols, &rows, &[]);
        assert!(
            !out.contains("| Type | Status |"),
            "unescaped pipe in header must not appear, got:\n{out}"
        );
        assert!(
            out.contains(r"Type \| Status"),
            "pipe in header should be escaped, got:\n{out}"
        );
        assert!(
            out.contains(r"a \| b"),
            "pipe in cell should be escaped, got:\n{out}"
        );
    }

    #[test]
    fn render_table_markdown_newlines_in_cells_become_spaces() {
        let cols = vec!["Col".to_string()];
        let rows = vec![vec!["line1\nline2".to_string()]];
        let out = render_table_markdown(&cols, &rows, &[]);
        assert!(
            out.contains("line1 line2"),
            "newline in cell should become space, got:\n{out}"
        );
    }

    #[test]
    fn render_table_html_produces_table_element() {
        let cols = vec!["Col A".to_string()];
        let rows = vec![vec!["val 1".to_string()]];
        let out = render_table_html(&cols, &rows, "srs-data-table", &[]);
        assert!(
            out.contains("<table class=\"srs-data-table\">"),
            "got: {out}"
        );
        assert!(out.contains("<th>Col A</th>"), "got: {out}");
        assert!(out.contains("<td>val 1</td>"), "got: {out}");
    }

    #[test]
    fn render_table_html_empty_class_omits_attribute() {
        let out = render_table_html(&[], &[], "", &[]);
        assert!(
            !out.contains("class="),
            "[T-Cx2] empty tableClass must omit class attribute, got: {out}"
        );
    }

    #[test]
    fn render_table_html_widths_emit_colgroup() {
        let widths = vec![0.3, 0.7];
        let out = render_table_html(&[], &[], "cls", &widths);
        assert!(out.contains("<colgroup>"), "got: {out}");
        assert!(out.contains("width:30%"), "got: {out}");
        assert!(out.contains("width:70%"), "got: {out}");
    }

    /// Build a MemoryStore with one type + TypeQuery doc_view for composite renderer tests.
    /// Creates a record pre-loaded with the given group entry and returns (store, record_id).
    /// Build a MemoryStore for the RFC-036 composite table tests, re-based onto
    /// the RFC-039 carrier: the Type carries an inline-composite list Field
    /// `rows` (range Type `table-row` with a `cells` list), the record carries
    /// the spec table shape (`columns` sibling + `rows` of `{cells}` entries),
    /// and the renderer binds **view-side** via `FieldView.compositeRenderer`.
    fn make_composite_table_store(
        composite_renderer: Option<&str>,
        columns: serde_json::Value,
        rows: serde_json::Value,
        theme: Option<srs_core::types::theme::Theme>,
    ) -> crate::store::memory::MemoryStore {
        make_composite_table_store_with(composite_renderer, columns, rows, theme, "markdown", None)
    }

    fn make_composite_table_store_with(
        composite_renderer: Option<&str>,
        columns: serde_json::Value,
        rows: serde_json::Value,
        theme: Option<srs_core::types::theme::Theme>,
        format: &str,
        label: Option<serde_json::Value>,
    ) -> crate::store::memory::MemoryStore {
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::field_type::ExactTypeRef;
        use srs_core::types::record::FieldValues;
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::view::{
            CompositeRendererBinding, DocumentSection, DocumentView, EmptyBehavior, FieldView,
            SectionSource, View,
        };

        let make_field = |id: &str, name: &str, field_type: FieldType| Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            field_type,
            description: name.to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let fields = vec![
            make_field("f-columns", "columns", FieldType::string().into_list()),
            make_field(
                "f-rows",
                "rows",
                FieldType::inline_ref(ExactTypeRef {
                    type_id: "t-row".to_string(),
                    type_version: 1,
                })
                .into_list(),
            ),
            make_field("f-cells", "cells", FieldType::string().into_list()),
            make_field("f-widths", "widths", FieldType::number().into_list()),
            make_field("f-subheading", "subheading", FieldType::string()),
            make_field("f-label", "label", FieldType::string()),
            make_field("f-title", "title", FieldType::string()),
        ];

        let row_type = RecordType {
            id: "t-row".to_string(),
            namespace: "com.test".to_string(),
            name: "table-row".to_string(),
            version: 1,
            description: "One table row".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-cells".to_string(),
                order: 0,
                required: false,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };
        let record_type = RecordType {
            id: "t-table-rec".to_string(),
            namespace: "com.test".to_string(),
            name: "table-record".to_string(),
            version: 1,
            description: "Record with an inline-composite table field".to_string(),
            fields: [
                "f-title",
                "f-columns",
                "f-rows",
                "f-widths",
                "f-subheading",
                "f-label",
            ]
            .iter()
            .enumerate()
            .map(|(i, id)| FieldAssignment {
                field_id: id.to_string(),
                order: i as u32,
                required: false,
                display_label: None,
                description: None,
            })
            .collect(),
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        // RFC-036: the renderer binding is view-owned, not Type-owned.
        let table_view = View {
            id: "v-table".to_string(),
            namespace: "com.test".to_string(),
            name: "table-view".to_string(),
            version: 1,
            description: "Binds the table renderer to the rows composite".to_string(),
            field_views: vec![
                FieldView {
                    composite_renderer: None,
                    field_id: "f-title".to_string(),
                    order: 0,
                    required: None,
                    visible: None,
                    display_label: None,
                },
                FieldView {
                    composite_renderer: composite_renderer.map(|r| CompositeRendererBinding {
                        renderer: r.to_string(),
                        roles: None,
                    }),
                    field_id: "f-rows".to_string(),
                    order: 1,
                    required: None,
                    visible: None,
                    display_label: None,
                },
            ],
            compatible_types: None,
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-table".to_string(),
            namespace: "com.test".to_string(),
            name: "table-view".to_string(),
            version: 1,
            description: "Table document view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "tables".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "com.test/table-record".to_string(),
                    lifecycle_state: None,
                    container_ids: None,
                    lifecycle_states: None,
                    exclude_lifecycle_states: None,
                    container_scope: None,
                },
                render_view_id: Some("v-table".to_string()),
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some(format.to_string()),
            depth_offset: None,
            theme_ref: theme
                .as_ref()
                .map(|t| srs_core::types::view::ThemeReference {
                    mode: srs_core::types::view::ThemeMode::Bundled,
                    path: None,
                    url: None,
                    theme_id: Some(t.id.clone()),
                }),
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-table".to_string(),
            namespace: "com.test".to_string(),
            name: "table-package".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: vec![row_type, record_type],
            relation_type_definitions: vec![],
            views: vec![table_view],
            document_views: vec![doc_view],
            themes: theme.into_iter().collect(),
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        // The spec table shape: record-level `columns`, `rows` of `{cells}`.
        let mut fv = FieldValues::new();
        fv.insert("title", serde_json::json!("Table Record"));
        fv.insert("columns", columns);
        fv.insert("rows", rows);
        if let Some(label) = label {
            fv.insert("label", label);
        }
        create_record(&store, "t-table-rec", 1, fv, None, None).unwrap();
        store
    }

    #[test]
    fn composite_table_renders_gfm_table_in_document_view() {
        let store = make_composite_table_store(
            Some("table"),
            serde_json::json!(["Col1", "Col2"]),
            serde_json::json!([{"cells": ["A", "B"]}, {"cells": ["C", "D"]}]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let out = &result.rendered;
        assert!(
            out.contains("| Col1 | Col2 |"),
            "expected header row, got:\n{out}"
        );
        assert!(
            out.contains("| --- | --- |"),
            "expected separator, got:\n{out}"
        );
        assert!(
            out.contains("| A | B |"),
            "expected data row 1, got:\n{out}"
        );
        assert!(
            out.contains("| C | D |"),
            "expected data row 2, got:\n{out}"
        );
        assert!(
            result.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn composite_table_no_raw_json_in_output() {
        let store = make_composite_table_store(
            Some("table"),
            serde_json::json!(["Q", "A"]),
            serde_json::json!([{"cells": ["q1", "a1"]}]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render");
        let out = &result.rendered;
        assert!(
            !out.contains("[\"Q\""),
            "raw JSON columns must not appear, got:\n{out}"
        );
        assert!(
            !out.contains("[\"q1\""),
            "raw JSON rows must not appear, got:\n{out}"
        );
    }

    #[test]
    fn unknown_composite_renderer_falls_back_and_emits_cr036_7_diagnostic() {
        let store = make_composite_table_store(
            Some("com.acme/gantt"),
            serde_json::json!(["Col"]),
            serde_json::json!([{"cells": ["val"]}]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should not hard-error on unknown renderer");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[CR-036-7]") && d.contains("com.acme/gantt")),
            "[CR-036-7] diagnostic expected, got: {:?}",
            result.diagnostics
        );
        assert!(
            !result.rendered.contains("| Col |"),
            "unknown renderer must not produce a GFM table, got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn caption_template_html_escapes_label_value() {
        use srs_core::types::theme::{ElementTemplates, Theme};

        let theme = Theme {
            id: "th-cap".to_string(),
            namespace: "com.test".to_string(),
            name: "cap-theme".to_string(),
            version: 1,
            description: "d".to_string(),
            targets: vec!["html".to_string()],
            assets: None,
            css_class_fields: None,
            page_templates: None,
            element_templates: Some(ElementTemplates {
                document_wrapper: None,
                section_wrapper: None,
                section_wrapper_overrides: None,
                record_wrapper: None,
                record_wrapper_overrides: None,
                field_row: None,
                composite_field_row_templates: None,
                composite_renderer_config: Some(std::collections::BTreeMap::from([(
                    "table".to_string(),
                    serde_json::json!({ "captionTemplate": "{{field-value}}" }),
                )])),
            }),
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };
        let store = make_composite_table_store_with(
            Some("table"),
            serde_json::json!(["Col"]),
            serde_json::json!([{"cells": ["val"]}]),
            Some(theme),
            "html",
            Some(serde_json::json!("<script>alert(1)</script>")),
        );

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let out = &result.rendered;
        assert!(
            !out.contains("<script>"),
            "raw <script> must not appear in HTML output, got:\n{out}"
        );
        assert!(
            out.contains("&lt;script&gt;"),
            "label must be HTML-escaped in captionTemplate output, got:\n{out}"
        );
    }

    #[test]
    fn empty_columns_and_rows_emits_cr036_2_diagnostic_and_skips_entry() {
        // One entry with neither cells nor columns/rows roles → skipped with
        // the [CR-036-2] diagnostic.
        let store = make_composite_table_store(
            Some("table"),
            serde_json::json!([]),
            serde_json::json!([{}]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should not hard-error");
        assert!(
            result.diagnostics.iter().any(|d| d.contains("[CR-036-2]")),
            "[CR-036-2] diagnostic expected for empty entry, got: {:?}",
            result.diagnostics
        );
        assert!(
            !result.rendered.contains("| --- |"),
            "empty entry must not produce a table row"
        );
    }

    // ── Issue #3: ContainerSubset field ordering ───────────────────────────────

    /// Build a MemoryStore with 3 records whose UUIDs sort in a different order than their
    /// title strings, so a broken field-sort path would produce the wrong sequence.
    ///
    /// UUID order (used by `add_member`): 001 < 002 < 003
    /// Title mapping:  001→"C-last",  002→"A-first",  003→"B-middle"
    /// UUID-order output: C-last, A-first, B-middle  ← neither asc nor desc alphabetical
    fn make_field_sort_store(direction: SortDirection) -> crate::store::memory::MemoryStore {
        use crate::container_service;
        use crate::package::Package;
        use srs_core::types::container::Container;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record::{FieldValues, Record};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::view::{
            DocumentSection, DocumentView, SectionOrdering, SectionSource,
        };

        let heading_field = Field {
            schema: None,
            id: "f-heading".to_string(),
            namespace: "com.test".to_string(),
            name: "heading".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Heading".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let record_type = RecordType {
            id: "t-record".to_string(),
            namespace: "com.test".to_string(),
            name: "item".to_string(),
            version: 1,
            description: "Item".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-heading".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-field-sort".to_string(),
            namespace: "com.test".to_string(),
            name: "field-sort-view".to_string(),
            version: 1,
            description: "View for field ordering".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "items".to_string(),
                title: Some("Items".to_string()),
                description: None,
                order: 0,
                source: SectionSource::ContainerSubset {
                    container_id: "00000000-0000-4000-8000-000000000c01".to_string(),
                    container_type: None,
                    type_filter: None,
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: Some("f-heading".to_string()),
                ordering: Some(SectionOrdering {
                    field_id: Some("f-heading".to_string()),
                    direction: Some(direction),
                }),
                required: None,
                empty_behavior: None,
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-field-sort".to_string(),
            namespace: "com.test".to_string(),
            name: "field-sort-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field],
            record_types: vec![record_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-000000000c01".to_string(),
                title: "Test Container".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        // Fixed UUIDs: 001→"C-last", 002→"A-first", 003→"B-middle"
        // add_member sorts by UUID → stored order: C-last, A-first, B-middle
        let records_data = [
            ("00000000-0000-4000-8000-000000000001", "C-last"),
            ("00000000-0000-4000-8000-000000000002", "A-first"),
            ("00000000-0000-4000-8000-000000000003", "B-middle"),
        ];

        for (id, title) in &records_data {
            let record = Record {
                field_meta: None,
                instance_id: id.to_string(),
                type_id: "t-record".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "item".to_string(),
                field_values: {
                    let mut fv = FieldValues::new();
                    fv.insert("heading", serde_json::json!(title));
                    fv
                },
                lifecycle_state: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                extra: std::collections::BTreeMap::new(),
            };
            let path = format!("records/{}.json", id);
            let value = serde_json::to_value(&record).unwrap();
            store.ensure_instance_dir("records").unwrap();
            store.save_instance_json(&path, &value).unwrap();

            let manifest = store.load_manifest().unwrap();
            store.save_manifest(&manifest).unwrap();

            container_service::add_member(&store, "00000000-0000-4000-8000-000000000c01", id)
                .unwrap();
        }

        store
    }

    #[test]
    fn container_subset_field_ordering_asc_sorts_by_string_value() {
        let store = make_field_sort_store(SortDirection::Asc);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-field-sort",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let rendered = &result.rendered;
        let a_pos = rendered
            .find("A-first")
            .expect("A-first not found in rendered output");
        let b_pos = rendered
            .find("B-middle")
            .expect("B-middle not found in rendered output");
        let c_pos = rendered
            .find("C-last")
            .expect("C-last not found in rendered output");
        assert!(
            a_pos < b_pos && b_pos < c_pos,
            "asc ordering: expected A→B→C, got:\n{}",
            rendered
        );
    }

    #[test]
    fn container_subset_field_ordering_desc_reverses_string_sort() {
        let store = make_field_sort_store(SortDirection::Desc);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-field-sort",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let rendered = &result.rendered;
        let a_pos = rendered
            .find("A-first")
            .expect("A-first not found in rendered output");
        let b_pos = rendered
            .find("B-middle")
            .expect("B-middle not found in rendered output");
        let c_pos = rendered
            .find("C-last")
            .expect("C-last not found in rendered output");
        assert!(
            c_pos < b_pos && b_pos < a_pos,
            "desc ordering: expected C→B→A, got:\n{}",
            rendered
        );
    }

    #[test]
    fn json_projection_container_subset_field_ordering_asc() {
        let store = make_field_sort_store(SortDirection::Asc);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-field-sort",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let projection = result
            .projection
            .expect("json format should produce a projection");
        let section = &projection.sections[0];
        assert_eq!(section.records.len(), 3);
        assert_eq!(
            section.records[0].record_heading.as_deref(),
            Some("A-first"),
            "first record should be A-first in asc order"
        );
        assert_eq!(
            section.records[1].record_heading.as_deref(),
            Some("B-middle"),
            "second record should be B-middle"
        );
        assert_eq!(
            section.records[2].record_heading.as_deref(),
            Some("C-last"),
            "third record should be C-last"
        );
    }

    #[test]
    fn json_projection_container_subset_field_ordering_desc() {
        let store = make_field_sort_store(SortDirection::Desc);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-field-sort",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let projection = result
            .projection
            .expect("json format should produce a projection");
        let section = &projection.sections[0];
        assert_eq!(section.records.len(), 3);
        assert_eq!(
            section.records[0].record_heading.as_deref(),
            Some("C-last"),
            "first record should be C-last in desc order"
        );
        assert_eq!(
            section.records[1].record_heading.as_deref(),
            Some("B-middle"),
            "second record should be B-middle"
        );
        assert_eq!(
            section.records[2].record_heading.as_deref(),
            Some("A-first"),
            "third record should be A-first"
        );
    }

    // ---------------------------------------------------------------------------
    // Theme auto-selection tests (#121)
    // ---------------------------------------------------------------------------

    /// Build a minimal MemoryStore for theme auto-selection tests. The document view has a
    /// themeRef targeting "markdown" plus optional themeVariants. One record with a body field.
    fn make_auto_select_store(
        extra_variants: Vec<srs_core::types::view::ThemeVariant>,
        extra_themes: Vec<srs_core::types::theme::Theme>,
    ) -> crate::store::memory::MemoryStore {
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::theme::{ElementTemplates, Theme};
        use srs_core::types::view::{
            DocumentSection, DocumentView, EmptyBehavior, SectionSource, ThemeMode, ThemeReference,
        };

        let body_field = Field {
            schema: None,
            id: "f-auto-body".to_string(),
            namespace: "com.test".to_string(),
            name: "body".to_string(),
            version: 1,
            field_type: FieldType::text(),
            description: "Body".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let rt = RecordType {
            id: "t-auto".to_string(),
            namespace: "com.test".to_string(),
            name: "section".to_string(),
            version: 1,
            description: "Section".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-auto-body".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        // Base theme targets markdown only.
        let base_theme = Theme {
            id: "theme-auto-base".to_string(),
            namespace: "com.test".to_string(),
            name: "base".to_string(),
            version: 1,
            description: "Markdown base theme".to_string(),
            targets: vec!["markdown".to_string()],
            assets: None,
            css_class_fields: None,
            page_templates: None,
            element_templates: Some(ElementTemplates {
                document_wrapper: None,
                section_wrapper: Some("BASE[{{content}}]".to_string()),
                section_wrapper_overrides: None,
                record_wrapper: None,
                record_wrapper_overrides: None,
                field_row: Some("BASE:{{field-value}}\n".to_string()),
                composite_field_row_templates: None,
                composite_renderer_config: None,
            }),
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let mut themes = vec![base_theme];
        themes.extend(extra_themes);

        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-auto-select".to_string(),
            namespace: "com.test".to_string(),
            name: "auto-select-view".to_string(),
            version: 1,
            description: "Auto-select test view".to_string(),
            container_type: None,
            preamble: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "s-auto".to_string(),
                order: 0,
                title: None,
                description: None,
                title_field_id: None,
                render_view_id: None,
                type_dispatch: None,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "com.test/section".to_string(),
                    lifecycle_state: None,
                    container_ids: None,
                    lifecycle_states: None,
                    exclude_lifecycle_states: None,
                    container_scope: None,
                },
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
            }],
            navigation_links: None,
            theme_ref: Some(ThemeReference {
                mode: ThemeMode::Bundled,
                theme_id: Some("theme-auto-base".to_string()),
                path: None,
                url: None,
            }),
            theme_variants: if extra_variants.is_empty() {
                None
            } else {
                Some(extra_variants)
            },
            format: Some("markdown".to_string()),
            depth_offset: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };

        let package = crate::package::Package {
            id: "00000000-0000-4000-8000-000000000p01".to_string(),
            namespace: "com.test".to_string(),
            name: "test-pkg".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![body_field],
            record_types: vec![rt],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes,
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        crate::store::memory::MemoryStore::new(manifest, package)
    }

    /// Make a theme that targets html with a distinctive document_wrapper so assertions
    /// can confirm the theme was applied even when there are no records to render.
    fn make_html_theme(id: &str, name: &str) -> srs_core::types::theme::Theme {
        use srs_core::types::theme::{ElementTemplates, Theme};
        Theme {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "HTML theme".to_string(),
            targets: vec!["html".to_string()],
            assets: None,
            css_class_fields: None,
            page_templates: None,
            element_templates: Some(ElementTemplates {
                document_wrapper: Some(format!("<div class=\"{name}\">{{{{content}}}}</div>")),
                section_wrapper: None,
                section_wrapper_overrides: None,
                record_wrapper: None,
                record_wrapper_overrides: None,
                field_row: Some(format!("<p class=\"{name}\">{{{{field-value}}}}</p>\n")),
                composite_field_row_templates: None,
                composite_renderer_config: None,
            }),
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn auto_select_theme_variant_by_format() {
        use srs_core::types::view::{ThemeMode, ThemeReference, ThemeVariant};

        let html_theme = make_html_theme("theme-auto-html", "html-prose");
        let variant = ThemeVariant {
            name: "html".to_string(),
            description: None,
            theme_ref: ThemeReference {
                mode: ThemeMode::Bundled,
                theme_id: Some("theme-auto-html".to_string()),
                path: None,
                url: None,
            },
        };
        let store = make_auto_select_store(vec![variant], vec![html_theme]);

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-auto-select",
            format: Some("html"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            !result.diagnostics.iter().any(|d| d.contains("[T-2]")),
            "expected no [T-2] diagnostic when auto-select finds a match, got: {:?}",
            result.diagnostics
        );
        // document_wrapper from the html theme wraps everything in a distinctive div
        assert!(
            result.rendered.contains("class=\"html-prose\""),
            "expected html-prose document_wrapper applied, got: {}",
            result.rendered
        );
    }

    #[test]
    fn auto_select_no_variant_match_emits_t2() {
        use srs_core::types::view::{ThemeMode, ThemeReference, ThemeVariant};

        // Variant targets "text", not "html" — so html request should still get [T-2].
        let text_theme = {
            use srs_core::types::theme::Theme;
            Theme {
                id: "theme-auto-text".to_string(),
                namespace: "com.test".to_string(),
                name: "text-prose".to_string(),
                version: 1,
                description: "Text theme".to_string(),
                targets: vec!["text".to_string()],
                assets: None,
                css_class_fields: None,
                page_templates: None,
                element_templates: None,
                stylesheet: None,
                typography: None,
                tags: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                extra: std::collections::BTreeMap::new(),
            }
        };
        let variant = ThemeVariant {
            name: "text".to_string(),
            description: None,
            theme_ref: ThemeReference {
                mode: ThemeMode::Bundled,
                theme_id: Some("theme-auto-text".to_string()),
                path: None,
                url: None,
            },
        };
        let store = make_auto_select_store(vec![variant], vec![text_theme]);

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-auto-select",
            format: Some("html"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.diagnostics.iter().any(|d| d.contains("[T-2]")),
            "expected [T-2] diagnostic when no variant matches, got: {:?}",
            result.diagnostics
        );
        assert!(
            !result.rendered.contains("BASE:"),
            "expected plain render without theme, got: {}",
            result.rendered
        );
    }

    #[test]
    fn auto_select_multiple_variants_match_uses_first() {
        use srs_core::types::view::{ThemeMode, ThemeReference, ThemeVariant};

        let html_theme_a = make_html_theme("theme-auto-html-a", "html-a");
        let html_theme_b = make_html_theme("theme-auto-html-b", "html-b");
        let variants = vec![
            ThemeVariant {
                name: "html-a".to_string(),
                description: None,
                theme_ref: ThemeReference {
                    mode: ThemeMode::Bundled,
                    theme_id: Some("theme-auto-html-a".to_string()),
                    path: None,
                    url: None,
                },
            },
            ThemeVariant {
                name: "html-b".to_string(),
                description: None,
                theme_ref: ThemeReference {
                    mode: ThemeMode::Bundled,
                    theme_id: Some("theme-auto-html-b".to_string()),
                    path: None,
                    url: None,
                },
            },
        ];
        let store = make_auto_select_store(variants, vec![html_theme_a, html_theme_b]);

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-auto-select",
            format: Some("html"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.diagnostics.iter().any(|d| d.contains("[T-3]")),
            "expected [T-3] ambiguity diagnostic, got: {:?}",
            result.diagnostics
        );
        // document_wrapper from html-a theme should wrap the document
        assert!(
            result.rendered.contains("class=\"html-a\""),
            "expected first variant (html-a) document_wrapper, got: {}",
            result.rendered
        );
        assert!(
            !result.rendered.contains("class=\"html-b\""),
            "expected second variant (html-b) document_wrapper NOT used, got: {}",
            result.rendered
        );
    }

    #[test]
    fn explicit_variant_overrides_auto_select() {
        // When an explicit theme_variant is requested, the existing path is used unchanged —
        // auto-selection does not run.
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000987",
            format: None,
            theme_variant: Some("print"),
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        // The existing fixture view uses a "print" themeVariant — verify it still applies.
        assert!(
            result.rendered.contains("PRINTDOC["),
            "expected explicit variant still applied, got: {}",
            result.rendered
        );
        assert!(
            !result.diagnostics.iter().any(|d| d.contains("[T-3]")),
            "expected no [T-3] ambiguity diagnostic for explicit variant, got: {:?}",
            result.diagnostics
        );
    }

    // ── RFC-008 typeFilter / typeDispatch tests ──────────────────────────────

    const RFC008_CONTAINER_ID: &str = "00000000-0000-4000-8000-000000000c09";

    /// MemoryStore with text + table records in a container, accepting a custom DocumentView.
    /// Fields/types/views are identical to make_hetero_store. Returns (store, text_id, table_id).
    fn make_rfc008_store(
        doc_view: DocumentView,
    ) -> (crate::store::memory::MemoryStore, String, String) {
        use crate::container_service;
        use crate::package::Package;
        use crate::record_store::create_record;
        use crate::relation_service;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::relation::Relation;
        use srs_core::types::view::{FieldView, View};

        let heading_field = Field {
            schema: None,
            id: "f-heading".to_string(),
            namespace: "com.test".to_string(),
            name: "heading".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Heading".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let body_field = Field {
            schema: None,
            id: "f-body".to_string(),
            namespace: "com.test".to_string(),
            name: "body".to_string(),
            version: 1,
            field_type: FieldType::text(),
            description: "Body text".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let caption_field = Field {
            schema: None,
            id: "f-caption".to_string(),
            namespace: "com.test".to_string(),
            name: "caption".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Caption for tables".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let text_type = RecordType {
            id: "t-text".to_string(),
            namespace: "com.test".to_string(),
            name: "section.text".to_string(),
            version: 1,
            description: "Text section".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-heading".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Heading".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "f-body".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Body".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };
        let table_type = RecordType {
            id: "t-table".to_string(),
            namespace: "com.test".to_string(),
            name: "section.table".to_string(),
            version: 1,
            description: "Table section".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-heading".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Heading".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "f-caption".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Caption".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        let text_only_view = View {
            id: "v-text-only".to_string(),
            namespace: "com.test".to_string(),
            name: "text-view".to_string(),
            version: 1,
            description: "View for text sections only".to_string(),
            field_views: vec![FieldView {
                composite_renderer: None,
                field_id: "f-body".to_string(),
                order: 0,
                required: Some(true),
                visible: None,
                display_label: Some("Content".to_string()),
            }],
            compatible_types: Some(vec!["com.test/section.text".to_string()]),
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-rfc008".to_string(),
            namespace: "com.test".to_string(),
            name: "rfc008-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field, body_field, caption_field],
            record_types: vec![text_type, table_type],
            relation_type_definitions: vec![
                srs_core::types::relation_type_definition::RelationTypeDefinition {
                    schema: None,
                    id: "00000000-0000-4000-8000-000000000rt1".to_string(),
                    namespace: "com.test".to_string(),
                    key: "precedes".to_string(),
                    label: "Precedes".to_string(),
                    description: "Ordering relation".to_string(),
                    category:
                        srs_core::types::relation_type_definition::RelationTypeCategory::Sequence,
                    canonical_direction: None,
                    irreflexive: Some(true),
                    inverse_type: None,
                    version: 1,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    allowed_source_types: None,
                    allowed_target_types: None,
                    require_same_semantic_object_type: None,
                    status: None,
                    updated_at: None,
                    properties: None,
                },
            ],
            views: vec![text_only_view],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: RFC008_CONTAINER_ID.to_string(),
                title: "RFC-008 Container".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        let text_record = create_record(
            &store,
            "t-text",
            1,
            {
                let mut fv = srs_core::types::record::FieldValues::new();
                fv.insert("heading", serde_json::json!("Introduction"));
                fv.insert("body", serde_json::json!("The introduction body."));
                fv
            },
            None,
            None,
        )
        .unwrap();
        let text_id = text_record.instance_id.clone();

        let table_record = create_record(
            &store,
            "t-table",
            1,
            {
                let mut fv = srs_core::types::record::FieldValues::new();
                fv.insert("heading", serde_json::json!("Summary Table"));
                fv.insert("caption", serde_json::json!("Table caption here"));
                fv
            },
            None,
            None,
        )
        .unwrap();
        let table_id = table_record.instance_id.clone();

        container_service::add_member(&store, RFC008_CONTAINER_ID, &text_id).unwrap();
        container_service::add_member(&store, RFC008_CONTAINER_ID, &table_id).unwrap();

        relation_service::create_relation_auto(
            &store,
            Relation {
                relation_id: String::new(),
                relation_type: "precedes".to_string(),
                source_instance_id: text_id.clone(),
                target_instance_id: table_id.clone(),
                asserted_by: None,
                confidence: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            },
        )
        .unwrap();

        (store, text_id, table_id)
    }

    /// Build a minimal ContainerSubset DocumentView for RFC-008 tests.
    fn rfc008_doc_view(
        type_filter: Option<Vec<String>>,
        render_view_id: Option<String>,
        type_dispatch: Option<std::collections::BTreeMap<String, String>>,
    ) -> DocumentView {
        use srs_core::types::view::EmptyBehavior;
        DocumentView {
            composite_renderers: None,
            id: "dv-rfc008".to_string(),
            namespace: "com.test".to_string(),
            name: "rfc008-view".to_string(),
            version: 1,
            description: "RFC-008 test document view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "body".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::ContainerSubset {
                    container_id: RFC008_CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter,
                },
                render_view_id,
                type_dispatch,
                title_field_id: Some("f-heading".to_string()),
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn type_filter_restricts_to_matching_types() {
        let view = rfc008_doc_view(Some(vec!["com.test/section.text".to_string()]), None, None);
        let (store, _text_id, _table_id) = make_rfc008_store(view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("Introduction"),
            "text record should appear; got:\n{}",
            result.rendered
        );
        assert!(
            !result.rendered.contains("Summary Table"),
            "table record should be filtered out; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn type_filter_empty_renders_all_members() {
        let view = rfc008_doc_view(Some(vec![]), None, None);
        let (store, _text_id, _table_id) = make_rfc008_store(view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("Introduction"),
            "text record should appear with empty filter; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Summary Table"),
            "table record should appear with empty filter; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn type_filter_absent_renders_all_members() {
        let view = rfc008_doc_view(None, None, None);
        let (store, _text_id, _table_id) = make_rfc008_store(view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("Introduction"),
            "text record should appear without filter; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Summary Table"),
            "table record should appear without filter; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn type_dispatch_selects_per_type_view() {
        // typeDispatch maps the text type to v-text-only (which shows "Content" label).
        // Table type has no dispatch entry → falls back to baseline (shows "Caption" label).
        let mut dispatch = std::collections::BTreeMap::new();
        dispatch.insert(
            "com.test/section.text".to_string(),
            "v-text-only".to_string(),
        );
        let view = rfc008_doc_view(None, None, Some(dispatch));
        let (store, _text_id, _table_id) = make_rfc008_store(view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("Content"),
            "text record should render with view display_label 'Content'; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Caption"),
            "table record should render its own type field 'Caption'; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn type_dispatch_fallback_to_render_view_id() {
        // typeDispatch has no matching key for either type → falls back to render_view_id.
        // render_view_id = v-text-only: text record satisfies it, table does not → diagnostic.
        let mut dispatch = std::collections::BTreeMap::new();
        dispatch.insert(
            "com.test/no-such-type".to_string(),
            "v-text-only".to_string(),
        );
        let view = rfc008_doc_view(None, Some("v-text-only".to_string()), Some(dispatch));
        let (store, _text_id, _table_id) = make_rfc008_store(view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        assert!(
            result.rendered.contains("Content"),
            "text record should apply the fallback view; got:\n{}",
            result.rendered
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[view-dispatch]")),
            "table record should emit a view-dispatch diagnostic; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn type_dispatch_fallback_baseline_when_no_view_id() {
        // No typeDispatch, no render_view_id: every record renders by its own type.
        // No view-dispatch diagnostic should be emitted.
        let view = rfc008_doc_view(None, None, None);
        let (store, _text_id, _table_id) = make_rfc008_store(view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        // Both records render their own type labels ("Body" and "Caption").
        assert!(
            result.rendered.contains("Body"),
            "text record should render its own type field 'Body'; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Caption"),
            "table record should render its own type field 'Caption'; got:\n{}",
            result.rendered
        );
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("[view-dispatch]")),
            "no view-dispatch diagnostic expected; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn cross_type_precedes_ordering_preserved() {
        // Container has: text1 → table → text2 (precedes chain).
        // typeFilter = ["com.test/section.text"]: retains text1, text2 only.
        // Ordering is computed over the full chain then projected, so text1 must appear before text2.
        use crate::container_service;
        use crate::record_store::create_record;
        use crate::relation_service;
        use srs_core::types::relation::Relation;

        let view = rfc008_doc_view(Some(vec!["com.test/section.text".to_string()]), None, None);
        let (store, _text1_id, table_id) = make_rfc008_store(view);

        // Add a second text record ("Conclusion") after the table in the precedes chain.
        let text2_record = create_record(
            &store,
            "t-text",
            1,
            {
                let mut fv = srs_core::types::record::FieldValues::new();
                fv.insert("heading", serde_json::json!("Conclusion"));
                fv.insert("body", serde_json::json!("The conclusion."));
                fv
            },
            None,
            None,
        )
        .unwrap();
        let text2_id = text2_record.instance_id.clone();

        container_service::add_member(&store, RFC008_CONTAINER_ID, &text2_id).unwrap();

        // table → text2 completes the chain: text1 → table → text2
        relation_service::create_relation_auto(
            &store,
            Relation {
                relation_id: String::new(),
                relation_type: "precedes".to_string(),
                source_instance_id: table_id,
                target_instance_id: text2_id,
                asserted_by: None,
                confidence: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            },
        )
        .unwrap();

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");

        let rendered = &result.rendered;
        let intro_pos = rendered
            .find("Introduction")
            .expect("text1 heading 'Introduction' not found");
        let conclusion_pos = rendered
            .find("Conclusion")
            .expect("text2 heading 'Conclusion' not found");
        assert!(
            !rendered.contains("Summary Table"),
            "table record should be filtered out; got:\n{}",
            rendered
        );
        assert!(
            intro_pos < conclusion_pos,
            "Introduction (text1) must appear before Conclusion (text2) after cross-type filter; got:\n{}",
            rendered
        );
    }

    #[test]
    fn type_filter_uses_package_resolved_type_key() {
        // Correct key "com.test/section.text" (package-resolved) → text record appears.
        // A key using denormalized/wrong name should not match.
        let correct_view =
            rfc008_doc_view(Some(vec!["com.test/section.text".to_string()]), None, None);
        let (store, _text_id, _table_id) = make_rfc008_store(correct_view);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            result.rendered.contains("Introduction"),
            "correct package-resolved key should match text record; got:\n{}",
            result.rendered
        );

        // Wrong key: same namespace but wrong name → no records match.
        let wrong_view =
            rfc008_doc_view(Some(vec!["com.test/text-section".to_string()]), None, None);
        let (store2, _text_id2, _table_id2) = make_rfc008_store(wrong_view);
        let result2 = render_document_view(RenderDocumentViewOptions {
            store: &store2,
            view_id: "dv-rfc008",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            !result2.rendered.contains("Introduction"),
            "wrong type key should not match any record; got:\n{}",
            result2.rendered
        );
    }

    // ── RFC-011 lifecycle filter and container scope tests ────────────────────

    /// Build a minimal MemoryStore pre-populated with records at given lifecycle states.
    /// Each record's type is "com.test/decision". Returns the store and the instance IDs.
    fn make_rfc011_store(
        dv: srs_core::types::view::DocumentView,
        records: &[(&str, Option<&str>)], // (instance_id, lifecycle_state)
    ) -> crate::store::memory::MemoryStore {
        use crate::manifest::Manifest;
        use crate::package::Package;
        use crate::store::RepositoryStore;

        let manifest = Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "rfc011-test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "rfc011-test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![dv],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        for (id, state) in records {
            let record = srs_core::types::record::Record {
                field_meta: None,
                instance_id: id.to_string(),
                type_id: "t-decision".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "decision".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: state.map(|s| s.to_string()),
                tags: None,
                created_at: None,
                updated_at: None,
                extra: std::collections::BTreeMap::new(),
            };
            let path = format!("records/{id}.json");
            store
                .save_instance_json(&path, &serde_json::to_value(&record).unwrap())
                .unwrap();
            let manifest = store.load_manifest().unwrap();
            store.save_manifest(&manifest).unwrap();
        }

        store
    }

    fn rfc011_dv(
        dv_id: &str,
        lifecycle_states: Option<Vec<String>>,
        exclude_lifecycle_states: Option<Vec<String>>,
        container_scope: Option<ContainerScope>,
        container_ids: Option<Vec<String>>,
        lifecycle_state: Option<String>,
    ) -> srs_core::types::view::DocumentView {
        srs_core::types::view::DocumentView {
            composite_renderers: None,
            id: dv_id.to_string(),
            namespace: "com.test".to_string(),
            name: dv_id.to_string(),
            version: 1,
            description: "RFC-011 test view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![srs_core::types::view::DocumentSection {
                composite_renderers: None,
                section_id: "s1".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "com.test/decision".to_string(),
                    lifecycle_state,
                    container_ids,
                    lifecycle_states,
                    exclude_lifecycle_states,
                    container_scope,
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: Some("Test".to_string()),
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn rfc011_instance_ids_in_result(result: &RenderResult) -> Vec<String> {
        result
            .projection
            .as_ref()
            .map(|p| {
                p.sections
                    .iter()
                    .flat_map(|s| s.records.iter().map(|r| r.instance_id.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn render_type_query_exclude_lifecycle_states() {
        let dv = rfc011_dv(
            "dv-exclude",
            None,
            Some(vec!["superseded".to_string()]),
            None,
            None,
            None,
        );
        let store = make_rfc011_store(
            dv,
            &[
                ("r-active", Some("active")),
                ("r-superseded", Some("superseded")),
            ],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-exclude",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(
            ids.contains(&"r-active".to_string()),
            "active record should be present: {ids:?}"
        );
        assert!(
            !ids.contains(&"r-superseded".to_string()),
            "superseded record should be excluded: {ids:?}"
        );
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn render_type_query_lifecycle_states_inclusive() {
        let dv = rfc011_dv(
            "dv-include",
            Some(vec!["active".to_string()]),
            None,
            None,
            None,
            None,
        );
        let store = make_rfc011_store(
            dv,
            &[
                ("r-draft", Some("draft")),
                ("r-active", Some("active")),
                ("r-superseded", Some("superseded")),
            ],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-include",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert_eq!(
            ids,
            vec!["r-active"],
            "only active record should be included: {ids:?}"
        );
    }

    /// #532: rendering the same view twice from the same store must be
    /// byte-identical, even when the section members are not linked by any
    /// `precedes` chain and share (or lack) `created_at`. Previously chain
    /// heads fell back to HashMap iteration order and shuffled across runs.
    #[test]
    fn render_document_view_ordering_is_deterministic() {
        // Records deliberately have created_at: None and no precedes relations —
        // the pure-tiebreak case.
        let dv = rfc011_dv("dv-determinism", None, None, None, None, None);
        let store = make_rfc011_store(
            dv,
            &[
                ("r-echo", None),
                ("r-alpha", None),
                ("r-delta", None),
                ("r-charlie", None),
                ("r-bravo", None),
            ],
        );

        let render = |format: &'static str| {
            render_document_view(RenderDocumentViewOptions {
                store: &store,
                view_id: "dv-determinism",
                format: Some(format),
                theme_variant: None,
                container_id: None,
                instance_id_filter: None,
            })
            .expect("render should succeed")
        };

        // Markdown path and JSON projection path must both be stable across runs.
        let md1 = render("markdown");
        let md2 = render("markdown");
        assert_eq!(
            md1.rendered, md2.rendered,
            "markdown render must be byte-identical across runs"
        );

        let json1 = render("json");
        let json2 = render("json");
        let ids1 = rfc011_instance_ids_in_result(&json1);
        let ids2 = rfc011_instance_ids_in_result(&json2);
        assert_eq!(ids1, ids2, "JSON projection order must be stable");

        // Documented tiebreak: created_at ascending, then instance_id ascending.
        // All created_at are absent here, so pure instance_id order applies.
        assert_eq!(
            ids1,
            vec!["r-alpha", "r-bravo", "r-charlie", "r-delta", "r-echo"],
            "records without precedes/created_at must order by instance_id"
        );
    }

    #[test]
    fn render_type_query_no_lifecycle_state_not_excluded() {
        // A record with no lifecycleState must NOT be removed by excludeLifecycleStates.
        let dv = rfc011_dv(
            "dv-no-state-not-excluded",
            None,
            Some(vec!["superseded".to_string()]),
            None,
            None,
            None,
        );
        let store = make_rfc011_store(
            dv,
            &[("r-none", None), ("r-superseded", Some("superseded"))],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-no-state-not-excluded",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(
            ids.contains(&"r-none".to_string()),
            "record with no lifecycleState must not be excluded: {ids:?}"
        );
        assert!(
            !ids.contains(&"r-superseded".to_string()),
            "superseded must be excluded: {ids:?}"
        );
    }

    #[test]
    fn render_type_query_no_lifecycle_state_excluded_by_include() {
        // A record with no lifecycleState IS excluded when lifecycleStates is non-empty.
        let dv = rfc011_dv(
            "dv-no-state-excluded-by-include",
            Some(vec!["active".to_string()]),
            None,
            None,
            None,
            None,
        );
        let store = make_rfc011_store(dv, &[("r-none", None), ("r-active", Some("active"))]);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-no-state-excluded-by-include",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(
            !ids.contains(&"r-none".to_string()),
            "record with no lifecycleState must be excluded by inclusion filter: {ids:?}"
        );
        assert!(
            ids.contains(&"r-active".to_string()),
            "active record must be included: {ids:?}"
        );
    }

    #[test]
    fn render_type_query_repository_scope() {
        // containerScope: "repository" must return records regardless of container.
        // Two containers each with one record — both must be returned.
        use crate::container_service;

        const C1_ID: &str = "00000000-0000-4000-8000-000000000c01";
        const C2_ID: &str = "00000000-0000-4000-8000-000000000c02";
        const R_IN_C1: &str = "00000000-0000-4000-8000-000000000001";
        const R_IN_C2: &str = "00000000-0000-4000-8000-000000000002";

        let dv = rfc011_dv(
            "dv-repo-scope",
            None,
            None,
            Some(ContainerScope::Repository),
            // container_ids narrowed to one container — must be ignored
            Some(vec![C1_ID.to_string()]),
            None,
        );
        let store = make_rfc011_store(dv, &[(R_IN_C1, Some("active")), (R_IN_C2, Some("active"))]);

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: C1_ID.to_string(),
                title: "Container 1".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();
        container_service::add_member(&store, C1_ID, R_IN_C1).unwrap();

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: C2_ID.to_string(),
                title: "Container 2".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();
        container_service::add_member(&store, C2_ID, R_IN_C2).unwrap();

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-repo-scope",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(
            ids.contains(&R_IN_C1.to_string()),
            "r-in-c1 must be in repo-scope result: {ids:?}"
        );
        assert!(
            ids.contains(&R_IN_C2.to_string()),
            "r-in-c2 must be in repo-scope result: {ids:?}"
        );
        assert_eq!(
            ids.len(),
            2,
            "both records must be present with repository scope: {ids:?}"
        );
    }

    #[test]
    fn render_type_query_backcompat_lifecycle_state() {
        // Back-compat: singular lifecycle_state field acts as lifecycleStates: [state].
        let dv = rfc011_dv(
            "dv-backcompat",
            None,
            None,
            None,
            None,
            Some("active".to_string()),
        );
        let store = make_rfc011_store(
            dv,
            &[("r-active", Some("active")), ("r-draft", Some("draft"))],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-backcompat",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert_eq!(
            ids,
            vec!["r-active"],
            "only active record should be included via backcompat filter: {ids:?}"
        );
    }

    #[test]
    fn render_rfc011_cross_store_roundtrip() {
        // Same TypeQuery with lifecycle filter returns the same instance IDs from MemoryStore
        // and from a FileStore backed by a serialised copy of the same data.
        use crate::store::FileStore;

        let dv_id = "dv-roundtrip";
        let dv = rfc011_dv(
            dv_id,
            None,
            Some(vec!["superseded".to_string()]),
            None,
            None,
            None,
        );

        let records: &[(&str, Option<&str>)] = &[
            ("rr-active", Some("active")),
            ("rr-superseded", Some("superseded")),
            ("rr-none", None),
        ];

        // ── MemoryStore result ──────────────────────────────────────────
        let mem_store = make_rfc011_store(dv.clone(), records);
        let mem_result = render_document_view(RenderDocumentViewOptions {
            store: &mem_store,
            view_id: dv_id,
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let mut mem_ids = rfc011_instance_ids_in_result(&mem_result);
        mem_ids.sort();

        // ── FileStore result ────────────────────────────────────────────
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Write records
        std::fs::create_dir_all(repo_root.join("records")).unwrap();
        for (id, state) in records {
            let record = srs_core::types::record::Record {
                field_meta: None,
                instance_id: id.to_string(),
                type_id: "t-decision".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "decision".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: state.map(|s| s.to_string()),
                tags: None,
                created_at: None,
                updated_at: None,
                extra: std::collections::BTreeMap::new(),
            };
            let path = format!("records/{id}.json");
            std::fs::write(
                repo_root.join(&path),
                serde_json::to_string_pretty(&record).unwrap(),
            )
            .unwrap();
        }

        // Write manifest.json — membership is the tree (RFC-038 [R1]).
        std::fs::write(
            repo_root.join("manifest.json"),
            serde_json::to_string_pretty(&serde_json::json!({"dataModelRevision": 2})).unwrap(),
        )
        .unwrap();

        // Write DocumentView as a separate file (FileStore loads via path references).
        std::fs::create_dir_all(repo_root.join("package/document-views")).unwrap();
        let dv_json = serde_json::to_value(&dv).unwrap();
        std::fs::write(
            repo_root.join("package/document-views/dv-roundtrip.json"),
            serde_json::to_string_pretty(&dv_json).unwrap(),
        )
        .unwrap();

        // Write package.json with path reference to the view file.
        std::fs::write(
            repo_root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "rfc011-file-pkg",
                "namespace": "com.test",
                "name": "rfc011-file",
                "version": "1.0.0",
                "documentViews": ["document-views/dv-roundtrip.json"],
            }))
            .unwrap(),
        )
        .unwrap();

        let file_store = FileStore::new(repo_root);
        let file_result = render_document_view(RenderDocumentViewOptions {
            store: &file_store,
            view_id: dv_id,
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let mut file_ids = rfc011_instance_ids_in_result(&file_result);
        file_ids.sort();

        assert_eq!(
            mem_ids, file_ids,
            "MemoryStore and FileStore must return the same instance IDs for the same lifecycle filter"
        );
        // Both should have active + none, not superseded
        assert!(mem_ids.contains(&"rr-active".to_string()));
        assert!(mem_ids.contains(&"rr-none".to_string()));
        assert!(!mem_ids.contains(&"rr-superseded".to_string()));
    }

    #[test]
    fn render_type_query_lifecycle_states_precedence_over_lifecycle_state() {
        // When both lifecycle_state and lifecycle_states are set, lifecycle_states wins.
        // lifecycle_state: "draft" would include the draft record;
        // lifecycle_states: ["active"] must override it and include only the active record.
        let dv = rfc011_dv(
            "dv-precedence",
            Some(vec!["active".to_string()]),
            None,
            None,
            None,
            Some("draft".to_string()),
        );
        let store = make_rfc011_store(
            dv,
            &[("r-active", Some("active")), ("r-draft", Some("draft"))],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-precedence",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert_eq!(
            ids,
            vec!["r-active"],
            "lifecycle_states must take precedence over lifecycle_state: {ids:?}"
        );
    }

    #[test]
    fn render_type_query_repository_scope_ignores_cli_container() {
        // containerScope: "repository" must ignore the cli container_id override,
        // returning all records regardless of the scoping argument.
        use crate::container_service;

        const C1_ID: &str = "00000000-0000-4000-8000-000000000d01";
        const C2_ID: &str = "00000000-0000-4000-8000-000000000d02";
        const R_IN_C1: &str = "00000000-0000-4000-8000-000000000011";
        const R_IN_C2: &str = "00000000-0000-4000-8000-000000000012";

        let dv = rfc011_dv(
            "dv-repo-cli-ignore",
            None,
            None,
            Some(ContainerScope::Repository),
            None,
            None,
        );
        let store = make_rfc011_store(dv, &[(R_IN_C1, Some("active")), (R_IN_C2, Some("active"))]);

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: C1_ID.to_string(),
                title: "Container 1".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();
        container_service::add_member(&store, C1_ID, R_IN_C1).unwrap();

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: C2_ID.to_string(),
                title: "Container 2".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();
        container_service::add_member(&store, C2_ID, R_IN_C2).unwrap();

        // Pass C1_ID as cli container_id — in repository scope, this must be ignored.
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-repo-cli-ignore",
            format: Some("json"),
            theme_variant: None,
            container_id: Some(C1_ID),
            instance_id_filter: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(
            ids.contains(&R_IN_C1.to_string()),
            "r-in-c1 must be present: {ids:?}"
        );
        assert!(
            ids.contains(&R_IN_C2.to_string()),
            "r-in-c2 must be present even though cli_container_id={C1_ID}: {ids:?}"
        );
        assert_eq!(
            ids.len(),
            2,
            "both records must appear with repository scope: {ids:?}"
        );
    }

    // ── #697 type-query zero-records diagnostic ───────────────────────────────

    #[test]
    fn type_query_zero_records_emits_diagnostic_markdown() {
        // #697: a TypeQuery that matches 0 records must emit a warning diagnostic.
        // The section output is still empty (emptyBehavior hide / default).
        let dv = rfc011_dv("dv-zero", None, None, None, None, None);
        let store = make_rfc011_store(dv, &[]); // no records of com.test/decision
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-zero",
            format: Some("markdown"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        assert!(
            result.diagnostics.iter().any(|d| d.contains("[section:s1]")
                && d.contains("type-query")
                && d.contains("com.test/decision")
                && d.contains("matched 0 records")),
            "expected zero-records diagnostic; got: {:?}",
            result.diagnostics
        );
        // Output should still be empty (only the preamble, no section content).
        assert!(
            !result.rendered.contains("decision"),
            "section content should be empty; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn type_query_zero_records_emits_diagnostic_json() {
        // #697: the JSON projection path must also emit the diagnostic.
        let dv = rfc011_dv("dv-zero-json", None, None, None, None, None);
        let store = make_rfc011_store(dv, &[]);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-zero-json",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        assert!(
            result.diagnostics.iter().any(|d| d.contains("[section:s1]")
                && d.contains("type-query")
                && d.contains("com.test/decision")
                && d.contains("matched 0 records")),
            "expected zero-records diagnostic on JSON path; got: {:?}",
            result.diagnostics
        );
        let projection = result
            .projection
            .expect("JSON path must produce a projection");
        assert!(
            projection.sections[0].records.is_empty(),
            "section records should be empty; got: {:?}",
            projection.sections[0].records
        );
    }

    #[test]
    fn type_query_zero_records_empty_behavior_hide_still_warns() {
        // #697: emptyBehavior:hide suppresses the output block, not the diagnostic.
        use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};

        let dv = DocumentView {
            composite_renderers: None,
            id: "dv-zero-hide".to_string(),
            namespace: "com.test".to_string(),
            name: "dv-zero-hide".to_string(),
            version: 1,
            description: "zero-records hide test".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "s-hide".to_string(),
                title: Some("Hidden Section".to_string()),
                description: None,
                order: 0,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "com.test/decision".to_string(),
                    lifecycle_state: None,
                    container_ids: None,
                    lifecycle_states: None,
                    exclude_lifecycle_states: None,
                    container_scope: None,
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };
        let store = make_rfc011_store(dv, &[]);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-zero-hide",
            format: Some("markdown"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[section:s-hide]")
                    && d.contains("type-query")
                    && d.contains("matched 0 records")),
            "emptyBehavior:hide must not suppress the diagnostic; got: {:?}",
            result.diagnostics
        );
        assert!(
            !result.rendered.contains("Hidden Section"),
            "emptyBehavior:hide must still hide the section title from rendered output; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn type_query_nonzero_records_no_spurious_diagnostic() {
        // Confirm the zero-records diagnostic is absent when the TypeQuery has matches.
        let dv = rfc011_dv("dv-nonzero", None, None, None, None, None);
        let store = make_rfc011_store(dv, &[("r1", None)]); // one record of com.test/decision
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-nonzero",
            format: Some("markdown"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("matched 0 records")),
            "no zero-records diagnostic expected when TypeQuery has matches; got: {:?}",
            result.diagnostics
        );
    }

    // ── Rule [N+37] identity field heading fallback tests ──────────────────────

    /// Minimal store for Rule [N+37] tests. Returns (store, view_no_title_id,
    /// view_with_title_id, view_no_identity_id, view_json_id).
    fn make_identity_fallback_store() -> (
        crate::store::memory::MemoryStore,
        String,
        String,
        String,
        String,
        String,
    ) {
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};

        let heading_field = Field {
            schema: None,
            id: "f-head".to_string(),
            namespace: "com.test".to_string(),
            name: "heading".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Heading field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let other_field = Field {
            schema: None,
            id: "f-other".to_string(),
            namespace: "com.test".to_string(),
            name: "other".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Other field (used as explicit titleFieldId in precedence test)"
                .to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        // Ineligible under `[N+1]` (closed value domain), used to prove the owner's
        // omit-not-substitute disposition (srs PR #341): a Type carrying both an
        // ineligible authored `titleFieldId` *and* an `identityFieldId` must emit no
        // heading at all, not fall through to `identityFieldId`.
        let closed_field = Field {
            schema: None,
            id: "f-closed".to_string(),
            namespace: "com.test".to_string(),
            name: "closed".to_string(),
            version: 1,
            field_type: FieldType::closed(),
            description: "Closed-domain field, ineligible as titleFieldId under [N+1]".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let identity_type = RecordType {
            id: "t-identity".to_string(),
            namespace: "com.test".to_string(),
            name: "identity-type".to_string(),
            version: 1,
            description: "Type with identityFieldId pointing to f-head".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-head".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Heading".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "f-other".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Other".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "f-closed".to_string(),
                    order: 2,
                    required: false,
                    display_label: Some("Closed".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: Some("f-head".to_string()),
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        // Type WITHOUT identityFieldId (for the no-heading regression test)
        let plain_type = RecordType {
            id: "t-plain".to_string(),
            namespace: "com.test".to_string(),
            name: "plain-type".to_string(),
            version: 1,
            description: "Type without identityFieldId".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-head".to_string(),
                order: 0,
                required: true,
                display_label: Some("Heading".to_string()),
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        let make_type_query_section = |sot: &str, title_field_id: Option<String>| DocumentSection {
            composite_renderers: None,
            section_id: "items".to_string(),
            title: Some("Items".to_string()),
            description: None,
            order: 0,
            source: SectionSource::TypeQuery {
                semantic_object_type: sot.to_string(),
                lifecycle_state: None,
                container_ids: None,
                lifecycle_states: None,
                exclude_lifecycle_states: None,
                container_scope: None,
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id,
            ordering: None,
            required: None,
            empty_behavior: Some(EmptyBehavior::Hide),
            relations_presentation: None,
        };

        let make_dv = |id: &str, name: &str, section: DocumentSection, format: &str| DocumentView {
            composite_renderers: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: name.to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![section],
            navigation_links: None,
            preamble: None,
            format: Some(format.to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let dv_no_title = make_dv(
            "dv-identity-fallback",
            "identity-fallback",
            make_type_query_section("com.test/identity-type", None),
            "markdown",
        );
        let dv_with_title = make_dv(
            "dv-title-takes-precedence",
            "title-takes-precedence",
            make_type_query_section("com.test/identity-type", Some("f-other".to_string())),
            "markdown",
        );
        let dv_no_identity = make_dv(
            "dv-no-identity",
            "no-identity",
            make_type_query_section("com.test/plain-type", None),
            "markdown",
        );
        let dv_json = make_dv(
            "dv-identity-json",
            "identity-json",
            make_type_query_section("com.test/identity-type", None),
            "json",
        );
        let dv_ineligible_title_with_identity = make_dv(
            "dv-ineligible-title-with-identity",
            "ineligible-title-with-identity",
            make_type_query_section("com.test/identity-type", Some("f-closed".to_string())),
            "markdown",
        );

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-identity".to_string(),
            namespace: "com.test".to_string(),
            name: "identity-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field, other_field, closed_field],
            record_types: vec![identity_type, plain_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![
                dv_no_title,
                dv_with_title,
                dv_no_identity,
                dv_json,
                dv_ineligible_title_with_identity,
            ],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        let fv_identity = {
            let mut fv = srs_core::types::record::FieldValues::new();
            fv.insert("heading", serde_json::json!("My Identity Heading"));
            fv.insert("other", serde_json::json!("Other Title"));
            fv.insert("closed", serde_json::json!("Closed Value"));
            fv
        };
        create_record(&store, "t-identity", 1, fv_identity, None, None).unwrap();

        let fv_plain = {
            let mut fv = srs_core::types::record::FieldValues::new();
            fv.insert("heading", serde_json::json!("Plain Record"));
            fv
        };
        create_record(&store, "t-plain", 1, fv_plain, None, None).unwrap();

        (
            store,
            "dv-identity-fallback".to_string(),
            "dv-title-takes-precedence".to_string(),
            "dv-no-identity".to_string(),
            "dv-identity-json".to_string(),
            "dv-ineligible-title-with-identity".to_string(),
        )
    }

    #[test]
    fn identity_field_id_fallback_emits_heading_markdown() {
        let (store, view_id, _, _, _, _) = make_identity_fallback_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            result.rendered.contains("### My Identity Heading"),
            "expected H3 heading from identityFieldId fallback (Rule [N+37]); got: {}",
            result.rendered
        );
        // Structured mode must NOT be activated by the identity fallback: the identity field
        // must still appear in the body field table (it is not skipped).
        assert!(
            result.rendered.contains("My Identity Heading")
                && result
                    .rendered
                    .lines()
                    .filter(|l| l.contains("My Identity Heading"))
                    .count()
                    >= 2,
            "identity field value must appear in both the heading and the body (structured mode off); got: {}",
            result.rendered
        );
    }

    #[test]
    fn title_field_id_takes_precedence_over_identity_field_id() {
        let (store, _, view_id, _, _, _) = make_identity_fallback_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        // titleFieldId=f-other → "Other Title"; identityFieldId (f-head) → "My Identity Heading"
        assert!(
            result.rendered.contains("### Other Title"),
            "expected heading from titleFieldId, not identityFieldId; got: {}",
            result.rendered
        );
        assert!(
            !result.rendered.contains("### My Identity Heading"),
            "identityFieldId heading must not appear when titleFieldId is set; got: {}",
            result.rendered
        );
    }

    #[test]
    fn no_identity_field_id_no_title_field_id_no_heading() {
        let (store, _, _, view_id, _, _) = make_identity_fallback_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            !result.rendered.contains("### "),
            "expected no H3 heading when both titleFieldId and identityFieldId are absent; got: {}",
            result.rendered
        );
    }

    #[test]
    fn identity_field_id_fallback_filestore_roundtrip() {
        let repo_root = render_identity_fixture_root();
        let store = FileStore::new(&repo_root);
        // identity-fallback-view: TypeQuery for fixture.render/identity-item, no titleFieldId
        // identity-item type has identityFieldId → heading field
        // The identity record's heading = "identity heading value"
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000985",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            result.rendered.contains("### identity heading value"),
            "expected H3 heading from identityFieldId via FileStore (Rule [N+37]); got: {}",
            result.rendered
        );
    }

    #[test]
    fn identity_field_id_fallback_record_heading_json() {
        let (store, _, _, _, view_id, _) = make_identity_fallback_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        let projection = result
            .projection
            .expect("json format should produce a projection");
        assert_eq!(
            projection.sections[0].records[0].record_heading.as_deref(),
            Some("My Identity Heading"),
            "record_heading should be populated from identityFieldId fallback in JSON projection"
        );
    }

    /// `[N+1]` ineligibility consequence, owner-decided (srs PR #341, 2026-08-02):
    /// an authored-but-ineligible `titleFieldId` must **omit** the heading, and
    /// must **not** fall through to the Type's `identityFieldId` even when one is
    /// present. `n1_repeatable_title_field_id_emits_no_record_heading` cannot
    /// discriminate this — its Type has no `identityFieldId`, so omission (b) and
    /// fall-through (a) are indistinguishable there. This fixture's Type carries
    /// both an ineligible `titleFieldId` (`f-closed`, closed value domain) *and*
    /// `identityFieldId: f-head`, so the two readings diverge and only (b) survives.
    #[test]
    fn n1_ineligible_title_field_id_omits_heading_without_identity_fallback() {
        let (store, _, _, _, _, view_id) = make_identity_fallback_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed");
        assert!(
            !result.rendered.contains("### "),
            "an ineligible titleFieldId must omit the heading rather than falling \
             through to identityFieldId; got: {}",
            result.rendered
        );
        // `identityFieldId`'s value ("My Identity Heading") legitimately still
        // appears as an ordinary body row — `f-head` is a ordinary required field.
        // What must NOT happen is that value being *promoted to a heading*, which
        // the "### " check above already rules out. Assert the negative directly
        // against the substituted heading form, for a self-explaining failure.
        assert!(
            !result.rendered.contains("### My Identity Heading"),
            "identityFieldId must NOT be substituted for an ineligible authored \
             titleFieldId (rejects reading (a)); got: {}",
            result.rendered
        );
        // Refusing it as a heading must not delete its value — same data-loss
        // guard as the repeatable case.
        assert!(
            result.rendered.contains("Closed Value"),
            "the ineligible title field's value must survive as an ordinary field row; got: {}",
            result.rendered
        );
    }

    // --- resolve_container_title unit tests (Phase 1 milestone) ---

    fn minimal_document_view() -> srs_core::types::view::DocumentView {
        srs_core::types::view::DocumentView {
            composite_renderers: None,
            id: "dv-test".to_string(),
            namespace: "com.test".to_string(),
            name: "test-view".to_string(),
            version: 1,
            description: "Test document view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![],
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn minimal_manifest_no_index() -> crate::manifest::Manifest {
        crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        }
    }

    #[test]
    fn resolve_container_title_falls_back_to_container_file() {
        // Container is present in the store's data but NOT in manifest.containerIndex.
        // This is the bug scenario: older repos with no containerIndex.
        use crate::store::memory::MemoryStore;
        use srs_core::types::container::Container;

        let store = MemoryStore::empty();
        let container = Container {
            container_id: "d0c8cba0-test-0001-0000-000000000001".to_string(),
            title: "Recognising decisions".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        store
            .save_container(&container)
            .expect("save_container must succeed");

        // Build a manifest with no containerIndex to simulate the bug scenario.
        // Note: save_container updates the store's internal manifest, but we own
        // this manifest separately — clearing container_index here proves the fix
        // reads the container file, not the index.
        let manifest = minimal_manifest_no_index();

        let dv = minimal_document_view();
        let title = resolve_container_title(
            &store,
            &dv,
            &manifest,
            Some("d0c8cba0-test-0001-0000-000000000001"),
        );
        assert_eq!(
            title, "Recognising decisions",
            "should return the container file title when containerIndex is absent"
        );
    }

    #[test]
    fn resolve_container_title_loads_container_directly() {
        // RFC-038 Change K retires manifest.containerIndex: a requested container_id
        // resolves by loading the container directly (catalog-backed), not through
        // an index-cached title.
        use crate::store::memory::MemoryStore;
        use srs_core::types::container::Container;

        let store = MemoryStore::empty();
        let container = Container {
            container_id: "idx-test-cid".to_string(),
            title: "File Title".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        store
            .save_container(&container)
            .expect("save_container must succeed");

        let manifest = minimal_manifest_no_index();

        let dv = minimal_document_view();
        let title = resolve_container_title(&store, &dv, &manifest, Some("idx-test-cid"));
        assert_eq!(title, "File Title");
    }

    #[test]
    fn resolve_container_title_no_container_id_falls_back_to_manifest() {
        // Regression: when no container_id is given and index is absent, repo title is used.
        use crate::store::memory::MemoryStore;

        let store = MemoryStore::empty();
        let mut manifest = minimal_manifest_no_index();
        manifest
            .extra
            .insert("title".to_string(), serde_json::json!("My Repo"));

        let dv = minimal_document_view();
        let title = resolve_container_title(&store, &dv, &manifest, None);
        assert_eq!(
            title, "My Repo",
            "should fall through to manifest title when no container_id is given"
        );
    }

    #[test]
    fn resolve_container_title_filestore_falls_back_to_container_file() {
        // Cross-store test: the container file under containers/ carries the
        // title; resolution discovers it through the store (catalog-backed),
        // no containerIndex involved (RFC-038 Change J).
        use crate::store::FileStore;
        use srs_core::types::container::Container;

        let tmp = tempfile::TempDir::new().unwrap();
        let repo_root = tmp.path();

        let cid = "fs-test-cid-0001";
        let container_path = "containers/fs-test-cid-0001.json";

        std::fs::create_dir_all(repo_root.join("containers")).unwrap();
        let container = Container {
            container_id: cid.to_string(),
            title: "FileStore Fallback Title".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        std::fs::write(
            repo_root.join(container_path),
            serde_json::to_string_pretty(&container).unwrap(),
        )
        .unwrap();

        std::fs::write(
            repo_root.join("manifest.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "dataModelRevision": 2
            }))
            .unwrap(),
        )
        .unwrap();

        let store = FileStore::new(repo_root);

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: repo_root.to_path_buf(),
        };

        let dv = minimal_document_view();
        let title = resolve_container_title(&store, &dv, &manifest, Some(cid));
        assert_eq!(
            title, "FileStore Fallback Title",
            "FileStore should return the container file title when index entry has no title"
        );
    }

    // ── #509 / #510: per-section degradation + Tier-0 note members ───────────

    const GOOD_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000c0de";
    const DANGLING_CONTAINER_ID: &str = "00000000-0000-4000-8000-0000000dead0";

    fn simple_field_and_type() -> (
        srs_core::types::field::Field,
        srs_core::types::record_type::RecordType,
    ) {
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let heading_field = Field {
            schema: None,
            id: "f-heading".to_string(),
            namespace: "com.test".to_string(),
            name: "heading".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Heading".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let item_type = RecordType {
            id: "t-item".to_string(),
            namespace: "com.test".to_string(),
            name: "item".to_string(),
            version: 1,
            description: "Item".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-heading".to_string(),
                order: 0,
                required: true,
                display_label: Some("Heading".to_string()),
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };
        (heading_field, item_type)
    }

    fn container_subset_section(
        section_id: &str,
        title: &str,
        order: i32,
        container_id: &str,
        required: Option<bool>,
    ) -> srs_core::types::view::DocumentSection {
        use srs_core::types::view::{DocumentSection, EmptyBehavior, SectionSource};
        DocumentSection {
            composite_renderers: None,
            section_id: section_id.to_string(),
            title: Some(title.to_string()),
            description: None,
            order,
            source: SectionSource::ContainerSubset {
                container_id: container_id.to_string(),
                container_type: None,
                type_filter: None,
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id: Some("f-heading".to_string()),
            ordering: None,
            required,
            empty_behavior: Some(EmptyBehavior::Hide),
            relations_presentation: None,
        }
    }

    /// MemoryStore with one container holding one record, and a document view with
    /// two ContainerSubset sections: "good" (resolvable) and "missing" (dangling).
    fn make_degradation_store(missing_required: Option<bool>) -> crate::store::memory::MemoryStore {
        use crate::container_service;
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::view::DocumentView;

        let (heading_field, item_type) = simple_field_and_type();
        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-degrade".to_string(),
            namespace: "com.test".to_string(),
            name: "degrade-view".to_string(),
            version: 1,
            description: "Degradation test view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![
                container_subset_section("good", "Good Section", 0, GOOD_CONTAINER_ID, None),
                container_subset_section(
                    "missing",
                    "Missing Section",
                    1,
                    DANGLING_CONTAINER_ID,
                    missing_required,
                ),
            ],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-degrade".to_string(),
            namespace: "com.test".to_string(),
            name: "degrade-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field],
            record_types: vec![item_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: GOOD_CONTAINER_ID.to_string(),
                title: "Good Container".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        let fv = {
            let mut fv = srs_core::types::record::FieldValues::new();
            fv.insert("heading", serde_json::json!("Alpha Record"));
            fv
        };
        let record = create_record(&store, "t-item", 1, fv, None, None).unwrap();
        container_service::add_member(&store, GOOD_CONTAINER_ID, &record.instance_id).unwrap();

        store
    }

    #[test]
    fn dangling_container_section_degrades_to_empty_with_warning() {
        // #509: a section whose containerId does not resolve renders as empty
        // (emptyBehavior hide → the section is omitted) with a warning diagnostic,
        // while the resolvable section still renders.
        let store = make_degradation_store(None);
        let result = render_document_view(RenderDocumentViewOptions::new(&store, "dv-degrade"))
            .expect("render must not fail on a dangling section containerId");

        assert!(
            result.rendered.contains("Good Section"),
            "resolvable section must render; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Alpha Record"),
            "resolvable section's record must render; got:\n{}",
            result.rendered
        );
        assert!(
            !result.rendered.contains("Missing Section"),
            "emptyBehavior hide: dangling section must be omitted; got:\n{}",
            result.rendered
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[section:missing]")
                    && d.contains(&format!("container not found: {DANGLING_CONTAINER_ID}"))),
            "expected a per-section container-not-found warning; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn dangling_container_required_section_renders_no_records_placeholder() {
        // #509: a required section over a dangling container degrades to the same
        // output as a genuinely empty required section ("No records.").
        let store = make_degradation_store(Some(true));
        let result = render_document_view(RenderDocumentViewOptions::new(&store, "dv-degrade"))
            .expect("render must not fail on a dangling section containerId");

        assert!(
            result.rendered.contains("Missing Section"),
            "required section title must render even when the container is dangling; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("No records."),
            "required empty section renders its placeholder; got:\n{}",
            result.rendered
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("container not found")),
            "expected container-not-found warning; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn json_projection_degrades_dangling_container_section() {
        // #509: the JSON projection path shares resolve_section_instances — a dangling
        // containerId yields an empty section plus a warning, not a failed render.
        let store = make_degradation_store(None);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-degrade",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("json projection must not fail on a dangling section containerId");

        let projection = result.projection.expect("json format returns a projection");
        let missing = projection
            .sections
            .iter()
            .find(|s| s.section_id == "missing")
            .expect("dangling section still present in projection");
        assert!(missing.records.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("container not found")),
            "expected container-not-found warning; got: {:?}",
            result.diagnostics
        );
    }

    /// MemoryStore whose container has a Tier-0 note as root and a typed record
    /// as member — the RFC-013 shape that `repo create` scaffolds.
    fn make_note_member_store() -> (crate::store::memory::MemoryStore, String, String) {
        use crate::container_service;
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::note::{Note, NoteSection};
        use srs_core::types::view::DocumentView;

        let (heading_field, item_type) = simple_field_and_type();
        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-notes".to_string(),
            namespace: "com.test".to_string(),
            name: "notes-view".to_string(),
            version: 1,
            description: "Note-member test view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![container_subset_section(
                "body",
                "Guide Body",
                0,
                GOOD_CONTAINER_ID,
                None,
            )],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-notes".to_string(),
            namespace: "com.test".to_string(),
            name: "notes-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field],
            record_types: vec![item_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: GOOD_CONTAINER_ID.to_string(),
                title: "Guides".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: Some("guide".to_string()),
                identity_instance_id: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        // Tier-0 identity note as container root (RFC-013 shape).
        let note = crate::services::create_note(
            &store,
            Note {
                instance_id: String::new(),
                title: Some("Guides".to_string()),
                tags: None,
                sections: vec![NoteSection {
                    name: "overview".to_string(),
                    label: Some("Overview".to_string()),
                    content: "Guide overview text.".to_string(),
                    content_hint: None,
                    tags: None,
                }],
                graduated_at: None,
                source_refs: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
            },
        )
        .unwrap()
        .note;
        container_service::add_root(&store, GOOD_CONTAINER_ID, &note.instance_id).unwrap();

        let fv = {
            let mut fv = srs_core::types::record::FieldValues::new();
            fv.insert("heading", serde_json::json!("Typed Member"));
            fv
        };
        let record = create_record(&store, "t-item", 1, fv, None, None).unwrap();
        container_service::add_member(&store, GOOD_CONTAINER_ID, &record.instance_id).unwrap();

        (store, note.instance_id.clone(), record.instance_id.clone())
    }

    #[test]
    fn container_subset_with_tier0_note_root_renders_note_and_records() {
        // #510: a Tier-0 note root/member renders through its note shape (title as
        // heading, section content as body) instead of failing the render.
        let (store, _note_id, _record_id) = make_note_member_store();
        let result = render_document_view(RenderDocumentViewOptions::new(&store, "dv-notes"))
            .expect("render must not fail on a tier-0 note container member");

        assert!(
            result.rendered.contains("Guides"),
            "note title must render as a heading; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Guide overview text."),
            "note section content must render as body text; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Typed Member"),
            "typed record member must still render; got:\n{}",
            result.rendered
        );
    }

    #[test]
    fn json_projection_skips_tier0_note_with_warning() {
        // #510 (JSON path): the document-view JSON output schema models typed records
        // only, so tier-0 notes are skipped with a warning instead of failing.
        let (store, note_id, record_id) = make_note_member_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-notes",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("json projection must not fail on a tier-0 note container member");

        let projection = result.projection.expect("json format returns a projection");
        let ids: Vec<&str> = projection
            .sections
            .iter()
            .flat_map(|s| s.records.iter().map(|r| r.instance_id.as_str()))
            .collect();
        assert!(
            ids.contains(&record_id.as_str()),
            "record projected: {ids:?}"
        );
        assert!(
            !ids.contains(&note_id.as_str()),
            "note not projected: {ids:?}"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains(&note_id) && d.contains("not representable")),
            "expected a note-skipped warning; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn fixed_instances_arm_skips_tier0_note_with_diagnostic() {
        // #527: a FixedInstances section referencing a Tier-0 note must skip it with a
        // diagnostic rather than pushing it into the records Vec.
        use crate::record_store::create_record;
        use srs_core::types::note::{Note, NoteSection};
        use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};

        const NOTE_ID: &str = "00000000-0000-4000-8000-0000000f1001";

        let (heading_field, item_type) = simple_field_and_type();
        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-fixed-note".to_string(),
            namespace: "com.test".to_string(),
            name: "fixed-note-view".to_string(),
            version: 1,
            description: "FixedInstances note guard regression test".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![
                // Section under test: FixedInstances references a Tier-0 note
                DocumentSection {
                    composite_renderers: None,
                    section_id: "fixed-sec".to_string(),
                    title: Some("Fixed".to_string()),
                    description: None,
                    order: 0,
                    source: SectionSource::FixedInstances {
                        instance_ids: vec![NOTE_ID.to_string()],
                    },
                    render_view_id: None,
                    type_dispatch: None,
                    title_field_id: Some("f-heading".to_string()),
                    ordering: None,
                    required: None,
                    empty_behavior: Some(EmptyBehavior::Hide),
                    relations_presentation: None,
                },
                // TypeQuery section: proves typed records still render (no regression)
                DocumentSection {
                    composite_renderers: None,
                    section_id: "typed-sec".to_string(),
                    title: Some("Items".to_string()),
                    description: None,
                    order: 1,
                    source: SectionSource::TypeQuery {
                        semantic_object_type: "com.test/item".to_string(),
                        lifecycle_state: None,
                        container_ids: None,
                        lifecycle_states: None,
                        exclude_lifecycle_states: None,
                        container_scope: None,
                    },
                    render_view_id: None,
                    type_dispatch: None,
                    title_field_id: Some("f-heading".to_string()),
                    ordering: None,
                    required: None,
                    empty_behavior: Some(EmptyBehavior::Hide),
                    relations_presentation: None,
                },
            ],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = crate::package::Package {
            id: "pkg-fixed-note".to_string(),
            namespace: "com.test".to_string(),
            name: "fixed-note-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field],
            record_types: vec![item_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        crate::services::create_note(
            &store,
            Note {
                instance_id: NOTE_ID.to_string(),
                title: Some("Skipped Note Title".to_string()),
                tags: None,
                sections: vec![NoteSection {
                    name: "body".to_string(),
                    label: None,
                    content: "This note must not appear in FixedInstances output.".to_string(),
                    content_hint: None,
                    tags: None,
                }],
                graduated_at: None,
                source_refs: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
            },
        )
        .unwrap();

        let fv = {
            let mut fv = srs_core::types::record::FieldValues::new();
            fv.insert("heading", serde_json::json!("Typed Item"));
            fv
        };
        create_record(&store, "t-item", 1, fv, None, None).unwrap();

        let result = render_document_view(RenderDocumentViewOptions::new(&store, "dv-fixed-note"))
            .expect("render must not fail when FixedInstances references a Tier-0 note");

        assert!(
            !result.rendered.contains("Skipped Note Title"),
            "note title must NOT appear in FixedInstances output; got:\n{}",
            result.rendered
        );
        assert!(
            result.rendered.contains("Typed Item"),
            "typed record must still render via TypeQuery section; got:\n{}",
            result.rendered
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[section:fixed-sec]")
                    && d.contains("FixedInstances:")
                    && d.contains(NOTE_ID)
                    && d.contains("notes are not rendered in document-view sections")),
            "expected FixedInstances note-skip diagnostic; got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn relation_query_arm_skips_tier0_note_with_diagnostic() {
        // #527: a RelationQuery section whose relations resolve to a Tier-0 note must skip
        // it with a diagnostic rather than pushing it into the records Vec.
        use crate::relation_service;
        use srs_core::types::note::{Note, NoteSection};
        use srs_core::types::relation::Relation;
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };
        use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};

        // Pre-specified IDs so they are known before the DocumentView is built.
        const SOURCE_ID: &str = "00000000-0000-4000-8000-000000009001";
        const NOTE_ID: &str = "00000000-0000-4000-8000-000000009002";
        const SECTION_ID: &str = "rq-sec";

        let (heading_field, item_type) = simple_field_and_type();
        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-rq-note".to_string(),
            namespace: "com.test".to_string(),
            name: "rq-note-view".to_string(),
            version: 1,
            description: "RelationQuery note guard regression test".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: SECTION_ID.to_string(),
                title: Some("Related".to_string()),
                description: None,
                order: 0,
                source: SectionSource::RelationQuery {
                    from_instance_id: SOURCE_ID.to_string(),
                    relation_type: "refers-to".to_string(),
                    direction: None,
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: Some("f-heading".to_string()),
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = crate::package::Package {
            id: "pkg-rq-note".to_string(),
            namespace: "com.test".to_string(),
            name: "rq-note-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![heading_field],
            record_types: vec![item_type],
            relation_type_definitions: vec![RelationTypeDefinition {
                schema: None,
                id: "00000000-0000-4000-8000-000000000rt3".to_string(),
                namespace: "com.test".to_string(),
                key: "refers-to".to_string(),
                label: "Refers To".to_string(),
                description: "Reference relation for tests".to_string(),
                category: RelationTypeCategory::Association,
                canonical_direction: None,
                irreflexive: None,
                inverse_type: None,
                version: 1,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                allowed_source_types: None,
                allowed_target_types: None,
                require_same_semantic_object_type: None,
                status: None,
                updated_at: None,
                properties: None,
            }],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        let make_note = |id: &str, title: &str| Note {
            instance_id: id.to_string(),
            title: Some(title.to_string()),
            tags: None,
            sections: vec![NoteSection {
                name: "body".to_string(),
                label: None,
                content: format!("Content of {title}."),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            meta: None,
        };

        crate::services::create_note(&store, make_note(SOURCE_ID, "Source Note")).unwrap();
        crate::services::create_note(&store, make_note(NOTE_ID, "Skipped Target Note")).unwrap();

        relation_service::create_relation_auto(
            &store,
            Relation {
                relation_id: String::new(),
                relation_type: "refers-to".to_string(),
                source_instance_id: SOURCE_ID.to_string(),
                target_instance_id: NOTE_ID.to_string(),
                asserted_by: None,
                confidence: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            },
        )
        .unwrap();

        let result = render_document_view(RenderDocumentViewOptions::new(&store, "dv-rq-note"))
            .expect("render must not fail when RelationQuery resolves to a Tier-0 note");

        assert!(
            !result.rendered.contains("Skipped Target Note"),
            "note title must NOT appear in RelationQuery output; got:\n{}",
            result.rendered
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[section:rq-sec]")
                    && d.contains("RelationQuery:")
                    && d.contains(NOTE_ID)
                    && d.contains("notes are not rendered in document-view sections")),
            "expected RelationQuery note-skip diagnostic; got: {:?}",
            result.diagnostics
        );
    }

    // ── relationsPresentation helpers (RFC-027, #668) ────────────────────────────

    use srs_core::types::relation::Relation;
    use srs_core::types::relation_type_definition::{
        RelationTypeCategory, RelationTypeDefinition, RelationTypeStatus,
    };
    use srs_core::types::view::{RelationPresentationEntry, RelationsPresentation};

    fn test_rtd(
        key: &str,
        label: &str,
        inverse_type: Option<&str>,
        retired: bool,
    ) -> RelationTypeDefinition {
        RelationTypeDefinition {
            schema: None,
            id: format!("rtd-{}", key.replace('/', "-")),
            version: 1,
            key: key.to_string(),
            namespace: "com.test".to_string(),
            label: label.to_string(),
            description: "Test RTD".to_string(),
            category: RelationTypeCategory::Association,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: inverse_type.map(|s| s.to_string()),
            irreflexive: None,
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: if retired {
                Some(RelationTypeStatus::Retired)
            } else {
                None
            },
            updated_at: None,
            properties: None,
        }
    }

    fn test_rel(id: &str, rtype: &str, src: &str, tgt: &str) -> Relation {
        Relation {
            relation_id: id.to_string(),
            relation_type: rtype.to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            asserted_by: None,
            confidence: None,
            created_at: None,
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        }
    }

    fn make_rp_store(
        rtds: Vec<RelationTypeDefinition>,
        relations: &[Relation],
    ) -> crate::store::memory::MemoryStore {
        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = crate::package::Package {
            id: "test-rp-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            // "f-title"/"title" bridges section.titleFieldId (a Field UUID) to
            // the name-keyed carrier in the display-label fallback tests.
            fields: vec![srs_core::types::field::Field::new(
                "f-title",
                "com.test",
                "title",
                srs_core::types::field::FieldType::string(),
            )],
            record_types: vec![],
            relation_type_definitions: rtds,
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);
        if !relations.is_empty() {
            let coll = serde_json::json!({
                "relations": relations.iter().map(|r| serde_json::to_value(r).unwrap()).collect::<Vec<_>>()
            });
            crate::store::write_relations_standalone_for_test(&store, &coll);
        }
        store
    }

    fn add_rp_record(
        store: &crate::store::memory::MemoryStore,
        id: &str,
        label_field: Option<(&str, &str)>,
    ) {
        use srs_core::types::record::{FieldValues, Record};
        let mut field_values = FieldValues::new();
        if let Some((name, val)) = label_field {
            field_values.insert(name, serde_json::json!(val));
        }

        let record = Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "t-test".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        };

        let path = format!("records/{}.json", id);
        store
            .save_instance_json(&path, &serde_json::to_value(&record).unwrap())
            .unwrap();
        let manifest = store.load_manifest().unwrap();
        store.save_manifest(&manifest).unwrap();
    }

    fn rp_section_for(
        entries: Vec<RelationPresentationEntry>,
    ) -> srs_core::types::view::DocumentSection {
        srs_core::types::view::DocumentSection {
            composite_renderers: None,
            section_id: "s-rp".to_string(),
            title: None,
            description: None,
            order: 0,
            source: srs_core::types::view::SectionSource::FixedInstances {
                instance_ids: vec!["rec-src".to_string()],
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
            relations_presentation: Some(RelationsPresentation {
                include: entries,
                label: None,
            }),
        }
    }

    fn src_rec(id: &str) -> srs_core::types::record::Record {
        srs_core::types::record::Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "t-test".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values: FieldValues::new(),
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    // ── humanize_relation_key ─────────────────────────────────────────────────

    #[test]
    fn humanize_relation_key_bare_name() {
        assert_eq!(humanize_relation_key("supersedes"), "Supersedes");
    }

    #[test]
    fn humanize_relation_key_namespaced() {
        assert_eq!(
            humanize_relation_key("com.example/depends-on"),
            "Depends on"
        );
    }

    // ── render_relations_block unit tests ─────────────────────────────────────

    #[test]
    fn relations_block_forward_edges_rendered() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            &[
                test_rel(
                    "eeeeeeee-0000-4000-8000-0000000000a1",
                    "links-to",
                    "rec-src",
                    "rec-a",
                ),
                test_rel(
                    "eeeeeeee-0000-4000-8000-0000000000a2",
                    "links-to",
                    "rec-src",
                    "rec-b",
                ),
            ],
        );
        add_rp_record(&store, "rec-src", None);
        add_rp_record(&store, "rec-a", None);
        add_rp_record(&store, "rec-b", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![
            test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-a",
            ),
            test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a2",
                "links-to",
                "rec-src",
                "rec-b",
            ),
        ];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(out.contains("rec-a"), "forward target rec-a not in: {out}");
        assert!(out.contains("rec-b"), "forward target rec-b not in: {out}");
        assert!(diag.is_empty(), "unexpected diagnostics: {diag:?}");
    }

    #[test]
    fn relations_block_inverse_edges_rendered() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-other",
                "rec-src",
            )],
        );
        add_rp_record(&store, "rec-other", None);
        add_rp_record(&store, "rec-src", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: Some(PresentationDirection::Inverse),
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "links-to",
            "rec-other",
            "rec-src",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("rec-other"),
            "inverse source rec-other not in: {out}"
        );
        assert!(diag.is_empty(), "unexpected diagnostics: {diag:?}");
    }

    #[test]
    fn relations_block_both_directions() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            &[
                test_rel(
                    "eeeeeeee-0000-4000-8000-0000000000a1",
                    "links-to",
                    "rec-src",
                    "rec-fwd",
                ),
                test_rel(
                    "eeeeeeee-0000-4000-8000-0000000000a2",
                    "links-to",
                    "rec-inv",
                    "rec-src",
                ),
            ],
        );
        add_rp_record(&store, "rec-src", None);
        add_rp_record(&store, "rec-fwd", None);
        add_rp_record(&store, "rec-inv", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: Some(PresentationDirection::Both),
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![
            test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-fwd",
            ),
            test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a2",
                "links-to",
                "rec-inv",
                "rec-src",
            ),
        ];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(out.contains("rec-fwd"), "forward target not in: {out}");
        assert!(out.contains("rec-inv"), "inverse source not in: {out}");
        assert!(diag.is_empty(), "unexpected diagnostics: {diag:?}");
    }

    #[test]
    fn relations_block_label_ladder_entry_override_wins() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "RTD Label", None, false)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-a",
            )],
        );
        add_rp_record(&store, "rec-a", None);
        add_rp_record(&store, "rec-src", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: Some("Custom Forward Label".to_string()),
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "links-to",
            "rec-src",
            "rec-a",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("Custom Forward Label"),
            "entry forwardLabel must win; got: {out}"
        );
        assert!(
            !out.contains("RTD Label"),
            "RTD label must be overridden; got: {out}"
        );
    }

    #[test]
    fn relations_block_label_ladder_rtd_label_fallback() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "RTD Label", None, false)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-a",
            )],
        );
        add_rp_record(&store, "rec-a", None);
        add_rp_record(&store, "rec-src", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "links-to",
            "rec-src",
            "rec-a",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("RTD Label"),
            "RTD label must be used when no entry override; got: {out}"
        );
    }

    #[test]
    fn relations_block_label_ladder_humanized_fallback() {
        let store = make_rp_store(
            vec![test_rtd("depends-on", "", None, false)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "depends-on",
                "rec-src",
                "rec-a",
            )],
        );
        add_rp_record(&store, "rec-a", None);
        add_rp_record(&store, "rec-src", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "depends-on".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "depends-on",
            "rec-src",
            "rec-a",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("Depends on"),
            "humanized key must be used when RTD label is empty; got: {out}"
        );
    }

    #[test]
    fn relations_block_no_output_when_zero_edges() {
        let store = make_rp_store(vec![test_rtd("links-to", "Links To", None, false)], &[]);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store,
            &section,
            &record,
            &[],
            &package,
            "markdown",
            &mut diag,
        )
        .unwrap();
        assert!(out.is_empty(), "no edges → empty output; got: {out:?}");
        assert!(diag.is_empty(), "no diagnostics expected; got: {diag:?}");
    }

    #[test]
    fn relations_block_dedupes_repeated_instance() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            &[
                test_rel(
                    "eeeeeeee-0000-4000-8000-0000000000a1",
                    "links-to",
                    "rec-src",
                    "rec-a",
                ),
                test_rel(
                    "eeeeeeee-0000-4000-8000-0000000000a2",
                    "links-to",
                    "rec-src",
                    "rec-a",
                ),
            ],
        );
        add_rp_record(&store, "rec-a", None);
        add_rp_record(&store, "rec-src", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![
            test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-a",
            ),
            test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a2",
                "links-to",
                "rec-src",
                "rec-a",
            ),
        ];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        let count = out.matches("rec-a").count();
        assert_eq!(
            count, 1,
            "rec-a should appear exactly once (deduped); got: {out}"
        );
    }

    #[test]
    fn relations_block_display_label_identity_field() {
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record::Record;
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let field = Field {
            schema: None,
            id: "f-name".to_string(),
            namespace: "com.test".to_string(),
            name: "name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let rt = RecordType {
            id: "t-named".to_string(),
            namespace: "com.test".to_string(),
            name: "named".to_string(),
            version: 1,
            description: "Named".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-name".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: Some("f-name".to_string()),
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = crate::package::Package {
            id: "test-rp-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![field],
            record_types: vec![rt],
            relation_type_definitions: vec![test_rtd("links-to", "Links To", None, false)],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);
        add_rp_record(&store, "rec-src", None);
        let relations_coll = serde_json::json!({
            "relations": [serde_json::to_value(test_rel("eeeeeeee-0000-4000-8000-0000000000a1", "links-to", "rec-src", "rec-named")).unwrap()]
        });
        crate::store::write_relations_standalone_for_test(&store, &relations_coll);

        let target = Record {
            field_meta: None,
            instance_id: "rec-named".to_string(),
            type_id: "t-named".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "named".to_string(),
            field_values: {
                let mut fv = srs_core::types::record::FieldValues::new();
                fv.insert("name", serde_json::json!("My Identity Label"));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        };
        let path = "records/rec-named.json".to_string();
        store
            .save_instance_json(&path, &serde_json::to_value(&target).unwrap())
            .unwrap();
        let manifest2 = store.load_manifest().unwrap();
        store.save_manifest(&manifest2).unwrap();

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package2 = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "links-to",
            "rec-src",
            "rec-named",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package2, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("My Identity Label"),
            "identityFieldId value must be used as display label; got: {out}"
        );
        assert!(
            !out.contains("rec-named"),
            "instanceId fallback must not appear when identity field is set; got: {out}"
        );
    }

    #[test]
    fn relations_block_display_label_title_field_fallback() {
        use srs_core::types::record::Record;

        let store = make_rp_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-titled",
            )],
        );
        add_rp_record(&store, "rec-src", None);

        let target = Record {
            field_meta: None,
            instance_id: "rec-titled".to_string(),
            type_id: "t-test".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values: {
                let mut fv = srs_core::types::record::FieldValues::new();
                fv.insert("title", serde_json::json!("Title Field Value"));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        };
        let path = "records/rec-titled.json".to_string();
        store
            .save_instance_json(&path, &serde_json::to_value(&target).unwrap())
            .unwrap();
        let manifest = store.load_manifest().unwrap();
        store.save_manifest(&manifest).unwrap();

        let mut section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        section.title_field_id = Some("f-title".to_string());

        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "links-to",
            "rec-src",
            "rec-titled",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("Title Field Value"),
            "section titleFieldId must be used as fallback display label; got: {out}"
        );
    }

    #[test]
    fn relations_block_display_label_instance_id_fallback() {
        let store = make_rp_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-no-label",
            )],
        );
        add_rp_record(&store, "rec-no-label", None);
        add_rp_record(&store, "rec-src", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "links-to".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "links-to",
            "rec-src",
            "rec-no-label",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.contains("rec-no-label"),
            "instanceId must be the last-resort display label; got: {out}"
        );
    }

    #[test]
    fn relations_block_retired_entry_skipped_with_diagnostic() {
        let store = make_rp_store(
            vec![test_rtd("old-type", "Old Type", None, true)],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "old-type",
                "rec-src",
                "rec-a",
            )],
        );
        add_rp_record(&store, "rec-a", None);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "old-type".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![test_rel(
            "eeeeeeee-0000-4000-8000-0000000000a1",
            "old-type",
            "rec-src",
            "rec-a",
        )];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.is_empty(),
            "retired RTD entry must emit no output; got: {out:?}"
        );
        assert!(
            diag.iter()
                .any(|d| d.contains("[I-027-2b]") && d.contains("old-type")),
            "expected I-027-2b diagnostic for retired RTD; got: {diag:?}"
        );
    }

    // ── project_record_json (relations field) ─────────────────────────────────

    fn make_rp_doc_store(
        rtds: Vec<RelationTypeDefinition>,
        source_id: &str,
        rp_entries: Vec<RelationPresentationEntry>,
        relations: &[Relation],
    ) -> crate::store::memory::MemoryStore {
        use srs_core::types::view::{DocumentSection, DocumentView, SectionSource};

        let dv = DocumentView {
            composite_renderers: None,
            id: "dv-rp-test".to_string(),
            namespace: "com.test".to_string(),
            name: "rp-test".to_string(),
            version: 1,
            description: "RP test view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "s-rp".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::FixedInstances {
                    instance_ids: vec![source_id.to_string()],
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
                relations_presentation: if rp_entries.is_empty() {
                    None
                } else {
                    Some(RelationsPresentation {
                        include: rp_entries,
                        label: None,
                    })
                },
            }],
            navigation_links: None,
            preamble: None,
            format: Some("json".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = crate::package::Package {
            id: "test-rp-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            // "f-title"/"title" bridges section.titleFieldId (a Field UUID) to
            // the name-keyed carrier in the display-label fallback tests.
            fields: vec![srs_core::types::field::Field::new(
                "f-title",
                "com.test",
                "title",
                srs_core::types::field::FieldType::string(),
            )],
            record_types: vec![],
            relation_type_definitions: rtds,
            views: vec![],
            document_views: vec![dv],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);
        add_rp_record(&store, source_id, None);
        if !relations.is_empty() {
            let coll = serde_json::json!({
                "relations": relations.iter().map(|r| serde_json::to_value(r).unwrap()).collect::<Vec<_>>()
            });
            crate::store::write_relations_standalone_for_test(&store, &coll);
        }
        store
    }

    #[test]
    fn project_record_json_relations_populated() {
        let store = make_rp_doc_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            "rec-src",
            vec![RelationPresentationEntry {
                relation_type: "links-to".to_string(),
                directions: None,
                forward_label: None,
                inverse_label: None,
            }],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-tgt",
            )],
        );
        add_rp_record(&store, "rec-tgt", None);

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rp-test",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let proj = result.projection.unwrap();
        let rec = &proj.sections[0].records[0];
        let relations = rec
            .relations
            .as_ref()
            .expect("relations must be populated when relationsPresentation set");
        assert_eq!(relations.len(), 1, "expected 1 relation row");
        assert_eq!(relations[0].label, "Links To");
        assert_eq!(relations[0].targets.len(), 1);
        assert_eq!(relations[0].targets[0].instance_id, "rec-tgt");
    }

    #[test]
    fn project_record_json_no_relations_when_absent() {
        let store = make_rp_doc_store(
            vec![test_rtd("links-to", "Links To", None, false)],
            "rec-src",
            vec![],
            &[test_rel(
                "eeeeeeee-0000-4000-8000-0000000000a1",
                "links-to",
                "rec-src",
                "rec-tgt",
            )],
        );
        add_rp_record(&store, "rec-tgt", None);

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-rp-test",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let proj = result.projection.unwrap();
        let rec = &proj.sections[0].records[0];
        assert!(
            rec.relations.is_none(),
            "relations must be None when relationsPresentation is absent"
        );
    }

    // ── cross-store roundtrip ────────────────────────────────────────────────

    #[test]
    fn relations_block_cross_store_roundtrip() {
        use srs_core::types::view::{DocumentSection, DocumentView, SectionSource};

        let dv_id = "dv-rp-roundtrip";
        let rtype = "links-to";
        let src_id = "rec-rr-src";
        let tgt_id = "rec-rr-tgt";

        let rp_entry = RelationPresentationEntry {
            relation_type: rtype.to_string(),
            directions: None,
            forward_label: Some("Links To".to_string()),
            inverse_label: None,
        };
        let dv = DocumentView {
            composite_renderers: None,
            id: dv_id.to_string(),
            namespace: "com.test".to_string(),
            name: dv_id.to_string(),
            version: 1,
            description: "Roundtrip test".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "s-rr".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::FixedInstances {
                    instance_ids: vec![src_id.to_string()],
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
                relations_presentation: Some(RelationsPresentation {
                    include: vec![rp_entry],
                    label: None,
                }),
            }],
            navigation_links: None,
            preamble: None,
            format: Some("json".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        let the_rtd = test_rtd(rtype, "Links To", None, false);
        let the_rel = test_rel(
            "00000000-0000-4000-8000-0000000000b1",
            rtype,
            src_id,
            tgt_id,
        );

        // ── MemoryStore path ───────────────────────────────────────────────
        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = crate::package::Package {
            id: "test-rp-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![the_rtd.clone()],
            views: vec![],
            document_views: vec![dv.clone()],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let mem_store = crate::store::memory::MemoryStore::new(manifest, package);
        add_rp_record(&mem_store, src_id, None);
        add_rp_record(&mem_store, tgt_id, None);
        let coll = serde_json::json!({
            "relations": [serde_json::to_value(&the_rel).unwrap()]
        });
        crate::store::write_relations_standalone_for_test(&mem_store, &coll);

        let mem_result = render_document_view(RenderDocumentViewOptions {
            store: &mem_store,
            view_id: dv_id,
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let mem_proj = mem_result.projection.unwrap();
        let mem_rel = mem_proj.sections[0].records[0]
            .relations
            .as_ref()
            .expect("MemoryStore: relations must be populated");
        assert_eq!(mem_rel.len(), 1, "MemoryStore: expected 1 row");

        // ── FileStore path ─────────────────────────────────────────────────
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join("records")).unwrap();
        for id in &[src_id, tgt_id] {
            let record = srs_core::types::record::Record {
                field_meta: None,
                instance_id: id.to_string(),
                type_id: "t-test".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "item".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: None,
                tags: None,
                created_at: None,
                updated_at: None,
                extra: std::collections::BTreeMap::new(),
            };
            std::fs::write(
                repo_root.join(format!("records/{id}.json")),
                serde_json::to_string_pretty(&record).unwrap(),
            )
            .unwrap();
        }

        std::fs::write(
            repo_root.join("manifest.json"),
            serde_json::to_string_pretty(&serde_json::json!({"dataModelRevision": 2})).unwrap(),
        )
        .unwrap();

        // Standalone relation object ([R11]) — collections are retired.
        std::fs::create_dir_all(repo_root.join("relations")).unwrap();
        let mut rel_value = serde_json::to_value(&the_rel).unwrap();
        rel_value["$schema"] =
            serde_json::Value::String(crate::store::RELATION_OBJECT_SCHEMA_URL.to_string());
        std::fs::write(
            repo_root.join(format!("relations/{}.json", the_rel.relation_id)),
            serde_json::to_string_pretty(&rel_value).unwrap(),
        )
        .unwrap();

        std::fs::create_dir_all(repo_root.join("package/document-views")).unwrap();
        std::fs::write(
            repo_root.join(format!("package/document-views/{dv_id}.json")),
            serde_json::to_string_pretty(&serde_json::to_value(&dv).unwrap()).unwrap(),
        )
        .unwrap();

        std::fs::create_dir_all(repo_root.join("package/relation-types")).unwrap();
        std::fs::write(
            repo_root.join(format!("package/relation-types/{rtype}.json")),
            serde_json::to_string_pretty(&serde_json::to_value(&the_rtd).unwrap()).unwrap(),
        )
        .unwrap();

        std::fs::write(
            repo_root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "rp-file-pkg",
                "namespace": "com.test",
                "name": "rp-file",
                "version": "1.0.0",
                "documentViews": [format!("document-views/{dv_id}.json")],
                "relationTypes": [format!("relation-types/{rtype}.json")],
            }))
            .unwrap(),
        )
        .unwrap();

        let file_store = FileStore::new(repo_root);
        let file_result = render_document_view(RenderDocumentViewOptions {
            store: &file_store,
            view_id: dv_id,
            format: Some("json"),
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .unwrap();
        let file_proj = file_result.projection.unwrap();
        let file_rel = file_proj.sections[0].records[0]
            .relations
            .as_ref()
            .expect("FileStore: relations must be populated");
        assert_eq!(file_rel.len(), 1, "FileStore: expected 1 row");

        assert_eq!(
            mem_rel[0].label, file_rel[0].label,
            "MemoryStore and FileStore must produce the same relation row label"
        );
        assert_eq!(
            mem_rel[0].targets.len(),
            file_rel[0].targets.len(),
            "MemoryStore and FileStore must have the same number of targets"
        );
        assert_eq!(
            mem_rel[0].targets[0].instance_id, file_rel[0].targets[0].instance_id,
            "MemoryStore and FileStore must agree on target instanceId"
        );
    }

    #[test]
    fn relations_block_nonresolving_entry_skipped_with_diagnostic() {
        let store = make_rp_store(vec![], &[]);

        let section = rp_section_for(vec![RelationPresentationEntry {
            relation_type: "unknown-type".to_string(),
            directions: None,
            forward_label: None,
            inverse_label: None,
        }]);
        let record = src_rec("rec-src");
        let package = store.load_package().unwrap();
        let relations = vec![];
        let mut diag = Vec::new();
        let out = render_relations_block(
            &store, &section, &record, &relations, &package, "markdown", &mut diag,
        )
        .unwrap();
        assert!(
            out.is_empty(),
            "non-resolving RTD must emit no output; got: {out:?}"
        );
        assert!(
            diag.iter().any(|d| d.contains("[I-027-2b]")
                && d.contains("unknown-type")
                && d.contains("does not resolve")),
            "expected I-027-2b diagnostic for non-resolving entry; got: {diag:?}"
        );
    }

    // ---------------------------------------------------------------------
    // RFC-037 — the normative field-row rendering baseline.
    //
    // Before this block the whole row path carried two markdown assertions and
    // nothing at all on `html`, `adoc`, `text`, empty values or the placeholder,
    // so any of those arms could be changed without a test noticing. There is one
    // test per conformance rule below, named for the rule it holds.
    // ---------------------------------------------------------------------

    fn scalar(value: &str) -> RowValue {
        RowValue::Scalar(value.to_string())
    }

    fn entries(values: &[&str]) -> RowValue {
        RowValue::Entries(values.iter().map(|v| v.to_string()).collect())
    }

    fn field_row(format: &str, name: &str, label: &str, value: &RowValue) -> String {
        format_field_row(format, RowIdentity::FieldName(name), label, value)
    }

    #[test]
    fn fr_037_3_scalar_row_forms_per_format() {
        assert_eq!(
            field_row("markdown", "rationale", "Rationale", &scalar("because")),
            "**Rationale**: because"
        );
        assert_eq!(
            field_row("adoc", "rationale", "Rationale", &scalar("because")),
            "*Rationale*: because"
        );
        assert_eq!(
            field_row("text", "rationale", "Rationale", &scalar("because")),
            "Rationale: because"
        );
    }

    #[test]
    fn fr_037_3_separator_is_colon_space() {
        // A literal U+003A U+0020 separates label from value in all three text
        // formats — not a tab, not two spaces, not a colon alone.
        for format in ["markdown", "adoc", "text"] {
            let row = field_row(format, "f", "L", &scalar("v"));
            assert!(
                row.ends_with(": v"),
                "{format} row must separate label and value with ': '; got {row:?}"
            );
        }
    }

    #[test]
    fn fr_037_4_html_scalar_structure() {
        assert_eq!(
            field_row("html", "rationale", "Rationale", &scalar("because")),
            "<div class=\"srs-field srs-fieldname-rationale\">\
             <strong class=\"srs-field-label\">Rationale</strong>: \
             <span class=\"srs-field-value\">because</span></div>"
        );
    }

    #[test]
    fn fr_037_5_multi_entry_renders_as_block_list_not_comma_joined() {
        assert_eq!(
            field_row("markdown", "tags", "Tags", &entries(&["alpha", "beta"])),
            "**Tags**:\n- alpha\n- beta"
        );
        assert_eq!(
            field_row("adoc", "tags", "Tags", &entries(&["alpha", "beta"])),
            "*Tags*:\n* alpha\n* beta"
        );
        assert_eq!(
            field_row("text", "tags", "Tags", &entries(&["alpha", "beta"])),
            "Tags:\n- alpha\n- beta"
        );
    }

    #[test]
    fn fr_037_5_label_line_keeps_its_colon_and_carries_no_value() {
        // "Derived, not decided" in the RFC: the colon is retained so punctuation
        // does not vary by cardinality. Held by a test so overturning it is a
        // visible change rather than a silent drift.
        let row = field_row("markdown", "tags", "Tags", &entries(&["alpha"]));
        assert_eq!(row.lines().next().unwrap(), "**Tags**:");
    }

    #[test]
    fn fr_037_5_single_element_sequence_still_renders_in_block_form() {
        // Cardinality selects the form, never element count — otherwise two
        // records of the same Type would disagree on structure.
        assert_eq!(
            field_row("markdown", "tags", "Tags", &entries(&["only"])),
            "**Tags**:\n- only"
        );
    }

    #[test]
    fn fr_037_5_entries_keep_sequence_order() {
        // Sequence order, not sorted order: array index order on a list-cardinality
        // Field, `FieldValue.entries` order on the repeatable path.
        assert_eq!(
            field_row(
                "text",
                "tags",
                "Tags",
                &entries(&["gamma", "alpha", "beta"])
            ),
            "Tags:\n- gamma\n- alpha\n- beta"
        );
    }

    #[test]
    fn fr_037_6_html_multi_entry_structure() {
        assert_eq!(
            field_row("html", "tags", "Tags", &entries(&["alpha", "beta"])),
            "<div class=\"srs-field srs-fieldname-tags\">\
             <strong class=\"srs-field-label\">Tags</strong>:\
             <ul><li class=\"srs-field-value\">alpha</li>\
             <li class=\"srs-field-value\">beta</li></ul></div>"
        );
    }

    #[test]
    fn fr_037_6_html_ul_carries_no_class() {
        let row = field_row("html", "tags", "Tags", &entries(&["alpha"]));
        assert!(
            row.contains("<ul>"),
            "the ul must carry no class; got {row}"
        );
    }

    #[test]
    fn fr_037_7_text_formats_separate_rows_with_a_blank_line() {
        // Not cosmetic: in CommonMark two unseparated rows are one soft-wrapped
        // paragraph, so without this the second row is not a row at all.
        for format in ["markdown", "adoc", "text"] {
            assert_eq!(row_separator(format), "\n\n", "format {format}");
        }
    }

    #[test]
    fn fr_037_7_html_inserts_no_separator_element() {
        assert_eq!(row_separator("html"), "\n");
    }

    #[test]
    fn fr_037_8_entry_continuation_indents_two_spaces() {
        // Two spaces — the width of the `- ` marker. Four would make CommonMark
        // read the continuation as an indented code block.
        assert_eq!(
            indent_entry_continuation("first line\nsecond line"),
            "first line\n  second line"
        );
    }

    #[test]
    fn fr_037_8_blank_line_does_not_terminate_an_entry() {
        // The item stays one item; the following block attaches at the same
        // content column, and the blank line stays genuinely blank rather than
        // gaining trailing whitespace.
        assert_eq!(
            indent_entry_continuation("para one\n\npara two"),
            "para one\n\n  para two"
        );
    }

    #[test]
    fn fr_037_8_adoc_uses_a_plus_continuation_between_blocks() {
        assert_eq!(
            adoc_entry_continuation("para one\n\npara two"),
            "para one\n+\npara two"
        );
    }

    #[test]
    fn fr_037_8_scalar_values_are_never_indented() {
        // Indenting a scalar would corrupt every multi-line markdown body in the
        // spec repository's own projection.
        assert_eq!(
            field_row("markdown", "body", "Body", &scalar("line one\nline two")),
            "**Body**: line one\nline two"
        );
    }

    #[test]
    fn fr_037_9_entries_rendering_to_nothing_are_dropped() {
        assert_eq!(
            render_field_value(&serde_json::json!(["alpha", "", "beta"]), None, "markdown"),
            Some(entries(&["alpha", "beta"]))
        );
    }

    #[test]
    fn fr_037_9_sequence_with_no_surviving_entries_is_absent() {
        assert_eq!(
            render_field_value(&serde_json::json!(["", ""]), None, "markdown"),
            None
        );
    }

    #[test]
    fn fr_037_10_empty_string_is_absent() {
        // The defect that put 86 label-with-no-value rows into the spec repo's
        // committed exports: `as_str()` succeeds on "", so the omit branch never
        // fired and the renderer emitted `**Content**: ` with a trailing space.
        assert_eq!(
            value_to_text_owned(&serde_json::json!(""), "markdown"),
            None
        );
        assert_eq!(
            render_field_value(&serde_json::json!(""), None, "markdown"),
            None
        );
    }

    #[test]
    fn fr_037_11_placeholder_is_the_literal_empty_marker() {
        assert_eq!(
            field_row("markdown", "owner", "Owner", &RowValue::Placeholder),
            "**Owner**: (empty)"
        );
        assert_eq!(
            field_row("adoc", "owner", "Owner", &RowValue::Placeholder),
            "*Owner*: (empty)"
        );
        assert_eq!(
            field_row("text", "owner", "Owner", &RowValue::Placeholder),
            "Owner: (empty)"
        );
    }

    #[test]
    fn fr_037_11_html_placeholder_carries_srs_empty_value() {
        assert_eq!(
            field_row("html", "owner", "Owner", &RowValue::Placeholder),
            "<div class=\"srs-field srs-fieldname-owner\">\
             <strong class=\"srs-field-label\">Owner</strong>: \
             <span class=\"srs-field-value srs-empty-value\">(empty)</span></div>"
        );
    }

    #[test]
    fn fr_037_12_identity_class_comes_from_field_name_not_display_label() {
        // The original defect: a Type setting displayLabel "Decision Rationale"
        // on field `rationale` emitted `srs-fieldname-decision-rationale`, so a
        // purely presentational change silently moved a selector themes target.
        let row = field_row("html", "rationale", "Decision Rationale", &scalar("v"));
        assert!(
            row.contains("srs-fieldname-rationale"),
            "identity class must derive from Field.name; got {row}"
        );
        assert!(
            !row.contains("srs-fieldname-decision-rationale"),
            "identity class must not derive from displayLabel; got {row}"
        );
    }

    #[test]
    fn fr_037_12_relation_row_swaps_in_srs_relationtype() {
        let row = format_field_row(
            "html",
            RowIdentity::RelationTypeKey("core/depends-on"),
            "Depends on",
            &scalar("Target"),
        );
        // The five-step rule has no replacement step for `/`, so it is deleted
        // and a namespaced key normalises without a separator. Ugly, deterministic,
        // and recorded in Change E so no implementer 'fixes' it unilaterally.
        assert!(row.contains("srs-relationtype-coredepends-on"), "got {row}");
        assert!(
            !row.contains("srs-fieldname-"),
            "a relation row has no Field.name and must omit srs-fieldname-*; got {row}"
        );
        assert!(
            row.contains("srs-field "),
            "srs-field is retained so a stylesheet can target both row kinds; got {row}"
        );
    }

    #[test]
    fn fr_037_14_prefixed_names_only_aliases_gone_at_cutover() {
        // [FR-037-14]: the #242 cutover fired — the unprefixed compatibility
        // aliases (`field-label`/`field-value`) are no longer emitted.
        let row = field_row("html", "f", "L", &scalar("v"));
        assert!(row.contains("class=\"srs-field-label\""), "{row}");
        assert!(row.contains("class=\"srs-field-value\""), "{row}");
        assert!(!row.contains("field-label field-label"), "{row}");
        assert!(
            !row.contains("srs-field-label field-label"),
            "unprefixed alias must be gone after the cutover: {row}"
        );
    }

    #[test]
    fn fr_037_15_relation_row_uses_the_same_markup_as_a_field_row() {
        // RFC-027 Change C rule 3's MUST, satisfied by construction rather than
        // by two hand-rolled matches agreeing with each other.
        for format in ["markdown", "adoc", "text", "html"] {
            let relation = format_field_row(
                format,
                RowIdentity::RelationTypeKey("depends-on"),
                "Depends on",
                &scalar("Target"),
            );
            let field = field_row(format, "depends-on", "Depends on", &scalar("Target"));
            assert_eq!(
                relation.replace("srs-relationtype-", "srs-fieldname-"),
                field,
                "{format}: relation and field rows must share label/value markup"
            );
        }
    }

    #[test]
    fn fr_037_16_text_formats_emit_label_and_value_verbatim() {
        // Field values in this model routinely are markup; escaping them would
        // corrupt the projection.
        let row = field_row("markdown", "body", "Body", &scalar("**bold** & <tag>"));
        assert_eq!(row, "**Body**: **bold** & <tag>");
    }

    #[test]
    fn fr_037_16_html_escapes_label_and_value() {
        let row = field_row("html", "f", "A & B", &scalar(&html_escape("<x>")));
        assert!(
            row.contains("A &amp; B"),
            "label must be escaped; got {row}"
        );
        assert!(
            row.contains("&lt;x&gt;"),
            "value must be escaped; got {row}"
        );
    }

    #[test]
    fn fr_037_17_markup_bearing_values_are_not_converted() {
        // The baseline has no rule for which values are markup, so html output
        // shows literal source rather than converted HTML.
        let value = value_to_text_owned(&serde_json::json!("# heading"), "html").unwrap();
        assert_eq!(value, "# heading");
    }

    #[test]
    fn fr_037_18_label_falls_back_to_raw_field_name_without_humanising() {
        // An author who wants human-facing text sets displayLabel; a baseline
        // that rewrites labels makes output non-invertible against the record.
        assert_eq!(
            field_row("markdown", "instance_id", "instance_id", &scalar("v")),
            "**instance_id**: v"
        );
    }

    #[test]
    fn fr_037_18_array_value_is_multi_entry_without_a_resolvable_field() {
        // The Tier 1 shape: no FieldAssignment, no Field UUID — array-ness alone
        // makes the value a sequence.
        assert_eq!(
            render_field_value(&serde_json::json!(["a", "b"]), None, "markdown"),
            Some(entries(&["a", "b"]))
        );
    }

    #[test]
    fn list_cardinality_values_are_a_sequence_too() {
        // RFC-039 Change D: `cardinality: "list"` is the one sequence mechanism
        // (successor of the retired ext:repeatable-fields entries path).
        let ft = FieldType::string().into_list();
        assert_eq!(
            render_field_value(
                &serde_json::json!(["first", "second"]),
                Some(&ft),
                "markdown"
            ),
            Some(entries(&["first", "second"]))
        );
    }

    /// A MemoryStore with one two-field type and a TypeQuery doc view, for the
    /// RFC-037 rules that live at the emission site rather than in the row
    /// primitive: empty-value omission, placeholder gating, and row separation
    /// as actually emitted.
    fn make_row_baseline_store(
        format: &str,
        empty_behavior: Option<srs_core::types::view::EmptyBehavior>,
        summary_required: bool,
        summary_value: serde_json::Value,
        l1_view: bool,
    ) -> crate::store::memory::MemoryStore {
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::view::{DocumentSection, DocumentView, SectionSource};

        let make_field = |id: &str, name: &str| Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: name.to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let assignment =
            |id: &str, order: u32, required: bool, label: Option<&str>| FieldAssignment {
                field_id: id.to_string(),
                order,
                required,
                display_label: label.map(|l| l.to_string()),
                description: None,
            };

        let record_type = RecordType {
            id: "t-row".to_string(),
            namespace: "com.test".to_string(),
            name: "row-record".to_string(),
            version: 1,
            description: "Record for field-row baseline tests".to_string(),
            fields: vec![
                assignment("f-heading", 0, false, None),
                // displayLabel differs from Field.name so [FR-037-12] is observable
                // end to end, not only in the primitive.
                assignment("f-summary", 1, summary_required, Some("Executive Summary")),
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            provenance: None,
        };

        let doc_view = DocumentView {
            composite_renderers: None,
            id: "dv-row".to_string(),
            namespace: "com.test".to_string(),
            name: "row-view".to_string(),
            version: 1,
            description: "Field-row baseline view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "s-row".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "com.test/row-record".to_string(),
                    lifecycle_state: None,
                    container_ids: None,
                    lifecycle_states: None,
                    exclude_lifecycle_states: None,
                    container_scope: None,
                },
                render_view_id: l1_view.then(|| "v-row-l1".to_string()),
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior,
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: Some(format.to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };

        // The L1 View marks summary required too, so on that path the only thing
        // suppressing the placeholder is the path itself.
        let l1_views = if l1_view {
            use srs_core::types::view::{ExportConfig, FieldView, View};
            vec![View {
                id: "v-row-l1".to_string(),
                namespace: "com.test".to_string(),
                name: "row-l1".to_string(),
                version: 1,
                description: "L1 View over the row record".to_string(),
                field_views: vec![
                    FieldView {
                        composite_renderer: None,
                        field_id: "f-heading".to_string(),
                        order: 0,
                        required: None,
                        visible: None,
                        display_label: None,
                    },
                    FieldView {
                        composite_renderer: None,
                        field_id: "f-summary".to_string(),
                        order: 1,
                        required: Some(true),
                        visible: None,
                        display_label: Some("Executive Summary".to_string()),
                    },
                ],
                compatible_types: None,
                protection: None,
                export_config: Some(ExportConfig {
                    format: None,
                    preamble: None,
                    field_order: None,
                    omit_empty_fields: None,
                }),
                tags: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                extra: std::collections::BTreeMap::new(),
            }]
        } else {
            vec![]
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-row".to_string(),
            namespace: "com.test".to_string(),
            name: "row-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                make_field("f-heading", "heading"),
                make_field("f-summary", "summary"),
            ],
            record_types: vec![record_type],
            relation_type_definitions: vec![],
            views: l1_views,
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        let values = {
            let mut fv = srs_core::types::record::FieldValues::new();
            fv.insert("heading", serde_json::json!("Heading value"));
            fv.insert("summary", summary_value);
            fv
        };
        create_record(&store, "t-row", 1, values, None, None).unwrap();
        store
    }

    fn render_rows(store: &crate::store::memory::MemoryStore) -> String {
        render_document_view(RenderDocumentViewOptions {
            store,
            view_id: "dv-row",
            format: None,
            theme_variant: None,
            container_id: None,
            instance_id_filter: None,
        })
        .expect("render should succeed")
        .rendered
    }

    #[test]
    fn fr_037_10_empty_string_emits_no_row_end_to_end() {
        // The 86-row defect, held at the level it actually manifested: a label
        // with a trailing space and no value in committed output.
        let store = make_row_baseline_store("markdown", None, false, serde_json::json!(""), false);
        let out = render_rows(&store);
        assert!(
            !out.contains("**Executive Summary**"),
            "an empty-string field must emit no row at all; got:\n{out}"
        );
        assert!(
            !out.contains(": \n") && !out.ends_with(": "),
            "no label may be emitted with an empty value; got:\n{out:?}"
        );
        assert!(
            out.contains("**heading**: Heading value"),
            "the present field must still render; got:\n{out}"
        );
    }

    /// [R5a]: `""` is a *present* value — a required field valued `""` passes
    /// write-time validation (`create_record` inside the store builder), yet
    /// still emits no row ([FR-037-10]: empty renders nothing).
    #[test]
    fn required_empty_string_validates_and_emits_no_row() {
        let store = make_row_baseline_store("markdown", None, true, serde_json::json!(""), false);
        let out = render_rows(&store);
        assert!(
            !out.contains("Executive Summary"),
            "a required field valued \"\" must emit no row; got:\n{out}"
        );
        assert!(
            out.contains("**heading**: Heading value"),
            "the present field must still render; got:\n{out}"
        );
    }

    #[test]
    fn fr_037_11_placeholder_requires_show_placeholder_and_required() {
        use srs_core::types::view::EmptyBehavior;
        let store = make_row_baseline_store(
            "markdown",
            Some(EmptyBehavior::ShowPlaceholder),
            true,
            serde_json::json!(""),
            false,
        );
        let out = render_rows(&store);
        assert!(
            out.contains("**Executive Summary**: (empty)"),
            "show-placeholder + required must emit the literal (empty); got:\n{out}"
        );
    }

    #[test]
    fn fr_037_11_no_placeholder_when_the_field_is_not_required() {
        // The condition is show-placeholder AND required: true. An optional field
        // is simply absent — before this rule it got a placeholder regardless.
        use srs_core::types::view::EmptyBehavior;
        let store = make_row_baseline_store(
            "markdown",
            Some(EmptyBehavior::ShowPlaceholder),
            false,
            serde_json::json!(""),
            false,
        );
        let out = render_rows(&store);
        assert!(
            !out.contains("(empty)"),
            "a non-required field must emit no placeholder row; got:\n{out}"
        );
    }

    #[test]
    fn fr_037_7_consecutive_rows_are_blank_line_separated_end_to_end() {
        // Two present rows must not become one soft-wrapped CommonMark paragraph.
        let store = make_row_baseline_store(
            "markdown",
            None,
            false,
            serde_json::json!("Summary value"),
            false,
        );
        let out = render_rows(&store);
        assert!(
            out.contains("**heading**: Heading value\n\n**Executive Summary**: Summary value"),
            "consecutive rows must be separated by a blank line; got:\n{out:?}"
        );
    }

    #[test]
    fn fr_037_12_display_label_does_not_move_the_identity_class_end_to_end() {
        // Field.name is `summary`; displayLabel is "Executive Summary". The class
        // must follow the name, the visible label must follow displayLabel.
        let store = make_row_baseline_store(
            "html",
            None,
            false,
            serde_json::json!("Summary value"),
            false,
        );
        let out = render_rows(&store);
        assert!(
            out.contains("srs-fieldname-summary"),
            "identity class must derive from Field.name; got:\n{out}"
        );
        assert!(
            !out.contains("srs-fieldname-executive-summary"),
            "identity class must not derive from displayLabel; got:\n{out}"
        );
        assert!(
            out.contains(">Executive Summary</strong>"),
            "the visible label must still be the displayLabel; got:\n{out}"
        );
    }

    #[test]
    fn fr_037_11_placeholder_does_not_reach_the_l1_view_path() {
        // "emptyBehavior in the L1 View path: when renderViewId is set, empty
        // field handling is governed by ExportConfig.omitEmptyFields on the
        // referenced L1 View. DocumentSection.emptyBehavior does not apply in the
        // L1 View rendering path." — srs-spec.md:1853. [FR-037-11] inherits that
        // exclusion, so show-placeholder + required still emits no placeholder here.
        use srs_core::types::view::EmptyBehavior;
        let store = make_row_baseline_store(
            "markdown",
            Some(EmptyBehavior::ShowPlaceholder),
            true,
            serde_json::json!(""),
            true,
        );
        let out = render_rows(&store);
        assert!(
            !out.contains("(empty)"),
            "emptyBehavior must not reach the L1 View path; got:\n{out}"
        );
    }
}
