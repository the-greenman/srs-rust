//! Transport-independent SRS MCP application surface.
//!
//! The application owns MCP capabilities and dispatch over an already-open
//! [`RepositoryStore`].  Native stdio remains an adapter in `server`; browser
//! bindings can invoke this same surface over their in-memory store.

use rmcp::model::{
    CallToolResult, GetPromptResult, Implementation, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, ReadResourceResult,
    ServerCapabilities, ServerInfo,
};
use rmcp::ErrorData as McpError;
use srs_repository::manifest::MIN_SUPPORTED_DATA_MODEL_REVISION;
use srs_repository::store::RepositoryStore;

use crate::{prompts, resources, tools};

const BUILD_NUMBER: Option<&str> = option_env!("SRS_BUILD_NUMBER");

fn release_version() -> String {
    let base = env!("CARGO_PKG_VERSION");
    match BUILD_NUMBER {
        Some(n) if !n.is_empty() => format!("{base}+build.{n}"),
        _ => format!("{base}+dev"),
    }
}

fn release_generation_description() -> String {
    format!("supports data-model generation >= {MIN_SUPPORTED_DATA_MODEL_REVISION} (RFC-038 [R21])")
}

const INSTRUCTIONS: &str = "This server exposes one SRS (Semantic Record System) repository. \
Orient before writing: read srs://<repositoryId>/map for counts and package info, and \
srs://<repositoryId>/navigation for the document structure. srs://<repositoryId>/agent-index is \
the one-page AI orientation index (identity, counts, types, sections, entry points). \
srs://<repositoryId>/tree is the recursive contains-tree from every root, and \
srs://<repositoryId>/tree/{instanceId} the subtree under one instance — descend from a \
navigation section or container member by its instanceId; container members also carry \
sectionContainerId when they root a sub-container. Read individual records via the \
srs://<repositoryId>/record/{instanceId} resource template, containers via \
srs://<repositoryId>/container/<containerId>, and rendered document views via \
srs://<repositoryId>/view/<compositionId>. Type schemas live at \
srs://<repositoryId>/type/{typeId} (also via the type_schema tool): read one before \
authoring records of an unfamiliar type — its properties are keyed by Field.name (the \
same keys record_create fieldValues uses, RFC-039) and carry aiGuidance. Protocols (staged \
processes) live at srs://<repositoryId>/protocol (list) and \
srs://<repositoryId>/protocol/{protocolId} (definition plus stages in order). Use the find tool for structured discovery \
(type, tag, lifecycle, tier, container, content match). Writes are validated: record_create, \
relation_create, and note_create enforce the repository's type and relation contracts and \
return diagnostics on rejection. Run repo_validate after a write batch and check its \
summary: summary.errors == 0 means the repository is consistent. Warnings are non-blocking, \
but review them. An empty diagnostics array means the repository is completely clean. \
Prompts: this server exposes one MCP prompt per installed blueprint. Call prompts/list \
to discover available blueprints; call prompts/get with a blueprint UUID to retrieve its \
full brief as rendered markdown — AI guidance, required types, structure, and protocol.";

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

    pub fn server_info() -> ServerInfo {
        InitializeResult::new(server_capabilities())
            .with_server_info(
                Implementation::new("srs-mcp", release_version())
                    .with_description(release_generation_description()),
            )
            .with_instructions(INSTRUCTIONS)
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
