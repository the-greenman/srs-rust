use crate::commands::{with_store, CliContext, RelationCommand};
use crate::output;
use crate::payload::{
    PrecedesChainSplicePayload, RelationDeletePayload, RelationListPayload, RelationPayload,
};
use anyhow::Result;
use srs_repository::relation_service::{
    create_relation_auto, delete_relation, get_relation_by_id, insert_into_precedes_chain,
    list_relations, move_in_precedes_chain, parse_relation_input, remove_from_precedes_chain,
    GetRelationResult, InsertIntoPrecedesChainInput, ListRelationsFilter, MoveInPrecedesChainInput,
    RemoveFromPrecedesChainInput,
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
        RelationCommand::ChainInsert => cmd_relation_chain_insert(ctx),
        RelationCommand::ChainRemove => cmd_relation_chain_remove(ctx),
        RelationCommand::ChainMove => cmd_relation_chain_move(ctx),
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

fn cmd_relation_chain_insert(ctx: CliContext) -> Result<String> {
    let input: InsertIntoPrecedesChainInput = serde_json::from_reader(std::io::stdin())?;
    match with_store(&ctx, |store| Ok(insert_into_precedes_chain(store, input)?)) {
        Ok(result) => output::serialize(
            "relation chain-insert",
            PrecedesChainSplicePayload {
                created: result.created,
                removed: result.removed,
            },
        ),
        Err(e) => Ok(output::err("relation chain-insert", vec![e.to_string()])),
    }
}

fn cmd_relation_chain_remove(ctx: CliContext) -> Result<String> {
    let input: RemoveFromPrecedesChainInput = serde_json::from_reader(std::io::stdin())?;
    match with_store(&ctx, |store| Ok(remove_from_precedes_chain(store, input)?)) {
        Ok(result) => output::serialize(
            "relation chain-remove",
            PrecedesChainSplicePayload {
                created: result.created,
                removed: result.removed,
            },
        ),
        Err(e) => Ok(output::err("relation chain-remove", vec![e.to_string()])),
    }
}

fn cmd_relation_chain_move(ctx: CliContext) -> Result<String> {
    let input: MoveInPrecedesChainInput = serde_json::from_reader(std::io::stdin())?;
    match with_store(&ctx, |store| Ok(move_in_precedes_chain(store, input)?)) {
        Ok(result) => output::serialize(
            "relation chain-move",
            PrecedesChainSplicePayload {
                created: result.created,
                removed: result.removed,
            },
        ),
        Err(e) => Ok(output::err("relation chain-move", vec![e.to_string()])),
    }
}
