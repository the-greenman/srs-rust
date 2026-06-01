use crate::commands::{with_store, CliContext, ProtocolCommand};
use crate::output;
use crate::payload::{
    ProtocolListEntry, ProtocolListPayload, ProtocolPayload, ProtocolStageEntry,
    ProtocolStagesPayload, ProtocolValidatePayload,
};
use anyhow::Result;
use srs_repository::error::RepositoryError;
use srs_repository::protocol_service::{
    export_protocol, get_protocol_by_id, import_protocol, list_protocol_stages, list_protocols,
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

    let protocols = protocols
        .into_iter()
        .map(|p| ProtocolListEntry {
            instance_id: p.instance_id,
            protocol_id: p.protocol_id,
            namespace: p.protocol_namespace,
            name: p.protocol_name,
            version: p.protocol_version,
            stage_count: p.stage_count,
        })
        .collect();

    output::serialize("protocol list", ProtocolListPayload { protocols })
}

fn cmd_protocol_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_protocol_by_id(store, &id)?))? {
        GetProtocolResult::Found {
            instance_id,
            mut protocol,
        } => {
            // Inject instanceId into the get response for consumer convenience
            if let Some(obj) = protocol.as_object_mut() {
                obj.insert(
                    "instanceId".to_string(),
                    serde_json::Value::String(instance_id),
                );
            }
            output::serialize("protocol get", ProtocolPayload { protocol })
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

    let stages = stages
        .into_iter()
        .map(|s| ProtocolStageEntry {
            stage_id: s.stage_id,
            name: s.name,
            order: s.order,
            depends_on: s.depends_on,
        })
        .collect();

    output::serialize("protocol stages", ProtocolStagesPayload { stages })
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

    output::serialize(
        "protocol validate",
        ProtocolValidatePayload {
            instance_id: result.instance_id,
            valid: result.valid,
            diagnostics: result.diagnostics,
        },
    )
}

fn cmd_protocol_export(ctx: CliContext, id: String) -> Result<String> {
    // export_protocol omits instanceId — output is the canonical import format
    match with_store(&ctx, |store| Ok(export_protocol(store, &id)?))? {
        GetProtocolResult::Found { protocol, .. } => {
            output::serialize("protocol export", ProtocolPayload { protocol })
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

    // Return the protocol struct (camelCase), not the raw record
    let protocol = with_store(&ctx, |store| {
        Ok(get_protocol_by_id(store, &result.instance_id)?)
    })?;

    match protocol {
        GetProtocolResult::Found {
            instance_id,
            mut protocol,
        } => {
            if let Some(obj) = protocol.as_object_mut() {
                obj.insert(
                    "instanceId".to_string(),
                    serde_json::Value::String(instance_id),
                );
            }
            output::serialize("protocol import", ProtocolPayload { protocol })
        }
        GetProtocolResult::NotFound => {
            // Should not happen — we just created it
            Err(anyhow::anyhow!(
                "Protocol was created but could not be retrieved (instance_id: {})",
                result.instance_id
            ))
        }
    }
}
