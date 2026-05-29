use crate::commands::{CliContext, RelationCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::relation_service::{
    get_relation_by_id, list_relations, GetRelationResult, ListRelationsFilter,
};

pub fn dispatch(ctx: CliContext, cmd: RelationCommand) -> Result<String> {
    match cmd {
        RelationCommand::List { json: _ } => cmd_relation_list(ctx),
        RelationCommand::Get { id, json: _ } => cmd_relation_get(ctx, id),
    }
}

fn cmd_relation_list(ctx: CliContext) -> Result<String> {
    let filter = ListRelationsFilter::default();
    let summaries = list_relations(&ctx.repo, filter)?;

    let relations: Vec<serde_json::Value> = summaries
        .into_iter()
        .map(|s| {
            json!({
                "relationId": s.relation_id,
                "relationType": s.relation_type,
                "sourceId": s.source_id,
                "targetId": s.target_id,
            })
        })
        .collect();

    Ok(output::ok("relation list", json!({ "relations": relations })))
}

fn cmd_relation_get(ctx: CliContext, id: String) -> Result<String> {
    match get_relation_by_id(&ctx.repo, &id)? {
        GetRelationResult::Found(relation) => {
            Ok(output::ok("relation get", json!({ "relation": relation })))
        }
        GetRelationResult::NotFound => Ok(output::err(
            "relation get",
            vec![format!("Relation with id '{}' not found", id)],
        )),
    }
}
