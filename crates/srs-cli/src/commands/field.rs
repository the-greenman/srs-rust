use crate::commands::{with_store, CliContext, FieldCommand};
use crate::output;
use crate::payload::{FieldDeletePayload, FieldListEntry, FieldListPayload, FieldPayload};
use anyhow::Result;
use srs_core::types::field::Field;
use srs_repository::package_service::{
    create_field_normalized, delete_field, get_field_by_id, list_fields_filtered, update_field,
    FieldListFilter, GetFieldResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: FieldCommand) -> Result<String> {
    match cmd {
        FieldCommand::List {
            namespace,
            package,
            json: _,
        } => cmd_field_list(ctx, namespace, package),
        FieldCommand::Get { id, json: _ } => cmd_field_get(ctx, id),
        FieldCommand::Create { package, json: _ } => cmd_field_create(ctx, package),
        FieldCommand::Update { id } => cmd_field_update(ctx, id),
        FieldCommand::Delete { id } => cmd_field_delete(ctx, id),
    }
}

fn cmd_field_list(
    ctx: CliContext,
    namespace: Option<String>,
    package: Option<String>,
) -> Result<String> {
    let summaries = with_store(&ctx, |store| {
        Ok(list_fields_filtered(
            store,
            FieldListFilter {
                namespace: namespace.clone(),
                package: package.as_deref().map(|p| Some(p.to_string())),
            },
        )?)
    })?;

    let fields = summaries
        .into_iter()
        .map(|s| FieldListEntry {
            id: s.id,
            namespace: s.namespace,
            name: s.name,
            version: s.version,
            source_package: s.source_package,
        })
        .collect();

    output::serialize("field list", FieldListPayload { fields })
}

fn cmd_field_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_field_by_id(store, &id)?))? {
        GetFieldResult::Found(field) => {
            output::serialize("field get", FieldPayload { field: *field })
        }
        GetFieldResult::NotFound => Ok(output::err(
            "field get",
            vec![format!("Field with id '{}' not found", id)],
        )),
    }
}

fn cmd_field_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let raw_value: serde_json::Value = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse field JSON: {}", e))?;

    let result = with_store(&ctx, |store| {
        Ok(create_field_normalized(store, raw_value, package.clone())?)
    })?;

    output::serialize(
        "field create",
        FieldPayload {
            field: result.field,
        },
    )
}

fn cmd_field_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let field: Field = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse field JSON: {}", e))?;

    if field.id != id {
        return Ok(output::err(
            "field update",
            vec![format!(
                "Field ID in body ('{}') does not match --id argument ('{}')",
                field.id, id
            )],
        ));
    }

    match with_store(&ctx, |store| Ok(update_field(store, field)?)) {
        Ok(result) => output::serialize(
            "field update",
            FieldPayload {
                field: result.field,
            },
        ),
        Err(e) => Ok(output::err("field update", vec![e.to_string()])),
    }
}

fn cmd_field_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_field(store, &id)?)) {
        Ok(result) => output::serialize("field delete", FieldDeletePayload { id: result.id }),
        Err(e) => Ok(output::err("field delete", vec![e.to_string()])),
    }
}
