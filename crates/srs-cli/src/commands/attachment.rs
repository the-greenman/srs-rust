use crate::commands::{with_store, AttachmentCommand, CliContext};
use crate::output;
use crate::payload::AttachmentListPayload;
use anyhow::Result;
use srs_repository::attachment_service;

pub fn dispatch(ctx: CliContext, cmd: AttachmentCommand) -> Result<String> {
    match cmd {
        AttachmentCommand::List => cmd_attachment_list(ctx),
    }
}

fn cmd_attachment_list(ctx: CliContext) -> Result<String> {
    let result = with_store(&ctx, |store| Ok(attachment_service::list_attachments(store)?))?;
    output::serialize("attachment list", AttachmentListPayload::from(result))
}
