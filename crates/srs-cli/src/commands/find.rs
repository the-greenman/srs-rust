use crate::commands::{parse_type_filter, with_store, CliContext, FindArgs};
use crate::output;
use crate::payload::FindPayload;
use anyhow::Result;
use srs_repository::discovery_service::{self, DiscoveryQuery, FindPage};
use srs_repository::error::RepositoryError;

pub fn dispatch(ctx: CliContext, args: FindArgs) -> Result<String> {
    let (type_namespace, type_name) = match args.type_filter {
        None => (args.type_namespace, args.type_name),
        Some(ref filter) => match parse_type_filter(filter) {
            Some((namespace, name)) => (Some(namespace), Some(name)),
            None => {
                return Ok(output::repo_err(
                    "find",
                    &RepositoryError::InvalidInput {
                        message: format!(
                            "Invalid type filter '{}'. Expected format: namespace/name",
                            filter
                        ),
                    },
                ))
            }
        },
    };

    // Container scope comes from the global `--container` flag, like `tree`.
    let query = DiscoveryQuery {
        type_id: args.type_id,
        type_namespace,
        type_name,
        container_id: ctx.container_id.clone(),
        tag: args.tag,
        lifecycle_state: args.lifecycle_state,
        lifecycle_states: args.lifecycle_states,
        exclude_lifecycle_states: args.exclude_lifecycle_state,
        tier: args.tier,
        content_match: args.text,
    };
    let page = FindPage {
        limit: args.limit,
        offset: args.offset,
        rank: args.rank,
        match_mode: args.match_mode,
        by_type_limit: args.by_type_limit,
        facets: args.facets,
        projection: args.projection,
    };
    let similar = args.similar;
    match with_store(&ctx, |store| {
        Ok(match &similar {
            Some(id) => discovery_service::similar(store, id, query, page)?,
            None => discovery_service::find(store, query, page)?,
        })
    }) {
        Ok(result) => output::serialize(
            "find",
            FindPayload {
                result: result.into(),
            },
        ),
        Err(e) => Ok(output::any_err("find", &e)),
    }
}
