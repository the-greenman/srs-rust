use crate::commands::{with_store, CliContext, ViewCommand};
use crate::output;
use crate::payload::{ViewDeletePayload, ViewListPayload, ViewPayload};
use anyhow::Result;
use srs_core::types::view::View;
use srs_repository::view_service::{
    create_view, delete_view, get_view_by_id, list_views_summary, update_view, CreateViewResult,
    DeleteViewResult, GetViewResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: ViewCommand) -> Result<String> {
    match cmd {
        ViewCommand::List { namespace, type_id } => cmd_view_list(ctx, namespace, type_id),
        ViewCommand::Get { id } => cmd_view_get(ctx, id),
        ViewCommand::Create => cmd_view_create(ctx),
        ViewCommand::Update { id } => cmd_view_update(ctx, id),
        ViewCommand::Delete { id } => cmd_view_delete(ctx, id),
    }
}

fn cmd_view_list(
    ctx: CliContext,
    namespace: Option<String>,
    type_id: Option<String>,
) -> Result<String> {
    match with_store(&ctx, |store| Ok(list_views_summary(store)?)) {
        Ok(mut views) => {
            if let Some(ns) = namespace {
                views.retain(|s| s.namespace == ns);
            }
            if let Some(tid) = type_id {
                views.retain(|s| {
                    s.compatible_types
                        .as_ref()
                        .is_some_and(|types| types.iter().any(|t| t == &tid))
                });
            }
            output::serialize("view list", ViewListPayload { views })
        }
        Err(e) => Ok(output::err("view list", vec![e.to_string()])),
    }
}

fn cmd_view_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_view_by_id(store, &id)?))? {
        GetViewResult::Found(view) => output::serialize("view get", ViewPayload { view: *view }),
        GetViewResult::NotFound => Ok(output::err(
            "view get",
            vec![format!("view not found: {id}")],
        )),
    }
}

fn cmd_view_create(ctx: CliContext) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let view: View = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse View JSON: {e}"))?;
    match with_store(&ctx, |store| Ok(create_view(store, view)?)) {
        Ok(CreateViewResult { view }) => output::serialize("view create", ViewPayload { view }),
        Err(e) => Ok(output::err("view create", vec![e.to_string()])),
    }
}

fn cmd_view_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let view: View = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse View JSON: {e}"))?;
    match with_store(&ctx, |store| Ok(update_view(store, &id, view)?)) {
        Ok(result) => output::serialize("view update", ViewPayload { view: result.view }),
        Err(e) => Ok(output::err("view update", vec![e.to_string()])),
    }
}

fn cmd_view_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_view(store, &id)?)) {
        Ok(DeleteViewResult { id }) => output::serialize("view delete", ViewDeletePayload { id }),
        Err(e) => Ok(output::err("view delete", vec![e.to_string()])),
    }
}
