//! Native rmcp adapter over the transport-agnostic [`SrsMcpApplication`].
//!
//! All SRS semantics (resources, prompts, tools, schemas) live in
//! `srs-mcp-core`. This module only translates between rmcp's typed models and
//! the core's JSON-native results (ADR-037), so the stdio server and the
//! browser dispatcher cannot drift.

use rmcp::model::{
    CallToolResult, ErrorCode, GetPromptResult, Implementation, InitializeResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    ProtocolVersion, ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::ErrorData as McpError;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use srs_mcp_core::{srs_metadata, McpApplication as _, McpApplicationError, SrsMcpApplication};
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

pub(crate) fn server_info() -> ServerInfo {
    InitializeResult::new(
        ServerCapabilities::builder()
            .enable_prompts()
            .enable_resources()
            .enable_tools()
            .build(),
    )
    // The JSON core and Streamable HTTP fixture intentionally pin the
    // browser-compatible profile. Do not inherit rmcp's moving default here or
    // native stdio would silently advertise another protocol revision.
    .with_protocol_version(ProtocolVersion::V_2025_06_18)
    .with_server_info(
        Implementation::new("srs-mcp", srs_metadata::release_version())
            .with_description(srs_metadata::release_generation_description()),
    )
    .with_instructions(srs_metadata::instructions())
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

pub(crate) fn list_tools(app: &mut App) -> Result<ListToolsResult, McpError> {
    call(app, "tools/list", None)
}

pub(crate) fn call_tool(
    app: &mut App,
    name: &str,
    arguments: Option<rmcp::model::JsonObject>,
) -> Result<CallToolResult, McpError> {
    call(
        app,
        "tools/call",
        Some(json!({ "name": name, "arguments": arguments })),
    )
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
