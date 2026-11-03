//! The MCP server handler: the boundary it serves, capabilities, and store access.
//!
//! `SrsMcpServer` holds only its boundary (RFC-049 [R8]): one repository, or the
//! `repository` entries of a registry. A fresh `FileStore` is constructed per
//! request for the repository that request names (`open_app`), mirroring the CLI's
//! per-invocation `with_store` semantics: no shared mutable state, and on-disk
//! changes are visible between calls (ADR-037).

use std::future::{ready, Future};
use std::path::{Path, PathBuf};

use rmcp::model::{
    CallToolRequestParams, CallToolResult, GetPromptRequestParams, GetPromptResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::ErrorData as McpError;
use rmcp::{RoleServer, ServerHandler};
use srs_repository::boundary_service::{Access, Boundary};
use srs_repository::error::RepositoryError;
use srs_repository::store::{FileStore, RepositoryStore};

use srs_mcp_core::{McpApplicationError, SrsMcpApplication};

use crate::application;

/// MCP server over one SRS repository, or over the repositories of a registry.
#[derive(Debug)]
pub struct SrsMcpServer {
    boundary: Boundary,
    /// RFC-046 host-supplied session actor, applied to every per-request store.
    session_actor: Option<serde_json::Value>,
    /// The tools advertised and accepted (srs-rust#1287); `full` by default.
    tool_profile: srs_mcp_core::tools::ToolProfile,
}

impl SrsMcpServer {
    /// Open the repository at `repo_path`, reading its identity from the
    /// manifest. Fails if the path does not hold a loadable SRS repository.
    ///
    /// A data-model generation mismatch ([R21]/[R9]-class refusal —
    /// `StorageGenerationUnsupported` or `RetiredManifestProperty`) gets its
    /// own message naming the mismatch and the fix, rather than surfacing as
    /// a bare parse error: this is exactly the failure a stale mounted
    /// server produces, and it is silent at the protocol level otherwise
    /// (srs-rust#858).
    pub fn new(repo_path: PathBuf) -> anyhow::Result<Self> {
        let store = FileStore::new(&repo_path);
        store.load_manifest().map_err(|e| match &e {
            RepositoryError::StorageGenerationUnsupported { .. }
            | RepositoryError::RetiredManifestProperty { .. } => anyhow::anyhow!(
                "data-model generation mismatch at {}: {e} — if this srs-mcp binary predates \
                 the repository's generation, fetch the current release asset from \
                 the-greenman/srs-rust",
                repo_path.display()
            ),
            _ => anyhow::anyhow!("not an SRS repository at {}: {}", repo_path.display(), e),
        })?;
        Ok(Self::with_boundary(Boundary::single(&repo_path)?))
    }

    /// Serve the `repository` entries of the registry at `registry_path` (RFC-049
    /// [R8]); `default_id` names the default repository ([R13]), else the first
    /// repository entry. Entries that fail to load, and a default that did not load,
    /// are reported on stderr before serving.
    pub fn from_registry(registry_path: &Path, default_id: Option<&str>) -> anyhow::Result<Self> {
        let (boundary, outcomes) = Boundary::from_registry(registry_path, default_id)?;
        for o in &outcomes {
            eprintln!(
                "srs mcp serve: registry entry {}: {} ({})",
                o.index, o.message, o.code
            );
        }
        if !boundary.default_loaded() {
            eprintln!(
                "srs mcp serve: the default repository '{}' has no usable registry entry; \
                 requests that rely on the default are refused (not-in-boundary)",
                boundary.default_id()
            );
        }
        Ok(Self::with_boundary(boundary))
    }

    fn with_boundary(boundary: Boundary) -> Self {
        Self {
            boundary,
            session_actor: None,
            tool_profile: Default::default(),
        }
    }

    /// Set the RFC-046 session actor stamped on everything this server creates.
    pub fn with_session_actor(mut self, actor: Option<serde_json::Value>) -> Self {
        self.session_actor = actor;
        self
    }

    /// Restrict the tools this server advertises and accepts (srs-rust#1287).
    pub fn with_tool_profile(mut self, profile: srs_mcp_core::tools::ToolProfile) -> Self {
        self.tool_profile = profile;
        self
    }

    /// The default repository's identity (`repositoryId`); with one repository, that one.
    pub fn repository_id(&self) -> &str {
        self.boundary.default_id()
    }

    /// A fresh application (and `FileStore`) for one request, over the repository it
    /// names (absent = the default) — per-invocation semantics, like the CLI.
    pub(crate) fn open_app(
        &self,
        repository_id: Option<&str>,
        access: Access,
    ) -> Result<application::App, RepositoryError> {
        let target = self.boundary.resolve(repository_id, access)?;
        let mut app = SrsMcpApplication::new(FileStore::new(&target.path), target.id);
        app.set_session_actor(self.session_actor.clone());
        app.set_tool_profile(self.tool_profile);
        Ok(app)
    }

    /// One tool call routed to the repository it names (RFC-049 [R16], [R17]). An unknown
    /// tool is reported as unknown before any routing.
    fn route_tool(&self, request: CallToolRequestParams) -> Result<CallToolResult, McpError> {
        if !srs_mcp_core::routing::known_tool(self.tool_profile, &request.name) {
            return Err(application::invalid(McpApplicationError::invalid_params(
                format!(
                    "unknown tool '{}' (not in the '{}' tool profile)",
                    request.name,
                    self.tool_profile.as_str()
                ),
            )));
        }
        let (repository_id, arguments) =
            srs_mcp_core::routing::take_repository_id(&request.name, request.arguments)
                .map_err(application::invalid)?;
        let access = if srs_mcp_core::routing::is_write(&request.name) {
            Access::Write
        } else {
            Access::Read
        };
        match self.open_app(repository_id.as_deref(), access) {
            Ok(mut app) => application::call_tool(&mut app, &request.name, arguments, access),
            Err(e) => Ok(application::tool_refusal(e)),
        }
    }

    /// Run `f` on the default repository's application.
    fn on_default<T>(
        &self,
        f: impl FnOnce(&mut application::App) -> Result<T, McpError>,
    ) -> Result<T, McpError> {
        let mut app = self
            .open_app(None, Access::Read)
            .map_err(application::boundary_error)?;
        f(&mut app)
    }
}

impl ServerHandler for SrsMcpServer {
    fn get_info(&self) -> ServerInfo {
        application::server_info(self.tool_profile, &self.boundary)
    }

    // Handlers are synchronous service calls wrapped in ready futures: the
    // stdio server is single-client, and blocking file I/O here is accepted
    // (ADR-037). Async never reaches the services.

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        ready(self.on_default(application::list_resources))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        ready(application::list_resource_templates(
            self.boundary.default_id(),
        ))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let repository_id = srs_mcp_core::uri::repository_segment(&request.uri);
        ready(
            self.open_app(repository_id, Access::Read)
                .map_err(application::boundary_error)
                .and_then(|mut app| application::read_resource(&mut app, &request.uri)),
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        ready(application::list_tools(
            self.tool_profile,
            self.boundary.is_registry(),
        ))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        ready(self.route_tool(request))
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        ready(self.on_default(application::list_prompts))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        ready(self.on_default(|app| {
            application::get_prompt(app, &request.name, request.arguments.as_ref())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_repository::manifest::MIN_SUPPORTED_DATA_MODEL_REVISION;

    fn test_server() -> SrsMcpServer {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"repositoryId": "test", "dataModelRevision": 11}"#,
        )
        .unwrap();
        SrsMcpServer::with_boundary(Boundary::single(dir.path()).unwrap())
    }

    #[test]
    fn server_info_reports_name_and_capabilities() {
        let info = test_server().get_info();
        assert_eq!(info.server_info.name, "srs-mcp");
        assert!(info.capabilities.prompts.is_some());
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.tools.is_some());
        assert!(info.instructions.is_some());
    }

    /// serverInfo carries the release build number and the supported
    /// data-model generation, not just the bare crate version
    /// (srs-rust#858) — this is the handshake half of the fix; a `cargo
    /// test` run has no `SRS_BUILD_NUMBER`, so it exercises the `+dev` path.
    #[test]
    fn server_info_carries_build_and_generation() {
        let info = test_server().get_info();
        assert_eq!(
            info.server_info.version,
            format!("{}+dev", env!("CARGO_PKG_VERSION")),
            "dev build (no SRS_BUILD_NUMBER) must report the +dev suffix"
        );
        let description = info
            .server_info
            .description
            .expect("serverInfo.description must name the supported data-model generation");
        assert!(
            description.contains(&MIN_SUPPORTED_DATA_MODEL_REVISION.to_string()),
            "description does not name the supported generation: {description}"
        );
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

    /// [R21]/[R9]-class refusal: a manifest that declares `dataModelRevision:
    /// 2` (so it clears the `>= MIN_SUPPORTED_DATA_MODEL_REVISION` gate) but
    /// still carries a Change-K retired property (`instanceIndex`) is exactly
    /// the "stale binary vs. current corpus, or vice versa" shape #858 names
    /// — the crate-internal test-activation machinery that used to gate this
    /// check is gone post-Phase-6-flip, so this fixture hits it unconditionally
    /// on a plain `FileStore`. Startup must name the mismatch and the fix
    /// rather than surface a bare parse error.
    #[test]
    fn open_store_on_generation_mismatch_names_the_fix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".srs")).unwrap();
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{
                "dataModelRevision": 2,
                "repositoryId": "11111111-1111-1111-1111-111111111111",
                "namespace": "com.example.stale",
                "packageRef": {"mode": "local", "path": "package"},
                "instanceIndex": []
            }"#,
        )
        .unwrap();

        let err = SrsMcpServer::new(dir.path().to_path_buf()).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("generation mismatch"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("fetch the current release asset from the-greenman/srs-rust"),
            "unexpected error: {message}"
        );
    }
}
