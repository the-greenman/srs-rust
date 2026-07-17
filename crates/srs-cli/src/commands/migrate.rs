// `srs migrate packet` is a read-only analysis/export command — it assembles a handoff
// packet for external AI tooling but does not modify the repository. It is not an upgrade
// migration and does not correspond to a `MIGRATIONS` registry entry (see ADR-032).

use crate::commands::{with_store, CliContext, MigrateCommand};
use crate::output;
use anyhow::{anyhow, Result};
use srs_repository::analysis::build_migration_packet_for_profile;

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

    let packet = with_store(&ctx, |store| {
        Ok(build_migration_packet_for_profile(store, "foundation")?)
    })?;
    Ok(output::ok("migrate packet", serde_json::to_value(packet)?))
}
