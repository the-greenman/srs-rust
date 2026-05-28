use crate::commands::{resolve_repo, RepoCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::analysis::build_repo_map;
use srs_repository::validation::validate_repository;
use std::path::PathBuf;

pub fn dispatch(cmd: RepoCommand) -> Result<String> {
    match cmd {
        RepoCommand::Map { repo, json: _ } => cmd_repo_map(repo),
        RepoCommand::Validate { repo, json: _ } => cmd_repo_validate(repo),
    }
}

fn cmd_repo_map(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let repo_map = build_repo_map(&repo_root)?;
    Ok(output::ok("repo map", json!({ "repoMap": repo_map })))
}

fn cmd_repo_validate(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let report = validate_repository(&repo_root)?;

    if report.is_ok() {
        Ok(output::ok(
            "repo validate",
            json!({
                "summary": report.summary,
                "diagnostics": report.diagnostics,
            }),
        ))
    } else {
        let diagnostics: Vec<String> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == srs_repository::validation::DiagnosticSeverity::Error)
            .map(|d| format!("[{}] {}", d.path, d.message))
            .collect();
        Ok(output::err("repo validate", diagnostics))
    }
}
