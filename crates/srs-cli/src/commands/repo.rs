use crate::commands::{resolve_repo, RepoCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::analysis::build_repo_map;
use std::path::PathBuf;

pub fn dispatch(cmd: RepoCommand) -> Result<String> {
    match cmd {
        RepoCommand::Map { repo, json: _ } => cmd_repo_map(repo),
    }
}

fn cmd_repo_map(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let repo_map = build_repo_map(&repo_root)?;
    Ok(output::ok("repo map", json!({ "repoMap": repo_map })))
}
