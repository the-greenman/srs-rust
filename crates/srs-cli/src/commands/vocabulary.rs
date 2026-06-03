use crate::commands::{with_store, CliContext, VocabularyCommand};
use crate::output;
use crate::payload::{VocabularyGetPayload, VocabularyListPayload};
use anyhow::Result;
use srs_repository::vocabulary_service;

pub fn dispatch(ctx: CliContext, cmd: VocabularyCommand) -> Result<String> {
    match cmd {
        VocabularyCommand::List { json: _ } => cmd_vocabulary_list(ctx),
        VocabularyCommand::Get { id, json: _ } => cmd_vocabulary_get(ctx, id),
    }
}

fn cmd_vocabulary_list(ctx: CliContext) -> Result<String> {
    let vocabularies = with_store(&ctx, |store| {
        Ok(vocabulary_service::list_vocabularies(store)?)
    })?;
    output::serialize("vocabulary list", VocabularyListPayload { vocabularies })
}

fn cmd_vocabulary_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(vocabulary_service::get_vocabulary_by_id(store, &id)?)
    })? {
        Some(vocabulary) => output::serialize(
            "vocabulary get",
            VocabularyGetPayload::Found {
                vocabulary: Box::new(vocabulary),
            },
        ),
        None => output::serialize("vocabulary get", VocabularyGetPayload::NotFound { id }),
    }
}
