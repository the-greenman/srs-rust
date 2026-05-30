use crate::commands::{with_store, CliContext, TagCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::tag_definition::TagDefinition;
use srs_repository::container_service::{
    add_member, get_container, is_member, list_members, remove_member,
};
use srs_repository::error::RepositoryError;
use srs_repository::tag_service::{
    create_tag_definition, delete_tag_definition, get_tag_definition_by_id, list_tag_definitions,
    list_tag_definitions_by_role, update_tag_definition, DeleteTagDefinitionResult,
    GetTagDefinitionResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: TagCommand) -> Result<String> {
    match cmd {
        TagCommand::List { role, json: _ } => cmd_tag_list(ctx, role),
        TagCommand::Get { id, json: _ } => cmd_tag_get(ctx, id),
        TagCommand::Create { json: _ } => cmd_tag_create(ctx),
        TagCommand::Update { id, json: _ } => cmd_tag_update(ctx, id),
        TagCommand::Delete { id, json: _ } => cmd_tag_delete(ctx, id),
    }
}

fn cmd_tag_list(ctx: CliContext, role: Option<String>) -> Result<String> {
    let mut summaries = with_store(&ctx, |store| {
        Ok(if let Some(role_filter) = role {
            list_tag_definitions_by_role(store, &role_filter)?
        } else {
            list_tag_definitions(store)?
        })
    })?;

    if let Some(ref cid) = ctx.container_id {
        let members = with_store(&ctx, |store| Ok(list_members(store, cid)?))?;
        summaries.retain(|s| members.iter().any(|id| id == &s.instance_id));
    }

    Ok(output::ok(
        "tag list",
        json!({ "tagDefinitions": summaries }),
    ))
}

fn cmd_tag_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_tag_definition_by_id(store, &id)?))? {
        GetTagDefinitionResult::Found(td) => {
            Ok(output::ok("tag get", json!({ "tagDefinition": *td })))
        }
        GetTagDefinitionResult::NotFound => Ok(output::err(
            "tag get",
            vec![format!("TagDefinition with id '{}' not found", id)],
        )),
    }
}

fn cmd_tag_create(ctx: CliContext) -> Result<String> {
    if let Some(ref cid) = ctx.container_id {
        match with_store(&ctx, |store| Ok(get_container(store, cid)?)) {
            Ok(_) => {}
            Err(e) => {
                if let Some(RepositoryError::ContainerNotFound { .. }) =
                    e.downcast_ref::<RepositoryError>()
                {
                    return Ok(output::err(
                        "tag create",
                        vec![format!("Container '{}' not found — no tag written", cid)],
                    ));
                }
                return Err(e);
            }
        }
    }

    // Read JSON from stdin - expects a TagDefinition
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let tag_definition: TagDefinition = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse TagDefinition JSON: {}", e))?;

    // Create the TagDefinition via the dedicated service
    let result = with_store(&ctx, |store| {
        Ok(create_tag_definition(store, tag_definition)?)
    })?;

    if let Some(ref cid) = ctx.container_id {
        if let Err(e) = with_store(&ctx, |store| {
            Ok(add_member(store, cid, &result.tag_definition.instance_id)?)
        }) {
            return Ok(output::err(
                "tag create",
                vec![format!("Tag created but failed to add to container: {}", e)],
            ));
        }
    }

    Ok(output::ok(
        "tag create",
        json!({ "tagDefinition": result.tag_definition }),
    ))
}

fn cmd_tag_update(ctx: CliContext, id: String) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let tag_definition: TagDefinition = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse TagDefinition JSON: {}", e))?;

    // Ensure the ID matches
    if tag_definition.instance_id != id {
        return Ok(output::err(
            "tag update",
            vec![format!(
                "Tag ID in JSON ({}) does not match command argument ({})",
                tag_definition.instance_id, id
            )],
        ));
    }

    // Update the TagDefinition
    let result = with_store(&ctx, |store| {
        Ok(update_tag_definition(store, tag_definition)?)
    })?;

    Ok(output::ok(
        "tag update",
        json!({ "tagDefinition": result.tag_definition }),
    ))
}

fn cmd_tag_delete(ctx: CliContext, id: String) -> Result<String> {
    if let Some(ref cid) = ctx.container_id {
        if !with_store(&ctx, |store| Ok(is_member(store, cid, &id)?))? {
            return Ok(output::err(
                "tag delete",
                vec![format!(
                    "Instance '{}' is not a member of container '{}' — delete refused",
                    id, cid
                )],
            ));
        }
        with_store(&ctx, |store| Ok(remove_member(store, cid, &id)?))?;
    }

    match with_store(&ctx, |store| Ok(delete_tag_definition(store, &id)?)) {
        Ok(DeleteTagDefinitionResult { instance_id }) => Ok(output::ok(
            "tag delete",
            json!({ "instanceId": instance_id }),
        )),
        Err(e) => Ok(output::err("tag delete", vec![e.to_string()])),
    }
}
