use crate::commands::{with_store, BlueprintCommand, CliContext};
use crate::output;
use crate::payload::{
    self as payload, BlueprintBriefPayload, BlueprintDeletePayload, BlueprintListEntry,
    BlueprintListPayload, BlueprintPayload, BlueprintSchemaPayload, BlueprintStructurePayload,
    BlueprintValidatePayload, BriefField, BriefProtocol, BriefRelationSpec, BriefStage, BriefType,
    RelationSpecEntry,
};
use anyhow::Result;
use srs_core::types::blueprint::Blueprint;
use srs_repository::blueprint_brief_service::{
    self as blueprint_brief_service, BlueprintBriefInput, BriefProtocolResult,
    BriefRelationSpecResult, BriefStageResult, BriefTypeResult,
};
use srs_repository::blueprint_schema_service::{
    self as blueprint_schema_svc, BlueprintSchemaInput,
};
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
        BlueprintCommand::Brief { id } => cmd_blueprint_brief(ctx, id),
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

fn cmd_blueprint_brief(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(blueprint_brief_service::blueprint_brief(
            store,
            BlueprintBriefInput {
                blueprint_id: id.clone(),
            },
        )?)
    }) {
        Ok(result) => {
            let rendered = blueprint_brief_service::render_brief_markdown(&result);
            output::serialize(
                "blueprint brief",
                BlueprintBriefPayload {
                    rendered,
                    blueprint_id: result.blueprint_id,
                    namespace: result.namespace,
                    name: result.name,
                    version: result.version,
                    ai_guidance: result.ai_guidance,
                    required_types: result.required_types,
                    types: result.types.into_iter().map(map_brief_type).collect(),
                    structure: result
                        .structure
                        .into_iter()
                        .map(map_brief_relation_spec)
                        .collect(),
                    protocol: result.protocol.map(map_brief_protocol),
                    diagnostics: result.diagnostics,
                },
            )
        }
        Err(e) => {
            if let Some(RepositoryError::BlueprintNotFound { .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "blueprint brief",
                    vec![format!("Blueprint '{id}' not found")],
                ));
            }
            Err(e)
        }
    }
}

fn map_brief_type(t: BriefTypeResult) -> BriefType {
    BriefType {
        type_id: t.type_id,
        namespace: t.namespace,
        name: t.name,
        ai_guidance: t.ai_guidance,
        fields: t
            .fields
            .into_iter()
            .map(|f| BriefField {
                field_id: f.field_id,
                name: f.name,
                order: f.order,
                required: f.required,
                value_type: f.value_type,
                ai_guidance: f.ai_guidance,
            })
            .collect(),
    }
}

fn map_brief_relation_spec(rs: BriefRelationSpecResult) -> BriefRelationSpec {
    BriefRelationSpec {
        relation_type: rs.relation_type,
        source_type_id: rs.source_type_id,
        target_type_id: rs.target_type_id,
        cardinality: rs.cardinality,
        required: rs.required,
    }
}

fn map_brief_protocol(p: BriefProtocolResult) -> BriefProtocol {
    BriefProtocol {
        protocol_id: p.protocol_id,
        protocol_name: p.protocol_name,
        stages: p.stages.into_iter().map(map_brief_stage).collect(),
    }
}

fn map_brief_stage(s: BriefStageResult) -> BriefStage {
    BriefStage {
        stage_id: s.stage_id,
        name: s.name,
        purpose: s.purpose,
        order: s.order,
        depends_on: s.depends_on,
        question: s.question,
        completion_criteria: s.completion_criteria,
        contributes_to: s.contributes_to.map(|refs| {
            refs.into_iter()
                .map(|r| payload::FieldRef {
                    field_id: r.field_id,
                    type_id: r.type_id,
                })
                .collect()
        }),
        ai_guidance: s.ai_guidance,
        output_type: s.output_type,
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
        Ok(blueprint_schema_svc::blueprint_schema(
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
