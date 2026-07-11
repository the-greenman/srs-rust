use crate::commands::{with_store, CliContext, PackageCommand};
use crate::output;
use crate::payload::{
    PackageCreatePayload, PackageImportPayload, PackageInstallConflictEntry,
    PackageInstallKindEntry, PackageInstallPayload, PackageListEntry, PackageListPayload,
    PackageRefEntry, PackageRefPayload, PackageUpdatePayload,
};
use anyhow::Result;
use srs_repository::manifest_service::{add_package_ref, remove_package_ref};
use srs_repository::package_install_service::{install_package, InstallPackageInput};
use srs_repository::package_service::{
    create_package, import_package_local, list_packages, update_package_metadata,
    CreatePackageInput, ImportPackageLocalInput, UpdatePackageMetadataInput,
};

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
        PackageCommand::Import { path } => cmd_package_import(ctx, path),
        PackageCommand::Install {
            source_dir,
            boundary,
            strict,
        } => cmd_package_install(ctx, source_dir, boundary, strict),
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

fn cmd_package_import(ctx: CliContext, path: String) -> Result<String> {
    let input = ImportPackageLocalInput {
        source_path: path.clone(),
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

fn cmd_package_install(
    ctx: CliContext,
    source_dir: String,
    boundary: Option<String>,
    strict: bool,
) -> Result<String> {
    let input = InstallPackageInput {
        source_dir,
        boundary_path: boundary,
        strict,
    };
    let result = with_store(&ctx, |store| Ok(install_package(store, input.clone())?))?;
    output::serialize(
        "package install",
        PackageInstallPayload {
            boundary_path: result.boundary_path,
            package_id: result.package_id,
            namespace: result.namespace,
            name: result.name,
            version: result.version,
            installed_at: result.installed_at,
            installed: result.installed,
            skipped_identical: result.skipped_identical,
            conflicts: result
                .conflicts
                .into_iter()
                .map(|c| PackageInstallConflictEntry {
                    kind: c.kind,
                    key: c.key,
                    source_id: c.source_id,
                    existing_id: c.existing_id,
                })
                .collect(),
            kinds: result
                .kinds
                .into_iter()
                .map(|k| PackageInstallKindEntry {
                    kind: k.kind,
                    installed: k.installed,
                    skipped_identical: k.skipped_identical,
                    conflicts: k.conflicts,
                })
                .collect(),
        },
    )
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
