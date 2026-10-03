use crate::commands::{with_store, CliContext, PackageCommand, PackageDependencyCommand};
use crate::output;
use crate::payload::{
    PackageCreatePayload, PackageDependenciesPayload, PackageExportPayload, PackageImportPayload,
    PackageImportsPayload, PackageInstallPayload, PackageListEntry, PackageListPayload,
    PackageRefEntry, PackageRefPayload, PackageUpdatePayload,
};
use anyhow::{Context, Result};
use srs_core::extensions::import_tracking::ImportMode;
use srs_repository::manifest_service::{add_package_ref, remove_package_ref};
use srs_repository::package_bundle::{export_package_bundle, ExportPackageInput};
use srs_repository::package_dependency_service::{
    add_package_dependency, check_bundle, list_package_dependencies, remove_package_dependency,
    AddPackageDependencyInput, BundleRequirements, RemovePackageDependencyInput,
};
use srs_repository::package_install_service::{
    install_package, install_package_bundle_bytes, InstallBundleOptions, InstallPackageInput,
};
use srs_repository::package_service::{
    create_package, import_package_local, list_package_imports, list_packages,
    update_package_metadata, CreatePackageInput, ImportPackageLocalInput, ListPackageImportsFilter,
    UpdatePackageMetadataInput,
};
use std::path::{Path, PathBuf};

pub fn dispatch(ctx: CliContext, cmd: PackageCommand) -> Result<String> {
    match cmd {
        PackageCommand::List => cmd_package_list(ctx),
        PackageCommand::Create {
            id,
            namespace,
            name,
            version,
            boundary_path,
        } => cmd_package_create(ctx, id, namespace, name, version, boundary_path),
        PackageCommand::Import { path, mode } => cmd_package_import(ctx, path, mode),
        PackageCommand::Install {
            source_dir,
            bundle,
            boundary,
            strict,
        } => cmd_package_install(ctx, source_dir, bundle, boundary, strict),
        PackageCommand::Export {
            selector,
            output,
            published_at,
            publisher,
            homepage,
            mode,
        } => cmd_package_export(
            ctx,
            output,
            ExportPackageInput {
                selector,
                published_at,
                publisher,
                homepage,
                mode: mode.parse().map_err(|e: String| anyhow::anyhow!(e))?,
            },
        ),
        PackageCommand::Update {
            selector,
            namespace,
            name,
            version,
        } => cmd_package_update(ctx, selector, namespace, name, version),
        PackageCommand::SliceCreate {
            id,
            namespace,
            name,
            version,
            boundary_path,
        } => cmd_package_create(ctx, id, namespace, name, version, boundary_path),
        PackageCommand::Imports => cmd_package_imports(ctx),
        PackageCommand::Dependency(sub) => cmd_package_dependency(ctx, sub),
        PackageCommand::Enable { path } => cmd_package_enable(ctx, path),
        PackageCommand::Disable { path } => cmd_package_disable(ctx, path),
    }
}

fn cmd_package_list(ctx: CliContext) -> Result<String> {
    let raw = with_store(&ctx, |store| Ok(list_packages(store)?))?;
    let packages = raw
        .into_iter()
        .map(|p| PackageListEntry {
            id: p.id,
            namespace: p.namespace,
            name: p.name,
            version: p.version,
            boundary_path: p.boundary_path,
            field_count: p.field_count,
            type_count: p.type_count,
        })
        .collect();
    output::serialize("package list", PackageListPayload { packages })
}

fn cmd_package_create(
    ctx: CliContext,
    id: String,
    namespace: String,
    name: String,
    version: String,
    boundary_path: String,
) -> Result<String> {
    let input = CreatePackageInput {
        id: id.clone(),
        namespace,
        name,
        version,
        boundary_path: Some(boundary_path),
    };
    let result = with_store(&ctx, |store| Ok(create_package(store, input.clone())?))?;
    output::serialize(
        "package create",
        PackageCreatePayload {
            id: result.id,
            boundary_path: result.boundary_path,
        },
    )
}

fn cmd_package_import(ctx: CliContext, path: String, mode: String) -> Result<String> {
    let import_mode = ImportMode::try_from(mode.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let input = ImportPackageLocalInput {
        source_path: path.clone(),
        mode: import_mode,
    };
    let result = with_store(&ctx, |store| {
        Ok(import_package_local(store, input.clone())?)
    })?;
    output::serialize(
        "package import",
        PackageImportPayload {
            selector: result.selector,
            id: result.id,
            namespace: result.namespace,
            name: result.name,
        },
    )
}

fn cmd_package_export(
    ctx: CliContext,
    out_path: PathBuf,
    input: ExportPackageInput,
) -> Result<String> {
    let export = with_store(&ctx, |s| Ok(export_package_bundle(s, input.clone())?))?;
    std::fs::write(&out_path, &export.text)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", out_path.display()))?;
    let path = out_path.to_string_lossy().into_owned();
    output::serialize(
        "package export",
        PackageExportPayload::new(path, export.summary),
    )
}

fn cmd_package_install(
    ctx: CliContext,
    source_dir: Option<String>,
    bundle: Option<PathBuf>,
    boundary_path: Option<String>,
    strict: bool,
) -> Result<String> {
    let result = match bundle {
        Some(path) => {
            let bytes = read_bundle_file(&path)?;
            let opts = InstallBundleOptions {
                boundary_path,
                strict,
            };
            with_store(&ctx, |s| {
                Ok(install_package_bundle_bytes(s, &bytes, opts.clone())?)
            })?
        }
        None => {
            let source_dir = source_dir.context("give <source_dir> or --bundle")?; // clap enforces
            let input = InstallPackageInput {
                source_dir,
                boundary_path,
                strict,
            };
            with_store(&ctx, |s| Ok(install_package(s, input.clone())?))?
        }
    };
    output::serialize("package install", PackageInstallPayload::from(result))
}

/// File I/O only: read the .srspkg bytes (no parsing, no logic).
fn read_bundle_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))
}

fn cmd_package_update(
    ctx: CliContext,
    selector: Option<String>,
    namespace: Option<String>,
    name: Option<String>,
    version: Option<String>,
) -> Result<String> {
    let input = UpdatePackageMetadataInput {
        namespace,
        name,
        version,
    };
    let result = with_store(&ctx, |store| {
        Ok(update_package_metadata(
            store,
            selector.clone(),
            input.clone(),
        )?)
    })?;
    let b = &result.boundary;
    output::serialize(
        "package update",
        PackageUpdatePayload {
            selector: b.selector.clone(),
            id: b.id.clone(),
            namespace: b.namespace.clone(),
            name: b.name.clone(),
            version: b.version.clone(),
        },
    )
}

fn cmd_package_enable(ctx: CliContext, path: String) -> Result<String> {
    let refs = with_store(&ctx, |store| Ok(add_package_ref(store, &path)?))?;
    let packages = refs
        .iter()
        .map(|r| PackageRefEntry {
            mode: r.mode.clone(),
            path: r.path.clone(),
        })
        .collect();
    output::serialize("package enable", PackageRefPayload { path, packages })
}

fn cmd_package_disable(ctx: CliContext, path: String) -> Result<String> {
    let refs = with_store(&ctx, |store| Ok(remove_package_ref(store, &path)?))?;
    let packages = refs
        .iter()
        .map(|r| PackageRefEntry {
            mode: r.mode.clone(),
            path: r.path.clone(),
        })
        .collect();
    output::serialize("package disable", PackageRefPayload { path, packages })
}

fn cmd_package_imports(ctx: CliContext) -> Result<String> {
    let result = with_store(&ctx, |store| {
        Ok(list_package_imports(
            store,
            ListPackageImportsFilter::default(),
        )?)
    })?;
    output::serialize("package imports", PackageImportsPayload::from(result))
}

fn cmd_package_dependency(ctx: CliContext, cmd: PackageDependencyCommand) -> Result<String> {
    let (command, result) = match cmd {
        PackageDependencyCommand::List { selector } => (
            "package dependency list",
            with_store(&ctx, |store| {
                Ok(list_package_dependencies(store, selector.clone())?)
            })?,
        ),
        PackageDependencyCommand::Add {
            selector,
            package_id,
            version,
            repair_legacy,
        } => {
            let input = AddPackageDependencyInput {
                selector,
                package_id,
                version,
                repair_legacy,
            };
            (
                "package dependency add",
                with_store(&ctx, |store| {
                    Ok(add_package_dependency(store, input.clone())?)
                })?,
            )
        }
        PackageDependencyCommand::Check => {
            let bundle: BundleRequirements = crate::input::from_stdin("package requirements")?;
            (
                "package dependency check",
                with_store(&ctx, |store| Ok(check_bundle(store, &bundle)?))?,
            )
        }
        PackageDependencyCommand::Remove {
            selector,
            package_id,
        } => {
            let input = RemovePackageDependencyInput {
                selector,
                package_id,
            };
            (
                "package dependency remove",
                with_store(&ctx, |store| {
                    Ok(remove_package_dependency(store, input.clone())?)
                })?,
            )
        }
    };
    output::serialize(command, PackageDependenciesPayload::from(result))
}
