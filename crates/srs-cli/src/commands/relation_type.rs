use crate::commands::{resolve_repo, RelationTypeCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::package::load_package;
use std::path::PathBuf;

pub fn dispatch(cmd: RelationTypeCommand) -> Result<String> {
    match cmd {
        RelationTypeCommand::List { repo, status } => cmd_relation_type_list(repo, status),
        RelationTypeCommand::Get { repo, id } => cmd_relation_type_get(repo, id),
    }
}

fn cmd_relation_type_list(repo: Option<PathBuf>, status_filter: Option<String>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let package = load_package(&repo_root)?;

    let relation_type_definitions: Vec<_> = package
        .relation_type_definitions
        .iter()
        .filter(|rt| {
            if let Some(ref filter) = status_filter {
                let rt_status = rt
                    .status
                    .as_ref()
                    .and_then(|s| serde_json::to_value(s).ok())
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "active".to_string());
                rt_status == *filter
            } else {
                true
            }
        })
        .map(|rt| serde_json::to_value(rt).unwrap_or(json!(null)))
        .collect();

    Ok(output::ok(
        "relation-type list",
        json!({ "relationTypeDefinitions": relation_type_definitions }),
    ))
}

fn cmd_relation_type_get(repo: Option<PathBuf>, id: String) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let package = load_package(&repo_root)?;

    match package.resolve_relation_type_by_id(&id) {
        Some(rt) => Ok(output::ok(
            "relation-type get",
            json!({ "relationTypeDefinition": rt }),
        )),
        None => Ok(output::err(
            "relation-type get",
            vec![format!("relation type definition not found: {}", id)],
        )),
    }
}
