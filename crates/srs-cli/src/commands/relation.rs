use crate::commands::{with_store, CliContext, RelationCommand};
use crate::output;
use crate::payload::{RelationDeletePayload, RelationListPayload, RelationPayload};
use anyhow::Result;
use srs_repository::relation_service::{
    create_relation_auto, delete_relation, get_relation_by_id, list_relations,
    parse_relation_input, GetRelationResult, ListRelationsFilter,
};

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
    let relations = with_store(&ctx, |store| Ok(list_relations(store, filter)?))?;
    output::serialize("relation list", RelationListPayload { relations })
}

fn cmd_relation_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_relation_by_id(store, &id)?))? {
        GetRelationResult::Found(relation) => output::serialize(
            "relation get",
            RelationPayload {
                relation: *relation,
            },
        ),
        GetRelationResult::NotFound => Ok(output::err(
            "relation get",
            vec![format!("Relation with id '{}' not found", id)],
        )),
    }
}

fn cmd_relation_create(ctx: CliContext) -> Result<String> {
    let raw = match crate::input::value_from_stdin("relation") {
        Ok(raw) => raw,
        Err(e) => return Ok(output::err("relation create", vec![e.to_string()])),
    };
    let relation = match parse_relation_input(raw) {
        Ok(relation) => relation,
        Err(e) => return Ok(output::err("relation create", vec![e.to_string()])),
    };

    match with_store(&ctx, |store| Ok(create_relation_auto(store, relation)?)) {
        Ok(result) => output::serialize(
            "relation create",
            RelationPayload {
                relation: result.relation,
            },
        ),
        Err(e) => Ok(output::err("relation create", vec![e.to_string()])),
    }
}

fn cmd_relation_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_relation(store, &id)?)) {
        Ok(result) => output::serialize(
            "relation delete",
            RelationDeletePayload {
                relation_id: result.relation_id,
                path: result.path,
            },
        ),
        Err(e) => Ok(output::err("relation delete", vec![e.to_string()])),
    }
}
