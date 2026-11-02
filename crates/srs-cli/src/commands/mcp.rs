use crate::commands::CliContext;
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum McpCommand {
    /// Serve this repository over the Model Context Protocol (stdio)
    Serve {
        /// Tools to advertise and accept: `full` (every tool, default), `context` (find, read,
        /// type_schema and the writes agent memory needs) or `read` (no tool that writes).
        /// Smaller profiles cost less context per session
        #[arg(long = "profile", value_name = "PROFILE", default_value = "full")]
        profile: srs_mcp::tools::ToolProfile,
        /// Serve every repository in this RFC-049 registry file instead of --repo. Requests
        /// name their repository by id (`srs://<repositoryId>/…`, or a tool's `repositoryId`);
        /// writes are routed to that one repository and refused on read-only entries
        #[arg(long = "registry", value_name = "FILE", conflicts_with = "repo")]
        registry: Option<std::path::PathBuf>,
        /// With --registry: the default repository id, for requests that name none
        /// (default: the registry's first repository entry)
        #[arg(long = "default", value_name = "REPOSITORY_ID", requires = "registry")]
        default: Option<String>,
    },
}

pub fn dispatch(ctx: CliContext, cmd: McpCommand) -> Result<String> {
    match cmd {
        // ADR-037 envelope carve-out: `mcp serve` speaks MCP JSON-RPC on
        // stdout, so it must never fall through to main's envelope printer —
        // it exits directly. Pre-serve failures go to stderr, not stdout.
        // Global --pretty/--container parse but are accepted-and-ignored here.
        McpCommand::Serve {
            profile,
            registry,
            default,
        } => {
            if let Err(e) = srs_mcp::serve_stdio(ctx.repo, registry, default, ctx.actor, profile) {
                eprintln!("srs mcp serve: {e:#}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
    }
}
