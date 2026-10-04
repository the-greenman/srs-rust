use crate::commands::{with_store, CliContext, SliceCommand};
use crate::output;
use crate::payload::SliceExportPayload;
use anyhow::Result;
use srs_repository::slice_service::{export_container_slice, ExportSliceInput};
use std::path::PathBuf;

pub fn dispatch(ctx: CliContext, cmd: SliceCommand) -> Result<String> {
    match cmd {
        SliceCommand::Export { output } => cmd_slice_export(ctx, output),
    }
}

fn cmd_slice_export(ctx: CliContext, output: PathBuf) -> Result<String> {
    let container_id = ctx
        .container_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("slice export requires --container <id>"))?;
    let export = with_store(&ctx, |store| {
        Ok(export_container_slice(
            store,
            ExportSliceInput {
                container_id,
                ..Default::default()
            },
        )?)
    })?;
    std::fs::write(&output, &export.bytes)
        .map_err(|e| anyhow::anyhow!("cannot write output file {:?}: {}", output, e))?;
    let s = export.summary;
    output::serialize(
        "slice export",
        SliceExportPayload {
            output_path: output.to_string_lossy().into_owned(),
            file_size_bytes: export.bytes.len() as u64,
            container_id: s.container_id,
            slice_repository_id: s.slice_repository_id,
            origin_repository_id: s.origin_repository_id,
            exported_at: s.exported_at,
            instance_count: s.instance_count,
            relation_count: s.relation_count,
            container_count: s.container_count,
            source_document_count: s.source_document_count,
            package_count: s.package_count,
            external_relation_ref_count: s.external_relation_ref_count,
        },
    )
}
