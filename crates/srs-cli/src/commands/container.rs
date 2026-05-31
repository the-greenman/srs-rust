use crate::commands::{
    with_store, CliContext, ContainerCommand, ContainerMembersCommand, ContainerRootsCommand,
};
use crate::output;
use crate::payload::{
    ContainerDeletePayload, ContainerListPayload, ContainerMembersMutatePayload,
    ContainerMembersPayload, ContainerPayload, ContainerRootsMutatePayload, ContainerRootsPayload,
    ContainerValidatePayload,
};
use anyhow::Result;
use srs_core::types::container::Container;
use srs_repository::container_service::{
    add_container_member, add_root, create_container, delete_container, get_container,
    list_container_members, list_containers, list_roots, remove_container_member, remove_root,
    update_container, validate_container_invariants, ContainerPatch,
};
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
    let containers = with_store(&ctx, |store| {
        Ok(list_containers(
            store,
            container_type.as_deref(),
            member_instance_id.as_deref(),
            root_instance_id.as_deref(),
        )?)
    })?;
    output::serialize("container list", ContainerListPayload { containers })
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
    match with_store(&ctx, |store| Ok(create_container(store, container)?)) {
        Ok(container) => output::serialize("container create", ContainerPayload { container }),
        Err(e) => Ok(output::err("container create", vec![e.to_string()])),
    }
}

fn cmd_get(ctx: CliContext, container_id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_container(store, &container_id)?)) {
        Ok(container) => output::serialize("container get", ContainerPayload { container }),
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
    match with_store(&ctx, |store| {
        Ok(update_container(store, &container_id, patch)?)
    }) {
        Ok(container) => output::serialize("container update", ContainerPayload { container }),
        Err(e) => Ok(output::err("container update", vec![e.to_string()])),
    }
}

fn cmd_delete(ctx: CliContext, container_id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_container(store, &container_id)?)) {
        Ok(id) => output::serialize(
            "container delete",
            ContainerDeletePayload { container_id: id },
        ),
        Err(e) => Ok(output::err("container delete", vec![e.to_string()])),
    }
}

fn dispatch_members(ctx: CliContext, cmd: ContainerMembersCommand) -> Result<String> {
    match cmd {
        ContainerMembersCommand::List { container_id } => {
            let member_instance_ids = with_store(&ctx, |store| {
                Ok(list_container_members(store, &container_id)?)
            })?;
            output::serialize(
                "container members list",
                ContainerMembersPayload {
                    container_id,
                    member_instance_ids,
                },
            )
        }
        ContainerMembersCommand::Add {
            container_id,
            instance_id,
        } => {
            let member_instance_ids = with_store(&ctx, |store| {
                Ok(add_container_member(store, &container_id, &instance_id)?)
            })?;
            output::serialize(
                "container members add",
                ContainerMembersMutatePayload {
                    container_id,
                    instance_id,
                    member_instance_ids,
                },
            )
        }
        ContainerMembersCommand::Remove {
            container_id,
            instance_id,
        } => {
            let member_instance_ids = with_store(&ctx, |store| {
                Ok(remove_container_member(store, &container_id, &instance_id)?)
            })?;
            output::serialize(
                "container members remove",
                ContainerMembersMutatePayload {
                    container_id,
                    instance_id,
                    member_instance_ids,
                },
            )
        }
    }
}

fn dispatch_roots(ctx: CliContext, cmd: ContainerRootsCommand) -> Result<String> {
    match cmd {
        ContainerRootsCommand::List { container_id } => {
            let root_instance_ids =
                with_store(&ctx, |store| Ok(list_roots(store, &container_id)?))?;
            output::serialize(
                "container roots list",
                ContainerRootsPayload {
                    container_id,
                    root_instance_ids,
                },
            )
        }
        ContainerRootsCommand::Add {
            container_id,
            instance_id,
        } => {
            let root_instance_ids = with_store(&ctx, |store| {
                Ok(add_root(store, &container_id, &instance_id)?)
            })?;
            output::serialize(
                "container roots add",
                ContainerRootsMutatePayload {
                    container_id,
                    instance_id,
                    root_instance_ids,
                },
            )
        }
        ContainerRootsCommand::Remove {
            container_id,
            instance_id,
        } => {
            let root_instance_ids = with_store(&ctx, |store| {
                Ok(remove_root(store, &container_id, &instance_id)?)
            })?;
            output::serialize(
                "container roots remove",
                ContainerRootsMutatePayload {
                    container_id,
                    instance_id,
                    root_instance_ids,
                },
            )
        }
    }
}

fn cmd_validate(ctx: CliContext, container_id: String) -> Result<String> {
    let report = with_store(&ctx, |store| {
        Ok(validate_container_invariants(store, &container_id)?)
    })?;
    if report.ok {
        output::serialize(
            "container validate",
            ContainerValidatePayload {
                ok: true,
                errors: vec![],
            },
        )
    } else {
        Ok(output::err("container validate", report.errors))
    }
}
