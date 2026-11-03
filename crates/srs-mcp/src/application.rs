//! Native rmcp adapter over the transport-agnostic [`SrsMcpApplication`].
//!
//! All SRS semantics (resources, prompts, tools, schemas) live in
//! `srs-mcp-core`. This module only translates between rmcp's typed models and
//! the core's JSON-native results (ADR-037), so the stdio server and the
//! browser dispatcher cannot drift.

use rmcp::model::{
    CallToolResult, ErrorCode, GetPromptResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, ReadResourceResult, ServerInfo,
};
use rmcp::ErrorData as McpError;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use srs_mcp_core::{srs_metadata, McpApplication as _, McpApplicationError, SrsMcpApplication};
use srs_repository::boundary_service::{Access, Boundary};
use srs_repository::error::RepositoryError;
use srs_repository::store::FileStore;

pub(crate) type App = SrsMcpApplication<FileStore>;

fn mcp_error(error: McpApplicationError) -> McpError {
    McpError::new(
        ErrorCode(error.code as i32),
        error.message,
        error.data.filter(|data| !data.is_null()),
    )
}

/// One core call, translated into an rmcp result model.
fn call<T: DeserializeOwned>(
    app: &mut App,
    method: &str,
    params: Option<Value>,
) -> Result<T, McpError> {
    let value = app.call(method, params.as_ref()).map_err(mcp_error)?;
    serde_json::from_value(value).map_err(|e| McpError::internal_error(e.to_string(), None))
}

/// The stdio server's `initialize` result: the one definition lives in
/// `srs_metadata::initialize_result_for()` (shared with the browser dispatcher), parsed here.
/// Serving several repositories, the instructions also name the default repository
/// and the boundary (RFC-049 [R13]).
pub(crate) fn server_info(
    profile: srs_mcp_core::tools::ToolProfile,
    boundary: &Boundary,
) -> ServerInfo {
    let mut info: ServerInfo = serde_json::from_value(srs_metadata::initialize_result_for(profile))
        .expect("srs_metadata::initialize_result_for() is a valid rmcp InitializeResult");
    if boundary.is_registry() {
        let members: Vec<String> = boundary
            .members()
            .iter()
            .map(|m| {
                let label = m
                    .label
                    .as_deref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default();
                let ro = if m.read_only { ", read-only" } else { "" };
                format!("{}{label}{ro}", m.id)
            })
            .collect();
        let note = format!(
            "\n\nThis server serves the repositories of a registry (RFC-049): {}. The default repository is {}{}. \
             Address another one by its id: srs://<repositoryId>/… for resources, and the optional \
             `repositoryId` argument on any tool. A write goes to that one repository; read-only \
             repositories refuse writes (read-only).",
            members.join("; "),
            boundary.default_id(),
            if boundary.default_loaded() { "" } else { " (its entry did not load: requests relying on it are refused)" },
        );
        info.instructions = Some(info.instructions.unwrap_or_default() + &note);
    }
    info
}

/// A boundary refusal on a resource or prompt request (RFC-049 [R5]).
pub(crate) fn boundary_error(e: RepositoryError) -> McpError {
    mcp_error(McpApplicationError::from_report(e.report()))
}

/// A boundary refusal on a tool call, as a tool error result carrying its code.
pub(crate) fn tool_refusal(e: RepositoryError) -> CallToolResult {
    serde_json::from_value(srs_mcp_core::tools::tool_err(e.report()))
        .expect("a tool error result is a valid CallToolResult")
}

pub(crate) fn list_resources(app: &mut App) -> Result<ListResourcesResult, McpError> {
    call(app, "resources/list", None)
}

/// The resource templates, for the default repository's id (they open no repository).
pub(crate) fn list_resource_templates(
    default_id: &str,
) -> Result<ListResourceTemplatesResult, McpError> {
    serde_json::from_value(srs_mcp_core::srs_resources::list_resource_templates(
        default_id,
    ))
    .map_err(|e| McpError::internal_error(e.to_string(), None))
}

pub(crate) fn read_resource(app: &mut App, uri: &str) -> Result<ReadResourceResult, McpError> {
    call(app, "resources/read", Some(json!({ "uri": uri })))
}

/// The tool list, from the profile alone (it opens no repository); serving a registry,
/// every tool also takes an optional `repositoryId` (RFC-049 [R13], [R17]).
pub(crate) fn list_tools(
    profile: srs_mcp_core::tools::ToolProfile,
    routed: bool,
) -> Result<ListToolsResult, McpError> {
    let mut list = srs_mcp_core::tools::list_tools_for(profile);
    if routed {
        srs_mcp_core::routing::advertise_repository_id(&mut list);
    }
    serde_json::from_value(list).map_err(|e| McpError::internal_error(e.to_string(), None))
}

/// One tool call. A write the target refuses on I/O is `write-refused` (RFC-049 [R17]).
pub(crate) fn call_tool(
    app: &mut App,
    name: &str,
    arguments: Option<rmcp::model::JsonObject>,
    access: Access,
) -> Result<CallToolResult, McpError> {
    let mut result = app
        .call(
            "tools/call",
            Some(&json!({ "name": name, "arguments": arguments })),
        )
        .map_err(mcp_error)?;
    if access == Access::Write {
        srs_mcp_core::routing::mark_write_refused(&mut result);
    }
    serde_json::from_value(result).map_err(|e| McpError::internal_error(e.to_string(), None))
}

/// An invalid-params error (an unknown tool, a malformed `repositoryId`).
pub(crate) fn invalid(e: McpApplicationError) -> McpError {
    mcp_error(e)
}

pub(crate) fn list_prompts(app: &mut App) -> Result<ListPromptsResult, McpError> {
    call(app, "prompts/list", None)
}

pub(crate) fn get_prompt(
    app: &mut App,
    name: &str,
    arguments: Option<&rmcp::model::JsonObject>,
) -> Result<GetPromptResult, McpError> {
    call(
        app,
        "prompts/get",
        Some(json!({ "name": name, "arguments": arguments })),
    )
}
