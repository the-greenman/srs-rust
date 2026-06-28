use crate::commands::{with_store, CliContext, FindArgs};
use crate::output;
use crate::payload::FindPayload;
use anyhow::Result;
use srs_repository::discovery_service::{self, DiscoveryQuery};

pub fn dispatch(ctx: CliContext, args: FindArgs) -> Result<String> {
    // Container scope comes from the global `--container` flag, like `tree`.
    let query = DiscoveryQuery {
        type_id: args.type_id,
        type_namespace: args.type_namespace,
        type_name: args.type_name,
        container_id: ctx.container_id.clone(),
        tag: args.tag,
        lifecycle_state: args.lifecycle_state,
        tier: args.tier,
        content_match: args.text,
    };
    match with_store(&ctx, |store| Ok(discovery_service::find(store, query)?)) {
        Ok(result) => output::serialize("find", FindPayload { result }),
        Err(e) => Ok(output::err("find", vec![e.to_string()])),
    }
}
