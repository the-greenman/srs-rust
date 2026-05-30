use crate::commands::{with_store, CliContext, PackageCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::manifest_service::{add_package_ref, remove_package_ref};
use srs_repository::package_service::{create_package, list_packages, CreatePackageInput};

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
        PackageCommand::Enable { path } => cmd_package_enable(ctx, path),
        PackageCommand::Disable { path } => cmd_package_disable(ctx, path),
    }
}

fn cmd_package_list(ctx: CliContext) -> Result<String> {
    let packages = with_store(&ctx, |store| Ok(list_packages(store)?))?;
    let packages: Vec<serde_json::Value> = packages
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id,
                "namespace": p.namespace,
                "name": p.name,
                "version": p.version,
                "boundaryPath": p.boundary_path,
                "fieldCount": p.field_count,
                "typeCount": p.type_count,
            })
        })
        .collect();
    Ok(output::ok("package list", json!({ "packages": packages })))
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
    Ok(output::ok(
        "package create",
        json!({ "id": result.id, "boundaryPath": result.boundary_path }),
    ))
}

fn cmd_package_enable(ctx: CliContext, path: String) -> Result<String> {
    let refs = with_store(&ctx, |store| Ok(add_package_ref(store, &path)?))?;
    let packages: Vec<serde_json::Value> = refs
        .iter()
        .map(|r| json!({"mode": r.mode, "path": r.path}))
        .collect();
    Ok(output::ok(
        "package enable",
        json!({ "path": path, "packages": packages }),
    ))
}

fn cmd_package_disable(ctx: CliContext, path: String) -> Result<String> {
    let refs = with_store(&ctx, |store| Ok(remove_package_ref(store, &path)?))?;
    let packages: Vec<serde_json::Value> = refs
        .iter()
        .map(|r| json!({"mode": r.mode, "path": r.path}))
        .collect();
    Ok(output::ok(
        "package disable",
        json!({ "path": path, "packages": packages }),
    ))
}
