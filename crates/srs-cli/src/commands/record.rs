use crate::commands::{CliContext, RecordCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::record::FieldValue;
use srs_repository::container_service::{
    add_member, get_container, is_member, list_members, remove_member,
};
use srs_repository::error::RepositoryError;
use srs_repository::package_service::{get_type_by_name, GetTypeResult};
use srs_repository::record_store::{
    create_record, delete_record, get_record_by_id, list_all_records, list_records_by_type,
    update_record,
};
use srs_repository::{FileStore, RepositoryStore};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: RecordCommand) -> Result<String> {
    match cmd {
        RecordCommand::List {
            type_filter,
            json: _,
        } => cmd_record_list(ctx, type_filter),
        RecordCommand::Get { id, json: _ } => cmd_record_get(ctx, id),
        RecordCommand::Create {
            type_filter,
            version,
            dir,
            json: _,
        } => cmd_record_create(ctx, type_filter, version, dir),
        RecordCommand::Update { id, json: _ } => cmd_record_update(ctx, id),
        RecordCommand::Delete { id, json: _ } => cmd_record_delete(ctx, id),
    }
}

fn cmd_record_list(ctx: CliContext, type_filter: Option<String>) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let mut records = match type_filter {
        None => list_all_records(&store)?,
        Some(ref filter) => match parse_type_filter(filter) {
            Some((namespace, name)) => list_records_by_type(&store, &namespace, &name)?,
            None => {
                return Ok(output::err(
                    "record list",
                    vec![format!(
                        "Invalid type filter '{}'. Expected format: namespace/name",
                        filter
                    )],
                ))
            }
        },
    };

    if let Some(ref cid) = ctx.container_id {
        let members = list_members(&store, cid)?;
        records.retain(|r| members.iter().any(|id| id == &r.instance_id));
    }

    Ok(output::ok("record list", json!({ "records": records })))
}

fn cmd_record_get(ctx: CliContext, id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match get_record_by_id(&store, &id)? {
        Some(record) => Ok(output::ok("record get", json!({ "record": record }))),
        None => Ok(output::err(
            "record get",
            vec![format!("Record with id '{}' not found", id)],
        )),
    }
}

fn cmd_record_create(
    ctx: CliContext,
    type_filter: String,
    version: Option<u32>,
    dir: String,
) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    if let Some(ref cid) = ctx.container_id {
        match get_container(&store, cid) {
            Ok(_) => {}
            Err(RepositoryError::ContainerNotFound { .. }) => {
                return Ok(output::err(
                    "record create",
                    vec![format!("Container '{}' not found — no record written", cid)],
                ))
            }
            Err(e) => return Err(e.into()),
        }
    }

    let (namespace, name) = match parse_type_filter(&type_filter) {
        Some(parts) => parts,
        None => {
            return Ok(output::err(
                "record create",
                vec![format!(
                    "Invalid type filter '{}'. Expected format: namespace/name",
                    type_filter
                )],
            ))
        }
    };

    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let field_values = match parse_field_values_payload(&stdin) {
        Ok(v) => v,
        Err(msg) => return Ok(output::err("record create", vec![msg])),
    };

    let record_type = match resolve_type(&ctx, &namespace, &name, version)? {
        Some(t) => t,
        None => {
            return Ok(output::err(
                "record create",
                vec![if let Some(v) = version {
                    format!("Type '{}'/{}@{} not found", namespace, name, v)
                } else {
                    format!("Type '{}/{}' not found", namespace, name)
                }],
            ))
        }
    };

    match create_record(
        &store,
        &record_type.id,
        record_type.version,
        field_values,
        &dir,
    ) {
        Ok(record) => {
            if let Some(ref cid) = ctx.container_id {
                if let Err(e) = add_member(&store, cid, &record.instance_id) {
                    return Ok(output::err(
                        "record create",
                        vec![format!(
                            "Record created but failed to add to container: {}",
                            e
                        )],
                    ));
                }
            }
            Ok(output::ok("record create", json!({ "record": record })))
        }
        Err(e) => Ok(output::err("record create", vec![e.to_string()])),
    }
}

fn cmd_record_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let field_values = match parse_field_values_payload(&stdin) {
        Ok(v) => v,
        Err(msg) => return Ok(output::err("record update", vec![msg])),
    };

    let store = FileStore::new(&ctx.repo);
    match update_record(&store, &id, field_values) {
        Ok(record) => Ok(output::ok("record update", json!({ "record": record }))),
        Err(e) => Ok(output::err("record update", vec![e.to_string()])),
    }
}

fn cmd_record_delete(ctx: CliContext, id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    if let Some(ref cid) = ctx.container_id {
        if !is_member(&store, cid, &id)? {
            return Ok(output::err(
                "record delete",
                vec![format!(
                    "Instance '{}' is not a member of container '{}' — delete refused",
                    id, cid
                )],
            ));
        }
        remove_member(&store, cid, &id)?;
    }

    match delete_record(&store, &id) {
        Ok(instance_id) => Ok(output::ok(
            "record delete",
            json!({ "instanceId": instance_id }),
        )),
        Err(e) => Ok(output::err("record delete", vec![e.to_string()])),
    }
}

fn parse_type_filter(type_filter: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = type_filter.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
}

fn parse_field_values_payload(stdin: &str) -> Result<Vec<FieldValue>, String> {
    let payload: serde_json::Value = serde_json::from_str(stdin)
        .map_err(|e| format!("Failed to parse record JSON from stdin: {}", e))?;

    let field_values_json = payload
        .get("fieldValues")
        .ok_or_else(|| "Missing fieldValues in record JSON".to_string())?;

    serde_json::from_value(field_values_json.clone()).map_err(|e| {
        format!(
            "fieldValues must be an array of {{fieldId, value}} objects: {}",
            e
        )
    })
}

fn resolve_type(
    ctx: &CliContext,
    namespace: &str,
    name: &str,
    version: Option<u32>,
) -> Result<Option<srs_core::types::record_type::RecordType>> {
    let store = FileStore::new(&ctx.repo);
    if let Some(version) = version {
        let package = store.load_package()?;
        let found = package
            .record_types
            .iter()
            .find(|rt| rt.namespace == namespace && rt.name == name && rt.version == version)
            .cloned();
        return Ok(found);
    }

    let result = get_type_by_name(&store, namespace, name)?;

    match result {
        GetTypeResult::Found(record_type) => Ok(Some(record_type)),
        GetTypeResult::NotFound => Ok(None),
    }
}
