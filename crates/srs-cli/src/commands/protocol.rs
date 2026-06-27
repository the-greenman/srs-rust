use crate::commands::{with_store, CliContext, ProtocolCommand};
use crate::output;
use crate::payload::{
    ProtocolDeletePayload, ProtocolListEntry, ProtocolListPayload, ProtocolPayload,
    ProtocolStagesPayload, ProtocolValidatePayload,
};
use anyhow::Result;
use srs_repository::error::RepositoryError;
use srs_repository::protocol_service::{
    delete_protocol, export_protocol, get_protocol_by_id, import_protocol, list_protocol_stages,
    list_protocols, update_protocol, validate_protocol_definition, GetProtocolResult,
    ImportProtocolInput,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: ProtocolCommand) -> Result<String> {
    match cmd {
        ProtocolCommand::List { json: _ } => cmd_protocol_list(ctx),
        ProtocolCommand::Get { id, json: _ } => cmd_protocol_get(ctx, id),
        ProtocolCommand::Stages { id, json: _ } => cmd_protocol_stages(ctx, id),
        ProtocolCommand::Validate { id, json: _ } => cmd_protocol_validate(ctx, id),
        ProtocolCommand::Export { id, json: _ } => cmd_protocol_export(ctx, id),
        ProtocolCommand::Create { package, json: _ } => cmd_protocol_create(ctx, package),
        ProtocolCommand::Import { package, json: _ } => cmd_protocol_import(ctx, package),
        ProtocolCommand::Update { id } => cmd_protocol_update(ctx, id),
        ProtocolCommand::Delete { id } => cmd_protocol_delete(ctx, id),
    }
}

/// Read a JSON object from stdin, accepting either a bare object or `{ "protocol": { ... } }`.
fn read_protocol_value_from_stdin() -> Result<serde_json::Value> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    let raw: serde_json::Value = serde_json::from_str(&buf)
        .map_err(|e| anyhow::anyhow!("Failed to parse protocol JSON: {}", e))?;
    Ok(raw.get("protocol").cloned().unwrap_or(raw))
}

fn cmd_protocol_list(ctx: CliContext) -> Result<String> {
    let protocols = with_store(&ctx, |store| Ok(list_protocols(store)?))?;

    let protocols = protocols
        .into_iter()
        .map(|p| ProtocolListEntry {
            protocol_id: p.protocol_id,
            namespace: p.protocol_namespace,
            name: p.protocol_name,
            version: p.protocol_version,
            stage_count: p.stage_count,
            source_package: p.source_package,
        })
        .collect();

    output::serialize("protocol list", ProtocolListPayload { protocols })
}

fn cmd_protocol_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_protocol_by_id(store, &id)?))? {
        GetProtocolResult::Found(protocol) => {
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
            protocol_id: result.protocol_id,
            valid: result.valid,
            diagnostics: result.diagnostics,
        },
    )
}

fn cmd_protocol_export(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(export_protocol(store, &id)?))? {
        GetProtocolResult::Found(protocol) => {
            output::serialize("protocol export", ProtocolPayload { protocol })
        }
        GetProtocolResult::NotFound => Ok(output::err(
            "protocol export",
            vec![format!("Protocol '{}' not found", id)],
        )),
    }
}

fn cmd_protocol_write(
    ctx: CliContext,
    package: Option<String>,
    label: &'static str,
) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let raw: serde_json::Value = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse protocol JSON: {}", e))?;
    let result = with_store(&ctx, |store| {
        Ok(import_protocol(
            store,
            ImportProtocolInput { raw },
            package,
        )?)
    })?;
    output::serialize(
        label,
        ProtocolPayload {
            protocol: result.protocol,
        },
    )
}

fn cmd_protocol_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    cmd_protocol_write(ctx, package, "protocol create")
}

fn cmd_protocol_import(ctx: CliContext, package: Option<String>) -> Result<String> {
    cmd_protocol_write(ctx, package, "protocol import")
}

fn cmd_protocol_update(ctx: CliContext, id: String) -> Result<String> {
    let value = read_protocol_value_from_stdin()?;

    let result = match with_store(&ctx, |store| {
        Ok(update_protocol(store, &id, value.clone())?)
    }) {
        Ok(r) => r,
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol update",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            return Err(e);
        }
    };

    output::serialize(
        "protocol update",
        ProtocolPayload {
            protocol: result.protocol,
        },
    )
}

fn cmd_protocol_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_protocol(store, &id)?)) {
        Ok(result) => output::serialize(
            "protocol delete",
            ProtocolDeletePayload {
                protocol_id: result.protocol_id,
            },
        ),
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol delete",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            Err(e)
        }
    }
}
