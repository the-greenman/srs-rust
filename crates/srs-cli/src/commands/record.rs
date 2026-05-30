use crate::commands::{with_store, CliContext, RecordCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::record_store::{
    create_record_in_context, delete_record_in_context, get_record_by_id, list_records_filtered,
    update_record, CreateRecordInput, RecordListFilter,
};
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
    let (type_namespace, type_name) = match type_filter {
        None => (None, None),
        Some(ref filter) => match parse_type_filter(filter) {
            Some((namespace, name)) => (Some(namespace), Some(name)),
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

    let records = with_store(&ctx, |store| {
        Ok(list_records_filtered(
            store,
            RecordListFilter {
                type_namespace,
                type_name,
                container_id: ctx.container_id.clone(),
            },
        )?)
    })?;

    Ok(output::ok("record list", json!({ "records": records })))
}

fn cmd_record_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_record_by_id(store, &id)?))? {
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
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: CreateRecordInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record create",
                vec![format!("Failed to parse record JSON from stdin: {}", e)],
            ))
        }
    };

    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(create_record_in_context(
            store,
            &type_filter,
            version,
            input,
            container_id,
            &dir,
        )?)
    }) {
        Ok(result) => Ok(output::ok(
            "record create",
            json!({ "record": result.record }),
        )),
        Err(e) => Ok(output::err("record create", vec![e.to_string()])),
    }
}

fn cmd_record_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: CreateRecordInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record update",
                vec![format!("Failed to parse record JSON from stdin: {}", e)],
            ))
        }
    };

    match with_store(&ctx, |store| {
        Ok(update_record(store, &id, input.field_values)?)
    }) {
        Ok(record) => Ok(output::ok("record update", json!({ "record": record }))),
        Err(e) => Ok(output::err("record update", vec![e.to_string()])),
    }
}

fn cmd_record_delete(ctx: CliContext, id: String) -> Result<String> {
    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(delete_record_in_context(store, id, container_id)?)
    }) {
        Ok(result) => Ok(output::ok(
            "record delete",
            json!({ "instanceId": result.instance_id }),
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
