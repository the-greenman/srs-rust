use crate::commands::{with_store, CliContext, RelationTypeCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::package_service::list_relation_types_filtered;

pub fn dispatch(ctx: CliContext, cmd: RelationTypeCommand) -> Result<String> {
    match cmd {
        RelationTypeCommand::List { status, json: _ } => cmd_relation_type_list(ctx, status),
        RelationTypeCommand::Get { id, json: _ } => cmd_relation_type_get(ctx, id),
    }
}

fn cmd_relation_type_list(ctx: CliContext, status_filter: Option<String>) -> Result<String> {
    let defs = with_store(&ctx, |store| {
        Ok(list_relation_types_filtered(store, status_filter)?)
    })?;
    let relation_type_definitions: Vec<_> = defs
        .into_iter()
        .map(|rt| serde_json::to_value(rt).unwrap_or(json!(null)))
        .collect();
    Ok(output::ok(
        "relation-type list",
        json!({ "relationTypeDefinitions": relation_type_definitions }),
    ))
}

fn cmd_relation_type_get(ctx: CliContext, id: String) -> Result<String> {
    let package = with_store(&ctx, |store| Ok(store.load_package()?))?;

    match package.resolve_relation_type_by_id(&id) {
        Some(rt) => Ok(output::ok(
            "relation-type get",
            json!({ "relationTypeDefinition": rt }),
        )),
        None => Ok(output::err(
            "relation-type get",
            vec![format!("relation type definition not found: {}", id)],
        )),
    }
}
