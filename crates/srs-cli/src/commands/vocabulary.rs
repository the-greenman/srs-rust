use crate::commands::{with_store, CliContext, VocabularyCommand};
use crate::output;
use crate::payload::{
    PromoteVocabularyBlockedPayload, PromoteVocabularyPayload, TermCreatePayload,
    VocabularyCreatePayload, VocabularyGetPayload, VocabularyListPayload,
};
use anyhow::Result;
use srs_core::types::{term::Term, vocabulary::Vocabulary};
use srs_repository::error::RepositoryError;
use srs_repository::vocabulary_service;
use std::io;

pub fn dispatch(ctx: CliContext, cmd: VocabularyCommand) -> Result<String> {
    match cmd {
        VocabularyCommand::List { json: _ } => cmd_vocabulary_list(ctx),
        VocabularyCommand::Get { id, json: _ } => cmd_vocabulary_get(ctx, id),
        VocabularyCommand::Create => cmd_vocabulary_create(ctx),
        VocabularyCommand::TermCreate { vocabulary_id } => cmd_term_create(ctx, vocabulary_id),
        VocabularyCommand::Promote { id } => cmd_vocabulary_promote(ctx, id),
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

fn cmd_vocabulary_create(ctx: CliContext) -> Result<String> {
    let vocabulary: Vocabulary = serde_json::from_reader(io::stdin())?;
    let result = with_store(&ctx, |store| {
        Ok(vocabulary_service::create_vocabulary(store, vocabulary)?)
    })?;
    output::serialize(
        "vocabulary create",
        VocabularyCreatePayload {
            vocabulary: result.vocabulary,
        },
    )
}

fn cmd_term_create(ctx: CliContext, vocabulary_id: String) -> Result<String> {
    let term: Term = serde_json::from_reader(io::stdin())?;
    let result = with_store(&ctx, |store| {
        Ok(vocabulary_service::create_term(
            store,
            &vocabulary_id,
            term,
        )?)
    })?;
    output::serialize(
        "vocabulary term-create",
        TermCreatePayload {
            term: result.term,
            vocabulary: result.vocabulary,
        },
    )
}

fn cmd_vocabulary_promote(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(vocabulary_service::promote_vocabulary(
            store,
            vocabulary_service::PromoteVocabularyInput {
                vocabulary_id: id.clone(),
            },
        )?)
    }) {
        Ok(r) => output::serialize(
            "vocabulary promote",
            PromoteVocabularyPayload {
                vocabulary: r.vocabulary,
            },
        ),
        Err(e) => {
            if let Some(RepositoryError::VocabularyPromotionBlocked {
                vocabulary_id,
                unresolvable_keys,
            }) = e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err_with_payload(
                    "vocabulary promote",
                    vec![e.to_string()],
                    PromoteVocabularyBlockedPayload {
                        vocabulary_id: vocabulary_id.clone(),
                        unresolvable_keys: unresolvable_keys.clone(),
                    },
                ));
            }
            Err(e)
        }
    }
}
