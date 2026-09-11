use crate::commands::{parse_type_filter, with_store, CliContext, FindArgs};
use crate::output;
use crate::payload::FindPayload;
use anyhow::Result;
use srs_repository::discovery_service::{self, DiscoveryQuery};

pub fn dispatch(ctx: CliContext, args: FindArgs) -> Result<String> {
    let (type_namespace, type_name) = match args.type_filter {
        None => (args.type_namespace, args.type_name),
        Some(ref filter) => match parse_type_filter(filter) {
            Some((namespace, name)) => (Some(namespace), Some(name)),
            None => {
                return Ok(output::err(
                    "find",
                    vec![format!(
                        "Invalid type filter '{}'. Expected format: namespace/name",
                        filter
                    )],
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
    match with_store(&ctx, |store| Ok(discovery_service::find(store, query)?)) {
        Ok(result) => output::serialize("find", FindPayload { result }),
        Err(e) => Ok(output::err("find", vec![e.to_string()])),
    }
}
