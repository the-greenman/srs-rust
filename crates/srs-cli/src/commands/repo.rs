use crate::commands::{with_store, CliContext, RepoCommand, RepoExtensionsCommand, StoreBackend};
use crate::output;
use crate::payload::{
    RepoCopyPayload, RepoCreatePayload, RepoExtensionsMutatePayload, RepoExtensionsPayload,
    RepoMapPayload, RepoValidatePayload,
};
use anyhow::{Context, Result};
use srs_repository::analysis::build_repo_map;
use srs_repository::manifest_service::{
    add_declared_extension, list_declared_extensions, remove_declared_extension,
};
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::repository_portability::copy_repository;
use srs_repository::validation::validate_repository;
use srs_repository::{FileStore, JsonStore};

pub fn dispatch(ctx: CliContext, cmd: RepoCommand) -> Result<String> {
    match cmd {
        RepoCommand::Create {
            repository_id,
            namespace,
            title,
            description,
            srs_version,
            package_id,
            package_name,
            package_version,
            package_namespace,
        } => cmd_repo_create(
            ctx,
            repository_id,
            namespace,
            title,
            description,
            srs_version,
            package_id,
            package_name,
            package_version,
            package_namespace,
        ),
        RepoCommand::Map { json: _ } => cmd_repo_map(ctx),
        RepoCommand::Copy {
            from,
            to,
            from_store,
            to_store,
        } => cmd_repo_copy(ctx, from, to, from_store, to_store),
        RepoCommand::Validate { json: _ } => cmd_repo_validate(ctx),
        RepoCommand::Extensions(ext_cmd) => cmd_repo_extensions_dispatch(ctx, ext_cmd),
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_repo_create(
    ctx: CliContext,
    repository_id: Option<String>,
    namespace: String,
    title: Option<String>,
    description: Option<String>,
    srs_version: String,
    package_id: Option<String>,
    package_name: String,
    package_version: String,
    package_namespace: Option<String>,
) -> Result<String> {
    let repository_id = repository_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let package_id = package_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let input = InitializeRepositoryInput {
        repository: RepositoryMetadata {
            repository_id,
            namespace: namespace.clone(),
            srs_version,
            title,
            description,
        },
        primary_package: PrimaryPackageMetadata {
            id: package_id,
            namespace: package_namespace.unwrap_or(namespace),
            name: package_name,
            version: package_version,
        },
    };

    let result = match ctx.store {
        StoreBackend::File => {
            let store = FileStore::new(&ctx.repo);
            create_repository_with_intent(&store, &input)?
        }
        StoreBackend::Json => {
            let store = JsonStore::create(&ctx.repo)
                .with_context(|| format!("Failed to create JsonStore at {}", ctx.repo.display()))?;
            create_repository_with_intent(&store, &input)?
        }
    };

    output::serialize(
        "repo create",
        RepoCreatePayload {
            repo_root: result.repo_root,
            repository_id: result.repository_id,
            package_id: result.package_id,
            root_note_id: result.root_note_id,
        },
    )
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
    let extensions = with_store(&ctx, |store| Ok(list_declared_extensions(store)?))?;
    output::serialize("repo extensions list", RepoExtensionsPayload { extensions })
}

fn cmd_repo_extensions_enable(ctx: CliContext, extension_id: String) -> Result<String> {
    let extensions = with_store(&ctx, |store| {
        Ok(add_declared_extension(store, &extension_id)?)
    })?;
    output::serialize(
        "repo extensions enable",
        RepoExtensionsMutatePayload {
            extension_id,
            extensions,
        },
    )
}

fn cmd_repo_extensions_disable(ctx: CliContext, extension_id: String) -> Result<String> {
    let extensions = with_store(&ctx, |store| {
        Ok(remove_declared_extension(store, &extension_id)?)
    })?;
    output::serialize(
        "repo extensions disable",
        RepoExtensionsMutatePayload {
            extension_id,
            extensions,
        },
    )
}

fn cmd_repo_map(ctx: CliContext) -> Result<String> {
    let repo_map = with_store(&ctx, |store| Ok(build_repo_map(store)?))?;
    output::serialize("repo map", RepoMapPayload { repo_map })
}

fn cmd_repo_copy(
    _ctx: CliContext,
    from: std::path::PathBuf,
    to: std::path::PathBuf,
    from_store: Option<StoreBackend>,
    to_store: Option<StoreBackend>,
) -> Result<String> {
    let from_store = from_store.unwrap_or_else(|| infer_copy_store(&from));
    let to_store = to_store.unwrap_or_else(|| infer_copy_store(&to));

    match (from_store, to_store) {
        (StoreBackend::File, StoreBackend::File) => {
            let source = FileStore::new(&from);
            let target = FileStore::new(&to);
            copy_repository(&source, &target)?;
        }
        (StoreBackend::File, StoreBackend::Json) => {
            let source = FileStore::new(&from);
            let target = JsonStore::create(&to)
                .with_context(|| format!("Failed to create JsonStore at {}", to.display()))?;
            copy_repository(&source, &target)?;
        }
        (StoreBackend::Json, StoreBackend::File) => {
            let source = JsonStore::open(&from)
                .with_context(|| format!("Failed to open JsonStore at {}", from.display()))?;
            let target = FileStore::new(&to);
            copy_repository(&source, &target)?;
        }
        (StoreBackend::Json, StoreBackend::Json) => {
            let source = JsonStore::open(&from)
                .with_context(|| format!("Failed to open JsonStore at {}", from.display()))?;
            let target = JsonStore::create(&to)
                .with_context(|| format!("Failed to create JsonStore at {}", to.display()))?;
            copy_repository(&source, &target)?;
        }
    }
    output::serialize("repo copy", RepoCopyPayload { from, to })
}

fn infer_copy_store(path: &std::path::Path) -> StoreBackend {
    if path.extension().and_then(|ext| ext.to_str()) == Some("srsj") || path.is_file() {
        StoreBackend::Json
    } else {
        StoreBackend::File
    }
}

fn cmd_repo_validate(ctx: CliContext) -> Result<String> {
    let report = with_store(&ctx, |store| Ok(validate_repository(store)?))?;

    if report.is_ok() {
        output::serialize("repo validate", RepoValidatePayload::from(report))
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
