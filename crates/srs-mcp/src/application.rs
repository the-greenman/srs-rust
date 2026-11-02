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
    if boundary.members().len() > 1 {
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
            "\n\nThis server serves several repositories (RFC-049): {}. The default repository is {}{}. \
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
    let report = e.report();
    serde_json::from_value(json!({
        "content": [{ "type": "text", "text": report.message }],
        "structuredContent": report,
        "isError": true,
    }))
    .expect("a tool error result is a valid CallToolResult")
}

/// Split a tool call's routing argument from its own arguments: `repositoryId`, or
/// for `read` the repository segment of its `uri`. The tool never sees `repositoryId`.
pub(crate) fn take_repository_id(
    name: &str,
    arguments: Option<rmcp::model::JsonObject>,
) -> (Option<String>, Option<rmcp::model::JsonObject>) {
    let mut arguments = arguments;
    let explicit = arguments
        .as_mut()
        .and_then(|a| a.remove("repositoryId"))
        .and_then(|v| v.as_str().map(ToString::to_string));
    let from_uri = (name == srs_mcp_core::tools::TOOL_READ)
        .then(|| {
            arguments
                .as_ref()?
                .get("uri")?
                .as_str()
                .map(ToString::to_string)
        })
        .flatten()
        .and_then(|u| srs_mcp_core::uri::repository_segment(&u).map(ToString::to_string));
    (explicit.or(from_uri), arguments)
}

pub(crate) fn list_resources(app: &mut App) -> Result<ListResourcesResult, McpError> {
    call(app, "resources/list", None)
}

pub(crate) fn list_resource_templates(
    app: &mut App,
) -> Result<ListResourceTemplatesResult, McpError> {
    call(app, "resources/templates/list", None)
}

pub(crate) fn read_resource(app: &mut App, uri: &str) -> Result<ReadResourceResult, McpError> {
    call(app, "resources/read", Some(json!({ "uri": uri })))
}

/// The tool list; serving several repositories (`routed`), every tool also takes an
/// optional `repositoryId` (RFC-049 [R13], [R17]).
pub(crate) fn list_tools(app: &mut App, routed: bool) -> Result<ListToolsResult, McpError> {
    let mut list = app.call("tools/list", None).map_err(mcp_error)?;
    if routed {
        for tool in list["tools"].as_array_mut().into_iter().flatten() {
            let schema = &mut tool["inputSchema"];
            if !schema["properties"].is_object() {
                schema["properties"] = json!({});
            }
            schema["properties"]["repositoryId"] = json!({
                "type": "string",
                "description": "The repository this call is for (default: the server's default repository)."
            });
        }
    }
    serde_json::from_value(list).map_err(|e| McpError::internal_error(e.to_string(), None))
}

/// One tool call. A write the target refuses on I/O (permissions, a lock) is
/// reported as `write-refused` (RFC-049 [R17]).
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
    if access == Access::Write && result["structuredContent"]["code"] == "io" {
        result["structuredContent"]["code"] = json!("write-refused");
    }
    serde_json::from_value(result).map_err(|e| McpError::internal_error(e.to_string(), None))
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
