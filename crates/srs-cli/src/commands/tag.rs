use crate::commands::{CliContext, TagCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::tag_definition::TagDefinition;
use srs_repository::tag_service::{
    create_tag_definition, get_tag_definition_by_id, list_tag_definitions,
    list_tag_definitions_by_role, GetTagDefinitionResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: TagCommand) -> Result<String> {
    match cmd {
        TagCommand::List { role, json: _ } => cmd_tag_list(ctx, role),
        TagCommand::Get { id, json: _ } => cmd_tag_get(ctx, id),
        TagCommand::Create { json: _ } => cmd_tag_create(ctx),
    }
}

fn cmd_tag_list(ctx: CliContext, role: Option<String>) -> Result<String> {
    let summaries = if let Some(role_filter) = role {
        list_tag_definitions_by_role(&ctx.repo, &role_filter)?
    } else {
        list_tag_definitions(&ctx.repo)?
    };

    Ok(output::ok(
        "tag list",
        json!({ "tagDefinitions": summaries }),
    ))
}

fn cmd_tag_get(ctx: CliContext, id: String) -> Result<String> {
    match get_tag_definition_by_id(&ctx.repo, &id)? {
        GetTagDefinitionResult::Found(td) => {
            Ok(output::ok("tag get", json!({ "tagDefinition": *td })))
        }
        GetTagDefinitionResult::NotFound => Ok(output::err(
            "tag get",
            vec![format!("TagDefinition with id '{}' not found", id)],
        )),
    }
}

fn cmd_tag_create(ctx: CliContext) -> Result<String> {
    // Read JSON from stdin - expects a TagDefinition
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let tag_definition: TagDefinition = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse TagDefinition JSON: {}", e))?;

    // Create the TagDefinition via the dedicated service
    let result = create_tag_definition(&ctx.repo, tag_definition)?;

    Ok(output::ok(
        "tag create",
        json!({ "tagDefinition": result.tag_definition }),
    ))
}
