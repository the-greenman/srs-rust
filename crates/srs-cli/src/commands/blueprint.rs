use crate::commands::{with_store, BlueprintCommand, CliContext};
use crate::output;
use crate::payload::{
    BlueprintDeletePayload, BlueprintListEntry, BlueprintListPayload, BlueprintPayload,
    BlueprintSchemaPayload, BlueprintStructurePayload, BlueprintValidatePayload, RelationSpecEntry,
};
use anyhow::Result;
use srs_core::types::blueprint::Blueprint;
use srs_repository::blueprint_schema_service::{self, BlueprintSchemaInput};
use srs_repository::blueprint_service::{
    create_blueprint, delete_blueprint, get_blueprint_by_id, list_blueprint_structure,
    list_blueprints_summary, update_blueprint, validate_blueprint_by_id, GetBlueprintResult,
};
use srs_repository::error::RepositoryError;
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: BlueprintCommand) -> Result<String> {
    match cmd {
        BlueprintCommand::List => cmd_blueprint_list(ctx),
        BlueprintCommand::Get { id } => cmd_blueprint_get(ctx, id),
        BlueprintCommand::Create { package } => cmd_blueprint_create(ctx, package),
        BlueprintCommand::Update { id } => cmd_blueprint_update(ctx, id),
        BlueprintCommand::Delete { id } => cmd_blueprint_delete(ctx, id),
        BlueprintCommand::Validate { id } => cmd_blueprint_validate(ctx, id),
        BlueprintCommand::Structure { id } => cmd_blueprint_structure(ctx, id),
        BlueprintCommand::Schema { id } => cmd_blueprint_schema(ctx, id),
    }
}

fn cmd_blueprint_list(ctx: CliContext) -> Result<String> {
    let result = with_store(&ctx, |store| Ok(list_blueprints_summary(store)?))?;

    let blueprints = result
        .summaries
        .into_iter()
        .map(|s| BlueprintListEntry {
            blueprint_id: s.id,
            namespace: s.namespace,
            name: s.name,
            version: s.version,
            description: s.description,
            root_type_count: s.root_type_count,
            source_package: s.source_package,
        })
        .collect();

    output::serialize(
        "blueprint list",
        BlueprintListPayload {
            blueprints,
            diagnostics: result.diagnostics,
        },
    )
}

fn cmd_blueprint_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_blueprint_by_id(store, &id)?))? {
        GetBlueprintResult::Found(bp) => {
            let blueprint = serde_json::to_value(*bp)
                .map_err(|e| anyhow::anyhow!("Failed to serialize blueprint: {e}"))?;
            output::serialize("blueprint get", BlueprintPayload { blueprint })
        }
        GetBlueprintResult::NotFound => Ok(output::err(
            "blueprint get",
            vec![format!("Blueprint '{id}' not found")],
        )),
    }
}

fn cmd_blueprint_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let blueprint: Blueprint = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse blueprint JSON: {e}"))?;

    let result = with_store(&ctx, |store| {
        Ok(create_blueprint(store, blueprint, package.clone())?)
    })?;

    let blueprint = serde_json::to_value(result.blueprint)
        .map_err(|e| anyhow::anyhow!("Failed to serialize blueprint: {e}"))?;
    output::serialize("blueprint create", BlueprintPayload { blueprint })
}

fn cmd_blueprint_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let blueprint: Blueprint = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse blueprint JSON: {e}"))?;

    let result = match with_store(&ctx, |store| Ok(update_blueprint(store, &id, blueprint)?)) {
        Ok(r) => r,
        Err(e) => {
            if let Some(RepositoryError::BlueprintNotFound { .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "blueprint update",
                    vec![format!("Blueprint '{id}' not found")],
                ));
            }
            return Err(e);
        }
    };

    let blueprint = serde_json::to_value(result.blueprint)
        .map_err(|e| anyhow::anyhow!("Failed to serialize blueprint: {e}"))?;
    output::serialize("blueprint update", BlueprintPayload { blueprint })
}

fn cmd_blueprint_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_blueprint(store, &id)?)) {
        Ok(result) => {
            output::serialize("blueprint delete", BlueprintDeletePayload { id: result.id })
        }
        Err(e) => {
            if let Some(RepositoryError::BlueprintNotFound { .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "blueprint delete",
                    vec![format!("Blueprint '{id}' not found")],
                ));
            }
            Err(e)
        }
    }
}

fn cmd_blueprint_validate(ctx: CliContext, id: String) -> Result<String> {
    let result = match with_store(&ctx, |store| Ok(validate_blueprint_by_id(store, &id)?)) {
        Ok(r) => r,
        Err(e) => {
            if let Some(RepositoryError::BlueprintNotFound { .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "blueprint validate",
                    vec![format!("Blueprint '{id}' not found")],
                ));
            }
            return Err(e);
        }
    };

    output::serialize(
        "blueprint validate",
        BlueprintValidatePayload {
            id: result.id,
            valid: result.valid,
            diagnostics: result.diagnostics,
        },
    )
}

fn cmd_blueprint_structure(ctx: CliContext, id: String) -> Result<String> {
    let specs = match with_store(&ctx, |store| Ok(list_blueprint_structure(store, &id)?)) {
        Ok(s) => s,
        Err(e) => {
            if let Some(RepositoryError::BlueprintNotFound { .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "blueprint structure",
                    vec![format!("Blueprint '{id}' not found")],
                ));
            }
            return Err(e);
        }
    };

    let relation_specs = specs
        .into_iter()
        .map(|rs| RelationSpecEntry {
            relation_type: rs.relation_type,
            source_type_id: rs.source_type.type_id,
            target_type_id: rs.target_type.type_id,
            cardinality: rs.cardinality,
            required: rs.required,
        })
        .collect();

    output::serialize(
        "blueprint structure",
        BlueprintStructurePayload { relation_specs },
    )
}

fn cmd_blueprint_schema(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(blueprint_schema_service::blueprint_schema(
            store,
            BlueprintSchemaInput {
                blueprint_id: id.clone(),
            },
        )?)
    }) {
        Ok(result) => output::serialize(
            "blueprint schema",
            BlueprintSchemaPayload {
                schema: result.schema,
                diagnostics: result.diagnostics,
            },
        ),
        Err(e) => {
            if let Some(RepositoryError::BlueprintNotFound { .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "blueprint schema",
                    vec![format!("Blueprint '{id}' not found")],
                ));
            }
            Err(e)
        }
    }
}
