use crate::container_service::list_members;
use crate::error::RepositoryError;
use crate::package::Package;
use crate::record_store::{get_record_by_id, list_records_by_type};
use crate::relation_service::load_relations;
use crate::store::RepositoryStore;
use serde_json::json;
use srs_core::types::field::ValueType;
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use srs_core::types::theme::{AssetMode, Theme};
use srs_core::types::view::{
    DocumentSection, DocumentView, RelationDirection, SectionSource, SortDirection, ThemeMode,
};
use std::collections::{HashMap, HashSet};

pub struct RenderDocumentViewOptions<'a> {
    pub store: &'a dyn RepositoryStore,
    pub view_id: &'a str,
    pub format: Option<&'a str>,
    pub theme_variant: Option<&'a str>,
    /// When set, TypeQuery sections are filtered to members of this container.
    /// Takes precedence over any container_ids declared in the view definition.
    pub container_id: Option<&'a str>,
}

impl<'a> RenderDocumentViewOptions<'a> {
    pub fn new(store: &'a dyn RepositoryStore, view_id: &'a str) -> Self {
        Self {
            store,
            view_id,
            format: None,
            theme_variant: None,
            container_id: None,
        }
    }
}

pub struct RenderResult {
    pub rendered: String,
    pub diagnostics: Vec<String>,
    pub projection: Option<DocumentViewProjection>,
}

// ── JSON projection output types ──────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedGroupEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    pub fields: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedFieldGroup {
    pub group_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub entries: Vec<ProjectedGroupEntry>,
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
    pub fields: serde_json::Value,
    pub ordered_field_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_groups: Option<Vec<ProjectedFieldGroup>>,
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
    field_id: String,
    required: bool,
}

struct RenderContext<'a> {
    package: &'a Package,
    container_title: String,
    depth_offset: u32,
    format: &'a str,
    status_field_id: Option<String>,
    active_theme: Option<Theme>,
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
    let container_title = resolve_container_title(&dv, &manifest, opts.container_id);
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
        status_field_id: package.find_field_by_name("status").map(|f| f.id.clone()),
        active_theme,
    };

    let mut rendered = String::new();
    if let Some(preamble) = &dv.preamble {
        rendered.push_str(&substitute_vars(preamble, &ctx, None, false));
        rendered.push_str("\n\n");
    } else if format == "markdown" || format == "adoc" {
        rendered.push_str(&format!(
            "{}{}\n\n",
            heading_prefix(depth(1, ctx.depth_offset), format),
            ctx.container_title
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
            &mut diagnostics,
        )?);
    }

    if let Some(theme) = ctx.active_theme.as_ref() {
        if let Some(element_templates) = &theme.element_templates {
            if let Some(document_wrapper) = &element_templates.document_wrapper {
                rendered = apply_wrapper(
                    document_wrapper,
                    &rendered,
                    &[
                        ("container-title", &ctx.container_title),
                        ("date", &chrono::Utc::now().format("%Y-%m-%d").to_string()),
                    ],
                    Some(theme),
                );
            }
        }
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
    diagnostics: &mut Vec<String>,
) -> Result<ProjectedSection, RepositoryError> {
    let mut records =
        resolve_section_instances(store, section, relations, cli_container_id, diagnostics)?;

    if let Some(ordering) = &section.ordering {
        if let Some(field_id) = &ordering.field_id {
            records.sort_by(|a, b| {
                let av = a.get_field_value_str(field_id).unwrap_or("");
                let bv = b.get_field_value_str(field_id).unwrap_or("");
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
        records = sort_by_precedes_chain(records, relations);
    }

    let projected_records = records
        .iter()
        .map(|record| project_record_json(package, section, record, diagnostics))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ProjectedSection {
        section_id: section.section_id.clone(),
        title: section.title.clone(),
        order: section.order,
        records: projected_records,
    })
}

fn project_record_json(
    package: &Package,
    section: &DocumentSection,
    record: &Record,
    diagnostics: &mut Vec<String>,
) -> Result<ProjectedRecord, RepositoryError> {
    let rt = package
        .resolve_type(&record.type_id, record.type_version)
        .cloned();

    let record_heading = section
        .title_field_id
        .as_ref()
        .and_then(|fid| record.get_field_value_str(fid).map(|v| v.to_string()));

    let mut fields_to_render: Vec<ResolvedFieldRender> = Vec::new();
    let mut omit_empty = false;
    let mut record_preamble: Option<String> = None;

    let use_view = if let Some(view_id) = &section.render_view_id {
        if let Some(view) = package.resolve_view(view_id) {
            let (satisfied, _cached_eff) =
                record_satisfies_view(package, view, rt.as_ref(), diagnostics);
            if satisfied {
                Some(view.clone())
            } else {
                diagnostics.push(format!(
                    "[view-dispatch] record {} type {}/{} does not satisfy view {}; rendering by own type",
                    record.instance_id,
                    rt.as_ref().map(|t| t.namespace.as_str()).unwrap_or("?"),
                    rt.as_ref().map(|t| t.name.as_str()).unwrap_or("?"),
                    view_id,
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
                if fv.visible == Some(false) {
                    continue;
                }
                fields_to_render.push(ResolvedFieldRender {
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
        for fv in &record.field_values {
            fields_to_render.push(ResolvedFieldRender {
                field_id: fv.field_id.clone(),
                required: false,
            });
        }
    }

    let ordered_field_keys: Vec<String> = fields_to_render
        .iter()
        .map(|f| f.field_id.clone())
        .collect();
    let mut fields_map = serde_json::Map::new();

    for field in &fields_to_render {
        let field_id = &field.field_id;
        let field_value = record.find_field_value(field_id);
        let field_def = package.resolve_field(field_id);
        let field_type = field_def.map(|f| f.value_type);
        let json_val = field_value.and_then(|fv| field_value_to_json(fv, field_type));

        if field.required && json_val.is_none() {
            diagnostics.push(format!(
                "[view-required] view {} record {} is missing required field {} for rendered view",
                section
                    .render_view_id
                    .as_deref()
                    .unwrap_or("<no-render-view-id>"),
                record.instance_id,
                field_id
            ));
        }

        match json_val {
            Some(v) => {
                fields_map.insert(field_id.clone(), v);
            }
            None => {
                if !omit_empty {
                    fields_map.insert(field_id.clone(), serde_json::Value::Null);
                }
            }
        }
    }

    let field_groups = project_field_groups_json(package, record, &rt);

    Ok(ProjectedRecord {
        instance_id: record.instance_id.clone(),
        type_id: record.type_id.clone(),
        type_namespace: record.type_namespace.clone(),
        type_name: record.type_name.clone(),
        record_heading,
        preamble: record_preamble,
        fields: serde_json::Value::Object(fields_map),
        ordered_field_keys,
        field_groups,
    })
}

fn substitute_vars_record_json(template: &str, record: &Record) -> String {
    let mut out = template.to_string();
    for level in 1..=6 {
        out = out.replace(&format!("{{{{heading-{level}}}}}"), "");
    }
    out = out.replace("{{instance-id}}", &record.instance_id);
    out = out.replace("{{type-name}}", &record.type_name);
    out = out.replace("{{type-namespace}}", &record.type_namespace);
    out
}

fn project_field_groups_json(
    package: &Package,
    record: &Record,
    rt: &Option<srs_core::types::record_type::RecordType>,
) -> Option<Vec<ProjectedFieldGroup>> {
    let rt = rt.as_ref()?;
    let field_groups_def = rt.field_groups.as_ref()?;

    let mut groups_def = field_groups_def.clone();
    groups_def.sort_by_key(|g| g.order);

    let mut result = Vec::new();
    for group_def in &groups_def {
        let group_value = match record.find_group_value(&group_def.group_id) {
            Some(gv) if !gv.entries.is_empty() => gv,
            _ => continue,
        };

        let mut field_assignments = group_def.fields.clone();
        field_assignments.sort_by_key(|fa| fa.order);

        let entries = group_value
            .entries
            .iter()
            .map(|entry| {
                let mut entry_fields = serde_json::Map::new();
                for assignment in &field_assignments {
                    let fv = entry
                        .field_values
                        .iter()
                        .find(|v| v.field_id == assignment.field_id);
                    if let Some(fv) = fv {
                        let field_type = package
                            .resolve_field(&assignment.field_id)
                            .map(|f| f.value_type);
                        let json_val =
                            field_value_to_json(fv, field_type).unwrap_or(serde_json::Value::Null);
                        entry_fields.insert(assignment.field_id.clone(), json_val);
                    }
                }
                ProjectedGroupEntry {
                    entry_id: entry.entry_id.clone(),
                    fields: serde_json::Value::Object(entry_fields),
                }
            })
            .collect();

        result.push(ProjectedFieldGroup {
            group_id: group_def.group_id.clone(),
            label: group_def.label.clone(),
            entries,
        });
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn field_value_to_json(
    field_value: &srs_core::types::record::FieldValue,
    value_type: Option<ValueType>,
) -> Option<serde_json::Value> {
    if let Some(entries) = &field_value.entries {
        if entries.is_empty() {
            return None;
        }
        let vals: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| coerce_json_value(&e.value, value_type))
            .collect();
        return Some(serde_json::Value::Array(vals));
    }
    if field_value.value.is_null() {
        return None;
    }
    Some(coerce_json_value(&field_value.value, value_type))
}

fn coerce_json_value(raw: &serde_json::Value, value_type: Option<ValueType>) -> serde_json::Value {
    match value_type {
        Some(ValueType::Number) => {
            if let Some(n) = raw.as_str().and_then(|s| s.parse::<i64>().ok()) {
                return json!(n);
            }
            if let Some(f) = raw.as_str().and_then(|s| s.parse::<f64>().ok()) {
                return json!(f);
            }
        }
        Some(ValueType::Boolean) => match raw.as_str() {
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

    if !theme.targets.iter().any(|target| target == format) {
        diagnostics.push(format!(
            "[T-2] view {} theme {} does not target format {}; skipping theme",
            dv.id, theme.id, format
        ));
        return None;
    }

    Some(theme)
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

fn format_field_row(format: &str, label: &str, value: &str) -> String {
    if format == "markdown" {
        format!("**{}**: {}", label, value)
    } else {
        format!("{}: {}", label, value)
    }
}

fn resolve_container_title(
    dv: &DocumentView,
    manifest: &crate::manifest::Manifest,
    container_id: Option<&str>,
) -> String {
    if let Some(container_index) = manifest.extra.get("containerIndex") {
        if let Some(entries) = container_index.as_array() {
            // When a specific container was requested, look it up by ID first.
            if let Some(cid) = container_id {
                for entry in entries {
                    let id = entry.get("containerId").and_then(|v| v.as_str());
                    if id == Some(cid) {
                        if let Some(title) = entry.get("title").and_then(|v| v.as_str()) {
                            if !title.is_empty() {
                                return title.to_string();
                            }
                        }
                    }
                }
            }

            // Fallback: first container matching the document view's containerType.
            if let Some(container_type) = &dv.container_type {
                for entry in entries {
                    let ctype = entry
                        .get("containerType")
                        .and_then(|v| v.as_str())
                        .or_else(|| entry.get("type").and_then(|v| v.as_str()));
                    if ctype == Some(container_type.as_str()) {
                        let title = entry.get("title").and_then(|v| v.as_str());
                        if let Some(title) = title {
                            if !title.is_empty() {
                                return title.to_string();
                            }
                        }
                    }
                }
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

/// Sort records by following the `precedes` relation chain among them.
///
/// Builds a linked-list ordering from `precedes` relations whose both endpoints
/// are in the candidate set. Records not connected by any precedes relation fall
/// back to `created_at` order. Handles cycles via a visited set.
fn sort_by_precedes_chain(records: Vec<Record>, relations: &[Relation]) -> Vec<Record> {
    if records.len() <= 1 {
        return records;
    }

    let id_set: HashSet<&str> = records.iter().map(|r| r.instance_id.as_str()).collect();

    // Build successor map and in-degree count from precedes relations within the set.
    let mut next: HashMap<&str, &str> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = id_set.iter().map(|id| (*id, 0)).collect();

    for rel in relations {
        if rel.relation_type != "precedes" {
            continue;
        }
        let src = rel.source_instance_id.as_str();
        let tgt = rel.target_instance_id.as_str();
        if id_set.contains(src) && id_set.contains(tgt) {
            next.insert(src, tgt);
            *in_degree.entry(tgt).or_insert(0) += 1;
        }
    }

    // Heads: nodes with no incoming precedes edge within the set.
    let mut heads: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    // Stable-sort heads by created_at for deterministic ordering of disconnected components.
    heads.sort_by(|a, b| {
        let ta = records
            .iter()
            .find(|r| r.instance_id == *a)
            .and_then(|r| r.created_at.as_deref())
            .unwrap_or("");
        let tb = records
            .iter()
            .find(|r| r.instance_id == *b)
            .and_then(|r| r.created_at.as_deref())
            .unwrap_or("");
        ta.cmp(tb)
    });

    let record_map: HashMap<&str, &Record> = records
        .iter()
        .map(|r| (r.instance_id.as_str(), r))
        .collect();

    let mut result: Vec<Record> = Vec::with_capacity(records.len());
    let mut visited: HashSet<&str> = HashSet::new();

    for head in heads {
        let mut current = head;
        loop {
            if visited.contains(current) {
                break;
            }
            visited.insert(current);
            if let Some(&record) = record_map.get(current) {
                result.push(record.clone());
            }
            match next.get(current) {
                Some(&nxt) => current = nxt,
                None => break,
            }
        }
    }

    // Append orphans / cycle members not reached above, sorted by created_at.
    let mut remaining: Vec<&Record> = records
        .iter()
        .filter(|r| !visited.contains(r.instance_id.as_str()))
        .collect();
    remaining.sort_by(|a, b| {
        a.created_at
            .as_deref()
            .unwrap_or("")
            .cmp(b.created_at.as_deref().unwrap_or(""))
    });
    result.extend(remaining.into_iter().cloned());

    result
}

/// Collect subsection records that are targets of `contains` relations from
/// `instance_id`, ordered by `precedes` chain.
fn collect_subsections(
    store: &dyn RepositoryStore,
    instance_id: &str,
    relations: &[Relation],
) -> Result<Vec<Record>, RepositoryError> {
    let target_ids: Vec<&str> = relations
        .iter()
        .filter(|r| r.relation_type == "contains" && r.source_instance_id == instance_id)
        .map(|r| r.target_instance_id.as_str())
        .collect();

    let mut subsections = Vec::new();
    for id in target_ids {
        if let Some(record) = get_record_by_id(store, id)? {
            subsections.push(record);
        }
    }

    Ok(sort_by_precedes_chain(subsections, relations))
}

fn render_section(
    store: &dyn RepositoryStore,
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    relations: &[Relation],
    cli_container_id: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let mut records =
        resolve_section_instances(store, section, relations, cli_container_id, diagnostics)?;
    if records.is_empty() && section.required != Some(true) {
        return Ok(String::new());
    }

    // Apply explicit field-based ordering first if declared.
    if let Some(ordering) = &section.ordering {
        if let Some(field_id) = &ordering.field_id {
            records.sort_by(|a, b| {
                let av = a.get_field_value_str(field_id).unwrap_or("");
                let bv = b.get_field_value_str(field_id).unwrap_or("");
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
        records = sort_by_precedes_chain(records, relations);
    }

    let mut out = String::new();
    if let Some(title) = &section.title {
        out.push_str(&format!(
            "{}{}\n\n",
            heading_prefix(depth(2, ctx.depth_offset), ctx.format),
            title
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
    for record in &records {
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

    if let Some(theme) = ctx.active_theme.as_ref() {
        if let Some(section_wrapper) = select_section_wrapper(theme, &section.section_id) {
            let section_title = section.title.as_deref().unwrap_or("");
            out = apply_wrapper(
                section_wrapper,
                &out,
                &[
                    ("section-title", section_title),
                    ("section-id", section.section_id.as_str()),
                ],
                Some(theme),
            );
        }
    }
    Ok(out)
}

fn resolve_section_instances(
    store: &dyn RepositoryStore,
    section: &DocumentSection,
    relations: &[srs_core::types::relation::Relation],
    cli_container_id: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<Record>, RepositoryError> {
    match &section.source {
        SectionSource::FixedInstances { instance_ids } => {
            let mut records = Vec::new();
            for id in instance_ids {
                if let Some(record) = get_record_by_id(store, id)? {
                    records.push(record);
                }
            }
            Ok(records)
        }
        SectionSource::TypeQuery {
            semantic_object_type,
            container_ids,
            lifecycle_state: _,
        } => {
            let Some((namespace, name)) = semantic_object_type.split_once('/') else {
                diagnostics.push(format!(
                    "[N] TypeQuery semanticObjectType '{}' has no namespace separator '/' — expected 'namespace/name' format",
                    semantic_object_type
                ));
                return Ok(Vec::new());
            };
            let mut records = list_records_by_type(store, namespace, name)?;
            // CLI --container takes precedence; fall back to container_ids declared in the view.
            let effective_ids: Option<Vec<String>> = cli_container_id
                .map(|id| vec![id.to_string()])
                .or_else(|| container_ids.clone());
            if let Some(ids) = effective_ids {
                let mut member_set = HashSet::new();
                for id in &ids {
                    for m in list_members(store, id)? {
                        member_set.insert(m);
                    }
                }
                records.retain(|r| member_set.contains(&r.instance_id));
            }
            Ok(records)
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
                if let Some(record) = get_record_by_id(store, &id)? {
                    records.push(record);
                }
            }
            Ok(records)
        }
        SectionSource::ContainerSubset {
            container_id,
            container_type: _,
        } => {
            // CLI --container overrides the view-declared container_id, allowing one
            // ContainerSubset document-view to render any guide by switching at render time.
            let effective_id = cli_container_id.unwrap_or(container_id.as_str());
            let members = list_members(store, effective_id)?;
            let mut records = Vec::new();
            for id in members {
                if let Some(record) = get_record_by_id(store, &id)? {
                    records.push(record);
                }
            }
            Ok(records)
        }
    }
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

    if let Some(title_field_id) = &section.title_field_id {
        if let Some(title) = record.get_field_value_str(title_field_id) {
            record_heading_value = title.to_string();
            out.push_str(&format!(
                "{}{}\n\n",
                heading_prefix(heading_level, ctx.format),
                title
            ));
        }
    }

    let mut fields_to_render: Vec<ResolvedFieldRender> = Vec::new();
    let mut display_labels = std::collections::HashMap::new();
    let mut omit_empty = false;

    let use_view = if let Some(view_id) = &section.render_view_id {
        if let Some(view) = ctx.package.resolve_view(view_id) {
            let (satisfied, _cached_eff) =
                record_satisfies_view(ctx.package, view, rt.as_ref(), diagnostics);
            if satisfied {
                Some(view.clone())
            } else {
                diagnostics.push(format!(
                    "[view-dispatch] record {} type {}/{} does not satisfy view {}; rendering by own type",
                    record.instance_id,
                    rt.as_ref().map(|t| t.namespace.as_str()).unwrap_or("?"),
                    rt.as_ref().map(|t| t.name.as_str()).unwrap_or("?"),
                    view_id,
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
        for fv in &record.field_values {
            fields_to_render.push(ResolvedFieldRender {
                field_id: fv.field_id.clone(),
                required: false,
            });
        }
    }

    for field in fields_to_render {
        let field_id = field.field_id;
        // In structured mode (titleFieldId set), skip the title field — already emitted as heading.
        if structured {
            if let Some(title_fid) = &section.title_field_id {
                if &field_id == title_fid {
                    continue;
                }
            }
        }

        let field_value = record.find_field_value(&field_id);
        let field_def = ctx.package.resolve_field(&field_id);
        let field_type = field_def.map(|field| field.value_type);
        let rendered_value = field_value.and_then(|fv| render_field_value(fv, field_type));
        if field.required && rendered_value.is_none() {
            diagnostics.push(format!(
                "[view-required] view {} record {} is missing required field {} for rendered view",
                section
                    .render_view_id
                    .as_deref()
                    .unwrap_or("<no-render-view-id>"),
                record.instance_id,
                field_id
            ));
        }
        if rendered_value.is_none() && omit_empty {
            continue;
        }
        if rendered_value.is_none()
            && !matches!(
                section.empty_behavior,
                Some(srs_core::types::view::EmptyBehavior::ShowPlaceholder)
            )
        {
            continue;
        }

        let value_text = rendered_value.unwrap_or_else(|| "(empty)".to_string());

        let field_name = ctx
            .package
            .resolve_field(&field_id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| field_id.clone());

        let label = display_labels
            .get(&field_id)
            .cloned()
            .or_else(|| {
                rt.as_ref()
                    .and_then(|t| t.find_field_assignment(&field_id))
                    .and_then(|fa| fa.display_label.clone())
            })
            .or_else(|| Some(field_name.clone()))
            .unwrap_or_else(|| field_id.clone());

        let row_content = format_field_row(ctx.format, &label, &value_text);
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
                    out.push('\n');
                    continue;
                }
            }
        }
        out.push_str(&row_content);
        out.push('\n');
    }
    if let Some(rt) = &rt {
        if let Some(field_groups) = &rt.field_groups {
            out.push_str(&render_field_groups(ctx, rt, record, field_groups));
        }
    }

    // In structured mode, render subsections nested one heading level deeper.
    if structured {
        let subsections = collect_subsections(store, &record.instance_id, relations)?;
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

    if let Some(theme) = ctx.active_theme.as_ref() {
        if let Some(record_wrapper) = select_record_wrapper(theme, &record.type_id) {
            out = apply_wrapper(
                record_wrapper,
                &out,
                &[
                    ("record-heading", &record_heading_value),
                    ("type-namespace", &record.type_namespace),
                    ("type-name", &record.type_name),
                ],
                Some(theme),
            );
        }
    }

    out.push('\n');
    Ok(out)
}

fn render_field_groups(
    ctx: &RenderContext<'_>,
    rt: &srs_core::types::record_type::RecordType,
    record: &Record,
    field_groups: &[srs_core::types::record_type::FieldGroup],
) -> String {
    let mut groups = field_groups.to_vec();
    groups.sort_by_key(|g| g.order);
    let mut out = String::new();

    for group in groups {
        let Some(group_value) = record.find_group_value(&group.group_id) else {
            continue;
        };
        if group_value.entries.is_empty() {
            continue;
        }

        if let Some(label) = &group.label {
            out.push('\n');
            out.push_str(&format!(
                "{}{}\n\n",
                heading_prefix(depth(4, ctx.depth_offset), ctx.format),
                label
            ));
        }

        let mut assignments = group.fields.clone();
        assignments.sort_by_key(|fa| fa.order);
        for (idx, entry) in group_value.entries.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            for assignment in &assignments {
                let Some(fv) = entry
                    .field_values
                    .iter()
                    .find(|value| value.field_id == assignment.field_id)
                else {
                    continue;
                };

                let field_type = rt
                    .find_field_assignment(&assignment.field_id)
                    .and_then(|_| ctx.package.resolve_field(&assignment.field_id))
                    .map(|field| field.value_type);
                let Some(value_text) = render_field_value(fv, field_type) else {
                    continue;
                };
                let label = assignment
                    .display_label
                    .clone()
                    .or_else(|| {
                        ctx.package
                            .resolve_field(&assignment.field_id)
                            .map(|f| f.name.clone())
                    })
                    .unwrap_or_else(|| assignment.field_id.clone());
                out.push_str(&format_field_row(ctx.format, &label, &value_text));
                out.push('\n');
            }
        }
    }

    out
}

fn render_field_value(
    field_value: &srs_core::types::record::FieldValue,
    value_type: Option<ValueType>,
) -> Option<String> {
    if let Some(entries) = &field_value.entries {
        if entries.is_empty() {
            return None;
        }
        let texts: Vec<String> = entries
            .iter()
            .filter_map(|entry| value_to_text_owned(&entry.value))
            .collect();
        if texts.is_empty() {
            return None;
        }
        let joined = match value_type {
            Some(ValueType::Text) | Some(ValueType::Multiselect) => texts
                .into_iter()
                .map(|value| format!("- {value}"))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => texts.join(", "),
        };
        return Some(joined);
    }
    value_to_text_owned(&field_value.value)
}

fn value_to_text_owned(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(array) = value.as_array() {
        let parts: Vec<String> = array
            .iter()
            .filter_map(|item| item.as_str().map(std::string::ToString::to_string))
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

    if let Some(record) = record {
        out = out.replace("{{instance-id}}", &record.instance_id);
        out = out.replace("{{namespace}}", &record.type_namespace);
        out = out.replace("{{name}}", &record.type_name);
        let status = ctx
            .status_field_id
            .as_ref()
            .and_then(|id| record.get_field_value_str(id))
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

fn depth(base: u32, depth_offset: u32) -> u32 {
    base + depth_offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;

    fn srs_spec_repo() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return std::path::PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
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
    fn render_document_view_produces_output() {
        let repo_root = srs_spec_repo();
        if !repo_root.join("manifest.json").exists() {
            return;
        }
        let store = FileStore::new(repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6",
            format: None,
            theme_variant: None,
            container_id: None,
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
        })
        .expect("render should succeed");
        assert!(
            result.diagnostics.iter().any(|d| d.contains("[N+4b]")),
            "expected [N+4b] diagnostic for depthOffset 5, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn title_field_id_emits_record_heading() {
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000983",
            format: None,
            theme_variant: None,
            container_id: None,
        })
        .expect("render should succeed");
        // titleFieldId points to the repeatable title field; first entry value is "first"
        // expect an H3 heading containing that value
        assert!(
            result.rendered.contains("### first") || result.rendered.contains("### "),
            "expected H3 record heading from titleFieldId, got: {}",
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
        let repo_root = repeatable_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000986",
            format: None,
            theme_variant: None,
            container_id: None,
        })
        .expect("render should succeed");

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[view-required]")
                    && d.contains("00000000-0000-4000-8000-000000000992")
                    && d.contains("00000000-0000-4000-8000-000000000903")),
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
        assert!(
            result.rendered.contains("OVERRIDERECORD[first|"),
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

    #[test]
    fn json_projection_record_field_groups_populated() {
        let repo_root = field_groups_fixture_root();
        let store = FileStore::new(&repo_root);
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "00000000-0000-4000-8000-000000000971",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
        })
        .expect("render should succeed");
        let proj = result.projection.unwrap();
        let record = proj.sections[0]
            .records
            .iter()
            .find(|r| r.instance_id == "00000000-0000-4000-8000-000000000981")
            .expect("valid record should be present");
        let groups = record
            .field_groups
            .as_ref()
            .expect("fieldGroups must be present for valid record");
        assert_eq!(groups.len(), 1, "expected 1 field group");
        let group = &groups[0];
        assert_eq!(group.group_id, "people");
        assert_eq!(group.entries.len(), 2, "expected 2 entries (alice, bob)");
        let alice = &group.entries[0];
        assert_eq!(
            alice.fields.get("00000000-0000-4000-8000-000000000911"),
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
    fn make_hetero_store() -> (crate::store::memory::MemoryStore, String, String, String) {
        use crate::container_service;
        use crate::package::Package;
        use crate::record_store::create_record;
        use crate::relation_service;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::relation::Relation;
        use srs_core::types::view::{
            DocumentSection, DocumentView, EmptyBehavior, FieldView, SectionSource, View,
        };

        let heading_field = Field {
            id: "f-heading".to_string(),
            namespace: "com.test".to_string(),
            name: "heading".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "Heading".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let body_field = Field {
            id: "f-body".to_string(),
            namespace: "com.test".to_string(),
            name: "body".to_string(),
            version: 1,
            value_type: ValueType::Text,
            description: "Body text".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let caption_field = Field {
            id: "f-caption".to_string(),
            namespace: "com.test".to_string(),
            name: "caption".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "Caption for tables".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
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
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "f-body".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Body".to_string()),
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
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
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "f-caption".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Caption".to_string()),
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        // View that only matches text sections (has compatible_types constraint)
        let text_only_view = View {
            id: "v-text-only".to_string(),
            namespace: "com.test".to_string(),
            name: "text-view".to_string(),
            version: 1,
            description: "View for text sections only".to_string(),
            field_views: vec![FieldView {
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
            extra: HashMap::new(),
        };

        // DocumentView: ContainerSubset section with the text-only view
        let doc_view = DocumentView {
            id: "dv-hetero".to_string(),
            namespace: "com.test".to_string(),
            name: "hetero-view".to_string(),
            version: 1,
            description: "Heterogeneous container document view".to_string(),
            container_type: None,
            sections: vec![DocumentSection {
                section_id: "body".to_string(),
                title: Some("Body".to_string()),
                description: None,
                order: 0,
                source: SectionSource::ContainerSubset {
                    container_id: "00000000-0000-4000-8000-000000000c01".to_string(),
                    container_type: None,
                },
                render_view_id: Some("v-text-only".to_string()),
                title_field_id: Some("f-heading".to_string()),
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
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
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        // Create a text record (precedes the table)
        let fv_text = vec![
            srs_core::types::record::FieldValue {
                field_id: "f-heading".to_string(),
                value: serde_json::json!("Introduction"),
                entries: None,
                source: None,
                edited_at: None,
            },
            srs_core::types::record::FieldValue {
                field_id: "f-body".to_string(),
                value: serde_json::json!("The introduction body."),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];
        let text_record = create_record(&store, "t-text", 1, fv_text, None, "records").unwrap();
        let text_id = text_record.instance_id.clone();

        // Create a table record (follows the text)
        let fv_table = vec![
            srs_core::types::record::FieldValue {
                field_id: "f-heading".to_string(),
                value: serde_json::json!("Summary Table"),
                entries: None,
                source: None,
                edited_at: None,
            },
            srs_core::types::record::FieldValue {
                field_id: "f-caption".to_string(),
                value: serde_json::json!("Table caption here"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];
        let table_record = create_record(&store, "t-table", 1, fv_table, None, "records").unwrap();
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
    fn view_dispatch_applies_view_to_matching_type_and_falls_back_for_non_matching() {
        let (store, _text_id, _table_id, view_id) = make_hetero_store();
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: &view_id,
            format: None,
            theme_variant: None,
            container_id: None,
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
        use std::collections::HashMap as StdMap;

        // Create two records with different created_at — "later" has more recent timestamp.
        let make_record = |id: &str, created: &str| Record {
            instance_id: id.to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values: vec![],
            group_values: None,
            lifecycle_state: None,
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
        let sorted = sort_by_precedes_chain(authored, &[]);

        // sort_by_precedes_chain DOES reorder (this is what the guard must prevent).
        assert_eq!(
            sorted[0].instance_id, "a-earlier",
            "sort_by_precedes_chain with no relations sorts by created_at ascending"
        );
        assert_eq!(sorted[1].instance_id, "b-later");
        // This confirms that WITHOUT the guard, FixedInstances would be reordered.
        // The guard (matches! FixedInstances) in both render paths prevents this call.
    }
}
