use crate::commands::{resolve_repo, TagCommand};
use crate::output;
use anyhow::{Context, Result};
use serde_json::json;
use srs_core::types::tag_definition::TagDefinition;
use srs_repository::tag_service::{
    create_tag_definition, get_tag_definition_by_id, list_tag_definitions,
    list_tag_definitions_by_role, GetTagDefinitionResult,
};
use std::io::{self, Read};
use std::path::PathBuf;

pub fn dispatch(cmd: TagCommand) -> Result<String> {
    match cmd {
        TagCommand::List { repo, role } => cmd_tag_list(repo, role),
        TagCommand::Get { repo, id } => cmd_tag_get(repo, id),
        TagCommand::Create { repo } => cmd_tag_create(repo),
    }
}

fn cmd_tag_list(repo: Option<PathBuf>, role: Option<String>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    let summaries = if let Some(role_filter) = role {
        list_tag_definitions_by_role(&repo_root, &role_filter)?
    } else {
        list_tag_definitions(&repo_root)?
    };

    Ok(output::ok("tag list", json!({ "tagDefinitions": summaries })))
}

fn cmd_tag_get(repo: Option<PathBuf>, id: String) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    match get_tag_definition_by_id(&repo_root, &id)? {
        GetTagDefinitionResult::Found(td) => {
            Ok(output::ok("tag get", json!({ "tagDefinition": *td })))
        }
        GetTagDefinitionResult::NotFound => Ok(output::err(
            "tag get",
            vec![format!("TagDefinition with id '{}' not found", id)],
        )),
    }
}

fn cmd_tag_create(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    // Read JSON from stdin - expects a TagDefinition
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let tag_definition: TagDefinition = serde_json::from_str(&stdin)
        .context("Failed to parse TagDefinition JSON")?;

    // Create the TagDefinition via the dedicated service
    let result = create_tag_definition(&repo_root, tag_definition)?;

    Ok(output::ok("tag create", json!({ "tagDefinition": result.tag_definition })))
}
