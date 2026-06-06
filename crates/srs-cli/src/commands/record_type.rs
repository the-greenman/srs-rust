use crate::commands::{with_store, CliContext, TypeCommand};
use crate::output::{self, OutputDTO};
use crate::payload::{
    TypeDeletePayload, TypeListEntry, TypeListPayload, TypePayload, TypeSchemaPayload,
};
use anyhow::Result;
use srs_core::types::record_type::RecordType;
use srs_repository::package_service::{
    create_type_in_package, delete_type, get_type_by_id_latest, list_types_filtered, update_type,
    GetTypeResult, TypeListFilter,
};
use srs_repository::type_schema_service::{type_schema, TypeSchemaInput};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: TypeCommand) -> Result<String> {
    match cmd {
        TypeCommand::List {
            namespace,
            package,
            json: _,
        } => cmd_type_list(ctx, namespace, package),
        TypeCommand::Get { id, json: _ } => cmd_type_get(ctx, id),
        TypeCommand::Create { package, json: _ } => cmd_type_create(ctx, package),
        TypeCommand::Update { id } => cmd_type_update(ctx, id),
        TypeCommand::Delete { id, version } => cmd_type_delete(ctx, id, version),
        TypeCommand::Schema { id, type_version } => cmd_type_schema(ctx, id, type_version),
    }
}

fn cmd_type_list(
    ctx: CliContext,
    namespace: Option<String>,
    package: Option<String>,
) -> Result<String> {
    let summaries = with_store(&ctx, |store| {
        Ok(list_types_filtered(
            store,
            TypeListFilter {
                namespace: namespace.clone(),
                package: package.as_deref().map(|p| Some(p.to_string())),
            },
        )?)
    })?;

    let types = summaries
        .into_iter()
        .map(|s| TypeListEntry {
            id: s.id,
            namespace: s.namespace,
            name: s.name,
            version: s.version,
            field_count: s.field_count,
            source_package: s.source_package,
        })
        .collect();

    output::serialize("type list", TypeListPayload { types })
}

fn cmd_type_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_type_by_id_latest(store, &id)?))? {
        GetTypeResult::Found(record_type) => {
            output::serialize("type get", TypePayload { record_type })
        }
        GetTypeResult::NotFound => Ok(output::err(
            "type get",
            vec![format!("Type with id '{}' not found", id)],
        )),
    }
}

fn cmd_type_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let record_type: RecordType = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse type JSON: {}", e))?;

    let result = with_store(&ctx, |store| {
        Ok(create_type_in_package(store, record_type, package.clone())?)
    })?;

    output::serialize(
        "type create",
        TypePayload {
            record_type: result.record_type,
        },
    )
}

fn cmd_type_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let record_type: RecordType = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse type JSON: {}", e))?;

    if record_type.id != id {
        return Ok(output::err(
            "type update",
            vec![format!(
                "Type ID in body ('{}') does not match --id argument ('{}')",
                record_type.id, id
            )],
        ));
    }

    match with_store(&ctx, |store| Ok(update_type(store, record_type)?)) {
        Ok(result) => output::serialize(
            "type update",
            TypePayload {
                record_type: result.record_type,
            },
        ),
        Err(e) => Ok(output::err("type update", vec![e.to_string()])),
    }
}

fn cmd_type_delete(ctx: CliContext, id: String, version: Option<u32>) -> Result<String> {
    // Resolve version: use provided value, or look up the latest.
    let resolved_version = match version {
        Some(v) => v,
        None => match with_store(&ctx, |store| Ok(get_type_by_id_latest(store, &id)?))? {
            GetTypeResult::Found(rt) => rt.version,
            GetTypeResult::NotFound => {
                return Ok(output::err(
                    "type delete",
                    vec![format!("Type with id '{}' not found", id)],
                ))
            }
        },
    };

    match with_store(&ctx, |store| Ok(delete_type(store, &id, resolved_version)?)) {
        Ok(result) => output::serialize("type delete", TypeDeletePayload { id: result.id }),
        Err(e) => Ok(output::err("type delete", vec![e.to_string()])),
    }
}

fn cmd_type_schema(ctx: CliContext, id: String, type_version: Option<u32>) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(type_schema(
            store,
            TypeSchemaInput {
                type_id: id.clone(),
                type_version,
            },
        )?)
    }) {
        Ok(result) => {
            let payload = serde_json::to_value(TypeSchemaPayload {
                schema: result.schema,
            })?;
            let dto = OutputDTO {
                ok: true,
                command: "type schema".to_string(),
                version: output::VERSION.to_string(),
                payload: Some(payload),
                diagnostics: if result.diagnostics.is_empty() {
                    None
                } else {
                    Some(result.diagnostics)
                },
            };
            Ok(dto.render(ctx.format, ctx.pretty))
        }
        Err(e) => Ok(output::err("type schema", vec![e.to_string()])),
    }
}
