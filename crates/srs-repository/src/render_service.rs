use crate::container_service::list_members;
use crate::error::RepositoryError;
use crate::manifest::load_manifest;
use crate::package::{load_package, Package};
use crate::record_store::{get_record_by_id, list_records_by_type};
use crate::relation_service::load_relations;
use srs_core::types::field::ValueType;
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use srs_core::types::view::{
    DocumentSection, DocumentView, RelationDirection, SectionSource, SortDirection,
};
use std::collections::{HashMap, HashSet};
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
    let format = opts
        .format
        .unwrap_or(dv.format.as_deref().unwrap_or("markdown"));
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
    repo_root: &Path,
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
        if let Some(record) = get_record_by_id(repo_root, id)? {
            subsections.push(record);
        }
    }

    Ok(sort_by_precedes_chain(subsections, relations))
}

fn render_section(
    repo_root: &Path,
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    relations: &[Relation],
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let mut records = resolve_section_instances(repo_root, section, relations, diagnostics)?;
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
    } else if matches!(&section.source, SectionSource::TypeQuery { .. }) {
        // For TypeQuery sections without explicit ordering, use precedes-chain sort.
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
            repo_root,
            ctx,
            section,
            record,
            record_heading_level,
            relations,
            diagnostics,
        )?);
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

fn render_record_at_level(
    repo_root: &Path,
    ctx: &RenderContext<'_>,
    section: &DocumentSection,
    record: &Record,
    heading_level: u32,
    relations: &[Relation],
    diagnostics: &mut Vec<String>,
) -> Result<String, RepositoryError> {
    let mut out = String::new();
    let rt = ctx
        .package
        .resolve_type(&record.type_id, record.type_version)
        .cloned();

    let structured = section.title_field_id.is_some();

    if let Some(title_field_id) = &section.title_field_id {
        if let Some(title) = record.get_field_value_str(title_field_id) {
            out.push_str(&format!(
                "{}{}\n\n",
                heading_prefix(heading_level, ctx.format),
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

        // In structured mode, Text/Multiselect fields render as plain prose (no label prefix).
        if structured
            && matches!(
                field_type,
                Some(ValueType::Text) | Some(ValueType::Multiselect)
            )
        {
            if !value_text.is_empty() {
                out.push_str(&value_text);
                out.push_str("\n\n");
            }
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

        if ctx.format == "markdown" {
            out.push_str(&format!("**{}**: {}\n", label, value_text));
        } else {
            out.push_str(&format!("{}: {}\n", label, value_text));
        }
    }
    if let Some(rt) = &rt {
        if let Some(field_groups) = &rt.field_groups {
            out.push_str(&render_field_groups(ctx, rt, record, field_groups));
        }
    }

    // In structured mode, render subsections nested one heading level deeper.
    if structured {
        let subsections = collect_subsections(repo_root, &record.instance_id, relations)?;
        for subsection in &subsections {
            out.push_str(&render_record_at_level(
                repo_root,
                ctx,
                section,
                subsection,
                heading_level + 1,
                relations,
                diagnostics,
            )?);
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
                if ctx.format == "markdown" {
                    out.push_str(&format!("**{}**: {}\n", label, value_text));
                } else {
                    out.push_str(&format!("{}: {}\n", label, value_text));
                }
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

    fn repeatable_fixture_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../srs-cli/tests/fixtures/repeatable-fields")
    }

    #[test]
    fn repeatable_field_entries_render_all_values() {
        let repo_root = repeatable_fixture_root();
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root: &repo_root,
            view_id: "00000000-0000-4000-8000-000000000981",
            format: None,
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
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root: &repo_root,
            view_id: "00000000-0000-4000-8000-000000000982",
            format: None,
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
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root: &repo_root,
            view_id: "00000000-0000-4000-8000-000000000983",
            format: None,
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
        // repeatable-doc-view has no titleFieldId — records render without an H3 heading
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root: &repo_root,
            view_id: "00000000-0000-4000-8000-000000000981",
            format: None,
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
    fn semantic_object_type_missing_slash_emits_diagnostic() {
        let repo_root = repeatable_fixture_root();
        let result = render_document_view(RenderDocumentViewOptions {
            repo_root: &repo_root,
            view_id: "00000000-0000-4000-8000-000000000984",
            format: None,
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
}
