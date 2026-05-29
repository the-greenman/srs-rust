use crate::container_service::list_members;
use crate::error::RepositoryError;
use crate::manifest::load_manifest;
use crate::package::{load_package, Package};
use crate::record_store::{get_record_by_id, list_records_by_type};
use crate::relation_service::load_relations;
use srs_core::types::record::Record;
use srs_core::types::view::{
    DocumentSection, DocumentView, RelationDirection, SectionSource, SortDirection,
};
use std::path::Path;

pub struct RenderDocumentViewOptions<'a> {
    pub repo_root: &'a Path,
    pub view_id: &'a str,
    pub format: Option<&'a str>,
}

pub struct RenderResult {
    pub rendered: String,
    pub diagnostics: Vec<String>,
}

struct RenderContext<'a> {
    package: &'a Package,
    container_title: String,
    depth_offset: u32,
    format: &'a str,
    status_field_id: Option<String>,
}

pub fn render_document_view(
    opts: RenderDocumentViewOptions<'_>,
) -> Result<RenderResult, RepositoryError> {
    let package = load_package(opts.repo_root)?;
    let dv = package.resolve_document_view(opts.view_id).ok_or_else(|| {
        RepositoryError::DocumentViewNotFound {
            view_id: opts.view_id.to_string(),
        }
    })?;
    let dv = dv.clone();

    let manifest = load_manifest(opts.repo_root)?;
    let mut diagnostics = Vec::new();
    let container_title = resolve_container_title(&dv, &manifest);
    let relations = load_relations(opts.repo_root)?;
    let format = opts.format.unwrap_or(dv.format.as_deref().unwrap_or("markdown"));
    let depth_offset = dv.depth_offset.unwrap_or(0);
    if depth_offset > 4 {
        diagnostics.push(format!(
            "[N+4b] depthOffset {depth_offset} exceeds 4; heading levels may exceed what standard renderers support"
        ));
    }

    let ctx = RenderContext {
        package: &package,
        container_title,
        depth_offset,
        format,
        status_field_id: package.find_field_by_name("status").map(|f| f.id.clone()),
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
            opts.repo_root,
            &ctx,
            section,
            &relations,
            &mut diagnostics,
        )?);
    }

    Ok(RenderResult {
        rendered,
        diagnostics,
    })
}

fn resolve_container_title(dv: &DocumentView, manifest: &crate::manifest::Manifest) -> String {
    if let Some(container_type) = &dv.container_type {
        if let Some(container_index) = manifest.extra.get("containerIndex") {
            if let Some(entries) = container_index.as_array() {
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

    manifest
        .extra
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("SRS")
        .to_string()
}

fn render_section(
    repo_root: &Path,
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    relations: &[srs_core::types::relation::Relation],
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let mut records = resolve_section_instances(repo_root, section, relations, diagnostics)?;
    if records.is_empty() && section.required != Some(true) {
        return Ok(String::new());
    }
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

    for record in &records {
        out.push_str(&render_record(ctx, section, record, diagnostics));
    }
    Ok(out)
}

fn resolve_section_instances(
    repo_root: &Path,
    section: &DocumentSection,
    relations: &[srs_core::types::relation::Relation],
    diagnostics: &mut Vec<String>,
) -> Result<Vec<Record>, RepositoryError> {
    match &section.source {
        SectionSource::FixedInstances { instance_ids } => {
            let mut records = Vec::new();
            for id in instance_ids {
                if let Some(record) = get_record_by_id(repo_root, id)? {
                    records.push(record);
                }
            }
            Ok(records)
        }
        SectionSource::TypeQuery {
            semantic_object_type,
            ..
        } => {
            let Some((namespace, name)) = semantic_object_type.split_once('/') else {
                diagnostics.push(format!(
                    "[N] TypeQuery semanticObjectType '{}' has no namespace separator '/' — expected 'namespace/name' format",
                    semantic_object_type
                ));
                return Ok(Vec::new());
            };
            list_records_by_type(repo_root, namespace, name)
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
                if let Some(record) = get_record_by_id(repo_root, &id)? {
                    records.push(record);
                }
            }
            Ok(records)
        }
        SectionSource::ContainerSubset {
            container_id,
            container_type: _,
        } => {
            let members = list_members(repo_root, container_id)?;
            let mut records = Vec::new();
            for id in members {
                if let Some(record) = get_record_by_id(repo_root, &id)? {
                    records.push(record);
                }
            }
            Ok(records)
        }
    }
}

fn render_record(
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    record: &Record,
    diagnostics: &mut Vec<String>,
) -> String {
    let mut out = String::new();
    let rt = ctx
        .package
        .resolve_type(&record.type_id, record.type_version)
        .cloned();

    if let Some(title_field_id) = &section.title_field_id {
        if let Some(title) = record.get_field_value_str(title_field_id) {
            out.push_str(&format!(
                "{}{}\n\n",
                heading_prefix(depth(3, ctx.depth_offset), ctx.format),
                title
            ));
        }
    }

    let mut field_ids: Vec<String> = Vec::new();
    let mut display_labels = std::collections::HashMap::new();
    let mut omit_empty = false;

    if let Some(view_id) = &section.render_view_id {
        if let Some(view) = ctx.package.resolve_view(view_id) {
            if let Some(export_config) = &view.export_config {
                if let Some(preamble) = &export_config.preamble {
                    out.push_str(&substitute_vars(preamble, ctx, Some(record), true));
                    out.push('\n');
                }
                omit_empty = export_config.omit_empty_fields == Some(true);
                if let Some(order) = &export_config.field_order {
                    field_ids = order.clone();
                }
            }
            if field_ids.is_empty() {
                let mut field_views = view.field_views.clone();
                field_views.sort_by_key(|fv| fv.order);
                for fv in field_views {
                    if fv.visible == Some(false) {
                        continue;
                    }
                    if let Some(label) = fv.display_label {
                        display_labels.insert(fv.field_id.clone(), label);
                    }
                    field_ids.push(fv.field_id);
                }
            }
        }
    } else if let Some(rt) = &rt {
        if rt.extra.contains_key("fieldOrder") {
            diagnostics.push(
                "[partial] ext:type-inheritance fieldOrder ignored; using FieldAssignment.order"
                    .to_string(),
            );
        }
        let mut assignments = rt.fields.clone();
        assignments.sort_by_key(|fa| fa.order);
        for fa in assignments {
            if let Some(label) = fa.display_label {
                display_labels.insert(fa.field_id.clone(), label);
            }
            field_ids.push(fa.field_id);
        }
    } else {
        for fv in &record.field_values {
            field_ids.push(fv.field_id.clone());
        }
    }

    for field_id in field_ids {
        let value = record.find_field_value(&field_id).map(|fv| &fv.value);
        if let Some(v) = value {
            if let Some(entries) = v.get("entries").and_then(|e| e.as_array()) {
                if !entries.is_empty() {
                    diagnostics.push(format!(
                        "[partial] repeatable field {} rendered as first entry only; ext:repeatable-fields not fully supported",
                        field_id
                    ));
                }
            }
        }

        let rendered_value = value
            .and_then(value_to_text)
            .map(std::string::ToString::to_string);
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

        let label = display_labels
            .get(&field_id)
            .cloned()
            .or_else(|| {
                rt.as_ref()
                    .and_then(|t| t.find_field_assignment(&field_id))
                    .and_then(|fa| fa.display_label.clone())
            })
            .or_else(|| ctx.package.resolve_field(&field_id).map(|f| f.name.clone()))
            .unwrap_or_else(|| field_id.clone());

        let value_text = rendered_value.unwrap_or_else(|| "(empty)".to_string());
        if ctx.format == "markdown" {
            out.push_str(&format!("**{}**: {}\n", label, value_text));
        } else {
            out.push_str(&format!("{}: {}\n", label, value_text));
        }
    }
    out.push('\n');
    out
}

fn value_to_text(value: &serde_json::Value) -> Option<&str> {
    // Baseline renderer currently supports scalar string field values only.
    // Non-string values are intentionally left unrendered for now.
    value.as_str()
}

fn substitute_vars(
    template: &str,
    ctx: &RenderContext<'_>,
    record: Option<&Record>,
    section_context: bool,
) -> String {
    let mut out = template.to_string();
    out = out.replace("{{container-title}}", &ctx.container_title);
    out = out.replace("{{date}}", &chrono::Utc::now().format("%Y-%m-%d").to_string());
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
        let repo_root = Path::new("/home/greenman/dev/semanticops/srs/srs");
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root,
            view_id: "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6",
            format: None,
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
        let repo_root = Path::new("/home/greenman/dev/semanticops/srs/srs");
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root,
            view_id: "00000000-0000-0000-0000-000000000000",
            format: None,
        });
        assert!(matches!(
            result,
            Err(RepositoryError::DocumentViewNotFound { .. })
        ));
    }
}
