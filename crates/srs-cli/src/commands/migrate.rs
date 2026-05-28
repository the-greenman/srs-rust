use crate::commands::{resolve_repo, MigrateCommand, FOUNDATION_SIGNAL_TAGS};
use crate::output;
use anyhow::{anyhow, Result};
use srs_repository::analysis::build_migration_packet;
use std::path::PathBuf;

pub fn dispatch(cmd: MigrateCommand) -> Result<String> {
    match cmd {
        MigrateCommand::Packet {
            repo,
            foundation,
            json: _,
        } => cmd_migrate_packet(repo, foundation),
    }
}

fn cmd_migrate_packet(repo: Option<PathBuf>, foundation: bool) -> Result<String> {
    if !foundation {
        return Err(anyhow!(
            "migrate packet currently requires the --foundation profile"
        ));
    }

    let repo_root = resolve_repo(repo)?;
    let packet = build_migration_packet(&repo_root, "foundation", FOUNDATION_SIGNAL_TAGS)?;
    Ok(output::ok("migrate packet", serde_json::to_value(packet)?))
}
