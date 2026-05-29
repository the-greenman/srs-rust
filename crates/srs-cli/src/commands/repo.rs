use crate::commands::{CliContext, RepoCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::analysis::build_repo_map;
use srs_repository::validation::validate_repository;

pub fn dispatch(ctx: CliContext, cmd: RepoCommand) -> Result<String> {
    match cmd {
        RepoCommand::Map { json: _ } => cmd_repo_map(ctx),
        RepoCommand::Validate { json: _ } => cmd_repo_validate(ctx),
    }
}

fn cmd_repo_map(ctx: CliContext) -> Result<String> {
    let repo_map = build_repo_map(&ctx.repo)?;
    Ok(output::ok("repo map", json!({ "repoMap": repo_map })))
}

fn cmd_repo_validate(ctx: CliContext) -> Result<String> {
    let report = validate_repository(&ctx.repo)?;

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
