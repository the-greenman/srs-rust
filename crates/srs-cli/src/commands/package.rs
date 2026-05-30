use crate::commands::{CliContext, PackageCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_repository::manifest_service::{add_package_ref, list_package_refs, remove_package_ref};
use srs_repository::FileStore;

pub fn dispatch(ctx: CliContext, cmd: PackageCommand) -> Result<String> {
    match cmd {
        PackageCommand::List => cmd_package_list(ctx),
        PackageCommand::Enable { path } => cmd_package_enable(ctx, path),
        PackageCommand::Disable { path } => cmd_package_disable(ctx, path),
    }
}

fn cmd_package_list(ctx: CliContext) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let refs = list_package_refs(&store)?;
    let packages: Vec<serde_json::Value> = refs
        .iter()
        .map(|r| json!({"mode": r.mode, "path": r.path}))
        .collect();
    Ok(output::ok("package list", json!({ "packages": packages })))
}

fn cmd_package_enable(ctx: CliContext, path: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let refs = add_package_ref(&store, &path)?;
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
    let store = FileStore::new(&ctx.repo);
    let refs = remove_package_ref(&store, &path)?;
    let packages: Vec<serde_json::Value> = refs
        .iter()
        .map(|r| json!({"mode": r.mode, "path": r.path}))
        .collect();
    Ok(output::ok(
        "package disable",
        json!({ "path": path, "packages": packages }),
    ))
}
