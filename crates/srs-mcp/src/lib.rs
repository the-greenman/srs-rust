//! MCP (Model Context Protocol) adapter over `srs-repository` services.
//!
//! Governed by ADR-037: this crate is the sole owner of the `rmcp`/`tokio`
//! dependencies. Every resource and tool handler is a thin wrapper — typed
//! input → exactly one `srs-repository` service call → the service's typed
//! result serialized. No business logic lives here.

mod resources;
pub mod server;
pub mod tools;
mod uri;

pub use server::SrsMcpServer;

use std::path::PathBuf;

/// Serve the repository at `repo_path` over MCP stdio, blocking until the
/// client disconnects.
///
/// Builds a current-thread tokio runtime internally so callers (the CLI)
/// stay fully synchronous. Async never leaves this crate (ADR-037).
pub fn serve_stdio(repo_path: PathBuf) -> anyhow::Result<()> {
    let server = SrsMcpServer::new(repo_path)?;
    // Stdio transport uses tokio's blocking pool for stdin/stdout — no I/O
    // driver needed. Timers ARE needed: rmcp's shutdown path uses tokio::time.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    runtime.block_on(async move {
        let service = rmcp::ServiceExt::serve(server, rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
