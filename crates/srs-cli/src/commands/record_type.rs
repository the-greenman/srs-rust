use crate::commands::{with_store, CliContext, TypeCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::record_type::RecordType;
use srs_repository::package_service::{
    create_type_in_package, get_type_by_id_latest, list_types, list_types_by_namespace,
    list_types_by_package, GetTypeResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: TypeCommand) -> Result<String> {
    match cmd {
        TypeCommand::List {
            namespace,
            package,
            json: _,
        } => cmd_type_list(ctx, namespace, package),
        TypeCommand::Get { id, json: _ } => cmd_type_get(ctx, id),
        TypeCommand::Create { package, json: _ } => cmd_type_create(ctx, package),
    }
}

fn cmd_type_list(
    ctx: CliContext,
    namespace: Option<String>,
    package: Option<String>,
) -> Result<String> {
    let summaries = with_store(&ctx, |store| {
        Ok(match (&namespace, &package) {
            (Some(ns), None) => list_types_by_namespace(store, ns)?,
            (None, Some(pkg)) => list_types_by_package(store, Some(pkg.as_str()))?,
            (Some(ns), Some(pkg)) => list_types_by_package(store, Some(pkg.as_str()))?
                .into_iter()
                .filter(|t| t.namespace == *ns)
                .collect(),
            (None, None) => list_types(store)?,
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
                "sourcePackage": s.source_package,
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

fn cmd_type_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let record_type: RecordType = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse type JSON: {}", e))?;

    let result = with_store(&ctx, |store| {
        Ok(create_type_in_package(store, record_type, package.clone())?)
    })?;

    Ok(output::ok(
        "type create",
        json!({ "type": result.record_type }),
    ))
}
