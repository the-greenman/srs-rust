use crate::commands::{with_store, CliContext, InstanceCommand};
use crate::output;
use crate::payload::InstanceGetPayload;
use anyhow::Result;
use srs_repository::error::RepositoryError;
use srs_repository::services::get_instance_by_id;

pub fn dispatch(ctx: CliContext, cmd: InstanceCommand) -> Result<String> {
    match cmd {
        InstanceCommand::Get { id } => cmd_instance_get(ctx, id),
    }
}

fn cmd_instance_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_instance_by_id(store, &id)?))? {
        Some(instance) => output::serialize("instance get", InstanceGetPayload::from(instance)),
        None => Ok(output::repo_err(
            "instance get",
            &RepositoryError::InstanceNotFound { id: id.clone() },
        )),
    }
}
