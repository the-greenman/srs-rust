use crate::commands::{CliContext, RenderCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::render_service::{render_document_view, RenderDocumentViewOptions};
use std::path::PathBuf;

pub fn dispatch(ctx: CliContext, cmd: RenderCommand) -> Result<String> {
    match cmd {
        RenderCommand::DocumentView {
            view,
            view_format,
            output,
        } => cmd_render_document_view(ctx, view, view_format, output),
    }
}

fn cmd_render_document_view(
    ctx: CliContext,
    view_id: String,
    format: Option<String>,
    output_path: Option<PathBuf>,
) -> Result<String> {
    match render_document_view(RenderDocumentViewOptions {
        repo_root: &ctx.repo,
        view_id: &view_id,
        format: format.as_deref(),
    }) {
        Ok(result) => {
            if let Some(path) = output_path {
                // Output delivery: writing caller-specified --output path is thin I/O glue,
                // not repository management. This is intentionally in the CLI layer.
                std::fs::write(&path, result.rendered.as_bytes()).map_err(|e| {
                    anyhow::anyhow!("failed to write output file {:?}: {}", path, e)
                })?;
            }
            Ok(output::ok(
                "render document-view",
                json!({
                    "rendered": result.rendered,
                    "diagnostics": result.diagnostics
                }),
            ))
        }
        Err(e) => Ok(output::err("render document-view", vec![e.to_string()])),
    }
}
