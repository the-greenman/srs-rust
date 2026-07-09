use crate::commands::{with_store, CliContext, RepoCommand, RepoExtensionsCommand, StoreBackend};
use crate::output;
use crate::payload::{
    InstancePathRename, RepoCopyPayload, RepoCreatePayload, RepoDiffInstanceAdded,
    RepoDiffInstanceModified, RepoDiffInstanceRemoved, RepoDiffInstances, RepoDiffManifest,
    RepoDiffPackage, RepoDiffPackageCategory, RepoDiffPackageItemAdded,
    RepoDiffPackageItemModified, RepoDiffPackageItemRemoved, RepoDiffPayload,
    RepoDiffRelationAdded, RepoDiffRelationModified, RepoDiffRelationRemoved, RepoDiffRelations,
    RepoDiffSummary, RepoExtensionsMutatePayload, RepoExtensionsPayload, RepoInitNewPayload,
    RepoMapPayload, RepoMigrateIdentityPayload, RepoNavigationPayload, RepoSetRootContainerPayload,
    RepoUpgradePayload, RepoValidatePayload,
};
use anyhow::{Context, Result};
use srs_repository::analysis::build_repo_map;
use srs_repository::diff::diff_repositories;
use srs_repository::manifest_service::{
    add_declared_extension, list_declared_extensions, remove_declared_extension,
    set_manifest_root_container, SetManifestRootContainerInput,
};
use srs_repository::migrate_identity_service;
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, init_new_repository, InitNewRepositoryInput,
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::repository_portability::copy_repository;
use srs_repository::upgrade_repository_paths;
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
        RepoCommand::Navigation => cmd_repo_navigation(ctx),
        RepoCommand::SetRootContainer {
            container_id,
            identity_instance_id,
        } => cmd_repo_set_root_container(ctx, container_id, identity_instance_id),
        RepoCommand::Copy {
            from,
            to,
            from_store,
            to_store,
        } => cmd_repo_copy(ctx, from, to, from_store, to_store),
        RepoCommand::Diff {
            from,
            to,
            from_store,
            to_store,
        } => cmd_repo_diff(ctx, from, to, from_store, to_store),
        RepoCommand::Validate { json: _ } => cmd_repo_validate(ctx),
        RepoCommand::Extensions(ext_cmd) => cmd_repo_extensions_dispatch(ctx, ext_cmd),
        RepoCommand::InitNew {
            repository_id,
            namespace,
            title,
            description,
        } => cmd_repo_init_new(ctx, repository_id, namespace, title, description),
        RepoCommand::Upgrade => cmd_repo_upgrade(ctx),
        RepoCommand::MigrateIdentity => cmd_repo_migrate_identity(ctx),
    }
}

fn cmd_repo_upgrade(ctx: CliContext) -> Result<String> {
    let store = match ctx.store {
        StoreBackend::File => FileStore::new(&ctx.repo),
        _ => {
            return Err(anyhow::anyhow!(
                "repo upgrade only supports file-backed repositories (--store file)"
            ))
        }
    };
    let result = upgrade_repository_paths(&store)?;
    let already_canonical_count = result.total_instances - result.renames.len();
    output::serialize(
        "repo upgrade",
        RepoUpgradePayload {
            already_canonical_count,
            renames: result
                .renames
                .into_iter()
                .map(|r| InstancePathRename {
                    instance_id: r.instance_id,
                    from_path: r.from_path,
                    to_path: r.to_path,
                })
                .collect(),
        },
    )
}

fn cmd_repo_migrate_identity(ctx: CliContext) -> Result<String> {
    let result = with_store(&ctx, |store| {
        migrate_identity_service::migrate_identity(store).map_err(anyhow::Error::from)
    })?;
    output::serialize(
        "repo migrate-identity",
        RepoMigrateIdentityPayload::from(result),
    )
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
            identity_instance_id: result.identity_instance_id,
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

fn cmd_repo_navigation(ctx: CliContext) -> Result<String> {
    let navigation = with_store(&ctx, |store| Ok(repository_navigation(store)?))?;
    output::serialize("repo navigation", RepoNavigationPayload { navigation })
}

fn cmd_repo_set_root_container(
    ctx: CliContext,
    container_id: String,
    identity_instance_id: String,
) -> Result<String> {
    let input = SetManifestRootContainerInput {
        container_id,
        identity_instance_id,
    };
    let result = with_store(&ctx, |store| Ok(set_manifest_root_container(store, input)?))?;
    output::serialize(
        "repo set-root-container",
        RepoSetRootContainerPayload {
            container_id: result.container_id,
            identity_instance_id: result.identity_instance_id,
        },
    )
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

fn cmd_repo_diff(
    _ctx: CliContext,
    from: std::path::PathBuf,
    to: std::path::PathBuf,
    from_store: Option<StoreBackend>,
    to_store: Option<StoreBackend>,
) -> Result<String> {
    let from_store = from_store.unwrap_or_else(|| infer_copy_store(&from));
    let to_store = to_store.unwrap_or_else(|| infer_copy_store(&to));

    let diff = match (from_store, to_store) {
        (StoreBackend::File, StoreBackend::File) => {
            let source = FileStore::new(&from);
            let target = FileStore::new(&to);
            diff_repositories(&source, &target)?
        }
        (StoreBackend::File, StoreBackend::Json) => {
            let source = FileStore::new(&from);
            let target = JsonStore::open(&to)
                .with_context(|| format!("Failed to open JsonStore at {}", to.display()))?;
            diff_repositories(&source, &target)?
        }
        (StoreBackend::Json, StoreBackend::File) => {
            let source = JsonStore::open(&from)
                .with_context(|| format!("Failed to open JsonStore at {}", from.display()))?;
            let target = FileStore::new(&to);
            diff_repositories(&source, &target)?
        }
        (StoreBackend::Json, StoreBackend::Json) => {
            let source = JsonStore::open(&from)
                .with_context(|| format!("Failed to open JsonStore at {}", from.display()))?;
            let target = JsonStore::open(&to)
                .with_context(|| format!("Failed to open JsonStore at {}", to.display()))?;
            diff_repositories(&source, &target)?
        }
    };

    output::serialize(
        "repo diff",
        RepoDiffPayload {
            from,
            to,
            summary: RepoDiffSummary {
                instances_added: diff.summary.instances_added,
                instances_removed: diff.summary.instances_removed,
                instances_modified: diff.summary.instances_modified,
                relations_added: diff.summary.relations_added,
                relations_removed: diff.summary.relations_removed,
                relations_modified: diff.summary.relations_modified,
                fields_added: diff.summary.fields_added,
                fields_removed: diff.summary.fields_removed,
                fields_modified: diff.summary.fields_modified,
                record_types_added: diff.summary.record_types_added,
                record_types_removed: diff.summary.record_types_removed,
                record_types_modified: diff.summary.record_types_modified,
                blueprints_added: diff.summary.blueprints_added,
                blueprints_removed: diff.summary.blueprints_removed,
                blueprints_modified: diff.summary.blueprints_modified,
                document_views_added: diff.summary.document_views_added,
                document_views_removed: diff.summary.document_views_removed,
                document_views_modified: diff.summary.document_views_modified,
            },
            manifest: RepoDiffManifest {
                namespace_changed: diff.manifest.namespace_changed,
                srs_version_changed: diff.manifest.srs_version_changed,
                extensions_added: diff.manifest.extensions_added,
                extensions_removed: diff.manifest.extensions_removed,
            },
            instances: RepoDiffInstances {
                added: diff
                    .instances
                    .added
                    .into_iter()
                    .map(|i| RepoDiffInstanceAdded {
                        instance_id: i.instance_id,
                        tier: i.tier,
                        value: i.value,
                    })
                    .collect(),
                removed: diff
                    .instances
                    .removed
                    .into_iter()
                    .map(|i| RepoDiffInstanceRemoved {
                        instance_id: i.instance_id,
                        tier: i.tier,
                        value: i.value,
                    })
                    .collect(),
                modified: diff
                    .instances
                    .modified
                    .into_iter()
                    .map(|i| RepoDiffInstanceModified {
                        instance_id: i.instance_id,
                        tier: i.tier,
                        from_value: i.from_value,
                        to_value: i.to_value,
                    })
                    .collect(),
            },
            relations: RepoDiffRelations {
                added: diff
                    .relations
                    .added
                    .into_iter()
                    .map(|r| RepoDiffRelationAdded {
                        relation_id: r.relation_id,
                        value: r.value,
                    })
                    .collect(),
                removed: diff
                    .relations
                    .removed
                    .into_iter()
                    .map(|r| RepoDiffRelationRemoved {
                        relation_id: r.relation_id,
                        value: r.value,
                    })
                    .collect(),
                modified: diff
                    .relations
                    .modified
                    .into_iter()
                    .map(|r| RepoDiffRelationModified {
                        relation_id: r.relation_id,
                        from_value: r.from_value,
                        to_value: r.to_value,
                    })
                    .collect(),
            },
            package: RepoDiffPackage {
                fields: map_pkg_category(diff.package.fields),
                record_types: map_pkg_category(diff.package.record_types),
                blueprints: map_pkg_category(diff.package.blueprints),
                document_views: map_pkg_category(diff.package.document_views),
            },
        },
    )
}

fn map_pkg_category(cat: srs_repository::diff::DiffPackageCategory) -> RepoDiffPackageCategory {
    RepoDiffPackageCategory {
        added: cat
            .added
            .into_iter()
            .map(|i| RepoDiffPackageItemAdded {
                id: i.id,
                namespace: i.namespace,
                name: i.name,
                version: i.version,
                value: i.value,
            })
            .collect(),
        removed: cat
            .removed
            .into_iter()
            .map(|i| RepoDiffPackageItemRemoved {
                id: i.id,
                namespace: i.namespace,
                name: i.name,
                version: i.version,
                value: i.value,
            })
            .collect(),
        modified: cat
            .modified
            .into_iter()
            .map(|i| RepoDiffPackageItemModified {
                id: i.id,
                namespace: i.namespace,
                name: i.name,
                from_value: i.from_value,
                to_value: i.to_value,
            })
            .collect(),
    }
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

fn cmd_repo_init_new(
    ctx: CliContext,
    repository_id: Option<String>,
    namespace: String,
    title: String,
    description: Option<String>,
) -> Result<String> {
    let input = InitNewRepositoryInput {
        repository_id,
        namespace,
        title,
        description,
    };
    let result = with_store(&ctx, |store| Ok(init_new_repository(store, input)?))?;
    output::serialize(
        "repo init-new",
        RepoInitNewPayload {
            repository_id: result.repository_id,
            namespace: result.namespace,
            package_id: result.package_id,
            package_version: result.package_version,
        },
    )
}
