use crate::commands::{CliContext, ExtensionCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::record::FieldValue;
use srs_repository::extension_service::{get_extension_by_id, list_extensions};
use srs_repository::record_store::{create_record, delete_record, update_record};
use srs_repository::FileStore;
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: ExtensionCommand) -> Result<String> {
    match cmd {
        ExtensionCommand::List { json: _ } => cmd_extension_list(ctx),
        ExtensionCommand::Get { id, json: _ } => cmd_extension_get(ctx, id),
        ExtensionCommand::Create { json: _ } => cmd_extension_create(ctx),
        ExtensionCommand::Update { id, json: _ } => cmd_extension_update(ctx, id),
        ExtensionCommand::Delete { id, json: _ } => cmd_extension_delete(ctx, id),
    }
}

fn cmd_extension_list(ctx: CliContext) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let records = list_extensions(&store)?;

    let extensions: Vec<serde_json::Value> = records
        .into_iter()
        .map(|r| {
            json!({
                "instanceId": r.instance_id,
                "namespace": r.extra.get("namespace").cloned().unwrap_or(serde_json::Value::Null),
                "name": r.extra.get("name").cloned().unwrap_or(serde_json::Value::Null),
                "version": r.extra.get("version").cloned().unwrap_or(serde_json::Value::Null),
                "type": format!("{}/{}", r.type_namespace, r.type_name),
            })
        })
        .collect();

    Ok(output::ok(
        "extension list",
        json!({ "extensions": extensions }),
    ))
}

fn cmd_extension_get(ctx: CliContext, id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match get_extension_by_id(&store, &id)? {
        Some(record) => Ok(output::ok("extension get", json!({ "extension": record }))),
        None => Ok(output::err(
            "extension get",
            vec![format!("Extension '{}' not found", id)],
        )),
    }
}

fn cmd_extension_create(ctx: CliContext) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let json_value: serde_json::Value = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse extension JSON: {}", e))?;

    // Extract field values from the JSON
    let field_values_json = json_value
        .get("fieldValues")
        .or_else(|| json_value.get("field_values"))
        .ok_or_else(|| anyhow::anyhow!("Missing fieldValues in extension JSON"))?
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("fieldValues must be an object"))?;

    let field_values: Vec<FieldValue> = field_values_json
        .iter()
        .map(|(k, v)| FieldValue {
            field_id: k.clone(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        })
        .collect();

    // Get type info from JSON or use defaults
    let type_id = json_value
        .get("typeId")
        .or_else(|| json_value.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("meta.extension");

    let type_version = json_value
        .get("typeVersion")
        .or_else(|| json_value.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    // Create the record
    let store = FileStore::new(&ctx.repo);
    let record = create_record(
        &store,
        type_id,
        type_version,
        field_values,
        "package/records",
    )?;

    Ok(output::ok(
        "extension create",
        json!({ "extension": record }),
    ))
}

fn cmd_extension_update(ctx: CliContext, id: String) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let field_values_json: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&stdin)
            .map_err(|e| anyhow::anyhow!("Failed to parse extension JSON: {}", e))?;

    let field_values: Vec<FieldValue> = field_values_json
        .iter()
        .map(|(k, v)| FieldValue {
            field_id: k.clone(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        })
        .collect();

    // Update the record
    let store = FileStore::new(&ctx.repo);
    let record = update_record(&store, &id, field_values)?;

    Ok(output::ok(
        "extension update",
        json!({ "extension": record }),
    ))
}

fn cmd_extension_delete(ctx: CliContext, id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match delete_record(&store, &id) {
        Ok(instance_id) => Ok(output::ok(
            "extension delete",
            json!({ "instanceId": instance_id }),
        )),
        Err(e) => Ok(output::err("extension delete", vec![e.to_string()])),
    }
}
