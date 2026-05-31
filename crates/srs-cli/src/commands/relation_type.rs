use crate::commands::{with_store, CliContext, RelationTypeCommand};
use crate::output;
use anyhow::Result;
use serde::Serialize;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_repository::package_service::{
    create_relation_type, delete_relation_type, list_relation_types_filtered, update_relation_type,
};
use std::io;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationTypeListPayload {
    relation_type_definitions: Vec<RelationTypeDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationTypeGetPayload {
    relation_type_definition: RelationTypeDefinition,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationTypeCreatePayload {
    relation_type_definition: RelationTypeDefinition,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationTypeUpdatePayload {
    relation_type_definition: RelationTypeDefinition,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationTypeDeletePayload {
    id: String,
}

pub fn dispatch(ctx: CliContext, cmd: RelationTypeCommand) -> Result<String> {
    match cmd {
        RelationTypeCommand::List { status, json: _ } => cmd_relation_type_list(ctx, status),
        RelationTypeCommand::Get { id, json: _ } => cmd_relation_type_get(ctx, id),
        RelationTypeCommand::Create {} => cmd_relation_type_create(ctx),
        RelationTypeCommand::Update { id } => cmd_relation_type_update(ctx, id),
        RelationTypeCommand::Delete { id } => cmd_relation_type_delete(ctx, id),
    }
}

fn cmd_relation_type_list(ctx: CliContext, status_filter: Option<String>) -> Result<String> {
    let relation_type_definitions = with_store(&ctx, |store| {
        Ok(list_relation_types_filtered(store, status_filter)?)
    })?;
    output::serialize(
        "relation-type list",
        RelationTypeListPayload {
            relation_type_definitions,
        },
    )
}

fn cmd_relation_type_get(ctx: CliContext, id: String) -> Result<String> {
    let package = with_store(&ctx, |store| Ok(store.load_package()?))?;

    match package.resolve_relation_type_by_id(&id) {
        Some(relation_type_definition) => output::serialize(
            "relation-type get",
            RelationTypeGetPayload {
                relation_type_definition: relation_type_definition.clone(),
            },
        ),
        None => Ok(output::err(
            "relation-type get",
            vec![format!("relation type definition not found: {}", id)],
        )),
    }
}

fn cmd_relation_type_create(ctx: CliContext) -> Result<String> {
    let def: RelationTypeDefinition = serde_json::from_reader(io::stdin().lock())
        .map_err(|e| anyhow::anyhow!("Failed to parse relation type JSON: {}", e))?;
    let result = with_store(&ctx, |store| Ok(create_relation_type(store, def)?))?;
    output::serialize(
        "relation-type create",
        RelationTypeCreatePayload {
            relation_type_definition: result.relation_type_definition,
        },
    )
}

fn cmd_relation_type_update(ctx: CliContext, _id: String) -> Result<String> {
    let def: RelationTypeDefinition = serde_json::from_reader(io::stdin().lock())
        .map_err(|e| anyhow::anyhow!("Failed to parse relation type JSON: {}", e))?;
    let result = with_store(&ctx, |store| Ok(update_relation_type(store, def)?))?;
    output::serialize(
        "relation-type update",
        RelationTypeUpdatePayload {
            relation_type_definition: result.relation_type_definition,
        },
    )
}

fn cmd_relation_type_delete(ctx: CliContext, id: String) -> Result<String> {
    let result = with_store(&ctx, |store| Ok(delete_relation_type(store, &id)?))?;
    output::serialize(
        "relation-type delete",
        RelationTypeDeletePayload { id: result.id },
    )
}
