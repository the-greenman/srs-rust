use crate::commands::CliContext;
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum McpCommand {
    /// Serve this repository over the Model Context Protocol (stdio)
    Serve,
}

pub fn dispatch(ctx: CliContext, cmd: McpCommand) -> Result<String> {
    match cmd {
        // ADR-037 envelope carve-out: `mcp serve` speaks MCP JSON-RPC on
        // stdout, so it must never fall through to main's envelope printer —
        // it exits directly. Pre-serve failures go to stderr, not stdout.
        // Global --pretty/--container parse but are accepted-and-ignored here.
        McpCommand::Serve => {
            if let Err(e) = srs_mcp::serve_stdio(ctx.repo) {
                eprintln!("srs mcp serve: {e:#}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
    }
}
