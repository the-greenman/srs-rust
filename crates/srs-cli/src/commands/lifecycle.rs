use crate::commands::{with_store, CliContext, LifecycleCommand};
use crate::output;
use crate::payload::{LifecycleGetPayload, LifecycleListPayload};
use anyhow::Result;
use srs_repository::lifecycle_service;

pub fn dispatch(ctx: CliContext, cmd: LifecycleCommand) -> Result<String> {
    match cmd {
        LifecycleCommand::List { json: _ } => cmd_lifecycle_list(ctx),
        LifecycleCommand::Get { id, json: _ } => cmd_lifecycle_get(ctx, id),
    }
}

fn cmd_lifecycle_list(ctx: CliContext) -> Result<String> {
    let lifecycles = with_store(&ctx, |store| Ok(lifecycle_service::list_lifecycles(store)?))?;
    output::serialize("lifecycle list", LifecycleListPayload { lifecycles })
}

fn cmd_lifecycle_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(lifecycle_service::get_lifecycle_by_id(store, &id)?)
    })? {
        Some(lifecycle) => output::serialize(
            "lifecycle get",
            LifecycleGetPayload::Found {
                lifecycle: Box::new(lifecycle),
            },
        ),
        None => output::serialize("lifecycle get", LifecycleGetPayload::NotFound { id }),
    }
}
