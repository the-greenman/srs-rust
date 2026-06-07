use crate::commands::{with_store, CliContext, TagCommand};
use crate::output;
use crate::payload::{TagListPayload, TagPayload};
use anyhow::Result;
use srs_repository::tag_service;

pub fn dispatch(ctx: CliContext, cmd: TagCommand) -> Result<String> {
    match cmd {
        TagCommand::List { json: _ } => cmd_tag_list(ctx),
        TagCommand::Get { id, json: _ } => cmd_tag_get(ctx, id),
    }
}

fn cmd_tag_list(ctx: CliContext) -> Result<String> {
    let terms = with_store(&ctx, |store| Ok(tag_service::list_terms(store)?))?;
    output::serialize("tag list", TagListPayload { terms })
}

fn cmd_tag_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(tag_service::get_term_by_id(store, &id)?))? {
        Some(term) => output::serialize("tag get", TagPayload { term }),
        None => Ok(output::err(
            "tag get",
            vec![format!("Term with id '{}' not found", id)],
        )),
    }
}
