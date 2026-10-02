use crate::commands::{with_store, CliContext, ContainerCommand, ContainerMembersCommand};
use crate::output;
use crate::payload::{
    ContainerDeletePayload, ContainerListPayload, ContainerMembersMutatePayload,
    ContainerMembersOutlinePayload, ContainerMembersPayload, ContainerPayload,
    ContainerValidatePayload, ContainerViewPayload,
};
use anyhow::Result;
use srs_core::arrangement::RelativeMove;
use srs_repository::container_service::{
    add_member, add_member_relative, create_container, delete_container, get_arrangement,
    get_container, get_outline, list_containers, move_member, move_member_relative, remove_member,
    repair_members, update_container, validate_container_invariants, ArrangementResult,
    ContainerCreateInput, ContainerListFilter, ContainerPatch,
};
use srs_repository::container_view_service::{resolve_container_view, ResolveContainerViewInput};
use srs_repository::error::RepositoryError;

pub fn dispatch(ctx: CliContext, cmd: ContainerCommand) -> Result<String> {
    match cmd {
        ContainerCommand::List {
            container_type,
            member_instance_id,
            anchor_instance_id,
        } => cmd_list(ctx, container_type, member_instance_id, anchor_instance_id),
        ContainerCommand::Create => cmd_create(ctx),
        ContainerCommand::Get { container_id } => cmd_get(ctx, container_id),
        ContainerCommand::Update { container_id } => cmd_update(ctx, container_id),
        ContainerCommand::Delete { container_id } => cmd_delete(ctx, container_id),
        ContainerCommand::Members(sub) => dispatch_members(ctx, sub),
        ContainerCommand::Validate { container_id } => cmd_validate(ctx, container_id),
        ContainerCommand::ResolveView {
            container_id,
            view_id,
        } => cmd_resolve_view(ctx, container_id, view_id),
    }
}

fn cmd_resolve_view(
    ctx: CliContext,
    container_id: String,
    view_id: Option<String>,
) -> Result<String> {
    let input = ResolveContainerViewInput {
        container_id,
        view_id,
    };
    match with_store(&ctx, |store| Ok(resolve_container_view(store, input)?)) {
        Ok(container_view) => output::serialize(
            "container resolve-view",
            ContainerViewPayload { container_view },
        ),
        Err(e) => Ok(output::err("container resolve-view", vec![e.to_string()])),
    }
}

fn cmd_list(
    ctx: CliContext,
    container_type: Option<String>,
    member_instance_id: Option<String>,
    anchor_instance_id: Option<String>,
) -> Result<String> {
    let filter = ContainerListFilter {
        container_type,
        member_instance_id,
        anchor_instance_id,
    };
    let containers = with_store(&ctx, |store| Ok(list_containers(store, &filter)?))?;
    output::serialize("container list", ContainerListPayload { containers })
}

fn cmd_create(ctx: CliContext) -> Result<String> {
    let input: ContainerCreateInput = match crate::input::from_stdin("container") {
        Ok(v) => v,
        Err(e) => return Ok(output::err("container create", vec![e.to_string()])),
    };
    match with_store(&ctx, |store| Ok(create_container(store, input.into())?)) {
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
    let patch: ContainerPatch = match crate::input::from_stdin("container patch") {
        Ok(v) => v,
        Err(e) => return Ok(output::err("container update", vec![e.to_string()])),
    };
    match with_store(&ctx, |store| {
        Ok(update_container(store, &container_id, patch)?)
    }) {
        Ok(result) => output::serialize_with_diagnostics(
            "container update",
            ContainerPayload {
                container: result.container,
            },
            result.diagnostics,
        ),
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

fn mutate_payload(
    container_id: String,
    instance_id: Option<String>,
    r: ArrangementResult,
) -> ContainerMembersMutatePayload {
    ContainerMembersMutatePayload {
        container_id,
        instance_id,
        members: r.members,
        promoted: r.promoted,
        removed: r.removed,
    }
}

/// Fold the `--before/--after/--into <ID>` flags into the `(relativeTo, placement)` pair.
fn relative_flags(
    before: Option<String>,
    after: Option<String>,
    into: Option<String>,
) -> (Option<String>, Option<&'static str>) {
    match (before, after, into) {
        (Some(t), _, _) => (Some(t), Some("before")),
        (_, Some(t), _) => (Some(t), Some("after")),
        (_, _, Some(t)) => (Some(t), Some("into")),
        _ => (None, None),
    }
}

fn dispatch_members(ctx: CliContext, cmd: ContainerMembersCommand) -> Result<String> {
    match cmd {
        ContainerMembersCommand::List { container_id } => {
            let members = with_store(&ctx, |store| Ok(get_arrangement(store, &container_id)?))?;
            output::serialize(
                "container members list",
                ContainerMembersPayload {
                    container_id,
                    members,
                },
            )
        }
        ContainerMembersCommand::Outline { container_id } => {
            let o = with_store(&ctx, |store| Ok(get_outline(store, &container_id)?))?;
            output::serialize(
                "container members outline",
                ContainerMembersOutlinePayload {
                    container_id: o.container_id,
                    anchor_instance_id: o.anchor_instance_id,
                    identity_instance_id: o.identity_instance_id,
                    entries: o.entries,
                    body: o.body,
                },
            )
        }
        ContainerMembersCommand::Add {
            container_id,
            instance_id,
            position,
            depth,
            before,
            after,
            into,
        } => {
            let (rel, placement) = relative_flags(before, after, into);
            let r = with_store(&ctx, |store| {
                match RelativeMove::parse(rel.as_deref(), placement, None)
                    .map_err(|message| RepositoryError::InvalidInput { message })?
                {
                    Some(RelativeMove::Place { target, placement }) => Ok(add_member_relative(
                        store,
                        &container_id,
                        &instance_id,
                        &target,
                        placement,
                    )?),
                    _ => Ok(add_member(
                        store,
                        &container_id,
                        &instance_id,
                        position,
                        depth,
                    )?),
                }
            })?;
            output::serialize(
                "container members add",
                mutate_payload(container_id, Some(instance_id), r),
            )
        }
        ContainerMembersCommand::Remove {
            container_id,
            instance_id,
        } => {
            let r = with_store(&ctx, |store| {
                Ok(remove_member(store, &container_id, &instance_id)?)
            })?;
            output::serialize(
                "container members remove",
                mutate_payload(container_id, Some(instance_id), r),
            )
        }
        ContainerMembersCommand::Move {
            container_id,
            instance_id,
            position,
            depth,
            before,
            after,
            into,
            indent,
            outdent,
            up,
            down,
        } => {
            let (rel, placement) = relative_flags(before, after, into);
            let shift = [
                (indent, "indent"),
                (outdent, "outdent"),
                (up, "up"),
                (down, "down"),
            ]
            .into_iter()
            .find_map(|(on, name)| on.then_some(name));
            let r = with_store(&ctx, |store| {
                match RelativeMove::parse(rel.as_deref(), placement, shift)
                    .map_err(|message| RepositoryError::InvalidInput { message })?
                {
                    Some(mv) => Ok(move_member_relative(
                        store,
                        &container_id,
                        &instance_id,
                        &mv,
                    )?),
                    None => Ok(move_member(
                        store,
                        &container_id,
                        &instance_id,
                        position,
                        depth,
                    )?),
                }
            })?;
            output::serialize(
                "container members move",
                mutate_payload(container_id, Some(instance_id), r),
            )
        }
        ContainerMembersCommand::Repair { container_id } => {
            let r = with_store(&ctx, |store| Ok(repair_members(store, &container_id)?))?;
            output::serialize(
                "container members repair",
                mutate_payload(container_id, None, r),
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
