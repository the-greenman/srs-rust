//! The MCP server handler: repository identity, capabilities, and store access.
//!
//! `SrsMcpServer` holds only the repository path and its identity — a fresh
//! `FileStore` is constructed per request (`open_store`), mirroring the CLI's
//! per-invocation `with_store` semantics: no shared mutable state, and on-disk
//! changes are visible between calls (ADR-037).

use std::future::{ready, Future};
use std::path::PathBuf;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, GetPromptRequestParams, GetPromptResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::ErrorData as McpError;
use rmcp::{RoleServer, ServerHandler};
use srs_repository::error::RepositoryError;
use srs_repository::store::{FileStore, RepositoryStore};

use crate::application::McpApplication;

/// MCP server over a single SRS repository.
#[derive(Debug)]
pub struct SrsMcpServer {
    repo_path: PathBuf,
    repository_id: String,
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
        let manifest = store.load_manifest().map_err(|e| match &e {
            RepositoryError::StorageGenerationUnsupported { .. }
            | RepositoryError::RetiredManifestProperty { .. } => anyhow::anyhow!(
                "data-model generation mismatch at {}: {e} — if this srs-mcp binary predates \
                 the repository's generation, fetch the current release asset from \
                 the-greenman/srs-rust",
                repo_path.display()
            ),
            _ => anyhow::anyhow!("not an SRS repository at {}: {}", repo_path.display(), e),
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
        McpApplication::server_info()
    }

    // Handlers are synchronous service calls wrapped in ready futures: the
    // stdio server is single-client, and blocking file I/O here is accepted
    // (ADR-037). Async never reaches the services.

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(McpApplication::new(&store, &self.repository_id).list_resources())
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(Ok(
            McpApplication::new(&store, &self.repository_id).list_resource_templates()
        ))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(McpApplication::new(&store, &self.repository_id).read_resource(&request.uri))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(Ok(
            McpApplication::new(&store, &self.repository_id).list_tools()
        ))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(
            McpApplication::new(&store, &self.repository_id)
                .call_tool(&request.name, request.arguments),
        )
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(McpApplication::new(&store, &self.repository_id).list_prompts())
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        let store = self.open_store();
        ready(
            McpApplication::new(&store, &self.repository_id)
                .get_prompt(&request.name, request.arguments.as_ref()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_repository::manifest::MIN_SUPPORTED_DATA_MODEL_REVISION;

    #[test]
    fn server_info_reports_name_and_capabilities() {
        let info = SrsMcpServer {
            repo_path: PathBuf::from("/nonexistent"),
            repository_id: "test".into(),
        }
        .get_info();
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
        let info = SrsMcpServer {
            repo_path: PathBuf::from("/nonexistent"),
            repository_id: "test".into(),
        }
        .get_info();
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
