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
    self as blueprint_brief_service, BlueprintBriefInput, BlueprintBriefResult, BriefProtocolResult,
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
            let rendered = render_brief_markdown(&result);
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

fn render_brief_markdown(result: &BlueprintBriefResult) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "# Blueprint: {}/{} v{}\n\n",
        result.namespace, result.name, result.version
    ));

    if let Some(guidance) = &result.ai_guidance {
        out.push_str(&format_guidance_prose(guidance));
        out.push('\n');
    }

    if !result.required_types.is_empty() {
        out.push_str("**Required types:**\n");
        for rt in &result.required_types {
            if let Some(id) = rt.get("typeId").and_then(|v| v.as_str()) {
                out.push_str(&format!("- `{id}`\n"));
            }
        }
        out.push('\n');
    }

    for t in &result.types {
        out.push_str(&format!("## Type: {}/{}\n\n", t.namespace, t.name));
        if let Some(guidance) = &t.ai_guidance {
            out.push_str(&format_guidance_prose(guidance));
            out.push('\n');
        }
        if !t.fields.is_empty() {
            out.push_str(
                "| Field | ValueType | Required | Purpose | Extraction | Negative | Examples |\n",
            );
            out.push_str("|---|---|---|---|---|---|---|\n");
            for f in &t.fields {
                let purpose = extract_str_field(&f.ai_guidance, "purpose");
                let extraction = extract_str_field(&f.ai_guidance, "extraction");
                let negative = extract_str_field(&f.ai_guidance, "negativeGuidance");
                let examples = extract_str_field(&f.ai_guidance, "examples");
                let required = if f.required { "yes" } else { "no" };
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} | {} | {} | {} |\n",
                    f.name, f.value_type, required, purpose, extraction, negative, examples
                ));
            }
            out.push('\n');
        }
    }

    if !result.structure.is_empty() {
        out.push_str("## Structure\n\n");
        for rs in &result.structure {
            let card = rs
                .cardinality
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            out.push_str(&format!(
                "- `{}` → `{}` via `{}`{}\n",
                rs.source_type_id, rs.target_type_id, rs.relation_type, card
            ));
        }
        out.push('\n');
    }

    if let Some(proto) = &result.protocol {
        out.push_str(&format!("## Protocol: {}\n\n", proto.protocol_name));
        for stage in &proto.stages {
            out.push_str(&format!("### {}. {}\n\n", stage.order, stage.name));
            // purpose is structural metadata (epistemic label); not rendered — question is the prose entry point
            if let Some(q) = &stage.question {
                out.push_str(&format!("**Question:** {q}\n\n"));
            }
            if let Some(cc) = &stage.completion_criteria {
                out.push_str(&format!("**Done when:** {cc}\n\n"));
            }
            if let Some(ct) = &stage.contributes_to {
                if !ct.is_empty() {
                    let labels: Vec<String> = ct
                        .iter()
                        .map(|r| match &r.type_id {
                            Some(tid) => format!("{}/{}", tid, r.field_id),
                            None => r.field_id.clone(),
                        })
                        .collect();
                    out.push_str(&format!("**Contributes to:** {}\n\n", labels.join(", ")));
                }
            }
            if let Some(dep) = stage.ai_guidance.as_ref() {
                out.push_str(&format_guidance_prose(dep));
                out.push('\n');
            }
        }
    }

    out
}

fn format_guidance_prose(guidance: &serde_json::Value) -> String {
    if let Some(s) = guidance.as_str() {
        return format!("{s}\n");
    }
    let mut out = String::new();
    if let Some(purpose) = guidance.get("purpose").and_then(|v| v.as_str()) {
        out.push_str(&format!("{purpose}\n"));
    }
    if let Some(extraction) = guidance.get("extraction").and_then(|v| v.as_str()) {
        out.push_str(&format!("\n**Extraction:** {extraction}\n"));
    }
    if let Some(neg) = guidance.get("negativeGuidance").and_then(|v| v.as_str()) {
        out.push_str(&format!("\n**Avoid:** {neg}\n"));
    }
    out
}

fn extract_str_field(guidance: &Option<serde_json::Value>, key: &str) -> String {
    guidance
        .as_ref()
        .and_then(|g| g.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::protocol::FieldRef;
    use srs_repository::blueprint_brief_service::{
        BlueprintBriefResult, BriefFieldResult, BriefProtocolResult, BriefStageResult,
        BriefTypeResult,
    };

    #[test]
    fn test_render_markdown_contains_blueprint_name() {
        let result = BlueprintBriefResult {
            blueprint_id: "bp-test".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-blueprint".to_string(),
            version: 1,
            ai_guidance: None,
            required_types: vec![],
            types: vec![BriefTypeResult {
                type_id: "type-111".to_string(),
                namespace: "test.ns".to_string(),
                name: "article".to_string(),
                ai_guidance: None,
                fields: vec![
                    BriefFieldResult {
                        field_id: "field-aaa".to_string(),
                        name: "title".to_string(),
                        order: 2,
                        required: true,
                        value_type: "string".to_string(),
                        ai_guidance: None,
                    },
                    BriefFieldResult {
                        field_id: "field-bbb".to_string(),
                        name: "summary".to_string(),
                        order: 1,
                        required: false,
                        value_type: "text".to_string(),
                        ai_guidance: None,
                    },
                ],
            }],
            structure: vec![],
            protocol: None,
            diagnostics: vec![],
        };
        let md = render_brief_markdown(&result);
        assert!(
            md.contains("test-blueprint"),
            "rendered markdown must contain blueprint name"
        );
        assert!(
            md.contains("title") || md.contains("summary"),
            "must contain at least one field name"
        );
    }

    #[test]
    fn test_render_brief_markdown_contributes_to() {
        let result = BlueprintBriefResult {
            blueprint_id: "bp-1".to_string(),
            namespace: "com.example".to_string(),
            name: "My Blueprint".to_string(),
            version: 1,
            ai_guidance: None,
            required_types: vec![],
            types: vec![],
            structure: vec![],
            protocol: Some(BriefProtocolResult {
                protocol_id: "proto-1".to_string(),
                protocol_name: "Example Protocol".to_string(),
                stages: vec![BriefStageResult {
                    stage_id: "s1".to_string(),
                    name: "Gather".to_string(),
                    purpose: None,
                    order: 1,
                    depends_on: vec![],
                    question: None,
                    completion_criteria: None,
                    contributes_to: Some(vec![
                        FieldRef {
                            field_id: "my-field".to_string(),
                            type_id: None,
                        },
                        FieldRef {
                            field_id: "other-field".to_string(),
                            type_id: Some("type-abc".to_string()),
                        },
                    ]),
                    ai_guidance: None,
                    output_type: None,
                }],
            }),
            diagnostics: vec![],
        };
        let md = render_brief_markdown(&result);
        assert!(
            md.contains("**Contributes to:** my-field, type-abc/other-field"),
            "expected correctly formatted contributes_to in markdown, got:\n{md}"
        );
    }
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
