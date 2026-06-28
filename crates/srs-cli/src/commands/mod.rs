pub mod blueprint;
pub mod container;
pub mod document_view;
pub mod extension;
pub mod field;
pub mod find;
pub mod lifecycle;
pub mod migrate;
pub mod note;
pub mod package;
pub mod protocol;
pub mod record;
pub mod record_type;
pub mod relation;
pub mod relation_type;
pub mod render;
pub mod repo;
pub mod tag;
pub mod term;
pub mod theme;
pub mod tree;
pub mod view;
pub mod vocabulary;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use srs_repository::detect::find_repo_root;
use srs_repository::{FileStore, JsonStore, RepositoryStore};
use std::path::{Path, PathBuf};

/// Output format for CLI commands
#[derive(Debug, Clone, Copy, Default, ValueEnum, PartialEq)]
pub enum OutputFormat {
    /// JSON output (default)
    #[default]
    Json,
    /// YAML output
    Yaml,
    /// Human-readable text output (planned, currently returns diagnostic)
    Text,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum StoreBackend {
    File,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryLocation {
    pub path: PathBuf,
    pub store: StoreBackend,
}

/// Resolve repository root from explicit path or auto-detect from cwd
pub fn resolve_repo(
    repo: Option<PathBuf>,
    backend: Option<StoreBackend>,
) -> Result<RepositoryLocation> {
    match (repo, backend) {
        (Some(path), Some(store)) => Ok(RepositoryLocation { path, store }),
        (Some(path), None) => Ok(RepositoryLocation {
            store: infer_store_from_location(&path),
            path,
        }),
        (None, Some(StoreBackend::File)) => {
            let cwd = std::env::current_dir()?;
            Ok(RepositoryLocation {
                path: find_repo_root(&cwd).context("Failed to find repository root")?,
                store: StoreBackend::File,
            })
        }
        (None, Some(StoreBackend::Json)) => {
            let cwd = std::env::current_dir()?;
            Ok(RepositoryLocation {
                path: find_json_repo_file(&cwd)
                    .map(|found| found.path)
                    .unwrap_or_else(|| cwd.join("repo.srsj")),
                store: StoreBackend::Json,
            })
        }
        (None, None) => resolve_repo_from_cwd(),
    }
}

fn infer_store_from_location(path: &Path) -> StoreBackend {
    if path.extension().and_then(|ext| ext.to_str()) == Some("srsj") || path.is_file() {
        StoreBackend::Json
    } else {
        StoreBackend::File
    }
}

#[derive(Debug, Clone)]
struct DiscoveredRepo {
    path: PathBuf,
    distance: usize,
}

fn resolve_repo_from_cwd() -> Result<RepositoryLocation> {
    let cwd = std::env::current_dir()?;
    let file_repo = find_repo_root(&cwd).ok().map(|path| DiscoveredRepo {
        distance: ancestor_distance(&cwd, &path),
        path,
    });
    let json_repo = find_json_repo_file(&cwd);

    match (file_repo, json_repo) {
        (Some(file_repo), Some(json_repo)) if file_repo.distance == json_repo.distance => {
            Err(anyhow!(
                "Found both file-backed and JSON-backed repositories at {}; pass --repo to choose one",
                file_repo.path.display()
            ))
        }
        (Some(file_repo), Some(json_repo)) if file_repo.distance < json_repo.distance => {
            Ok(RepositoryLocation {
                path: file_repo.path,
                store: StoreBackend::File,
            })
        }
        (Some(_), Some(json_repo)) => Ok(RepositoryLocation {
            path: json_repo.path,
            store: StoreBackend::Json,
        }),
        (Some(file_repo), None) => Ok(RepositoryLocation {
            path: file_repo.path,
            store: StoreBackend::File,
        }),
        (None, Some(json_repo)) => Ok(RepositoryLocation {
            path: json_repo.path,
            store: StoreBackend::Json,
        }),
        (None, None) => find_repo_root(&cwd)
            .context("Failed to find repository root")
            .map(|path| RepositoryLocation {
                path,
                store: StoreBackend::File,
            }),
    }
}

fn find_json_repo_file(start: &Path) -> Option<DiscoveredRepo> {
    let mut current = start.to_path_buf();
    let mut distance = 0;

    loop {
        let repo_file = current.join("repo.srsj");
        if repo_file.is_file() {
            return Some(DiscoveredRepo {
                path: repo_file,
                distance,
            });
        }

        if !current.pop() {
            return None;
        }
        distance += 1;
    }
}

fn ancestor_distance(start: &Path, ancestor: &Path) -> usize {
    start
        .strip_prefix(ancestor)
        .map(|relative| relative.components().count())
        .unwrap_or(usize::MAX)
}

fn resolve_repo_for_create(
    repo: Option<PathBuf>,
    backend: Option<StoreBackend>,
) -> Result<RepositoryLocation> {
    match (repo, backend) {
        (Some(path), Some(store)) => Ok(RepositoryLocation { path, store }),
        (Some(path), None) => Ok(RepositoryLocation {
            store: infer_store_from_location(&path),
            path,
        }),
        (None, Some(StoreBackend::Json)) => Ok(RepositoryLocation {
            path: std::env::current_dir()?.join("repo.srsj"),
            store: StoreBackend::Json,
        }),
        (None, _) => Ok(RepositoryLocation {
            path: std::env::current_dir()?,
            store: StoreBackend::File,
        }),
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

    /// Storage backend override. By default the backend is inferred from the repository location.
    #[arg(long = "store", global = true, value_enum)]
    pub store: Option<StoreBackend>,

    /// Pretty-print JSON output (no effect on text format)
    #[arg(long, global = true)]
    pub pretty: bool,

    /// Container scope: constrains list/create/delete to this container's membership
    #[arg(long = "container", global = true)]
    pub container_id: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Global CLI context passed to command handlers
#[allow(dead_code)]
pub struct CliContext {
    pub repo: PathBuf,
    pub store: StoreBackend,
    pub format: OutputFormat,
    pub pretty: bool,
    pub container_id: Option<String>,
}

pub fn with_store<T>(
    ctx: &CliContext,
    f: impl FnOnce(&dyn RepositoryStore) -> Result<T>,
) -> Result<T> {
    match ctx.store {
        StoreBackend::File => {
            let store = FileStore::new(&ctx.repo);
            f(&store)
        }
        StoreBackend::Json => {
            let store = JsonStore::open(&ctx.repo)
                .with_context(|| format!("Failed to open JsonStore at {}", ctx.repo.display()))?;
            f(&store)
        }
    }
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
    /// Blueprint definition commands (ext:blueprint)
    #[command(subcommand)]
    Blueprint(BlueprintCommand),
    /// Container grouping and membership commands
    #[command(subcommand)]
    Container(ContainerCommand),
    /// Render document outputs from views
    #[command(subcommand)]
    Render(RenderCommand),
    /// Package management commands
    #[command(subcommand)]
    Package(PackageCommand),
    /// Theme definition management
    #[command(subcommand)]
    Theme(ThemeCommand),
    /// View (L1 field view) definition management
    #[command(subcommand)]
    View(ViewCommand),
    /// Document view (L2 render view) definition management
    #[command(subcommand, name = "document-view")]
    DocumentView(DocumentViewCommand),
    /// Vocabulary definition commands (RFC-006)
    #[command(subcommand)]
    Vocabulary(VocabularyCommand),
    /// Lifecycle definition commands (RFC-006)
    #[command(subcommand)]
    Lifecycle(LifecycleCommand),
    /// Term definition commands (RFC-006)
    #[command(subcommand)]
    Term(TermCommand),
    /// Show the hierarchical record tree rooted at top-level or specified instances
    Tree(TreeArgs),
    /// Discover instances by structured filters + content search (ext:discovery)
    Find(FindArgs),
}

#[derive(Subcommand)]
pub enum ContainerCommand {
    /// List all containers, optionally filtered by containerType or membership role
    List {
        /// Filter by containerType
        #[arg(long = "type")]
        container_type: Option<String>,
        /// Return only containers where this instance appears in memberInstanceIds OR rootInstanceIds
        #[arg(long = "member")]
        member_instance_id: Option<String>,
        /// Return only containers where this instance appears specifically in rootInstanceIds
        #[arg(long = "root")]
        root_instance_id: Option<String>,
    },
    /// Create a new container (reads JSON from stdin)
    Create,
    /// Get a container by ID
    Get { container_id: String },
    /// Update a container (reads partial JSON patch from stdin)
    Update { container_id: String },
    /// Delete a container by ID
    Delete { container_id: String },
    /// Member instance management
    #[command(subcommand)]
    Members(ContainerMembersCommand),
    /// Root instance management
    #[command(subcommand)]
    Roots(ContainerRootsCommand),
    /// Validate container invariants
    Validate { container_id: String },
    /// Resolve a structured container view: root + ordered members + DocumentView-driven
    /// column spec + per-member display label (issue #254).
    ResolveView {
        container_id: String,
        /// Optional DocumentView UUID override; defaults to the view matched from the
        /// container's root type binding.
        #[arg(long = "view-id")]
        view_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContainerMembersCommand {
    List {
        container_id: String,
    },
    Add {
        container_id: String,
        instance_id: String,
    },
    Remove {
        container_id: String,
        instance_id: String,
    },
}

#[derive(Subcommand)]
pub enum ContainerRootsCommand {
    List {
        container_id: String,
    },
    Add {
        container_id: String,
        instance_id: String,
    },
    Remove {
        container_id: String,
        instance_id: String,
    },
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
    /// List all distinct tags used across tier-0 notes, with note counts
    List,
    /// Audit note tag usage (replaces audit-tags); optionally scoped to a note's tags
    Map {
        /// Scope audit to notes sharing tags with this note ID
        #[arg(long)]
        id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RepoCommand {
    /// Create a new repository at the target path
    Create {
        /// Repository ID (UUID); auto-generated if omitted
        #[arg(long = "repository-id")]
        repository_id: Option<String>,
        /// Repository namespace
        #[arg(long)]
        namespace: String,
        /// Repository title (display name)
        #[arg(long)]
        title: Option<String>,
        /// Repository description
        #[arg(long)]
        description: Option<String>,
        /// SRS version stored in manifest
        #[arg(long = "srs-version", default_value = "2.0-draft")]
        srs_version: String,
        /// Primary package ID (UUID); auto-generated if omitted
        #[arg(long = "package-id")]
        package_id: Option<String>,
        /// Primary package name
        #[arg(long = "package-name", default_value = "primary")]
        package_name: String,
        /// Primary package version
        #[arg(long = "package-version", default_value = "1.0.0")]
        package_version: String,
        /// Primary package namespace (defaults to repository namespace)
        #[arg(long = "package-namespace")]
        package_namespace: Option<String>,
    },
    /// Copy a repository from source to target path using logical portability
    Copy {
        /// Source repository root
        #[arg(long = "from")]
        from: PathBuf,
        /// Target repository root
        #[arg(long = "to")]
        to: PathBuf,
        /// Source store backend override. By default inferred from --from.
        #[arg(long = "from-store", value_enum)]
        from_store: Option<StoreBackend>,
        /// Target store backend override. By default inferred from --to.
        #[arg(long = "to-store", value_enum)]
        to_store: Option<StoreBackend>,
    },
    /// Diff two repository copies, keyed on stable instance_id / relation_id
    Diff {
        /// Source (from) repository root
        #[arg(long = "from")]
        from: PathBuf,
        /// Target (to) repository root
        #[arg(long = "to")]
        to: PathBuf,
        /// Source store backend override. By default inferred from --from.
        #[arg(long = "from-store", value_enum)]
        from_store: Option<StoreBackend>,
        /// Target store backend override. By default inferred from --to.
        #[arg(long = "to-store", value_enum)]
        to_store: Option<StoreBackend>,
    },
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
    /// Resolve structural repository navigation from the root container
    Navigation,
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
    /// Create a new relation type definition (reads JSON from stdin)
    Create {},
    /// Update an existing relation type definition (reads JSON from stdin)
    Update {
        /// The UUID id of the relation type definition to update
        id: String,
    },
    /// Delete a relation type definition by its UUID id
    Delete {
        /// The UUID id of the relation type definition to delete
        id: String,
    },
}

#[derive(Subcommand)]
pub enum TagCommand {
    /// List tag terms from package vocabularies (RFC-006)
    List {
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a vocabulary term by ID
    Get {
        /// Term ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum VocabularyCommand {
    /// List all vocabularies in the package
    List {
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a vocabulary by id
    Get {
        /// Vocabulary UUID id
        id: String,
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a new vocabulary (reads JSON from stdin)
    Create,
    /// Add a term to an existing vocabulary (reads term JSON from stdin)
    TermCreate {
        /// Vocabulary UUID id to add the term to
        #[arg(long = "vocabulary-id")]
        vocabulary_id: String,
    },
    /// Promote a vocabulary from open to closed mode (V10 pre-flight)
    Promote {
        /// Vocabulary UUID id to promote
        id: String,
    },
    /// Inspect the in-use tag set for an open vocabulary (V10 pre-flight, read-only)
    DeriveTagSet {
        /// Vocabulary UUID id to derive the tag set for
        id: String,
    },
}

#[derive(Subcommand)]
pub enum LifecycleCommand {
    /// List all lifecycles in the package
    List {
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Get a lifecycle by id
    Get {
        /// Lifecycle UUID id
        id: String,
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Create a lifecycle (reads full Lifecycle JSON from stdin)
    Create,
}

#[derive(Subcommand)]
pub enum TermCommand {
    /// List all terms from all package vocabularies
    List,
    /// Get a term by id
    Get {
        /// Term UUID id
        id: String,
    },
}

#[derive(Subcommand)]
pub enum ThemeCommand {
    /// List theme definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Get a theme definition by ID
    Get {
        /// Theme ID
        id: String,
    },
    /// Create a new theme definition (reads JSON from stdin)
    Create {
        /// Package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
    },
    /// Update a theme definition (reads full JSON from stdin)
    Update {
        /// Theme ID
        id: String,
    },
    /// Delete a theme definition by ID
    Delete {
        /// Theme ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum ViewCommand {
    /// List view (L1) definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by compatible semantic object type hint
        #[arg(long = "type-id")]
        type_id: Option<String>,
    },
    /// Get a view definition by ID
    Get {
        /// View ID
        id: String,
    },
    /// Create a new view definition (reads JSON from stdin)
    Create {
        /// Package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
    },
    /// Update a view definition (reads full JSON from stdin)
    Update {
        /// View ID
        id: String,
    },
    /// Delete a view definition by ID
    Delete {
        /// View ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum DocumentViewCommand {
    /// List document view (L2) definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by containerType
        #[arg(long = "container-type")]
        container_type: Option<String>,
        /// Filter to views whose rootTypeRefs include this Type UUID (RFC-009)
        #[arg(long = "root-type")]
        root_type: Option<String>,
    },
    /// Get a document view definition by ID
    Get {
        /// DocumentView ID
        id: String,
    },
    /// Create a new document view definition (reads JSON from stdin)
    Create {
        /// Package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
    },
    /// Update a document view definition (reads full JSON from stdin)
    Update {
        /// DocumentView ID
        id: String,
    },
    /// Delete a document view definition by ID
    Delete {
        /// DocumentView ID
        id: String,
    },
    /// List DocumentViews whose rootTypeRefs match the root instance type of a container
    #[command(name = "list-for-container")]
    ListForContainer {
        /// Container ID
        container_id: String,
    },
}

#[derive(Subcommand)]
pub enum FieldCommand {
    /// List field definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
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
        /// Package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Update an existing field definition by ID (reads full field JSON from stdin)
    Update {
        /// Field definition ID
        id: String,
    },
    /// Delete a field definition by ID
    Delete {
        /// Field definition ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum TypeCommand {
    /// List type definitions
    List {
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
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
    /// Create a type definition (reads JSON from stdin)
    Create {
        /// Package boundary path (omit for primary package, pass path for sub-package)
        #[arg(long)]
        package: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Update an existing type definition by ID (reads full type JSON from stdin)
    Update {
        /// Type definition ID
        id: String,
    },
    /// Delete a type definition by ID
    Delete {
        /// Type definition ID
        id: String,
        /// Version to delete (defaults to latest)
        #[arg(long)]
        version: Option<u32>,
    },
    /// Emit a draft-07 JSON Schema for a record's fieldValues of this Type
    Schema {
        /// Type definition ID
        id: String,
        /// Type version (defaults to latest)
        #[arg(long)]
        type_version: Option<u32>,
    },
}

#[derive(Subcommand)]
pub enum RecordCommand {
    /// List records, optionally filtered by type (namespace/name format). Omit --type to list all records.
    List {
        /// Type filter (namespace/name format, e.g. com.example/my-type). Omit to list all records.
        #[arg(long = "type", visible_alias = "type-filter")]
        type_filter: Option<String>,
        /// Filter to only records that carry this tag key in their manifest index.
        #[arg(long)]
        tag: Option<String>,
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
        /// Output directory relative to repo root (defaults to records/tier-2)
        #[arg(long)]
        dir: Option<String>,
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
    /// Validate a record input from stdin without persisting (editor preflight)
    Validate,
    /// Delete a record by ID
    Delete {
        /// Record instance ID
        id: String,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Transition a record's lifecycle state (ext:lifecycle)
    Transition {
        /// Record instance ID
        #[arg(long)]
        id: String,
    },
    /// Create a successor record that supersedes or refines this one (ext:lifecycle)
    Successor {
        /// Record instance ID of the predecessor
        #[arg(long)]
        id: String,
    },
    /// Revision management commands (ext:addressability)
    #[command(subcommand)]
    Revision(RecordRevisionCommand),
    /// Tag management commands for tier-2 records
    #[command(subcommand)]
    Tag(RecordTagCommand),
}

#[derive(Subcommand)]
pub enum RecordTagCommand {
    /// Add a tag to a tier-2 record
    Add {
        /// Record instance ID
        id: String,
        /// Tag key to add (e.g. construct:field, concern:lifecycle)
        tag: String,
    },
    /// Remove a tag from a tier-2 record
    Remove {
        /// Record instance ID
        id: String,
        /// Tag key to remove
        tag: String,
    },
    /// List all distinct tags used across tier-2 records, with record counts
    List,
}

#[derive(Subcommand)]
pub enum RecordRevisionCommand {
    /// List revisions for a record instance
    List {
        /// Record instance ID
        #[arg(long)]
        id: String,
        /// Optional field ID filter
        #[arg(long = "field-id")]
        field_id: Option<String>,
        /// Maximum number of revisions to return
        #[arg(long)]
        limit: Option<usize>,
        /// Number of revisions to skip (pagination)
        #[arg(long)]
        offset: Option<usize>,
    },
    /// Get a single revision by revision ID
    Get {
        /// Record instance ID
        #[arg(long)]
        id: String,
        /// Revision ID
        #[arg(long = "revision-id")]
        revision_id: String,
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
    /// Create a new protocol definition (reads JSON from stdin)
    Create {
        /// Target sub-package path, e.g. "package/ext". Defaults to primary package.
        #[arg(long)]
        package: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Import a protocol definition (reads JSON from stdin; alias for `create`)
    Import {
        /// Target sub-package path, e.g. "package/ext". Defaults to primary package.
        #[arg(long)]
        package: Option<String>,
        /// Deprecated: JSON output is now the default (no-op)
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Update a protocol definition (full replace; reads JSON from stdin)
    Update {
        /// Protocol ID (protocolId)
        id: String,
    },
    /// Delete a protocol definition
    Delete {
        /// Protocol instance ID
        id: String,
    },
    /// Find the first protocol whose target type matches the given type ID
    FindByTargetType {
        /// Type ID to match against protocolTargetType
        #[arg(long, name = "type-id")]
        type_id: String,
    },
}

#[derive(Subcommand)]
pub enum BlueprintCommand {
    /// List blueprint definitions
    List,
    /// Get a blueprint definition by ID
    Get {
        /// Blueprint definition ID (UUID)
        id: String,
    },
    /// Create a new blueprint definition (reads JSON from stdin)
    Create {
        /// Target sub-package path, e.g. "package/ext". Defaults to primary package.
        #[arg(long)]
        package: Option<String>,
    },
    /// Update an existing blueprint definition (reads JSON from stdin)
    Update {
        /// Blueprint definition ID (UUID)
        id: String,
    },
    /// Delete a blueprint definition by ID
    Delete {
        /// Blueprint definition ID (UUID)
        id: String,
    },
    /// Validate a blueprint definition
    Validate {
        /// Blueprint definition ID (UUID)
        id: String,
    },
    /// List the relation structure declared by a blueprint
    Structure {
        /// Blueprint definition ID (UUID)
        id: String,
    },
    /// Emit a nested draft-07 JSON Schema for a whole multi-record document declared by this Blueprint
    Schema {
        /// Blueprint definition ID (UUID)
        id: String,
    },
    /// Compose full layered guidance context (aiGuidance + fields + protocol) for a Blueprint
    Brief {
        /// Blueprint definition ID (UUID)
        id: String,
    },
}

#[derive(Subcommand)]
pub enum RenderCommand {
    /// Render a document view
    DocumentView {
        /// DocumentView UUID
        #[arg(long = "view")]
        view: String,
        /// Optional render format override (markdown, text, adoc)
        #[arg(long = "view-format")]
        view_format: Option<String>,
        /// Optional named theme variant defined on the DocumentView
        #[arg(long = "theme-variant")]
        theme_variant: Option<String>,
        /// Optional output file path for rendered content
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum PackageCommand {
    /// List package boundaries (primary + declared sub-packages)
    List,
    /// Create a new sub-package boundary
    Create {
        /// Package UUID
        #[arg(long = "id")]
        id: String,
        /// Package namespace (e.g. com.example)
        #[arg(long)]
        namespace: String,
        /// Package name (kebab-case)
        #[arg(long)]
        name: String,
        /// Package version (semver, e.g. 1.0.0)
        #[arg(long, default_value = "1.0.0")]
        version: String,
        /// Boundary path relative to repo root (e.g. package/my-ext)
        #[arg(long = "path")]
        boundary_path: String,
    },
    /// Import a pre-existing local directory as a package boundary
    Import {
        /// Path relative to repo root of a directory containing a package.json
        #[arg(long = "path")]
        path: String,
    },
    /// Update package boundary metadata (namespace, name, or version)
    Update {
        /// Boundary path (omit for primary package)
        #[arg(long = "selector")]
        selector: Option<String>,
        /// New namespace
        #[arg(long)]
        namespace: Option<String>,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New version
        #[arg(long)]
        version: Option<String>,
    },
    /// Create a new package slice (alias for create; permanent alias, not intended to diverge)
    SliceCreate {
        /// Package UUID
        #[arg(long = "id")]
        id: String,
        /// Package namespace (e.g. com.example)
        #[arg(long)]
        namespace: String,
        /// Package name (kebab-case)
        #[arg(long)]
        name: String,
        /// Package version (semver, e.g. 1.0.0)
        #[arg(long, default_value = "1.0.0")]
        version: String,
        /// Boundary path relative to repo root (e.g. package/my-ext)
        #[arg(long = "path")]
        boundary_path: String,
    },
    /// [Deprecated: use `package import` instead] Enable a local sub-package
    #[command(hide = true)]
    Enable {
        /// Relative path to the sub-package directory (e.g. package/spec-authoring-core)
        path: String,
    },
    /// [Deprecated: use `package import` instead] Disable a local sub-package
    #[command(hide = true)]
    Disable {
        /// Relative path to the sub-package directory (e.g. package/spec-authoring-core)
        path: String,
    },
}

#[derive(Args)]
pub struct TreeArgs {
    /// Start the tree from this instance ID (repeatable; omit to auto-detect top-level records)
    #[arg(long = "from", action = clap::ArgAction::Append)]
    pub from: Vec<String>,
    /// Edge type to follow for parent → child traversal (default: contains)
    #[arg(long = "relation-type", default_value = "contains")]
    pub relation_type: String,
    /// Maximum recursion depth (omit for unlimited)
    #[arg(long = "depth")]
    pub depth: Option<u32>,
    /// Only show records whose namespace/name matches this type (e.g. com.example/section)
    #[arg(long = "type")]
    pub type_filter: Option<String>,
}

/// Flags for `srs find` — the `ext:discovery` query axes. Container scope comes from
/// the global `--container`. Unspecified axes are wildcards.
#[derive(Args)]
pub struct FindArgs {
    /// Free-text recall-floor search over the record's text projection
    #[arg(long = "text")]
    pub text: Option<String>,
    /// Exact match on Record.typeId
    #[arg(long = "type-id")]
    pub type_id: Option<String>,
    /// Exact match on Record.typeNamespace
    #[arg(long = "type-namespace")]
    pub type_namespace: Option<String>,
    /// Exact match on Record.typeName
    #[arg(long = "type-name")]
    pub type_name: Option<String>,
    /// Tag predicate (repeatable; AND-conjunction — instance must carry all)
    #[arg(long = "tag", action = clap::ArgAction::Append)]
    pub tag: Vec<String>,
    /// Exact match on Record.lifecycleState (requires ext:lifecycle)
    #[arg(long = "lifecycle-state")]
    pub lifecycle_state: Option<String>,
    /// Instance tier filter (0=Note, 1=TypedRecord, 2=Record). Phase 1 serves Tier 2.
    #[arg(long = "tier")]
    pub tier: Option<u8>,
}

pub fn dispatch(cli: Cli) -> Result<String> {
    // repo create targets explicit --repo or current dir; it must not require existing .srs.
    let location = match &cli.command {
        Commands::Repo(RepoCommand::Create { .. }) => {
            resolve_repo_for_create(cli.repo.clone(), cli.store)?
        }
        Commands::Repo(RepoCommand::Copy { .. }) | Commands::Repo(RepoCommand::Diff { .. }) => {
            match &cli.repo {
                Some(path) => RepositoryLocation {
                    path: path.clone(),
                    store: cli.store.unwrap_or_else(|| infer_store_from_location(path)),
                },
                None => RepositoryLocation {
                    path: std::env::current_dir()?,
                    store: cli.store.unwrap_or(StoreBackend::File),
                },
            }
        }
        _ => resolve_repo(cli.repo.clone(), cli.store)?,
    };

    // Build context for command handlers
    let ctx = CliContext {
        repo: location.path,
        store: location.store,
        format: cli.format,
        pretty: cli.pretty,
        container_id: cli.container_id,
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
        Commands::Blueprint(bp_cmd) => blueprint::dispatch(ctx, bp_cmd),
        Commands::Container(cmd) => container::dispatch(ctx, cmd),
        Commands::Render(cmd) => render::dispatch(ctx, cmd),
        Commands::Package(pkg_cmd) => package::dispatch(ctx, pkg_cmd),
        Commands::Theme(theme_cmd) => theme::dispatch(ctx, theme_cmd),
        Commands::View(view_cmd) => view::dispatch(ctx, view_cmd),
        Commands::DocumentView(dv_cmd) => document_view::dispatch(ctx, dv_cmd),
        Commands::Vocabulary(vocab_cmd) => vocabulary::dispatch(ctx, vocab_cmd),
        Commands::Lifecycle(lc_cmd) => lifecycle::dispatch(ctx, lc_cmd),
        Commands::Term(term_cmd) => term::dispatch(ctx, term_cmd),
        Commands::Tree(args) => tree::dispatch(ctx, args),
        Commands::Find(args) => find::dispatch(ctx, args),
    }
}
