//! Transport-independent SRS MCP application surface.
//!
//! The application owns MCP capabilities and dispatch over an already-open
//! [`RepositoryStore`].  Native stdio remains an adapter in `server`; browser
//! bindings can invoke this same surface over their in-memory store.

use rmcp::model::{
    CallToolResult, GetPromptResult, Implementation, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, ProtocolVersion,
    ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::ErrorData as McpError;
use srs_mcp_core::srs_metadata;
use srs_repository::store::RepositoryStore;

use crate::{prompts, resources, tools};

/// A complete SRS MCP application backed by one active repository store.
///
/// It has no filesystem, transport, runtime, or provider ownership.  The
/// caller decides the lifetime and persistence semantics of `store`.
pub struct McpApplication<'store> {
    store: &'store dyn RepositoryStore,
    repository_id: &'store str,
}

impl<'store> McpApplication<'store> {
    pub fn new(store: &'store dyn RepositoryStore, repository_id: &'store str) -> Self {
        Self {
            store,
            repository_id,
        }
    }

    pub fn repository_id(&self) -> &str {
        self.repository_id
    }

    pub fn server_info() -> ServerInfo {
        InitializeResult::new(server_capabilities())
            // The JSON core and Streamable HTTP fixture intentionally pin the
            // browser-compatible profile. Do not inherit rmcp's moving
            // default here or native stdio will silently advertise another
            // protocol revision.
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_server_info(
                Implementation::new("srs-mcp", srs_metadata::release_version())
                    .with_description(srs_metadata::release_generation_description()),
            )
            .with_instructions(srs_metadata::instructions())
    }

    pub fn list_resources(&self) -> Result<ListResourcesResult, McpError> {
        resources::list_resources(self.store, self.repository_id)
    }

    pub fn list_resource_templates(&self) -> ListResourceTemplatesResult {
        resources::list_resource_templates(self.repository_id)
    }

    pub fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        resources::read_resource(self.store, self.repository_id, uri)
    }

    pub fn list_tools(&self) -> ListToolsResult {
        tools::list_tools()
    }

    pub fn call_tool(
        &self,
        name: &str,
        arguments: Option<rmcp::model::JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        tools::call_tool(self.store, name, arguments)
    }

    pub fn list_prompts(&self) -> Result<ListPromptsResult, McpError> {
        prompts::list_prompts(self.store)
    }

    pub fn get_prompt(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
    ) -> Result<GetPromptResult, McpError> {
        prompts::get_prompt(self.store, name, arguments)
    }
}

/// Capabilities advertised by every SRS MCP application, independent of its
/// storage backend or transport adapter.
pub(crate) fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities::builder()
        .enable_prompts()
        .enable_resources()
        .enable_tools()
        .build()
}
