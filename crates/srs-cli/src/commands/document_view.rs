use crate::commands::{with_store, CliContext, DocumentViewCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::view::DocumentView;
use srs_repository::view_service::{
    create_document_view, delete_document_view, get_document_view_by_id,
    list_document_views_summary, update_document_view, CreateDocumentViewResult,
    DeleteDocumentViewResult, GetDocumentViewResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: DocumentViewCommand) -> Result<String> {
    match cmd {
        DocumentViewCommand::List {
            namespace,
            container_type,
        } => cmd_document_view_list(ctx, namespace, container_type),
        DocumentViewCommand::Get { id } => cmd_document_view_get(ctx, id),
        DocumentViewCommand::Create => cmd_document_view_create(ctx),
        DocumentViewCommand::Update { id } => cmd_document_view_update(ctx, id),
        DocumentViewCommand::Delete { id } => cmd_document_view_delete(ctx, id),
    }
}

fn cmd_document_view_list(
    ctx: CliContext,
    namespace: Option<String>,
    container_type: Option<String>,
) -> Result<String> {
    match with_store(&ctx, |store| Ok(list_document_views_summary(store)?)) {
        Ok(mut summaries) => {
            if let Some(ns) = namespace {
                summaries.retain(|s| s.namespace == ns);
            }
            if let Some(ct) = container_type {
                summaries.retain(|s| s.container_type.as_deref() == Some(ct.as_str()));
            }
            Ok(output::ok(
                "document-view list",
                json!({ "documentViews": summaries }),
            ))
        }
        Err(e) => Ok(output::err("document-view list", vec![e.to_string()])),
    }
}

fn cmd_document_view_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_document_view_by_id(store, &id)?))? {
        GetDocumentViewResult::Found(dv) => Ok(output::ok(
            "document-view get",
            json!({ "documentView": *dv }),
        )),
        GetDocumentViewResult::NotFound => Ok(output::err(
            "document-view get",
            vec![format!("document view not found: {id}")],
        )),
    }
}

fn cmd_document_view_create(ctx: CliContext) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let dv: DocumentView = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse DocumentView JSON: {e}"))?;
    match with_store(&ctx, |store| Ok(create_document_view(store, dv)?)) {
        Ok(CreateDocumentViewResult { document_view }) => Ok(output::ok(
            "document-view create",
            json!({ "documentView": document_view }),
        )),
        Err(e) => Ok(output::err("document-view create", vec![e.to_string()])),
    }
}

fn cmd_document_view_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let dv: DocumentView = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse DocumentView JSON: {e}"))?;
    match with_store(&ctx, |store| Ok(update_document_view(store, &id, dv)?)) {
        Ok(result) => Ok(output::ok(
            "document-view update",
            json!({ "documentView": result.document_view }),
        )),
        Err(e) => Ok(output::err("document-view update", vec![e.to_string()])),
    }
}

fn cmd_document_view_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_document_view(store, &id)?)) {
        Ok(DeleteDocumentViewResult { id }) => {
            Ok(output::ok("document-view delete", json!({ "id": id })))
        }
        Err(e) => Ok(output::err("document-view delete", vec![e.to_string()])),
    }
}
