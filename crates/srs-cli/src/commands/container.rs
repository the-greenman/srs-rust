use crate::commands::{
    CliContext, ContainerCommand, ContainerMembersCommand, ContainerRootsCommand,
};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::container::Container;
use srs_repository::container_service::{
    add_member, add_root, create_container, delete_container, get_container, list_containers,
    list_members, list_roots, remove_member, remove_root, update_container,
    validate_container_invariants, ContainerPatch,
};
use srs_repository::FileStore;
use std::io;

pub fn dispatch(ctx: CliContext, cmd: ContainerCommand) -> Result<String> {
    match cmd {
        ContainerCommand::List {
            container_type,
            member_instance_id,
            root_instance_id,
        } => cmd_list(ctx, container_type, member_instance_id, root_instance_id),
        ContainerCommand::Create => cmd_create(ctx),
        ContainerCommand::Get { container_id } => cmd_get(ctx, container_id),
        ContainerCommand::Update { container_id } => cmd_update(ctx, container_id),
        ContainerCommand::Delete { container_id } => cmd_delete(ctx, container_id),
        ContainerCommand::Members(sub) => dispatch_members(ctx, sub),
        ContainerCommand::Roots(sub) => dispatch_roots(ctx, sub),
        ContainerCommand::Validate { container_id } => cmd_validate(ctx, container_id),
    }
}

fn cmd_list(
    ctx: CliContext,
    container_type: Option<String>,
    member_instance_id: Option<String>,
    root_instance_id: Option<String>,
) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let containers = list_containers(
        &store,
        container_type.as_deref(),
        member_instance_id.as_deref(),
        root_instance_id.as_deref(),
    )?;
    Ok(output::ok(
        "container list",
        json!({ "containers": containers }),
    ))
}

fn cmd_create(ctx: CliContext) -> Result<String> {
    let container: Container = match serde_json::from_reader(io::stdin()) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "container create",
                vec![format!("Failed to parse JSON: {}", e)],
            ))
        }
    };
    let store = FileStore::new(&ctx.repo);
    match create_container(&store, container) {
        Ok(container) => Ok(output::ok(
            "container create",
            json!({ "container": container }),
        )),
        Err(e) => Ok(output::err("container create", vec![e.to_string()])),
    }
}

fn cmd_get(ctx: CliContext, container_id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match get_container(&store, &container_id) {
        Ok(container) => Ok(output::ok(
            "container get",
            json!({ "container": container }),
        )),
        Err(e) => Ok(output::err("container get", vec![e.to_string()])),
    }
}

fn cmd_update(ctx: CliContext, container_id: String) -> Result<String> {
    let patch: ContainerPatch = match serde_json::from_reader(io::stdin()) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "container update",
                vec![format!("Failed to parse JSON: {}", e)],
            ))
        }
    };
    let store = FileStore::new(&ctx.repo);
    match update_container(&store, &container_id, patch) {
        Ok(container) => Ok(output::ok(
            "container update",
            json!({ "container": container }),
        )),
        Err(e) => Ok(output::err("container update", vec![e.to_string()])),
    }
}

fn cmd_delete(ctx: CliContext, container_id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match delete_container(&store, &container_id) {
        Ok(id) => Ok(output::ok("container delete", json!({ "containerId": id }))),
        Err(e) => Ok(output::err("container delete", vec![e.to_string()])),
    }
}

fn dispatch_members(ctx: CliContext, cmd: ContainerMembersCommand) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match cmd {
        ContainerMembersCommand::List { container_id } => {
            let ids = list_members(&store, &container_id)?;
            Ok(output::ok(
                "container members list",
                json!({ "containerId": container_id, "memberInstanceIds": ids }),
            ))
        }
        ContainerMembersCommand::Add {
            container_id,
            instance_id,
        } => {
            let ids = add_member(&store, &container_id, &instance_id)?;
            Ok(output::ok(
                "container members add",
                json!({ "containerId": container_id, "instanceId": instance_id, "memberInstanceIds": ids }),
            ))
        }
        ContainerMembersCommand::Remove {
            container_id,
            instance_id,
        } => {
            let ids = remove_member(&store, &container_id, &instance_id)?;
            Ok(output::ok(
                "container members remove",
                json!({ "containerId": container_id, "instanceId": instance_id, "memberInstanceIds": ids }),
            ))
        }
    }
}

fn dispatch_roots(ctx: CliContext, cmd: ContainerRootsCommand) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    match cmd {
        ContainerRootsCommand::List { container_id } => {
            let ids = list_roots(&store, &container_id)?;
            Ok(output::ok(
                "container roots list",
                json!({ "containerId": container_id, "rootInstanceIds": ids }),
            ))
        }
        ContainerRootsCommand::Add {
            container_id,
            instance_id,
        } => {
            let ids = add_root(&store, &container_id, &instance_id)?;
            Ok(output::ok(
                "container roots add",
                json!({ "containerId": container_id, "instanceId": instance_id, "rootInstanceIds": ids }),
            ))
        }
        ContainerRootsCommand::Remove {
            container_id,
            instance_id,
        } => {
            let ids = remove_root(&store, &container_id, &instance_id)?;
            Ok(output::ok(
                "container roots remove",
                json!({ "containerId": container_id, "instanceId": instance_id, "rootInstanceIds": ids }),
            ))
        }
    }
}

fn cmd_validate(ctx: CliContext, container_id: String) -> Result<String> {
    let store = FileStore::new(&ctx.repo);
    let report = validate_container_invariants(&store, &container_id)?;
    if report.ok {
        Ok(output::ok(
            "container validate",
            json!({ "ok": true, "errors": [] }),
        ))
    } else {
        Ok(output::err("container validate", report.errors))
    }
}
