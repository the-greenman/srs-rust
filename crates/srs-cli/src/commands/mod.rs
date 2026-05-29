pub mod extension;
pub mod field;
pub mod migrate;
pub mod note;
pub mod protocol;
pub mod record;
pub mod record_type;
pub mod relation;
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
    /// Field definition commands
    #[command(subcommand)]
    Field(FieldCommand),
    /// Type definition commands
    #[command(subcommand)]
    Type(TypeCommand),
    /// Generic record commands
    #[command(subcommand)]
    Record(RecordCommand),
    /// Relation commands
    #[command(subcommand)]
    Relation(RelationCommand),
    /// Extension definition commands
    #[command(subcommand)]
    Extension(ExtensionCommand),
    /// Protocol definition commands
    #[command(subcommand)]
    Protocol(ProtocolCommand),
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
    /// Update an existing note (reads JSON from stdin)
    Update {
        /// Note instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Delete a note by ID
    Delete {
        /// Note instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Tag management commands
    #[command(subcommand)]
    Tag(NoteTagCommand),
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
pub enum NoteTagCommand {
    /// Add a tag to a note
    Add {
        /// Note instance ID
        id: String,
        /// Tag to add
        tag: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Remove a tag from a note
    Remove {
        /// Note instance ID
        id: String,
        /// Tag to remove
        tag: String,
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
    /// Extension management commands
    #[command(subcommand)]
    Extensions(RepoExtensionsCommand),
}

#[derive(Subcommand)]
pub enum RepoExtensionsCommand {
    /// List declared extensions
    List {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Enable (add) a declared extension
    Enable {
        /// Extension ID to enable (e.g., ext:repository)
        extension_id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Disable (remove) a declared extension
    Disable {
        /// Extension ID to disable (e.g., ext:repository)
        extension_id: String,
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
    /// Update an existing tag definition (reads JSON from stdin)
    Update {
        /// TagDefinition instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Delete a tag definition by ID
    Delete {
        /// TagDefinition instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum FieldCommand {
    /// List field definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a field definition by ID
    Get {
        /// Field definition ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a new field definition (reads JSON from stdin)
    Create {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum TypeCommand {
    /// List type definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a type definition by ID
    Get {
        /// Type definition ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RecordCommand {
    /// List records by type
    List {
        /// Type filter (namespace/name format)
        #[arg(long = "type", visible_alias = "type-filter")]
        type_filter: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a record by ID
    Get {
        /// Record instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a record (reads JSON from stdin)
    Create {
        /// Type filter (namespace/name format)
        #[arg(long = "type", visible_alias = "type-filter")]
        type_filter: String,
        /// Optional type version override (defaults to latest for namespace/name)
        #[arg(long)]
        version: Option<u32>,
        /// Optional output directory relative to repo root
        #[arg(long, default_value = "package/records")]
        dir: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Update a record by ID (reads JSON from stdin)
    Update {
        /// Record instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Delete a record by ID
    Delete {
        /// Record instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RelationCommand {
    /// List relations
    List {
        /// Filter by source instance ID
        #[arg(long)]
        source: Option<String>,
        /// Filter by target instance ID
        #[arg(long)]
        target: Option<String>,
        /// Filter by relation type
        #[arg(long = "type")]
        relation_type: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a relation (reads JSON from stdin)
    Create {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a relation by ID
    Get {
        /// Relation ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Delete a relation by ID
    Delete {
        /// Relation ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ExtensionCommand {
    /// List extension definitions
    List {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get an extension definition by ID
    Get {
        /// Extension instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a new extension definition (reads JSON from stdin)
    Create {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Update an extension definition (reads JSON from stdin)
    Update {
        /// Extension instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Delete an extension definition
    Delete {
        /// Extension instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ProtocolCommand {
    /// List protocol definitions
    List {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a protocol definition by ID
    Get {
        /// Protocol instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// List stages for a protocol
    Stages {
        /// Protocol instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Validate a protocol definition
    Validate {
        /// Protocol instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Export a protocol definition to portable JSON
    Export {
        /// Protocol instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Import a protocol definition (reads JSON from stdin)
    Import {
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
        Commands::Field(field_cmd) => field::dispatch(ctx, field_cmd),
        Commands::Type(type_cmd) => record_type::dispatch(ctx, type_cmd),
        Commands::Record(record_cmd) => record::dispatch(ctx, record_cmd),
        Commands::Relation(relation_cmd) => relation::dispatch(ctx, relation_cmd),
        Commands::Extension(ext_cmd) => extension::dispatch(ctx, ext_cmd),
        Commands::Protocol(proto_cmd) => protocol::dispatch(ctx, proto_cmd),
    }
}
