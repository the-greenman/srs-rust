use crate::commands::{CliContext, TypeCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::package_service::{
    get_type_by_id_latest, list_types, list_types_by_namespace, GetTypeResult,
};
use srs_repository::FileStore;

pub fn dispatch(ctx: CliContext, cmd: TypeCommand) -> Result<String> {
    match cmd {
        TypeCommand::List { namespace, json: _ } => cmd_type_list(ctx, namespace),
        TypeCommand::Get { id, json: _ } => cmd_type_get(ctx, id),
    }
}

fn cmd_type_list(ctx: CliContext, namespace: Option<String>) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let summaries = if let Some(ns) = namespace {
        list_types_by_namespace(&store, &ns)?
    } else {
        list_types(&store)?
    };

    let types: Vec<serde_json::Value> = summaries
        .into_iter()
        .map(|s| {
            json!({
                "id": s.id,
                "namespace": s.namespace,
                "name": s.name,
                "version": s.version,
                "fieldCount": s.field_count,
            })
        })
        .collect();

    Ok(output::ok("type list", json!({ "types": types })))
}

fn cmd_type_get(ctx: CliContext, id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match get_type_by_id_latest(&store, &id)? {
        GetTypeResult::Found(record_type) => {
            Ok(output::ok("type get", json!({ "type": record_type })))
        }
        GetTypeResult::NotFound => Ok(output::err(
            "type get",
            vec![format!("Type with id '{}' not found", id)],
        )),
    }
}
