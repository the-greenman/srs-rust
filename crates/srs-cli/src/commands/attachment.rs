use crate::commands::{with_store, AttachmentCommand, CliContext};
use crate::output;
use crate::payload::{AttachmentAddPayload, AttachmentListPayload};
use anyhow::{Context as _, Result};
use srs_repository::attachment_service::{self, AddAttachmentInput, ListAttachmentsFilter};

pub fn dispatch(ctx: CliContext, cmd: AttachmentCommand) -> Result<String> {
    match cmd {
        AttachmentCommand::List => cmd_attachment_list(ctx),
        AttachmentCommand::Add {
            source,
            subdir,
            title,
            content_type,
        } => cmd_attachment_add(ctx, source, subdir, title, content_type),
    }
}

fn cmd_attachment_list(ctx: CliContext) -> Result<String> {
    let result = with_store(&ctx, |store| {
        Ok(attachment_service::list_attachments(
            store,
            ListAttachmentsFilter::default(),
        )?)
    })?;
    output::serialize("attachment list", AttachmentListPayload::from(result))
}

fn cmd_attachment_add(
    ctx: CliContext,
    source: std::path::PathBuf,
    subdir: Option<String>,
    title: Option<String>,
    content_type: Option<String>,
) -> Result<String> {
    let file_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("source path has no file name: {}", source.display()))?;
    let content = std::fs::read(&source)
        .with_context(|| format!("failed to read source file: {}", source.display()))?;
    let input = AddAttachmentInput {
        file_name,
        content,
        subdir,
        title,
        content_type,
    };
    let result =
        with_store(&ctx, |store| Ok(attachment_service::add_attachment(store, input)?))?;
    output::serialize("attachment add", AttachmentAddPayload::from(result))
}
