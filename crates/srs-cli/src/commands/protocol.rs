use crate::commands::{CliContext, ProtocolCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::record::FieldValue;
use srs_repository::error::RepositoryError;
use srs_repository::protocol_service::{
    get_protocol_by_id, list_protocol_stages, list_protocols, validate_protocol_definition,
    GetProtocolResult,
};
use srs_repository::record_store::create_record;
use srs_repository::FileStore;
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: ProtocolCommand) -> Result<String> {
    match cmd {
        ProtocolCommand::List { json: _ } => cmd_protocol_list(ctx),
        ProtocolCommand::Get { id, json: _ } => cmd_protocol_get(ctx, id),
        ProtocolCommand::Stages { id, json: _ } => cmd_protocol_stages(ctx, id),
        ProtocolCommand::Validate { id, json: _ } => cmd_protocol_validate(ctx, id),
        ProtocolCommand::Export { id, json: _ } => cmd_protocol_export(ctx, id),
        ProtocolCommand::Import { json: _ } => cmd_protocol_import(ctx),
    }
}

fn cmd_protocol_list(ctx: CliContext) -> Result<String> {
    let protocols = list_protocols(&ctx.repo)?;

    let summaries: Vec<serde_json::Value> = protocols
        .into_iter()
        .map(|p| {
            json!({
                "instanceId": p.instance_id,
                "protocolId": p.protocol_id,
                "namespace": p.protocol_namespace,
                "name": p.protocol_name,
                "version": p.protocol_version,
                "stageCount": p.stage_count,
            })
        })
        .collect();

    Ok(output::ok(
        "protocol list",
        json!({ "protocols": summaries }),
    ))
}

fn cmd_protocol_get(ctx: CliContext, id: String) -> Result<String> {
    match get_protocol_by_id(&ctx.repo, &id)? {
        GetProtocolResult::Found {
            instance_id,
            protocol,
        } => {
            let mut protocol_json = serde_json::to_value(&protocol)?;
            if let Some(obj) = protocol_json.as_object_mut() {
                obj.insert(
                    "instanceId".to_string(),
                    serde_json::Value::String(instance_id),
                );
            }
            Ok(output::ok(
                "protocol get",
                json!({ "protocol": protocol_json }),
            ))
        }
        GetProtocolResult::NotFound => Ok(output::err(
            "protocol get",
            vec![format!("Protocol '{}' not found", id)],
        )),
    }
}

fn cmd_protocol_stages(ctx: CliContext, id: String) -> Result<String> {
    let stages = match list_protocol_stages(&ctx.repo, &id) {
        Ok(stages) => stages,
        Err(RepositoryError::NotFound { .. }) => {
            return Ok(output::err(
                "protocol stages",
                vec![format!("Protocol '{}' not found", id)],
            ));
        }
        Err(e) => return Err(e.into()),
    };

    let stages_json: Vec<serde_json::Value> = stages
        .into_iter()
        .map(|s| {
            json!({
                "stageId": s.stage_id,
                "name": s.name,
                "order": s.order,
                "dependsOn": s.depends_on,
            })
        })
        .collect();

    Ok(output::ok(
        "protocol stages",
        json!({ "stages": stages_json }),
    ))
}

fn cmd_protocol_validate(ctx: CliContext, id: String) -> Result<String> {
    let result = match validate_protocol_definition(&ctx.repo, &id) {
        Ok(result) => result,
        Err(RepositoryError::NotFound { .. }) => {
            return Ok(output::err(
                "protocol validate",
                vec![format!("Protocol '{}' not found", id)],
            ));
        }
        Err(e) => return Err(e.into()),
    };

    Ok(output::ok(
        "protocol validate",
        json!({
            "instanceId": result.instance_id,
            "valid": result.valid,
            "diagnostics": result.diagnostics,
        }),
    ))
}

fn cmd_protocol_export(ctx: CliContext, id: String) -> Result<String> {
    match get_protocol_by_id(&ctx.repo, &id)? {
        GetProtocolResult::Found { protocol, .. } => {
            // Export as a portable JSON definition (just the protocol struct)
            Ok(output::ok(
                "protocol export",
                json!({ "protocol": protocol }),
            ))
        }
        GetProtocolResult::NotFound => Ok(output::err(
            "protocol export",
            vec![format!("Protocol '{}' not found", id)],
        )),
    }
}

fn cmd_protocol_import(ctx: CliContext) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let json_value: serde_json::Value = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse protocol JSON: {}", e))?;

    // Extract protocol fields from the JSON
    // The import format should match the exported protocol structure
    let protocol_json = json_value.get("protocol").unwrap_or(&json_value);

    protocol_json
        .get("fieldValues")
        .or_else(|| protocol_json.get("protocolStages").map(|_| protocol_json))
        .ok_or_else(|| anyhow::anyhow!("Missing protocol fields in JSON"))?
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Protocol fields must be an object"))?;

    // Build field values from the protocol definition
    let mut field_values: Vec<FieldValue> = vec![];

    // Add standard protocol fields
    if let Some(v) = protocol_json
        .get("protocolId")
        .or_else(|| protocol_json.get("protocol-id"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-id".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolNamespace")
        .or_else(|| protocol_json.get("protocol-namespace"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-namespace".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolName")
        .or_else(|| protocol_json.get("protocol-name"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-name".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolVersion")
        .or_else(|| protocol_json.get("protocol-version"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-version".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolTargetType")
        .or_else(|| protocol_json.get("protocol-target-type"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-target-type".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolStages")
        .or_else(|| protocol_json.get("protocol-stages"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-stages".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolCreatedAt")
        .or_else(|| protocol_json.get("protocol-created-at"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-created-at".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }

    // Create the protocol record
    let store = FileStore::new(&ctx.repo);
    let record = create_record(&store, "meta.protocol", 1, field_values, "package/records")?;

    Ok(output::ok("protocol import", json!({ "protocol": record })))
}
