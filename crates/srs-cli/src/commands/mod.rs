pub mod note;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
}

pub fn dispatch(cli: Cli) -> Result<String> {
    match cli.command {
        Commands::Note(note_cmd) => note::dispatch(note_cmd),
    }
}
