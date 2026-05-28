pub mod migrate;
pub mod note;
pub mod repo;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use srs_repository::detect::find_repo_root;
use std::path::PathBuf;

pub const FOUNDATION_SIGNAL_TAGS: &[&str] = &[
    "foundations",
    "meaning-first",
    "semantic-state",
    "fundamental-tensions",
    "domain",
    "problems",
    "protocol",
    "projection",
    "projections",
    "human-ai-collaboration",
    "distributed-knowledge",
    "addressability",
];

pub fn resolve_repo(repo: Option<PathBuf>) -> Result<PathBuf> {
    match repo {
        Some(path) => Ok(path),
        None => {
            let cwd = std::env::current_dir()?;
            find_repo_root(&cwd).context("Failed to find repository root")
        }
    }
}

#[derive(Parser)]
#[command(name = "srs")]
#[command(about = "Semantic Record System CLI")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Note management commands
    #[command(subcommand)]
    Note(NoteCommand),
    /// Repository inspection commands
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Migration handoff commands
    #[command(subcommand)]
    Migrate(MigrateCommand),
}

#[derive(Subcommand)]
pub enum NoteCommand {
    /// List notes in the repository
    List {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
    /// Get a note by ID
    Get {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Note instance ID
        id: String,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
    /// Create a new note
    Create {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
    /// Add a tag to a note
    Tag {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Note instance ID
        id: String,
        /// Tag to add
        add_tag: String,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
    /// Audit note tag usage
    AuditTags {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
    /// List foundation notes selected by deterministic tag signals
    Foundations {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RepoCommand {
    /// Emit a deterministic repository map
    Map {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum MigrateCommand {
    /// Emit a migration handoff packet
    Packet {
        /// Repository path (defaults to CWD)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Emit the foundation migration profile
        #[arg(long)]
        foundation: bool,
        /// JSON output flag (silent no-op, output is always JSON)
        #[arg(long)]
        json: bool,
    },
}

pub fn dispatch(cli: Cli) -> Result<String> {
    match cli.command {
        Commands::Note(note_cmd) => note::dispatch(note_cmd),
        Commands::Repo(repo_cmd) => repo::dispatch(repo_cmd),
        Commands::Migrate(migrate_cmd) => migrate::dispatch(migrate_cmd),
    }
}
