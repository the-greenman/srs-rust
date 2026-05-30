use crate::commands::{CliContext, MigrateCommand};
use crate::output;
use anyhow::{anyhow, Result};
use srs_repository::analysis::{build_migration_packet, load_analysis_profile};
use srs_repository::FileStore;

pub fn dispatch(ctx: CliContext, cmd: MigrateCommand) -> Result<String> {
    match cmd {
        MigrateCommand::Packet {
            foundation,
            json: _,
        } => cmd_migrate_packet(ctx, foundation),
    }
}

fn cmd_migrate_packet(ctx: CliContext, foundation: bool) -> Result<String> {
    if !foundation {
        return Err(anyhow!(
            "migrate packet currently requires the --foundation profile"
        ));
    }

    let store = FileStore::new(&ctx.repo);
    let profile = load_analysis_profile(&store, "foundation")?;
    let packet = build_migration_packet(&store, &profile.profile_id, &profile.include_tags)?;
    Ok(output::ok("migrate packet", serde_json::to_value(packet)?))
}
