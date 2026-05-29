use crate::commands::{CliContext, RecordCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::record_store::{
    get_record_by_id, list_records_by_type,
};

pub fn dispatch(ctx: CliContext, cmd: RecordCommand) -> Result<String> {
    match cmd {
        RecordCommand::List { type_filter, json: _ } => cmd_record_list(ctx, type_filter),
        RecordCommand::Get { id, json: _ } => cmd_record_get(ctx, id),
    }
}

fn cmd_record_list(ctx: CliContext, type_filter: String) -> Result<String> {
    // Parse namespace/name from the type filter
    let parts: Vec<&str> = type_filter.split('/').collect();
    if parts.len() != 2 {
        return Ok(output::err(
            "record list",
            vec![format!("Invalid type filter '{}'. Expected format: namespace/name", type_filter)],
        ));
    }
    
    let namespace = parts[0];
    let name = parts[1];

    let records = list_records_by_type(&ctx.repo, namespace, name)?;

    Ok(output::ok("record list", json!({ "records": records })))
}

fn cmd_record_get(ctx: CliContext, id: String) -> Result<String> {
    match get_record_by_id(&ctx.repo, &id)? {
        Some(record) => {
            Ok(output::ok("record get", json!({ "record": record })))
        }
        None => Ok(output::err(
            "record get",
            vec![format!("Record with id '{}' not found", id)],
        )),
    }
}
