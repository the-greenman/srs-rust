use crate::commands::{with_store, CliContext, TermCommand};
use crate::output;
use crate::payload::{TermGetPayload, TermListPayload};
use anyhow::Result;
use srs_repository::vocabulary_service;

pub fn dispatch(ctx: CliContext, cmd: TermCommand) -> Result<String> {
    match cmd {
        TermCommand::List => cmd_term_list(ctx),
        TermCommand::Get { id } => cmd_term_get(ctx, id),
    }
}

fn cmd_term_list(ctx: CliContext) -> Result<String> {
    let terms = with_store(&ctx, |store| Ok(vocabulary_service::list_terms(store)?))?;
    output::serialize("term list", TermListPayload { terms })
}

fn cmd_term_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(vocabulary_service::get_term_by_id(store, &id)?)
    })? {
        Some(term) => output::serialize(
            "term get",
            TermGetPayload::Found {
                term: Box::new(term),
            },
        ),
        None => output::serialize("term get", TermGetPayload::NotFound { id }),
    }
}
