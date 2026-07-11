use crate::commands::{with_store, CliContext, RelationTypeCommand};
use crate::output;
use crate::payload::{RelationTypeDeletePayload, RelationTypeListPayload, RelationTypePayload};
use anyhow::Result;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_repository::package_service::{
    create_relation_type_normalized, delete_relation_type, list_relation_types_filtered,
    update_relation_type, RelationTypeListFilter,
};

pub fn dispatch(ctx: CliContext, cmd: RelationTypeCommand) -> Result<String> {
    match cmd {
        RelationTypeCommand::List { status, json: _ } => cmd_relation_type_list(ctx, status),
        RelationTypeCommand::Get { id, json: _ } => cmd_relation_type_get(ctx, id),
        RelationTypeCommand::Create { package } => cmd_relation_type_create(ctx, package),
        RelationTypeCommand::Update { id } => cmd_relation_type_update(ctx, id),
        RelationTypeCommand::Delete { id } => cmd_relation_type_delete(ctx, id),
    }
}

fn cmd_relation_type_list(ctx: CliContext, status_filter: Option<String>) -> Result<String> {
    let relation_type_definitions = with_store(&ctx, |store| {
        Ok(list_relation_types_filtered(
            store,
            RelationTypeListFilter {
                status: status_filter,
            },
        )?)
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
            RelationTypePayload {
                relation_type_definition: relation_type_definition.clone(),
            },
        ),
        None => Ok(output::err(
            "relation-type get",
            vec![format!("relation type definition not found: {}", id)],
        )),
    }
}

fn cmd_relation_type_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let raw = crate::input::value_from_stdin("relation type")?;
    let result = with_store(&ctx, |store| {
        Ok(create_relation_type_normalized(
            store,
            raw,
            package.clone(),
        )?)
    })?;
    output::serialize(
        "relation-type create",
        RelationTypePayload {
            relation_type_definition: result.relation_type_definition,
        },
    )
}

fn cmd_relation_type_update(ctx: CliContext, _id: String) -> Result<String> {
    let def: RelationTypeDefinition = crate::input::from_stdin("relation type")?;
    let result = with_store(&ctx, |store| Ok(update_relation_type(store, def)?))?;
    output::serialize(
        "relation-type update",
        RelationTypePayload {
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
