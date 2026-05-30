use crate::commands::{with_store, CliContext, RelationCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::relation::Relation;
use srs_repository::relation_service::{
    create_relation_auto, delete_relation, get_relation_by_id, list_relations, GetRelationResult,
    ListRelationsFilter,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: RelationCommand) -> Result<String> {
    match cmd {
        RelationCommand::List {
            source,
            target,
            relation_type,
            json: _,
        } => cmd_relation_list(ctx, source, target, relation_type),
        RelationCommand::Create { json: _ } => cmd_relation_create(ctx),
        RelationCommand::Get { id, json: _ } => cmd_relation_get(ctx, id),
        RelationCommand::Delete { id, json: _ } => cmd_relation_delete(ctx, id),
    }
}

fn cmd_relation_list(
    ctx: CliContext,
    source: Option<String>,
    target: Option<String>,
    relation_type: Option<String>,
) -> Result<String> {
    let filter = ListRelationsFilter {
        source,
        target,
        relation_type,
        container_id: ctx.container_id.clone(),
    };
    let summaries = with_store(&ctx, |store| Ok(list_relations(store, filter)?))?;

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

    Ok(output::ok(
        "relation list",
        json!({ "relations": relations }),
    ))
}

fn cmd_relation_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_relation_by_id(store, &id)?))? {
        GetRelationResult::Found(relation) => {
            Ok(output::ok("relation get", json!({ "relation": relation })))
        }
        GetRelationResult::NotFound => Ok(output::err(
            "relation get",
            vec![format!("Relation with id '{}' not found", id)],
        )),
    }
}

fn cmd_relation_create(ctx: CliContext) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let relation: Relation = match serde_json::from_str(&stdin) {
        Ok(relation) => relation,
        Err(e) => {
            return Ok(output::err(
                "relation create",
                vec![format!("Failed to parse relation JSON: {}", e)],
            ))
        }
    };

    match with_store(&ctx, |store| Ok(create_relation_auto(store, relation)?)) {
        Ok(result) => Ok(output::ok(
            "relation create",
            json!({ "relation": result.relation }),
        )),
        Err(e) => Ok(output::err("relation create", vec![e.to_string()])),
    }
}

fn cmd_relation_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_relation(store, &id)?)) {
        Ok(result) => Ok(output::ok(
            "relation delete",
            json!({
                "relationId": result.relation_id,
                "path": "relations/relations-collection.json"
            }),
        )),
        Err(e) => Ok(output::err("relation delete", vec![e.to_string()])),
    }
}
