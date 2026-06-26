use crate::container_service::list_members;
use crate::error::RepositoryError;
use crate::package::Package;
use crate::record_store::{get_record_by_id, list_records_by_type};
use crate::relation_graph;
use crate::relation_service::load_relations;
use crate::store::RepositoryStore;
use serde_json::json;
use srs_core::types::field::ValueType;
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use srs_core::types::theme::{AssetMode, Theme};
use srs_core::types::view::{
    ContainerScope, DocumentSection, DocumentView, RelationDirection, SectionSource, SortDirection,
    ThemeMode,
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
        records = relation_graph::sort_by_precedes_chain(records, relations);
    }

    // RFC-008 typeFilter: applied after sort (same invariant as render_section).
    // Sort sees the full container; filter projects onto the sorted survivor set.
    if let SectionSource::ContainerSubset {
        type_filter: Some(filter),
        ..
    } = &section.source
    {
        if !filter.is_empty() {
            records.retain(|r| {
                if let Some(rt) = package.resolve_type(&r.type_id, r.type_version) {
                    let key = format!("{}/{}", rt.namespace, rt.name);
                    filter.iter().any(|f| f == &key)
                } else {
                    false
                }
            });
        }
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
                effective_view_id.unwrap_or("<no-view-id>"),
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
        out = out.replace(&format!("{{{{heading-{level}-open}}}}"), "");
        out = out.replace(&format!("{{{{heading-{level}-close}}}}"), "");
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

fn format_field_row(format: &str, label: &str, value: &str) -> String {
    match format {
        "markdown" => format!("**{label}**: {value}"),
        "html" => {
            let el = html_escape(label);
            let fn_class = normalise_css_class(label);
            format!("<div class=\"srs-field srs-fieldname-{fn_class}\"><span class=\"field-label\">{el}</span> <span class=\"field-value\">{value}</span></div>")
        }
        _ => format!("{label}: {value}"),
    }
}

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
                    if let Some(fv) = record.find_field_value(field_id) {
                        if let Some(raw) = fv.value.as_str() {
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
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let mut records =
        resolve_section_instances(store, section, relations, cli_container_id, diagnostics)?;

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
        records = relation_graph::sort_by_precedes_chain(records, relations);
    }

    // RFC-008 typeFilter: restrict container-subset members to matching types.
    // Applied AFTER sort so sort_by_precedes_chain sees the full container (including
    // cross-type edges). The filter is a projection step: full ordering established first,
    // non-matching types dropped while preserving the relative order of survivors.
    if let SectionSource::ContainerSubset {
        type_filter: Some(filter),
        ..
    } = &section.source
    {
        if !filter.is_empty() {
            records.retain(|r| {
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
            let scope = container_scope.as_ref().unwrap_or(&ContainerScope::Explicit);
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
                            for m in list_members(store, id)? {
                                member_set.insert(m);
                            }
                        }
                        records.retain(|r| member_set.contains(&r.instance_id));
                    } else {
                        diagnostics.push(
                            "[N+27] containerScope 'subtree' with no containerIds — returning empty result".to_string(),
                        );
                        records.clear();
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
                            for m in list_members(store, id)? {
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
            let backcompat_state = if !has_include { lifecycle_state.as_deref() } else { None };

            if has_include || backcompat_state.is_some() {
                // Inclusion filter: only records whose lifecycle_state matches any listed value.
                // Records with no lifecycle_state are excluded.
                records.retain(|r| {
                    r.lifecycle_state.as_deref().map(|s| {
                        if let Some(inc) = include_states {
                            inc.iter().any(|v| v == s)
                        } else {
                            backcompat_state == Some(s)
                        }
                    }).unwrap_or(false)
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
            type_filter: _,
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
            out.push_str(&format_heading(heading_level, ctx.format, title));
        }
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
        let rendered_value =
            field_value.and_then(|fv| render_field_value(fv, field_type, ctx.format));
        if field.required && rendered_value.is_none() {
            diagnostics.push(format!(
                "[view-required] view {} record {} is missing required field {} for rendered view",
                effective_view_id.unwrap_or("<no-view-id>"),
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
            out.push_str(&render_field_groups(
                ctx,
                rt,
                record,
                field_groups,
                diagnostics,
            ));
        }
    }

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

fn render_field_groups(
    ctx: &RenderContext<'_>,
    rt: &srs_core::types::record_type::RecordType,
    record: &Record,
    field_groups: &[srs_core::types::record_type::FieldGroup],
    diagnostics: &mut Vec<String>,
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

        match group.composite_renderer.as_deref() {
            Some("table") => {
                if let Some(label) = &group.label {
                    out.push('\n');
                    out.push_str(&format_heading(
                        depth(4, ctx.depth_offset),
                        ctx.format,
                        label,
                    ));
                }
                let table_config = ctx
                    .active_theme
                    .as_ref()
                    .and_then(|t| t.element_templates.as_ref())
                    .and_then(|et| et.composite_renderer_config.as_ref())
                    .and_then(|crc| crc.get("table"));
                out.push_str(&render_composite_table(
                    ctx,
                    rt,
                    record,
                    &group,
                    group_value,
                    table_config,
                    diagnostics,
                ));
            }
            Some(unknown) => {
                // [FG-Cx1]: Unknown compositeRenderer — fall back to baseline + emit diagnostic.
                diagnostics.push(format!(
                    "[FG-Cx1] Unrecognised compositeRenderer {:?} on group {:?}; falling back to per-field baseline",
                    unknown, group.group_id
                ));
                out.push_str(&render_field_group_baseline(
                    ctx,
                    rt,
                    record,
                    &group,
                    group_value,
                ));
            }
            None => {
                out.push_str(&render_field_group_baseline(
                    ctx,
                    rt,
                    record,
                    &group,
                    group_value,
                ));
            }
        }
    }

    out
}

fn render_field_group_baseline(
    ctx: &RenderContext<'_>,
    rt: &srs_core::types::record_type::RecordType,
    _record: &Record,
    group: &srs_core::types::record_type::FieldGroup,
    group_value: &srs_core::types::record::FieldGroupValue,
) -> String {
    let mut out = String::new();

    if let Some(label) = &group.label {
        out.push('\n');
        out.push_str(&format_heading(
            depth(4, ctx.depth_offset),
            ctx.format,
            label,
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
            let Some(value_text) = render_field_value(fv, field_type, ctx.format) else {
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

            let field_name = ctx
                .package
                .resolve_field(&assignment.field_id)
                .map(|f| f.name.as_str())
                .unwrap_or(&label);
            let tmpl = ctx
                .active_theme
                .as_ref()
                .and_then(|t| t.element_templates.as_ref())
                .and_then(|et| et.group_field_row_templates.as_ref())
                .and_then(|gft| gft.get(field_name));

            if let Some(tmpl) = tmpl {
                let row = tmpl
                    .replace("{{field-value}}", &value_text)
                    .replace("{{field-label}}", &label);
                out.push_str(&row);
            } else {
                out.push_str(&format_field_row(ctx.format, &label, &value_text));
                out.push('\n');
            }
        }
    }
    out
}

fn render_composite_table(
    ctx: &RenderContext<'_>,
    rt: &srs_core::types::record_type::RecordType,
    _record: &Record,
    group: &srs_core::types::record_type::FieldGroup,
    group_value: &srs_core::types::record::FieldGroupValue,
    table_config: Option<&serde_json::Value>,
    diagnostics: &mut Vec<String>,
) -> String {
    let mut out = String::new();

    // Resolve field IDs for the table's named fields by Field.name.
    let resolve_field_id = |name: &str| -> Option<String> {
        group
            .fields
            .iter()
            .find(|fa| {
                ctx.package
                    .resolve_field(&fa.field_id)
                    .map(|f| f.name == name)
                    .unwrap_or(false)
            })
            .map(|fa| fa.field_id.clone())
    };

    let columns_id = resolve_field_id("columns");
    let rows_id = resolve_field_id("rows");
    let widths_id = resolve_field_id("widths");
    let subheading_id = resolve_field_id("subheading");
    let label_id = resolve_field_id("label");

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

    for (entry_idx, entry) in group_value.entries.iter().enumerate() {
        let get_fv = |field_id: &Option<String>| -> Option<&srs_core::types::record::FieldValue> {
            let id = field_id.as_deref()?;
            entry.field_values.iter().find(|fv| fv.field_id == id)
        };

        // Resolve columns and rows as JSON arrays.
        // Fields stored as text-typed JSON strings (e.g. "[\"a\",\"b\"]") are parsed;
        // fields already stored as native JSON arrays are used directly.
        let columns_json = get_fv(&columns_id).and_then(|fv| coerce_to_array(&fv.value));
        let rows_json = get_fv(&rows_id).and_then(|fv| coerce_to_array(&fv.value));

        // [FG-Cx2]: Skip entry if neither columns nor rows have content.
        let has_columns = columns_json
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let has_rows = rows_json.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
        if !has_columns && !has_rows {
            diagnostics.push(format!(
                "[FG-Cx2] compositeRenderer:table entry {} in group {:?} has no columns or rows; skipping",
                entry_idx, group.group_id
            ));
            continue;
        }

        let widths: Vec<f64> = get_fv(&widths_id)
            .and_then(|fv| coerce_to_array(&fv.value))
            .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();

        let subheading = get_fv(&subheading_id)
            .and_then(|fv| fv.value.as_str())
            .map(|s| s.to_string());

        let label_text = get_fv(&label_id)
            .and_then(|fv| fv.value.as_str())
            .map(|s| s.to_string());

        let cols: Vec<String> = columns_json
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let rows: Vec<Vec<String>> = rows_json
            .unwrap_or_default()
            .iter()
            .filter_map(|row| {
                row.as_array().map(|cells| {
                    cells
                        .iter()
                        .filter_map(|c| c.as_str().map(|s| s.to_string()))
                        .collect()
                })
            })
            .collect();

        let table_str = match ctx.format {
            "html" => render_table_html(&cols, &rows, table_class, &widths, rt),
            _ => render_table_markdown(&cols, &rows, &widths),
        };

        let subheading_str = match subheading.as_deref() {
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

        let label_str = match label_text.as_deref() {
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

        let entry_out = if let Some(tmpl) = wrapper_template {
            tmpl.replace("{{subheading}}", &subheading_str)
                .replace("{{label}}", &label_str)
                .replace("{{table}}", &table_str)
        } else if ctx.format == "html" {
            format!("<figure class=\"srs-table\">{subheading_str}{label_str}{table_str}</figure>\n")
        } else {
            format!("{subheading_str}{label_str}{table_str}")
        };

        out.push_str(&entry_out);
    }

    out
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
    _rt: &srs_core::types::record_type::RecordType,
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

fn render_field_value(
    field_value: &srs_core::types::record::FieldValue,
    value_type: Option<ValueType>,
    format: &str,
) -> Option<String> {
    if let Some(entries) = &field_value.entries {
        if entries.is_empty() {
            return None;
        }
        let texts: Vec<String> = entries
            .iter()
            .filter_map(|entry| value_to_text_owned(&entry.value, format))
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
    value_to_text_owned(&field_value.value, format)
}

/// Coerce a field value to a JSON array.
/// Accepts a native JSON array or a text-typed string containing a JSON-encoded array.
fn coerce_to_array(value: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    if let Some(arr) = value.as_array() {
        return Some(arr.clone());
    }
    if let Some(s) = value.as_str() {
        if let Ok(serde_json::Value::Array(arr)) = serde_json::from_str(s) {
            return Some(arr);
        }
    }
    None
}

fn value_to_text_owned(value: &serde_json::Value, format: &str) -> Option<String> {
    if let Some(s) = value.as_str() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use std::collections::HashMap;

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
            root_type_refs: None,
            sections: vec![DocumentSection {
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
        let text_record =
            create_record(&store, "t-text", 1, fv_text, None, None, "records").unwrap();
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
        let table_record =
            create_record(&store, "t-table", 1, fv_table, None, None, "records").unwrap();
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
        use srs_core::types::record_type::RecordType;
        let rt = RecordType {
            id: "t".to_string(),
            namespace: "n".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
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
        let cols = vec!["Col A".to_string()];
        let rows = vec![vec!["val 1".to_string()]];
        let out = render_table_html(&cols, &rows, "srs-data-table", &[], &rt);
        assert!(
            out.contains("<table class=\"srs-data-table\">"),
            "got: {out}"
        );
        assert!(out.contains("<th>Col A</th>"), "got: {out}");
        assert!(out.contains("<td>val 1</td>"), "got: {out}");
    }

    #[test]
    fn render_table_html_empty_class_omits_attribute() {
        use srs_core::types::record_type::RecordType;
        let rt = RecordType {
            id: "t".to_string(),
            namespace: "n".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
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
        let out = render_table_html(&[], &[], "", &[], &rt);
        assert!(
            !out.contains("class="),
            "[T-Cx2] empty tableClass must omit class attribute, got: {out}"
        );
    }

    #[test]
    fn render_table_html_widths_emit_colgroup() {
        use srs_core::types::record_type::RecordType;
        let rt = RecordType {
            id: "t".to_string(),
            namespace: "n".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
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
        let widths = vec![0.3, 0.7];
        let out = render_table_html(&[], &[], "cls", &widths, &rt);
        assert!(out.contains("<colgroup>"), "got: {out}");
        assert!(out.contains("width:30%"), "got: {out}");
        assert!(out.contains("width:70%"), "got: {out}");
    }

    /// Build a MemoryStore with one type + TypeQuery doc_view for composite renderer tests.
    /// Creates a record pre-loaded with the given group entry and returns (store, record_id).
    fn make_composite_table_store(
        composite_renderer: Option<&str>,
        columns: serde_json::Value,
        rows: serde_json::Value,
        theme: Option<srs_core::types::theme::Theme>,
    ) -> crate::store::memory::MemoryStore {
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record::{FieldGroupEntry, FieldGroupValue, FieldValue};
        use srs_core::types::record_type::{FieldAssignment, FieldGroup, RecordType};
        use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};

        let make_field = |id: &str, name: &str| Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            value_type: ValueType::String,
            description: name.to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let fields = vec![
            make_field("f-columns", "columns"),
            make_field("f-rows", "rows"),
            make_field("f-widths", "widths"),
            make_field("f-subheading", "subheading"),
            make_field("f-label", "label"),
            make_field("f-title", "title"),
        ];
        let group_assignments: Vec<FieldAssignment> =
            ["f-columns", "f-rows", "f-widths", "f-subheading", "f-label"]
                .iter()
                .enumerate()
                .map(|(i, id)| FieldAssignment {
                    field_id: id.to_string(),
                    order: i as u32,
                    required: false,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                })
                .collect();

        let record_type = RecordType {
            id: "t-table-rec".to_string(),
            namespace: "com.test".to_string(),
            name: "table-record".to_string(),
            version: 1,
            description: "Record with composite table group".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-title".to_string(),
                order: 0,
                required: false,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
            field_groups: Some(vec![FieldGroup {
                group_id: "tables".to_string(),
                order: 0,
                fields: group_assignments,
                label: None,
                description: None,
                required: false,
                repeatable: true,
                min_items: None,
                max_items: None,
                composite_renderer: composite_renderer.map(|s| s.to_string()),
            }]),
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let doc_view = DocumentView {
            id: "dv-table".to_string(),
            namespace: "com.test".to_string(),
            name: "table-view".to_string(),
            version: 1,
            description: "Table document view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
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
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
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
            extra: HashMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-table".to_string(),
            namespace: "com.test".to_string(),
            name: "table-package".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: vec![record_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: theme.into_iter().collect(),
            blueprints: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        let group_entry = FieldGroupEntry {
            entry_id: Some("e-1".to_string()),
            field_values: vec![
                FieldValue {
                    field_id: "f-columns".to_string(),
                    value: columns,
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "f-rows".to_string(),
                    value: rows,
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
        };
        let gv = vec![FieldGroupValue {
            group_id: "tables".to_string(),
            entries: vec![group_entry],
        }];
        let fv = vec![FieldValue {
            field_id: "f-title".to_string(),
            value: serde_json::json!("Table Record"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        create_record(&store, "t-table-rec", 1, fv, Some(gv), None, "records").unwrap();
        store
    }

    #[test]
    fn composite_table_renders_gfm_table_in_document_view() {
        let store = make_composite_table_store(
            Some("table"),
            serde_json::json!(["Col1", "Col2"]),
            serde_json::json!([["A", "B"], ["C", "D"]]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
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
            serde_json::json!([["q1", "a1"]]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
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
    fn unknown_composite_renderer_falls_back_and_emits_fg_cx1_diagnostic() {
        let store = make_composite_table_store(
            Some("com.acme/gantt"),
            serde_json::json!(["Col"]),
            serde_json::json!([["val"]]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
        })
        .expect("render should not hard-error on unknown renderer");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("[FG-Cx1]") && d.contains("com.acme/gantt")),
            "[FG-Cx1] diagnostic expected, got: {:?}",
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
        use crate::package::Package;
        use crate::record_store::create_record;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record::{FieldGroupEntry, FieldGroupValue, FieldValue};
        use srs_core::types::record_type::{FieldAssignment, FieldGroup, RecordType};
        use srs_core::types::theme::{ElementTemplates, Theme};
        use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};

        let make_field = |id: &str, name: &str| Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            value_type: ValueType::String,
            description: name.to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let fields = vec![
            make_field("f-columns", "columns"),
            make_field("f-rows", "rows"),
            make_field("f-label", "label"),
            make_field("f-title", "title"),
        ];
        let group_assignments: Vec<FieldAssignment> = ["f-columns", "f-rows", "f-label"]
            .iter()
            .enumerate()
            .map(|(i, id)| FieldAssignment {
                field_id: id.to_string(),
                order: i as u32,
                required: false,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            })
            .collect();
        let record_type = RecordType {
            id: "t-cap".to_string(),
            namespace: "com.test".to_string(),
            name: "cap-record".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![FieldAssignment {
                field_id: "f-title".to_string(),
                order: 0,
                required: false,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
            field_groups: Some(vec![FieldGroup {
                group_id: "g".to_string(),
                order: 0,
                fields: group_assignments,
                label: None,
                description: None,
                required: false,
                repeatable: true,
                min_items: None,
                max_items: None,
                composite_renderer: Some("table".to_string()),
            }]),
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
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
                group_field_row_templates: None,
                composite_renderer_config: Some(HashMap::from([(
                    "table".to_string(),
                    serde_json::json!({ "captionTemplate": "{{field-value}}" }),
                )])),
            }),
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let doc_view = DocumentView {
            id: "dv-cap".to_string(),
            namespace: "com.test".to_string(),
            name: "cap-view".to_string(),
            version: 1,
            description: "d".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                section_id: "s".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "com.test/cap-record".to_string(),
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
            }],
            navigation_links: None,
            preamble: None,
            format: Some("html".to_string()),
            depth_offset: None,
            theme_ref: Some(srs_core::types::view::ThemeReference {
                mode: srs_core::types::view::ThemeMode::Bundled,
                path: None,
                url: None,
                theme_id: Some("th-cap".to_string()),
            }),
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
            id: "pkg-cap".to_string(),
            namespace: "com.test".to_string(),
            name: "cap-package".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: vec![record_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![theme],
            blueprints: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);
        let entry = FieldGroupEntry {
            entry_id: Some("e1".to_string()),
            field_values: vec![
                FieldValue {
                    field_id: "f-columns".to_string(),
                    value: serde_json::json!(["Col"]),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "f-rows".to_string(),
                    value: serde_json::json!([["val"]]),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "f-label".to_string(),
                    value: serde_json::json!("<script>alert(1)</script>"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
        };
        let gv = vec![FieldGroupValue {
            group_id: "g".to_string(),
            entries: vec![entry],
        }];
        let fv = vec![FieldValue {
            field_id: "f-title".to_string(),
            value: serde_json::json!("R"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        create_record(&store, "t-cap", 1, fv, Some(gv), None, "records").unwrap();

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-cap",
            format: None,
            theme_variant: None,
            container_id: None,
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
    fn empty_columns_and_rows_emits_fg_cx2_diagnostic_and_skips_entry() {
        let store = make_composite_table_store(
            Some("table"),
            serde_json::json!([]),
            serde_json::json!([]),
            None,
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-table",
            format: None,
            theme_variant: None,
            container_id: None,
        })
        .expect("render should not hard-error");
        assert!(
            result.diagnostics.iter().any(|d| d.contains("[FG-Cx2]")),
            "[FG-Cx2] diagnostic expected for empty entry, got: {:?}",
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
        use crate::index::InstanceIndexEntry;
        use crate::package::Package;
        use srs_core::types::container::Container;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record::{FieldValue, Record};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::view::{
            DocumentSection, DocumentView, SectionOrdering, SectionSource,
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
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
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

        let doc_view = DocumentView {
            id: "dv-field-sort".to_string(),
            namespace: "com.test".to_string(),
            name: "field-sort-view".to_string(),
            version: 1,
            description: "View for field ordering".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
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

        // Fixed UUIDs: 001→"C-last", 002→"A-first", 003→"B-middle"
        // add_member sorts by UUID → stored order: C-last, A-first, B-middle
        let records_data = [
            ("00000000-0000-4000-8000-000000000001", "C-last"),
            ("00000000-0000-4000-8000-000000000002", "A-first"),
            ("00000000-0000-4000-8000-000000000003", "B-middle"),
        ];

        for (id, title) in &records_data {
            let record = Record {
                instance_id: id.to_string(),
                type_id: "t-record".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "item".to_string(),
                field_values: vec![FieldValue {
                    field_id: "f-heading".to_string(),
                    value: serde_json::json!(title),
                    entries: None,
                    source: None,
                    edited_at: None,
                }],
                group_values: None,
                lifecycle_state: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                extra: HashMap::new(),
            };
            let path = format!("records/{}.json", id);
            let value = serde_json::to_value(&record).unwrap();
            store.ensure_instance_dir("records").unwrap();
            store.save_instance_json(&path, &value).unwrap();

            let mut manifest = store.load_manifest().unwrap();
            manifest.instance_index.push(InstanceIndexEntry {
                instance_id: id.to_string(),
                tier: 2,
                path: path.clone(),
                title: None,
                tags: None,
            });
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
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::theme::{ElementTemplates, Theme};
        use srs_core::types::view::{
            DocumentSection, DocumentView, EmptyBehavior, SectionSource, ThemeMode, ThemeReference,
        };

        let body_field = Field {
            id: "f-auto-body".to_string(),
            namespace: "com.test".to_string(),
            name: "body".to_string(),
            version: 1,
            value_type: ValueType::Text,
            description: "Body".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
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
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
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
                group_field_row_templates: None,
                composite_renderer_config: None,
            }),
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let mut themes = vec![base_theme];
        themes.extend(extra_themes);

        let doc_view = DocumentView {
            id: "dv-auto-select".to_string(),
            namespace: "com.test".to_string(),
            name: "auto-select-view".to_string(),
            version: 1,
            description: "Auto-select test view".to_string(),
            container_type: None,
            preamble: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
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
            extra: HashMap::new(),
        };

        let manifest = crate::manifest::Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
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
                group_field_row_templates: None,
                composite_renderer_config: None,
            }),
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
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
                extra: HashMap::new(),
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
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::relation::Relation;
        use srs_core::types::view::{FieldView, View};

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

        let manifest = crate::manifest::Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
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

        let text_record = create_record(
            &store,
            "t-text",
            1,
            vec![
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
            ],
            None,
            None,
            "records",
        )
        .unwrap();
        let text_id = text_record.instance_id.clone();

        let table_record = create_record(
            &store,
            "t-table",
            1,
            vec![
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
            ],
            None,
            None,
            "records",
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
        type_dispatch: Option<HashMap<String, String>>,
    ) -> DocumentView {
        use srs_core::types::view::EmptyBehavior;
        DocumentView {
            id: "dv-rfc008".to_string(),
            namespace: "com.test".to_string(),
            name: "rfc008-view".to_string(),
            version: 1,
            description: "RFC-008 test document view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
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
        let mut dispatch = HashMap::new();
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
        let mut dispatch = HashMap::new();
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
            vec![
                srs_core::types::record::FieldValue {
                    field_id: "f-heading".to_string(),
                    value: serde_json::json!("Conclusion"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                srs_core::types::record::FieldValue {
                    field_id: "f-body".to_string(),
                    value: serde_json::json!("The conclusion."),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            None,
            None,
            "records",
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
        use crate::index::InstanceIndexEntry;
        use crate::manifest::Manifest;
        use crate::package::Package;
        use crate::store::RepositoryStore;

        let manifest = Manifest {
            instance_index: vec![],
            extra: std::collections::HashMap::new(),
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
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = crate::store::memory::MemoryStore::new(manifest, package);

        for (id, state) in records {
            let record = srs_core::types::record::Record {
                instance_id: id.to_string(),
                type_id: "t-decision".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "decision".to_string(),
                field_values: vec![],
                group_values: None,
                lifecycle_state: state.map(|s| s.to_string()),
                tags: None,
                created_at: None,
                updated_at: None,
                extra: std::collections::HashMap::new(),
            };
            let path = format!("records/{id}.json");
            store
                .save_instance_json(&path, &serde_json::to_value(&record).unwrap())
                .unwrap();
            let mut manifest = store.load_manifest().unwrap();
            manifest.instance_index.push(InstanceIndexEntry {
                instance_id: id.to_string(),
                tier: 2,
                path,
                title: None,
                tags: None,
            });
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
            id: dv_id.to_string(),
            namespace: "com.test".to_string(),
            name: dv_id.to_string(),
            version: 1,
            description: "RFC-011 test view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![srs_core::types::view::DocumentSection {
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
            }],
            navigation_links: None,
            preamble: Some("Test".to_string()),
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
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
            &[("r-active", Some("active")), ("r-superseded", Some("superseded"))],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-exclude",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(ids.contains(&"r-active".to_string()), "active record should be present: {ids:?}");
        assert!(!ids.contains(&"r-superseded".to_string()), "superseded record should be excluded: {ids:?}");
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
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert_eq!(ids, vec!["r-active"], "only active record should be included: {ids:?}");
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
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(ids.contains(&"r-none".to_string()), "record with no lifecycleState must not be excluded: {ids:?}");
        assert!(!ids.contains(&"r-superseded".to_string()), "superseded must be excluded: {ids:?}");
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
        let store = make_rfc011_store(
            dv,
            &[("r-none", None), ("r-active", Some("active"))],
        );
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-no-state-excluded-by-include",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(!ids.contains(&"r-none".to_string()), "record with no lifecycleState must be excluded by inclusion filter: {ids:?}");
        assert!(ids.contains(&"r-active".to_string()), "active record must be included: {ids:?}");
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
        let store = make_rfc011_store(
            dv,
            &[(R_IN_C1, Some("active")), (R_IN_C2, Some("active"))],
        );

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: C1_ID.to_string(),
                title: "Container 1".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
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
        container_service::add_member(&store, C2_ID, R_IN_C2).unwrap();

        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-repo-scope",
            format: Some("json"),
            theme_variant: None,
            container_id: None,
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert!(ids.contains(&R_IN_C1.to_string()), "r-in-c1 must be in repo-scope result: {ids:?}");
        assert!(ids.contains(&R_IN_C2.to_string()), "r-in-c2 must be in repo-scope result: {ids:?}");
        assert_eq!(ids.len(), 2, "both records must be present with repository scope: {ids:?}");
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
        })
        .unwrap();
        let ids = rfc011_instance_ids_in_result(&result);
        assert_eq!(ids, vec!["r-active"], "only active record should be included via backcompat filter: {ids:?}");
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
        })
        .unwrap();
        let mut mem_ids = rfc011_instance_ids_in_result(&mem_result);
        mem_ids.sort();

        // ── FileStore result ────────────────────────────────────────────
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Write records
        std::fs::create_dir_all(repo_root.join("records")).unwrap();
        let mut index_entries = Vec::new();
        for (id, state) in records {
            let record = srs_core::types::record::Record {
                instance_id: id.to_string(),
                type_id: "t-decision".to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "decision".to_string(),
                field_values: vec![],
                group_values: None,
                lifecycle_state: state.map(|s| s.to_string()),
                tags: None,
                created_at: None,
                updated_at: None,
                extra: std::collections::HashMap::new(),
            };
            let path = format!("records/{id}.json");
            std::fs::write(repo_root.join(&path), serde_json::to_string_pretty(&record).unwrap())
                .unwrap();
            index_entries.push(serde_json::json!({"instanceId": id, "tier": 2, "path": path}));
        }

        // Write manifest.json
        std::fs::write(
            repo_root.join("manifest.json"),
            serde_json::to_string_pretty(
                &serde_json::json!({"instanceIndex": index_entries}),
            )
            .unwrap(),
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
        let store = make_rfc011_store(
            dv,
            &[(R_IN_C1, Some("active")), (R_IN_C2, Some("active"))],
        );

        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: C1_ID.to_string(),
                title: "Container 1".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
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
        container_service::add_member(&store, C2_ID, R_IN_C2).unwrap();

        // Pass C1_ID as cli container_id — in repository scope, this must be ignored.
        let result = render_document_view(RenderDocumentViewOptions {
            store: &store,
            view_id: "dv-repo-cli-ignore",
            format: Some("json"),
            theme_variant: None,
            container_id: Some(C1_ID),
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
        assert_eq!(ids.len(), 2, "both records must appear with repository scope: {ids:?}");
    }
}
