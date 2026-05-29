pub mod migrate;
pub mod note;
pub mod relation_type;
pub mod repo;
pub mod tag;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use srs_repository::detect::find_repo_root;
use std::path::PathBuf;

/// Output format for CLI commands
#[derive(Debug, Clone, Copy, Default, ValueEnum, PartialEq)]
pub enum OutputFormat {
    /// JSON output (default)
    #[default]
    Json,
    /// Human-readable text output (planned, currently returns diagnostic)
    Text,
}

/// Resolve repository root from explicit path or auto-detect from cwd
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
    /// Repository path (defaults to auto-detect from current directory)
    #[arg(long, global = true)]
    pub repo: Option<PathBuf>,

    /// Output format
    #[arg(long, global = true, value_enum, default_value = "json")]
    pub format: OutputFormat,

    /// Pretty-print JSON output (no effect on text format)
    #[arg(long, global = true)]
    pub pretty: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Global CLI context passed to command handlers
#[allow(dead_code)]
pub struct CliContext {
    pub repo: PathBuf,
    pub format: OutputFormat,
    pub pretty: bool,
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
    /// Tag definition management commands
    #[command(subcommand)]
    Tag(TagCommand),
    /// Relation type definition commands
    #[command(subcommand)]
    RelationType(RelationTypeCommand),
}

#[derive(Subcommand)]
pub enum NoteCommand {
    /// List notes in the repository
    List {
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a note by ID
    Get {
        /// Note instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a new note (reads JSON from stdin)
    Create {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Add a tag to a note
    Tag {
        /// Note instance ID
        id: String,
        /// Tag to add
        add_tag: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Audit note tag usage
    AuditTags {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// List foundation notes selected by deterministic tag signals
    Foundations {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RepoCommand {
    /// Emit a deterministic repository map
    Map {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Validate all repository instances against their canonical JSON schemas
    Validate {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum MigrateCommand {
    /// Emit a migration handoff packet
    Packet {
        /// Emit the foundation migration profile
        #[arg(long)]
        foundation: bool,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RelationTypeCommand {
    /// List relation type definitions loaded from the package
    List {
        /// Filter by status (active, deprecated, tombstone, retired)
        #[arg(long)]
        status: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a relation type definition by its UUID id
    Get {
        /// The UUID id of the relation type definition
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum TagCommand {
    /// List tag definitions
    List {
        /// Filter by role
        #[arg(long)]
        role: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a tag definition by ID
    Get {
        /// TagDefinition instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a new tag definition (reads JSON from stdin)
    Create {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

pub fn dispatch(cli: Cli) -> Result<String> {
    // Resolve repository path using global option
    let repo_root = resolve_repo(cli.repo)?;

    // Build context for command handlers
    let ctx = CliContext {
        repo: repo_root,
        format: cli.format,
        pretty: cli.pretty,
    };

    match cli.command {
        Commands::Note(note_cmd) => note::dispatch(ctx, note_cmd),
        Commands::Repo(repo_cmd) => repo::dispatch(ctx, repo_cmd),
        Commands::Migrate(migrate_cmd) => migrate::dispatch(ctx, migrate_cmd),
        Commands::Tag(tag_cmd) => tag::dispatch(ctx, tag_cmd),
        Commands::RelationType(rt_cmd) => relation_type::dispatch(ctx, rt_cmd),
    }
}
