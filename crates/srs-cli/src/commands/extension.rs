use crate::commands::{with_store, CliContext, ExtensionCommand};
use crate::output;
use crate::payload::{DeletedPayload, ExtensionListPayload, ExtensionPayload};
use anyhow::Result;
use srs_repository::extension_service::{
    create_extension, delete_extension, get_extension_by_id, list_extensions, update_extension,
    CreateExtensionInput,
};

pub fn dispatch(ctx: CliContext, cmd: ExtensionCommand) -> Result<String> {
    match cmd {
        ExtensionCommand::List { json: _ } => cmd_extension_list(ctx),
        ExtensionCommand::Get { id, json: _ } => cmd_extension_get(ctx, id),
        ExtensionCommand::Create { json: _ } => cmd_extension_create(ctx),
        ExtensionCommand::Update { id, json: _ } => cmd_extension_update(ctx, id),
        ExtensionCommand::Delete { id, json: _ } => cmd_extension_delete(ctx, id),
    }
}

fn cmd_extension_list(ctx: CliContext) -> Result<String> {
    let extensions = with_store(&ctx, |store| Ok(list_extensions(store)?))?;
    output::serialize("extension list", ExtensionListPayload { extensions })
}

fn cmd_extension_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_extension_by_id(store, &id)?))? {
        Some(extension) => output::serialize("extension get", ExtensionPayload { extension }),
        None => Ok(output::err(
            "extension get",
            vec![format!("Extension '{}' not found", id)],
        )),
    }
}

fn cmd_extension_create(ctx: CliContext) -> Result<String> {
    let raw = crate::input::value_from_stdin("extension")?;

    let result = with_store(&ctx, |store| {
        Ok(create_extension(store, CreateExtensionInput { raw })?)
    })?;

    output::serialize(
        "extension create",
        ExtensionPayload {
            extension: result.record,
        },
    )
}

fn cmd_extension_update(ctx: CliContext, id: String) -> Result<String> {
    let raw = crate::input::value_from_stdin("extension")?;

    let result = with_store(&ctx, |store| {
        Ok(update_extension(store, &id, CreateExtensionInput { raw })?)
    })?;

    output::serialize(
        "extension update",
        ExtensionPayload {
            extension: result.record,
        },
    )
}

fn cmd_extension_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_extension(store, &id)?)) {
        Ok(instance_id) => output::serialize("extension delete", DeletedPayload { instance_id }),
        Err(e) => Ok(output::err("extension delete", vec![e.to_string()])),
    }
}
