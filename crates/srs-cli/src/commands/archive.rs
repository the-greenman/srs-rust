use crate::commands::{with_new_file_store, with_store, ArchiveCommand, CliContext};
use crate::output;
use crate::payload::{ArchivePackPayload, ArchiveUnpackPayload};
use anyhow::Result;
use std::path::PathBuf;

pub fn dispatch(ctx: CliContext, cmd: ArchiveCommand) -> Result<String> {
    match cmd {
        ArchiveCommand::Pack { output } => cmd_archive_pack(ctx, output),
        ArchiveCommand::Unpack { source, target } => cmd_archive_unpack(source, target),
    }
}

fn cmd_archive_pack(ctx: CliContext, output: PathBuf) -> Result<String> {
    let mut file = std::fs::File::create(&output)
        .map_err(|e| anyhow::anyhow!("cannot create output file {:?}: {}", output, e))?;
    with_store(&ctx, |store| {
        srs_repository::archive_pack(store, &mut file).map_err(anyhow::Error::from)
    })?;
    let file_size_bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    output::serialize(
        "archive pack",
        ArchivePackPayload {
            output_path: output.to_string_lossy().into_owned(),
            file_size_bytes,
        },
    )
}

fn cmd_archive_unpack(source: PathBuf, target: PathBuf) -> Result<String> {
    let file = std::fs::File::open(&source)
        .map_err(|e| anyhow::anyhow!("cannot open archive {:?}: {}", source, e))?;
    let repository_id = with_new_file_store(&target, |store| {
        srs_repository::archive_unpack(file, store).map_err(anyhow::Error::from)?;
        let manifest = store.load_manifest().map_err(anyhow::Error::from)?;
        Ok(manifest
            .extra
            .get("repositoryId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string())
    })?;
    output::serialize(
        "archive unpack",
        ArchiveUnpackPayload {
            target_dir: target.to_string_lossy().into_owned(),
            repository_id,
        },
    )
}
