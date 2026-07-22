//! The MCP server handler: repository identity, capabilities, and store access.
//!
//! `SrsMcpServer` holds only the repository path and its identity — a fresh
//! `FileStore` is constructed per request (`open_store`), mirroring the CLI's
//! per-invocation `with_store` semantics: no shared mutable state, and on-disk
//! changes are visible between calls (ADR-037).

use std::future::{ready, Future};
use std::path::PathBuf;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::ErrorData as McpError;
use rmcp::{RoleServer, ServerHandler};
use srs_repository::store::{FileStore, RepositoryStore};

use crate::{resources, tools};

/// Guidance shown to MCP clients at initialize time. Mirrors the discovery
/// ladder in `srs-usage.md`: orient first, then read, then write, then validate.
const INSTRUCTIONS: &str = "This server exposes one SRS (Semantic Record System) repository. \
Orient before writing: read srs://<repositoryId>/map for counts and package info, and \
srs://<repositoryId>/navigation for the document structure. Read individual records via the \
srs://<repositoryId>/record/{instanceId} resource template, containers via \
srs://<repositoryId>/container/<containerId>, and rendered document views via \
srs://<repositoryId>/view/<documentViewId>. Type schemas live at \
srs://<repositoryId>/type/{typeId} (also via the type_schema tool): read one before \
authoring records of an unfamiliar type — it carries each field's UUID (x-srs-field-id) \
and aiGuidance. Use the find tool for structured discovery \
(type, tag, lifecycle, tier, container, content match). Writes are validated: record_create, \
relation_create, and note_create enforce the repository's type and relation contracts and \
return diagnostics on rejection. Run repo_validate after a write batch and check its \
diagnostics array — an empty array means the repository is consistent.";

/// MCP server over a single SRS repository.
#[derive(Debug)]
pub struct SrsMcpServer {
    repo_path: PathBuf,
    repository_id: String,
}

impl SrsMcpServer {
    /// Open the repository at `repo_path`, reading its identity from the
    /// manifest. Fails if the path does not hold a loadable SRS repository.
    pub fn new(repo_path: PathBuf) -> anyhow::Result<Self> {
        let store = FileStore::new(&repo_path);
        let manifest = store.load_manifest().map_err(|e| {
            anyhow::anyhow!("not an SRS repository at {}: {}", repo_path.display(), e)
        })?;
        let repository_id = manifest
            .extra
            .get("repositoryId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(Self {
            repo_path,
            repository_id,
        })
    }

    /// The repository identity from the manifest (`repositoryId`).
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    /// A fresh store for one request — per-invocation semantics, like the CLI.
    pub(crate) fn open_store(&self) -> FileStore {
        FileStore::new(&self.repo_path)
    }
}

impl ServerHandler for SrsMcpServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_resources()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new("srs-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    // Handlers are synchronous service calls wrapped in ready futures: the
    // stdio server is single-client, and blocking file I/O here is accepted
    // (ADR-037). Async never reaches the services.

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        ready(resources::list_resources(self))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        ready(Ok(resources::list_resource_templates(self)))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        ready(resources::read_resource(self, &request.uri))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        ready(Ok(tools::list_tools()))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        ready(tools::call_tool(self, &request.name, request.arguments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_reports_name_and_capabilities() {
        let info = SrsMcpServer {
            repo_path: PathBuf::from("/nonexistent"),
            repository_id: "test".into(),
        }
        .get_info();
        assert_eq!(info.server_info.name, "srs-mcp");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.tools.is_some());
        assert!(info.instructions.is_some());
    }

    #[test]
    fn open_store_on_missing_repo_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = SrsMcpServer::new(dir.path().join("no-repo-here")).unwrap_err();
        assert!(
            err.to_string().contains("not an SRS repository"),
            "unexpected error: {err}"
        );
    }
}
