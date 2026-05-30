use crate::commands::{with_store, CliContext, TypeCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::record_type::RecordType;
use srs_repository::package_service::{
    create_type, get_type_by_id_latest, list_types, list_types_by_namespace, GetTypeResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: TypeCommand) -> Result<String> {
    match cmd {
        TypeCommand::List { namespace, json: _ } => cmd_type_list(ctx, namespace),
        TypeCommand::Get { id, json: _ } => cmd_type_get(ctx, id),
        TypeCommand::Create { json: _ } => cmd_type_create(ctx),
    }
}

fn cmd_type_list(ctx: CliContext, namespace: Option<String>) -> Result<String> {
    let summaries = with_store(&ctx, |store| {
        Ok(if let Some(ns) = namespace {
            list_types_by_namespace(store, &ns)?
        } else {
            list_types(store)?
        })
    })?;

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
    match with_store(&ctx, |store| Ok(get_type_by_id_latest(store, &id)?))? {
        GetTypeResult::Found(record_type) => {
            Ok(output::ok("type get", json!({ "type": record_type })))
        }
        GetTypeResult::NotFound => Ok(output::err(
            "type get",
            vec![format!("Type with id '{}' not found", id)],
        )),
    }
}

fn cmd_type_create(ctx: CliContext) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let record_type: RecordType = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse type JSON: {}", e))?;

    let result = with_store(&ctx, |store| Ok(create_type(store, record_type)?))?;

    Ok(output::ok(
        "type create",
        json!({ "type": result.record_type }),
    ))
}
