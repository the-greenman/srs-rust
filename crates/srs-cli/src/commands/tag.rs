use crate::commands::{with_store, CliContext, TagCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::tag_definition::TagDefinition;
use srs_repository::tag_service::{
    create_tag_definition_in_context, delete_tag_definition_in_context, get_tag_definition_by_id,
    list_tag_definitions_by_role, list_tag_definitions_filtered, update_tag_definition_validated,
    DeleteTagDefinitionResult, GetTagDefinitionResult, TagListFilter,
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
    let container_id = ctx.container_id.clone();
    let summaries = with_store(&ctx, |store| {
        Ok(if let Some(role_filter) = role {
            // Role filter: get by role first, then apply container filter manually
            let by_role = list_tag_definitions_by_role(store, &role_filter)?;
            if let Some(ref cid) = container_id {
                let all_filtered = list_tag_definitions_filtered(
                    store,
                    TagListFilter {
                        container_id: Some(cid.clone()),
                    },
                )?;
                let member_ids: std::collections::HashSet<_> =
                    all_filtered.iter().map(|s| s.instance_id.clone()).collect();
                by_role
                    .into_iter()
                    .filter(|s| member_ids.contains(&s.instance_id))
                    .collect()
            } else {
                by_role
            }
        } else {
            list_tag_definitions_filtered(
                store,
                TagListFilter {
                    container_id: container_id.clone(),
                },
            )?
        })
    })?;

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
    // Read JSON from stdin - expects a TagDefinition
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let tag_definition: TagDefinition = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse TagDefinition JSON: {}", e))?;

    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(create_tag_definition_in_context(
            store,
            tag_definition,
            container_id,
        )?)
    }) {
        Ok(result) => Ok(output::ok(
            "tag create",
            json!({ "tagDefinition": result.tag_definition }),
        )),
        Err(e) => Ok(output::err("tag create", vec![e.to_string()])),
    }
}

fn cmd_tag_update(ctx: CliContext, id: String) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let tag_definition: TagDefinition = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse TagDefinition JSON: {}", e))?;

    match with_store(&ctx, |store| {
        Ok(update_tag_definition_validated(store, &id, tag_definition)?)
    }) {
        Ok(result) => Ok(output::ok(
            "tag update",
            json!({ "tagDefinition": result.tag_definition }),
        )),
        Err(e) => Ok(output::err("tag update", vec![e.to_string()])),
    }
}

fn cmd_tag_delete(ctx: CliContext, id: String) -> Result<String> {
    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(delete_tag_definition_in_context(store, id, container_id)?)
    }) {
        Ok(DeleteTagDefinitionResult { instance_id }) => Ok(output::ok(
            "tag delete",
            json!({ "instanceId": instance_id }),
        )),
        Err(e) => Ok(output::err("tag delete", vec![e.to_string()])),
    }
}
