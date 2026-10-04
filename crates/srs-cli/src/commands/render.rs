use crate::commands::{with_store, CliContext, RenderCommand};
use crate::output;
use crate::payload::{
    CompositionProjection, ExportBundlePayload, OkfBundlePayload, RenderCompositionPayload,
    RenderMarkdownPayload,
};
use anyhow::Result;
use srs_repository::export_service::{export_record_bundle, ExportBundleInput};
use srs_repository::okf_export_service::{
    group_relation_links_by_type, OkfBundle, OkfEntry, OkfExportInput,
};
use srs_repository::render_service::{render_composition, RenderCompositionOptions};
use std::path::{Path, PathBuf};

pub fn dispatch(ctx: CliContext, cmd: RenderCommand) -> Result<String> {
    match cmd {
        RenderCommand::Composition {
            view,
            view_format,
            theme_variant,
            instance,
            exclude,
            output,
        } => cmd_render_composition(
            ctx,
            view,
            view_format,
            theme_variant,
            instance,
            exclude,
            output,
        ),
        RenderCommand::ExportBundle {
            view,
            instance,
            output,
        } => cmd_render_export_bundle(ctx, view, instance, output),
        RenderCommand::OkfBundle {
            container_id,
            output,
        } => cmd_render_okf_bundle(ctx, container_id, output),
        RenderCommand::Markdown => {
            let mut md = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut md)?;
            output::serialize(
                "render markdown",
                RenderMarkdownPayload {
                    html: srs_core::markdown::render_markdown(&md),
                },
            )
        }
    }
}

fn cmd_render_composition(
    ctx: CliContext,
    view_id: String,
    format: Option<String>,
    theme_variant: Option<String>,
    instance: Option<String>,
    exclude: Vec<String>,
    output_path: Option<PathBuf>,
) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(render_composition(RenderCompositionOptions {
            store,
            view_id: &view_id,
            format: format.as_deref(),
            theme_variant: theme_variant.as_deref(),
            container_id: ctx.container_id.as_deref(),
            exclude_instance_ids: &exclude,
            instance_id_filter: instance.as_deref(),
        })?)
    }) {
        Ok(result) => {
            let projection = result.projection.map(CompositionProjection::from);
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
                "render composition",
                RenderCompositionPayload {
                    rendered: result.rendered,
                    diagnostics: result.diagnostics,
                    projection,
                },
            )
        }
        Err(e) => Ok(output::err("render composition", vec![e.to_string()])),
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
        let body = build_body(entry);
        let heading = entry.display_label.replace('\n', " ").replace('\r', "");
        let related = build_related_section(entry);
        let content = format!("{frontmatter}\n# {heading}\n\n{body}{related}");
        let entry_path = dir.join(&entry.path);
        if let Some(parent) = entry_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!("cannot create output directory {:?}: {}", parent, e)
            })?;
        }
        std::fs::write(&entry_path, content.as_bytes())
            .map_err(|e| anyhow::anyhow!("failed to write {:?}: {}", entry.path, e))?;
        index_lines.push(format!("- [{}]({})", entry.display_label, entry.path));
    }

    std::fs::write(&index_path, index_lines.join("\n").as_bytes())
        .map_err(|e| anyhow::anyhow!("failed to write index.md: {}", e))?;

    // entry files + index.md
    Ok(bundle.entries.len() + 1)
}

/// Tier-0 note content (`note_text`), if any, followed by one `## <heading>`
/// section per text-formatted field (srs-rust#1106) — OKF's frontmatter is for
/// metadata, markdown body for content, which a `text`/markdown field is.
fn build_body(entry: &OkfEntry) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(note_text) = entry.note_text.as_deref() {
        if !note_text.is_empty() {
            parts.push(note_text.to_string());
        }
    }
    for (heading, content) in &entry.body_sections {
        parts.push(format!("## {heading}\n\n{content}"));
    }
    parts.join("\n\n")
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
    for (relation_type, paths) in group_relation_links_by_type(&entry.outgoing_relations) {
        let list = paths.join(", ");
        lines.push(format!("{relation_type}: [{list}]"));
    }
    lines.push("---".to_string());
    lines.join("\n")
}

/// A human-readable `## Related` section — the frontmatter list above is the
/// machine-readable contract, this is what an agent reading the file sees.
fn build_related_section(entry: &OkfEntry) -> String {
    if entry.outgoing_relations.is_empty() {
        return String::new();
    }
    let mut lines = vec![String::new(), "## Related".to_string(), String::new()];
    for link in &entry.outgoing_relations {
        lines.push(format!(
            "- {}: [{}]({})",
            link.relation_type, link.target_display_label, link.target_path
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}
