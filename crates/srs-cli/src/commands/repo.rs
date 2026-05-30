use crate::commands::{CliContext, RepoCommand, RepoExtensionsCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::analysis::build_repo_map;
use srs_repository::manifest_service::{
    add_declared_extension, list_declared_extensions, remove_declared_extension,
};
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::repository_portability::copy_repository;
use srs_repository::validation::validate_repository;
use srs_repository::FileStore;

pub fn dispatch(ctx: CliContext, cmd: RepoCommand) -> Result<String> {
    match cmd {
        RepoCommand::Create {
            repository_id,
            namespace,
            srs_version,
            package_id,
            package_name,
            package_version,
            package_namespace,
        } => cmd_repo_create(
            ctx,
            repository_id,
            namespace,
            srs_version,
            package_id,
            package_name,
            package_version,
            package_namespace,
        ),
        RepoCommand::Map { json: _ } => cmd_repo_map(ctx),
        RepoCommand::Copy { from, to } => cmd_repo_copy(ctx, from, to),
        RepoCommand::Validate { json: _ } => cmd_repo_validate(ctx),
        RepoCommand::Extensions(ext_cmd) => cmd_repo_extensions_dispatch(ctx, ext_cmd),
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_repo_create(
    ctx: CliContext,
    repository_id: String,
    namespace: String,
    srs_version: String,
    package_id: String,
    package_name: String,
    package_version: String,
    package_namespace: Option<String>,
) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let input = InitializeRepositoryInput {
        repository: RepositoryMetadata {
            repository_id,
            namespace: namespace.clone(),
            srs_version,
        },
        primary_package: PrimaryPackageMetadata {
            id: package_id,
            namespace: package_namespace.unwrap_or(namespace),
            name: package_name,
            version: package_version,
        },
    };

    let result = create_repository(&store, &input)?;
    Ok(output::ok(
        "repo create",
        json!({
            "repoRoot": result.repo_root,
        }),
    ))
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
    let store = FileStore::new(&ctx.repo);
    let extensions = list_declared_extensions(&store)?;
    Ok(output::ok(
        "repo extensions list",
        json!({ "extensions": extensions }),
    ))
}

fn cmd_repo_extensions_enable(ctx: CliContext, extension_id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let extensions = add_declared_extension(&store, &extension_id)?;
    Ok(output::ok(
        "repo extensions enable",
        json!({ "extensionId": extension_id, "extensions": extensions }),
    ))
}

fn cmd_repo_extensions_disable(ctx: CliContext, extension_id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let extensions = remove_declared_extension(&store, &extension_id)?;
    Ok(output::ok(
        "repo extensions disable",
        json!({ "extensionId": extension_id, "extensions": extensions }),
    ))
}

fn cmd_repo_map(ctx: CliContext) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let repo_map = build_repo_map(&store)?;
    Ok(output::ok("repo map", json!({ "repoMap": repo_map })))
}

fn cmd_repo_copy(
    _ctx: CliContext,
    from: std::path::PathBuf,
    to: std::path::PathBuf,
) -> Result<String> {
    let source = FileStore::new(&from);
    let target = FileStore::new(&to);
    copy_repository(&source, &target)?;
    Ok(output::ok(
        "repo copy",
        json!({
            "from": from,
            "to": to,
        }),
    ))
}

fn cmd_repo_validate(ctx: CliContext) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let report = validate_repository(&store)?;

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
