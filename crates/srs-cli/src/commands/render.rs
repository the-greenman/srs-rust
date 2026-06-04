use crate::commands::{with_store, CliContext, RenderCommand};
use crate::output;
use crate::payload::{
    DocumentViewProjection, ProjectedFieldGroup, ProjectedGroupEntry, ProjectedRecord,
    ProjectedSection, RenderDocumentViewPayload,
};
use anyhow::Result;
use srs_repository::render_service::{
    render_document_view, DocumentViewProjection as SvcProjection,
    ProjectedFieldGroup as SvcFieldGroup, ProjectedGroupEntry as SvcGroupEntry,
    ProjectedRecord as SvcRecord, ProjectedSection as SvcSection, RenderDocumentViewOptions,
};
use std::path::PathBuf;

pub fn dispatch(ctx: CliContext, cmd: RenderCommand) -> Result<String> {
    match cmd {
        RenderCommand::DocumentView {
            view,
            view_format,
            theme_variant,
            output,
        } => cmd_render_document_view(ctx, view, view_format, theme_variant, output),
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
    output_path: Option<PathBuf>,
) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(render_document_view(RenderDocumentViewOptions {
            store,
            view_id: &view_id,
            format: format.as_deref(),
            theme_variant: theme_variant.as_deref(),
            container_id: ctx.container_id.as_deref(),
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
