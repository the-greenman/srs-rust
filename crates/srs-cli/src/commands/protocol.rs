use crate::commands::{with_store, CliContext, ProtocolCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::error::RepositoryError;
use srs_repository::protocol_service::{
    get_protocol_by_id, import_protocol, list_protocol_stages, list_protocols,
    validate_protocol_definition, GetProtocolResult, ImportProtocolInput,
};
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
    let protocols = with_store(&ctx, |store| Ok(list_protocols(store)?))?;

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
    match with_store(&ctx, |store| Ok(get_protocol_by_id(store, &id)?))? {
        GetProtocolResult::Found { protocol, .. } => {
            Ok(output::ok("protocol get", json!({ "protocol": protocol })))
        }
        GetProtocolResult::NotFound => Ok(output::err(
            "protocol get",
            vec![format!("Protocol '{}' not found", id)],
        )),
    }
}

fn cmd_protocol_stages(ctx: CliContext, id: String) -> Result<String> {
    let stages = match with_store(&ctx, |store| Ok(list_protocol_stages(store, &id)?)) {
        Ok(stages) => stages,
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol stages",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            return Err(e);
        }
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
    let result = match with_store(&ctx, |store| Ok(validate_protocol_definition(store, &id)?)) {
        Ok(result) => result,
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol validate",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            return Err(e);
        }
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
    match with_store(&ctx, |store| Ok(get_protocol_by_id(store, &id)?))? {
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
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let raw: serde_json::Value = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse protocol JSON: {}", e))?;

    let result = with_store(&ctx, |store| {
        Ok(import_protocol(store, ImportProtocolInput { raw })?)
    })?;

    Ok(output::ok(
        "protocol import",
        json!({ "protocol": result.record }),
    ))
}
