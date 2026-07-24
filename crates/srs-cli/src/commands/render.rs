use crate::commands::{with_store, CliContext, RenderCommand};
use crate::output;
use crate::payload::{
    DocumentViewProjection, ExportBundlePayload, OkfBundlePayload, ProjectedFieldGroup,
    ProjectedGroupEntry, ProjectedRecord, ProjectedRelationRow, ProjectedRelationTarget,
    ProjectedSection, RenderDocumentViewPayload,
};
use anyhow::Result;
use srs_repository::export_service::{export_record_bundle, ExportBundleInput};
use srs_repository::okf_export_service::{OkfBundle, OkfEntry, OkfExportInput};
use srs_repository::render_service::{
    render_document_view, DocumentViewProjection as SvcProjection,
    ProjectedFieldGroup as SvcFieldGroup, ProjectedGroupEntry as SvcGroupEntry,
    ProjectedRecord as SvcRecord, ProjectedRelationRow as SvcRelationRow,
    ProjectedRelationTarget as SvcRelationTarget, ProjectedSection as SvcSection,
    RenderDocumentViewOptions,
};
use std::path::{Path, PathBuf};

pub fn dispatch(ctx: CliContext, cmd: RenderCommand) -> Result<String> {
    match cmd {
        RenderCommand::DocumentView {
            view,
            view_format,
            theme_variant,
            instance,
            output,
        } => cmd_render_document_view(ctx, view, view_format, theme_variant, instance, output),
        RenderCommand::ExportBundle {
            view,
            instance,
            output,
        } => cmd_render_export_bundle(ctx, view, instance, output),
        RenderCommand::OkfBundle {
            container_id,
            output,
        } => cmd_render_okf_bundle(ctx, container_id, output),
    }
}

fn map_group_entry(e: SvcGroupEntry) -> ProjectedGroupEntry {
    ProjectedGroupEntry {
        entry_id: e.entry_id,
        fields: e.fields,
    }
}

fn map_field_group(g: SvcFieldGroup) -> ProjectedFieldGroup {
    ProjectedFieldGroup {
        group_id: g.group_id,
        label: g.label,
        entries: g.entries.into_iter().map(map_group_entry).collect(),
    }
}

fn map_relation_target(t: SvcRelationTarget) -> ProjectedRelationTarget {
    ProjectedRelationTarget {
        instance_id: t.instance_id,
        display_label: t.display_label,
    }
}

fn map_relation_row(row: SvcRelationRow) -> ProjectedRelationRow {
    ProjectedRelationRow {
        label: row.label,
        targets: row.targets.into_iter().map(map_relation_target).collect(),
    }
}

fn map_record(r: SvcRecord) -> ProjectedRecord {
    ProjectedRecord {
        instance_id: r.instance_id,
        type_id: r.type_id,
        type_namespace: r.type_namespace,
        type_name: r.type_name,
        record_heading: r.record_heading,
        preamble: r.preamble,
        fields: r.fields,
        ordered_field_keys: r.ordered_field_keys,
        field_groups: r
            .field_groups
            .map(|gs| gs.into_iter().map(map_field_group).collect()),
        relations: r
            .relations
            .map(|rows| rows.into_iter().map(map_relation_row).collect()),
    }
}

fn map_section(s: SvcSection) -> ProjectedSection {
    ProjectedSection {
        section_id: s.section_id,
        title: s.title,
        order: s.order,
        records: s.records.into_iter().map(map_record).collect(),
    }
}

fn map_projection(p: SvcProjection) -> DocumentViewProjection {
    DocumentViewProjection {
        schema: p.schema,
        document_view_id: p.document_view_id,
        container_id: p.container_id,
        generated_at: p.generated_at,
        container_title: p.container_title,
        preamble: p.preamble,
        sections: p.sections.into_iter().map(map_section).collect(),
    }
}

fn cmd_render_document_view(
    ctx: CliContext,
    view_id: String,
    format: Option<String>,
    theme_variant: Option<String>,
    instance: Option<String>,
    output_path: Option<PathBuf>,
) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(render_document_view(RenderDocumentViewOptions {
            store,
            view_id: &view_id,
            format: format.as_deref(),
            theme_variant: theme_variant.as_deref(),
            container_id: ctx.container_id.as_deref(),
            instance_id_filter: instance.as_deref(),
        })?)
    }) {
        Ok(result) => {
            let projection = result.projection.map(map_projection);
            if let Some(path) = output_path {
                // Output delivery: writing caller-specified --output path is thin I/O glue,
                // not repository management. This is intentionally in the CLI layer.
                let content = if let Some(ref proj) = projection {
                    serde_json::to_string_pretty(proj)
                        .map_err(|e| anyhow::anyhow!("failed to serialize projection: {}", e))?
                } else {
                    result.rendered.clone()
                };
                std::fs::write(&path, content.as_bytes()).map_err(|e| {
                    anyhow::anyhow!("failed to write output file {:?}: {}", path, e)
                })?;
            }
            output::serialize(
                "render document-view",
                RenderDocumentViewPayload {
                    rendered: result.rendered,
                    diagnostics: result.diagnostics,
                    projection,
                },
            )
        }
        Err(e) => Ok(output::err("render document-view", vec![e.to_string()])),
    }
}

fn cmd_render_export_bundle(
    ctx: CliContext,
    view_id: String,
    instance_id: String,
    output_path: PathBuf,
) -> Result<String> {
    let mut file = std::fs::File::create(&output_path)
        .map_err(|e| anyhow::anyhow!("cannot create output file {:?}: {}", output_path, e))?;
    match with_store(&ctx, |store| {
        Ok(export_record_bundle(
            store,
            ExportBundleInput {
                instance_id: instance_id.clone(),
                view_id: view_id.clone(),
                format: None,
            },
            &mut file,
        )?)
    }) {
        Ok(meta) => output::serialize(
            "render export-bundle",
            ExportBundlePayload {
                rendered_filename: meta.rendered_filename,
                attachment_count: meta.attachment_count,
                output_path: output_path.to_string_lossy().into_owned(),
                diagnostics: meta.diagnostics,
            },
        ),
        Err(e) => Ok(output::err("render export-bundle", vec![e.to_string()])),
    }
}

fn cmd_render_okf_bundle(
    ctx: CliContext,
    container_id: String,
    output_path: PathBuf,
) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(srs_repository::export_okf_bundle(
            store,
            OkfExportInput {
                container_id: container_id.clone(),
            },
        )?)
    }) {
        Ok(bundle) => {
            std::fs::create_dir_all(&output_path).map_err(|e| {
                anyhow::anyhow!("cannot create output directory {:?}: {}", output_path, e)
            })?;
            let file_count = write_okf_bundle_to_dir(&bundle, &output_path)?;
            output::serialize(
                "render okf-bundle",
                OkfBundlePayload {
                    file_count,
                    output_dir: output_path.to_string_lossy().into_owned(),
                    diagnostics: bundle.diagnostics,
                },
            )
        }
        Err(e) => Ok(output::err("render okf-bundle", vec![e.to_string()])),
    }
}

fn write_okf_bundle_to_dir(bundle: &OkfBundle, dir: &Path) -> Result<usize> {
    let index_path = dir.join("index.md");
    let mut index_lines: Vec<String> = Vec::new();
    index_lines.push(format!("# {}", bundle.container_title));
    index_lines.push(String::new());

    for entry in &bundle.entries {
        let frontmatter = build_frontmatter(entry);
        let body = entry.note_text.as_deref().unwrap_or("").to_string();
        let heading = entry.display_label.replace('\n', " ").replace('\r', "");
        let content = format!("{frontmatter}\n# {heading}\n\n{body}");
        std::fs::write(dir.join(&entry.path), content.as_bytes())
            .map_err(|e| anyhow::anyhow!("failed to write {:?}: {}", entry.path, e))?;
        index_lines.push(format!("- [{}]({})", entry.display_label, entry.path));
    }

    std::fs::write(&index_path, index_lines.join("\n").as_bytes())
        .map_err(|e| anyhow::anyhow!("failed to write index.md: {}", e))?;

    // entry files + index.md
    Ok(bundle.entries.len() + 1)
}

fn build_frontmatter(entry: &OkfEntry) -> String {
    let mut lines = vec![
        "---".to_string(),
        format!("srs_id: {}", entry.instance_id),
        format!("type: {}", entry.type_label),
    ];
    for (name, value) in &entry.field_pairs {
        lines.push(format!("{name}: {value}"));
    }
    lines.push("---".to_string());
    lines.join("\n")
}
