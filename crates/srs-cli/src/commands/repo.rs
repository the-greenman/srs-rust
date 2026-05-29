use crate::commands::{CliContext, RepoCommand, RepoExtensionsCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::analysis::build_repo_map;
use srs_repository::manifest_service::{
    add_declared_extension, list_declared_extensions, remove_declared_extension,
};
use srs_repository::validation::validate_repository;

pub fn dispatch(ctx: CliContext, cmd: RepoCommand) -> Result<String> {
    match cmd {
        RepoCommand::Map { json: _ } => cmd_repo_map(ctx),
        RepoCommand::Validate { json: _ } => cmd_repo_validate(ctx),
        RepoCommand::Extensions(ext_cmd) => cmd_repo_extensions_dispatch(ctx, ext_cmd),
    }
}

fn cmd_repo_extensions_dispatch(ctx: CliContext, cmd: RepoExtensionsCommand) -> Result<String> {
    match cmd {
        RepoExtensionsCommand::List { json: _ } => cmd_repo_extensions_list(ctx),
        RepoExtensionsCommand::Enable {
            extension_id,
            json: _,
        } => cmd_repo_extensions_enable(ctx, extension_id),
        RepoExtensionsCommand::Disable {
            extension_id,
            json: _,
        } => cmd_repo_extensions_disable(ctx, extension_id),
    }
}

fn cmd_repo_extensions_list(ctx: CliContext) -> Result<String> {
    let extensions = list_declared_extensions(&ctx.repo)?;
    Ok(output::ok(
        "repo extensions list",
        json!({ "extensions": extensions }),
    ))
}

fn cmd_repo_extensions_enable(ctx: CliContext, extension_id: String) -> Result<String> {
    let extensions = add_declared_extension(&ctx.repo, &extension_id)?;
    Ok(output::ok(
        "repo extensions enable",
        json!({ "extensionId": extension_id, "extensions": extensions }),
    ))
}

fn cmd_repo_extensions_disable(ctx: CliContext, extension_id: String) -> Result<String> {
    let extensions = remove_declared_extension(&ctx.repo, &extension_id)?;
    Ok(output::ok(
        "repo extensions disable",
        json!({ "extensionId": extension_id, "extensions": extensions }),
    ))
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
            .map(|d| format!("[{}] {}", d.relative_path, d.message))
            .collect();
        Ok(output::err("repo validate", diagnostics))
    }
}
