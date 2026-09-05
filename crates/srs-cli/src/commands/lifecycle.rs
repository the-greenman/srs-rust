use crate::commands::{with_store, CliContext, LifecycleCommand};
use crate::output;
use crate::payload::{
    LifecycleCreatePayload, LifecycleGetPayload, LifecycleListPayload, LifecycleUpdatePayload,
};
use anyhow::Result;
use srs_repository::error::RepositoryError;
use srs_repository::lifecycle_service;

pub fn dispatch(ctx: CliContext, cmd: LifecycleCommand) -> Result<String> {
    match cmd {
        LifecycleCommand::List { json: _ } => cmd_lifecycle_list(ctx),
        LifecycleCommand::Get { id, json: _ } => cmd_lifecycle_get(ctx, id),
        LifecycleCommand::Create { package } => cmd_lifecycle_create(ctx, package),
        LifecycleCommand::Update { id } => cmd_lifecycle_update(ctx, id),
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

fn cmd_lifecycle_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let raw = crate::input::value_from_stdin("lifecycle")?;
    let result = with_store(&ctx, |store| {
        Ok(lifecycle_service::create_lifecycle_normalized(
            store,
            raw,
            package.clone(),
        )?)
    })?;
    output::serialize(
        "lifecycle create",
        LifecycleCreatePayload {
            lifecycle: result.lifecycle,
        },
    )
}

fn cmd_lifecycle_update(ctx: CliContext, id: String) -> Result<String> {
    let raw = crate::input::value_from_stdin("lifecycle")?;
    with_store(&ctx, |store| {
        match lifecycle_service::update_lifecycle_normalized(store, &id, raw.clone()) {
            Ok(result) => output::serialize(
                "lifecycle update",
                LifecycleUpdatePayload {
                    lifecycle: result.lifecycle,
                },
            ),
            // RFC-028: R2a + R5 violations are reported together, one
            // diagnostic per violation, rather than folded into a single
            // message string.
            Err(RepositoryError::LifecycleValidation { violations }) => {
                Ok(output::err("lifecycle update", violations))
            }
            Err(e) => Ok(output::err("lifecycle update", vec![e.to_string()])),
        }
    })
}
