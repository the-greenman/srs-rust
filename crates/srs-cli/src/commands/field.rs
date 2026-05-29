use crate::commands::{CliContext, FieldCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::field::Field;
use srs_repository::package_service::{
    create_field, get_field_by_id, list_fields, list_fields_by_namespace, GetFieldResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: FieldCommand) -> Result<String> {
    match cmd {
        FieldCommand::List { namespace, json: _ } => cmd_field_list(ctx, namespace),
        FieldCommand::Get { id, json: _ } => cmd_field_get(ctx, id),
        FieldCommand::Create { json: _ } => cmd_field_create(ctx),
    }
}

fn cmd_field_list(ctx: CliContext, namespace: Option<String>) -> Result<String> {
    let summaries = if let Some(ns) = namespace {
        list_fields_by_namespace(&ctx.repo, &ns)?
    } else {
        list_fields(&ctx.repo)?
    };

    let fields: Vec<serde_json::Value> = summaries
        .into_iter()
        .map(|s| {
            json!({
                "id": s.id,
                "namespace": s.namespace,
                "name": s.name,
                "version": s.version,
            })
        })
        .collect();

    Ok(output::ok("field list", json!({ "fields": fields })))
}

fn cmd_field_get(ctx: CliContext, id: String) -> Result<String> {
    match get_field_by_id(&ctx.repo, &id)? {
        GetFieldResult::Found(field) => Ok(output::ok("field get", json!({ "field": field }))),
        GetFieldResult::NotFound => Ok(output::err(
            "field get",
            vec![format!("Field with id '{}' not found", id)],
        )),
    }
}

fn cmd_field_create(ctx: CliContext) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let raw_value: serde_json::Value = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse field JSON: {}", e))?;
    let normalized = normalize_field_input(raw_value);

    let field: Field = serde_json::from_value(normalized)
        .map_err(|e| anyhow::anyhow!("Failed to parse field JSON: {}", e))?;

    let result = create_field(&ctx.repo, field)?;

    Ok(output::ok("field create", json!({ "field": result.field })))
}

fn normalize_field_input(value: serde_json::Value) -> serde_json::Value {
    let mut obj = match value {
        serde_json::Value::Object(map) => map,
        other => return other,
    };

    // Permit a minimal payload for field create; repository service will
    // backfill an empty createdAt with the current timestamp.
    obj.entry("description".to_string())
        .or_insert_with(|| serde_json::Value::String(String::new()));
    obj.entry("aiGuidance".to_string())
        .or_insert_with(|| json!({}));
    obj.entry("createdAt".to_string())
        .or_insert_with(|| serde_json::Value::String(String::new()));

    serde_json::Value::Object(obj)
}
