use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use srs_repository::governance_scaffold_service::{
    create_governance_repository, CreateGovernanceRepositoryInput,
};
use srs_repository::srsj_migration_service;
use srs_repository::JsonStore;
use std::collections::HashSet;

mod find_query;
mod governance;
mod render;
mod srs;
mod tui_app;
mod tui_data;
mod tui_input;
mod tui_render;
mod tui_state;

use governance::{by_key, by_root_type, GOVERNANCE_CONTAINERS};
use render::{container_list, record_detail, section, ContainerRow};
use srs::run_srs;

/// Governance-flow exploration CLI.
///
/// Composes `srs` commands into a friendly governance verb set.
/// Target data: srs/docs/spec/examples/gallery-project-v2
/// (or gallery.srsj once migrated — see the-greenman/srs#91).
#[derive(Parser)]
#[command(name = "srs-gov", version, about)]
struct Cli {
    /// Repository path (forwarded to srs as --repo)
    #[arg(long, global = true, default_value = ".")]
    repo: String,

    /// Print the underlying srs command(s) instead of running them
    #[arg(long, global = true)]
    explain: bool,

    /// Print raw srs JSON envelopes instead of friendly output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List members of a governance container (view-on-container).
    ///
    /// Runtime filters (--search/--tag/--all) shape the human-readable list. With --json
    /// the unfiltered `container resolve-view` envelope is printed (filters are not applied);
    /// use --explain to see the composed `srs find` command for the filtered data.
    #[command(name = "list")]
    List {
        /// Container key (e.g. decision_log, articles, roles)
        key: String,
        /// Free-text search over member content (forwarded to `srs find --text`)
        #[arg(long)]
        search: Option<String>,
        /// Narrow to members carrying this tag (repeatable; forwarded to `srs find --tag`)
        #[arg(long)]
        tag: Vec<String>,
        /// Show all members, including the view's default-hidden lifecycle states
        /// (drops the authored excludeLifecycleStates exclusion)
        #[arg(long)]
        all: bool,
    },
    /// Get a record from a governance container
    #[command(name = "get")]
    Get {
        /// Container key
        key: String,
        /// Instance ID (or unique prefix)
        id: String,
    },
    /// Dry-run: print the srs command to create a new member record
    #[command(name = "create")]
    Create {
        /// Container key
        key: String,
        /// Child type to create (e.g. "decision")
        child: String,
        /// Value for the title field
        #[arg(long)]
        title: Option<String>,
        /// Value for the decision_statement field (decisions only)
        #[arg(long)]
        statement: Option<String>,
    },
    /// Create a new governance repository from the canonical seed
    #[command(name = "repo-create")]
    RepoCreate {
        /// Output path for the new .srsj file
        #[arg(long, default_value = "governance.srsj")]
        output: String,
        /// Organisation name (becomes the repository title and charter article title)
        #[arg(long, default_value = "Governance Document")]
        title: String,
        /// Purpose statement written into the charter article
        #[arg(long)]
        purpose: Option<String>,
    },
    /// Open the read-only governance terminal UI
    #[command(name = "tui")]
    Tui {
        /// Render the first frame through a test backend and exit
        #[arg(long)]
        smoke: bool,
    },
    /// Source document attachment commands
    #[command(name = "attachment")]
    Attachment {
        #[command(subcommand)]
        command: AttachmentSubcommand,
    },
}

#[derive(Subcommand)]
enum AttachmentSubcommand {
    /// Add a file as a source-document attachment to this repository
    #[command(name = "add")]
    Add {
        /// Path to the local source file to store
        source: std::path::PathBuf,
        /// Optional subdirectory within source-documents/ (e.g. "phase-1")
        #[arg(long)]
        subdir: Option<String>,
        /// Optional human-readable title for the attachment
        #[arg(long)]
        title: Option<String>,
        /// MIME type override (auto-detected from file extension if omitted)
        #[arg(long = "content-type")]
        content_type: Option<String>,
    },
    /// List source-document attachments in this repository
    #[command(name = "list")]
    List,
}

fn main() {
    if let Err(e) = run() {
        // {:#} prints the full anyhow context chain (not just the top-level
        // .context() message) — the underlying srs diagnostics were previously
        // swallowed, leaving only e.g. "error: load repository navigation".
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => cmd_top(&cli.repo, cli.explain, cli.json),
        Some(Commands::List {
            key,
            search,
            tag,
            all,
        }) => cmd_list(
            &key,
            &cli.repo,
            cli.explain,
            cli.json,
            search.as_deref(),
            &tag,
            all,
        ),
        Some(Commands::Get { key, id }) => cmd_get(&key, &id, &cli.repo, cli.explain, cli.json),
        Some(Commands::Create {
            key,
            child,
            title,
            statement,
        }) => cmd_create(
            &key,
            &child,
            title.as_deref(),
            statement.as_deref(),
            &cli.repo,
            cli.explain,
        ),
        Some(Commands::RepoCreate {
            output,
            title,
            purpose,
        }) => cmd_repo_create(&output, &title, purpose.as_deref()),
        Some(Commands::Tui { smoke }) => tui_app::run_tui(&cli.repo, smoke),
        Some(Commands::Attachment { command }) => {
            cmd_attachment(&cli.repo, cli.explain, cli.json, command)
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level: list governance containers
// ---------------------------------------------------------------------------

fn cmd_top(repo: &str, explain: bool, json: bool) -> Result<()> {
    if explain {
        println!("# Underlying srs command:");
        run_srs(&["repo", "navigation"], repo, true, false)?;
        return Ok(());
    }

    let payload = run_srs(&["repo", "navigation"], repo, false, json)?;
    if json {
        return Ok(());
    }

    let nav = &payload["navigation"];
    let identity_label = nav["identity"]["displayLabel"]
        .as_str()
        .unwrap_or("(untitled)");
    let empty_sections = vec![];
    let sections = nav["sections"].as_array().unwrap_or(&empty_sections);

    let mut rows: Vec<ContainerRow> = Vec::new();
    for section in sections {
        let type_ns = section["typeNamespace"].as_str().unwrap_or("");
        let type_name = section["typeName"].as_str().unwrap_or("");
        let section_container_id = section["sectionContainerId"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let def_opt = by_root_type(type_ns, type_name);
        let key = def_opt.map(|d| d.key).unwrap_or(type_name);
        let icon = def_opt.map(|d| d.icon).unwrap_or("·");
        // Degrade gracefully: a single unreadable container must not abort the listing.
        let member_count = container_member_count(repo, &section_container_id).unwrap_or(0);
        rows.push(ContainerRow {
            icon,
            key: key.to_string(),
            container_type: type_name.to_string(),
            member_count,
            container_id: section_container_id,
        });
    }

    if rows.is_empty() {
        println!("No governance sections found in {repo}");
        println!("(Has srs repo set-root-container been run?)");
        return Ok(());
    }

    container_list(&format!("Governance   —   {identity_label}"), &rows);
    Ok(())
}

// ---------------------------------------------------------------------------
// <key> list — render view on container
// ---------------------------------------------------------------------------

fn cmd_list(
    key: &str,
    repo: &str,
    explain: bool,
    json: bool,
    search: Option<&str>,
    tags: &[String],
    all: bool,
) -> Result<()> {
    let def = by_key(key)
        .ok_or_else(|| anyhow::anyhow!("unknown key '{key}'. Known: {}", known_keys()))?;

    // 1. Find the container id
    let container_id = resolve_container_id(def, repo)?;

    // Authored list = container resolve-view (columns + ordered members + authored
    // default-hidden states, srs-rust#254 / ADR-020) composed with a runtime srs find
    // query (lifecycle exclusion + content/tag, #217). The authored excludeLifecycleStates
    // are applied unless --all; --search → find --text; --tag → find --tag. The interactive
    // result is the resolve-view members intersected with the find hit set.
    let payload = run_srs(
        &["container", "resolve-view", &container_id],
        repo,
        false,
        json,
    )?;
    let cv = &payload["containerView"];

    // Authored default-hidden lifecycle states (empty unless the view is a type-query).
    let authored_excludes: Vec<String> = cv["excludeLifecycleStates"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let effective_excludes: Vec<&str> = if all {
        Vec::new()
    } else {
        authored_excludes.iter().map(String::as_str).collect()
    };

    if explain {
        println!("# Underlying srs commands (resolve-view srs-rust#254, find #217):");
        run_srs(&["repo", "navigation"], repo, true, false)?;
        run_srs(
            &["container", "resolve-view", &container_id],
            repo,
            true,
            false,
        )?;
        if find_query::needs_find_query(&effective_excludes, search, tags) {
            let find_args =
                find_query::build_find_args(&container_id, &effective_excludes, search, tags);
            let refs: Vec<&str> = find_args.iter().map(String::as_str).collect();
            run_srs(&refs, repo, true, false)?;
        }
        return Ok(());
    }

    if json {
        // Structural view envelope already printed by run_srs above (json=true). Runtime
        // filters apply to the human-readable output; for raw filtered data use `srs find`
        // (shown by --explain).
        return Ok(());
    }

    // Resolve the runtime hit set (instanceIds surviving the find query), if any.
    // A find query is only issued when a runtime filter is active (exclusion, search, or tag).
    // With none active the authored member list is shown verbatim — preserving the
    // pre-#298 output (and keeping a container-subset view, which has no exclusion, identical).
    let allowed =
        find_query::resolve_hit_set(repo, &container_id, &effective_excludes, search, tags)?;

    let root_label = cv["root"]["displayLabel"].as_str().unwrap_or("");
    let columns: Vec<(&str, &str)> = cv["columns"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some((
                        c["displayLabel"].as_str()?,
                        c["fieldId"].as_str().unwrap_or(""),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    // root
    let root_id = cv["root"]["instanceId"].as_str().unwrap_or("");
    println!();
    println!("  {} — {root_label}", &root_id[..8.min(root_id.len())]);
    println!();

    // header from column spec
    if !columns.is_empty() {
        let col_labels: Vec<&str> = columns.iter().map(|(l, _)| *l).collect();
        println!("  {}", col_labels.join("  ·  "));
        println!("  {}", "─".repeat(70));
    }

    // members (excluding root), intersected with the find hit set when a runtime filter
    // is active (display = resolve-view members ∩ find hits, in resolve-view order).
    let root_id_full = cv["root"]["instanceId"].as_str().unwrap_or("");
    let members = cv["members"].as_array();
    let non_root: Vec<&serde_json::Value> = members
        .map(|a| {
            a.iter()
                .filter(|m| m["instanceId"].as_str() != Some(root_id_full))
                .filter(|m| match (&allowed, m["instanceId"].as_str()) {
                    (Some(set), Some(iid)) => set.contains(iid),
                    (Some(_), None) => false,
                    (None, _) => true,
                })
                .collect()
        })
        .unwrap_or_default();

    for m in &non_root {
        let iid = m["instanceId"].as_str().unwrap_or("");
        let label = m["displayLabel"].as_str().unwrap_or("(untitled)");
        println!("  {:<8}  {label}", &iid[..8.min(iid.len())]);
    }
    println!();

    // ID index for use with srs-gov get
    if !non_root.is_empty() {
        section("Member IDs  (use with: srs-gov get)");
        for m in &non_root {
            if let Some(iid) = m["instanceId"].as_str() {
                println!("  {iid}");
            }
        }
        println!();
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// <key> get <id>
// ---------------------------------------------------------------------------

fn cmd_get(key: &str, id: &str, repo: &str, explain: bool, json: bool) -> Result<()> {
    let _def = by_key(key)
        .ok_or_else(|| anyhow::anyhow!("unknown key '{key}'. Known: {}", known_keys()))?;

    if explain {
        println!("# Underlying srs commands:");
        run_srs(&["record", "get", id], repo, true, false)?;
        println!("# then: srs type schema <typeId> to get field display labels");
        return Ok(());
    }

    // 1. Fetch the record
    let record_payload = run_srs(&["record", "get", id], repo, false, json)?;
    if json {
        return Ok(());
    }

    let record = &record_payload["record"];
    let type_id = record["typeId"].as_str().unwrap_or("");
    let type_version = record["typeVersion"].as_u64().unwrap_or(1);
    let field_values: Vec<serde_json::Value> = record["fieldValues"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    // 2. Fetch the type schema for labels and order
    let tv = type_version.to_string();
    let schema_payload = run_srs(
        &["type", "schema", type_id, "--type-version", &tv],
        repo,
        false,
        false,
    )?;
    let schema_props = &schema_payload["schema"];

    record_detail(id, schema_props, &field_values);
    Ok(())
}

// ---------------------------------------------------------------------------
// <key> create <child>  (dry-run)
// ---------------------------------------------------------------------------

fn cmd_create(
    key: &str,
    child: &str,
    title: Option<&str>,
    statement: Option<&str>,
    repo: &str,
    explain: bool,
) -> Result<()> {
    let def = by_key(key)
        .ok_or_else(|| anyhow::anyhow!("unknown key '{key}'. Known: {}", known_keys()))?;

    // Resolve child type
    let type_ref = def
        .creatable
        .iter()
        .find(|(name, _)| *name == child)
        .map(|(_, ns)| *ns)
        .ok_or_else(|| {
            let available: Vec<&str> = def.creatable.iter().map(|(n, _)| *n).collect();
            anyhow::anyhow!(
                "unknown child type '{child}' for '{key}'. Available: {}",
                available.join(", ")
            )
        })?;

    let container_id = resolve_container_id(def, repo)?;

    // Resolve namespace/name → UUID so type schema can look it up
    let (ns, name) = type_ref
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("expected 'namespace/name' in type_ref: {type_ref}"))?;
    let (type_uuid, type_version) = resolve_type_id(ns, name, repo)?;

    // Fetch type schema to discover required fields and fieldIds
    let tv = type_version.to_string();
    let schema_payload = run_srs(
        &["type", "schema", &type_uuid, "--type-version", &tv],
        repo,
        false,
        false,
    )?;

    let props = schema_payload["schema"]["properties"].as_object();
    let required_arr = schema_payload["schema"]["required"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let required_set: HashSet<&str> = required_arr.iter().filter_map(|v| v.as_str()).collect();

    // Build ordered field list
    let mut fields: Vec<(i64, String, String, bool)> = Vec::new();
    if let Some(p) = props {
        for (name, prop) in p {
            let order = prop["x-srs-order"].as_i64().unwrap_or(99);
            let fid = match prop["x-srs-field-id"].as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            let req = required_set.contains(name.as_str());
            fields.push((order, name.clone(), fid, req));
        }
        fields.sort_by_key(|f| f.0);
    }

    // Build field values JSON
    let mut fv_entries: Vec<serde_json::Value> = Vec::new();
    for (_, name, fid, req) in &fields {
        let placeholder = match name.as_str() {
            "title" => title.unwrap_or("<TITLE>"),
            "decision_statement" => statement.unwrap_or("<DECISION STATEMENT>"),
            _ if *req => "<REQUIRED>",
            _ => continue,
        };
        fv_entries.push(serde_json::json!({
            "fieldId": fid,
            "value": placeholder,
        }));
    }
    let input = serde_json::json!({ "fieldValues": fv_entries });
    let input_json = serde_json::to_string_pretty(&input)?;

    println!();
    println!(
        "# Dry-run: command to create a new {child} in {}",
        def.label
    );
    println!("# Run this to write. Nothing is written now.");
    println!("#");
    println!("# The --container flag creates the record AND adds it to the");
    println!("# container in one step. Lifecycle defaults to 'draft'.");
    println!();
    println!(
        "srs record create --type {type_ref} --container {container_id} --repo {repo} <<'EOF'"
    );
    println!("{input_json}");
    println!("EOF");
    println!();

    if explain {
        println!("# Container resolved via:");
        run_srs(&["repo", "navigation"], repo, true, false)?;
        println!("# Schema lookup used:");
        run_srs(
            &["type", "schema", &type_uuid, "--type-version", &tv],
            repo,
            true,
            false,
        )?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Look up a type by namespace + name and return (UUID, version).
fn resolve_type_id(namespace: &str, name: &str, repo: &str) -> anyhow::Result<(String, u64)> {
    let payload = run_srs(&["type", "list"], repo, false, false)?;
    let types = payload["types"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("type list returned no types array"))?;
    types
        .iter()
        .find(|t| t["namespace"].as_str() == Some(namespace) && t["name"].as_str() == Some(name))
        .and_then(|t| {
            let id = t["id"].as_str()?.to_string();
            let ver = t["version"].as_u64()?;
            Some((id, ver))
        })
        .ok_or_else(|| anyhow::anyhow!("type '{namespace}/{name}' not found in repo"))
}

fn known_keys() -> String {
    GOVERNANCE_CONTAINERS
        .iter()
        .map(|d| d.key)
        .collect::<Vec<_>>()
        .join(", ")
}

fn container_member_count(repo: &str, container_id: &str) -> Result<usize> {
    let payload = run_srs(&["container", "get", container_id], repo, false, false)?;
    Ok(payload["container"]["memberInstanceIds"]
        .as_array()
        .map(|ids| ids.len())
        .unwrap_or(0))
}

/// Resolve the containerId for a governance section via `srs repo navigation`.
///
/// Matches on `typeNamespace`/`typeName` from the navigation UUID chain (RFC-009),
/// replacing the soft-deprecated `containerType` string filter.
fn resolve_container_id(def: &governance::ContainerTypeDef, repo: &str) -> Result<String> {
    let payload = run_srs(&["repo", "navigation"], repo, false, false)?;
    let empty_sections = vec![];
    let sections = payload["navigation"]["sections"]
        .as_array()
        .unwrap_or(&empty_sections);

    sections
        .iter()
        .find(|s| {
            s["typeNamespace"].as_str() == Some(def.root_type_namespace)
                && s["typeName"].as_str() == Some(def.root_type_name)
        })
        .and_then(|s| s["sectionContainerId"].as_str())
        .map(String::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no section of type '{}/{}' found via repository navigation in {repo}",
                def.root_type_namespace,
                def.root_type_name,
            )
        })
}

// ---------------------------------------------------------------------------
// attachment — add/list source-document attachments
// ---------------------------------------------------------------------------

fn cmd_attachment(repo: &str, explain: bool, json: bool, sub: AttachmentSubcommand) -> Result<()> {
    match sub {
        AttachmentSubcommand::Add {
            source,
            subdir,
            title,
            content_type,
        } => cmd_attachment_add(repo, explain, json, source, subdir, title, content_type),
        AttachmentSubcommand::List => cmd_attachment_list(repo, explain, json),
    }
}

fn cmd_attachment_add(
    repo: &str,
    explain: bool,
    json: bool,
    source: std::path::PathBuf,
    subdir: Option<String>,
    title: Option<String>,
    content_type: Option<String>,
) -> Result<()> {
    let source_str = source
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("source path is not valid UTF-8: {}", source.display()))?;

    // Collect positional + optional flag args. Owned strings must outlive the args slice.
    let mut arg_parts: Vec<String> = vec![
        "attachment".into(),
        "add".into(),
        source_str.to_string(),
    ];
    if let Some(s) = subdir {
        arg_parts.push("--subdir".into());
        arg_parts.push(s);
    }
    if let Some(t) = title {
        arg_parts.push("--title".into());
        arg_parts.push(t);
    }
    if let Some(ct) = content_type {
        arg_parts.push("--content-type".into());
        arg_parts.push(ct);
    }
    let args: Vec<&str> = arg_parts.iter().map(String::as_str).collect();

    let payload = run_srs(&args, repo, explain, json)?;
    if json || explain {
        return Ok(());
    }

    let content_path = payload["contentPath"].as_str().unwrap_or("");
    let document_id = payload["documentId"].as_str().unwrap_or("");
    let base_dir = payload["sourceDocumentsPath"]
        .as_str()
        .unwrap_or("source-documents");
    render::attachment_added(content_path, document_id, base_dir);
    Ok(())
}

fn cmd_attachment_list(repo: &str, explain: bool, json: bool) -> Result<()> {
    let payload = run_srs(&["attachment", "list"], repo, explain, json)?;
    if json || explain {
        return Ok(());
    }

    let base_dir = payload["sourceDocumentsPath"].as_str().unwrap_or("source-documents");
    let empty_entries = vec![];
    let entries = payload["entries"].as_array().unwrap_or(&empty_entries);
    render::attachment_list(base_dir, entries);
    Ok(())
}

// ---------------------------------------------------------------------------
// repo-create — stamp a new governance .srsj from the embedded seed
// ---------------------------------------------------------------------------

/// Canonical seed for com.mudemocracy.governance @1.0.0.
///
/// Vendored byte-copy of the deterministic seed artifact (ADR-017) — never hand-edit it.
/// Regenerate from the canonical package and re-vendor when the package is republished:
///
/// ```sh
/// # in the srs spec repo, with a built srs binary:
/// SRS_BIN=<srs-rust>/target/debug/srs node scripts/build-governance-seed.mjs
/// cp <srs>/packages/com.mudemocracy.governance/1.0.0/seed/empty-governance-document.srsj \
///    <srs-rust>/crates/srs-gov/assets/governance-seed.srsj
/// ```
///
/// `build-governance-seed.mjs --check` proves the seed rebuilds byte-for-byte (srs#38).
const GOVERNANCE_SEED: &str = include_str!("../assets/governance-seed.srsj");

fn cmd_repo_create(output: &str, title: &str, purpose: Option<&str>) -> Result<()> {
    use std::io::Write;

    let out_path = std::path::Path::new(output);
    if out_path.exists() {
        bail!("output path already exists: {output}");
    }

    let srsj_content = srsj_migration_service::migrate_rfc014(GOVERNANCE_SEED)
        .context("RFC-014 migration failed")?;
    let store = JsonStore::from_srsj(&srsj_content).context("failed to load seed into store")?;

    let result = create_governance_repository(
        &store,
        CreateGovernanceRepositoryInput {
            namespace: None,
            title: title.to_string(),
            purpose: purpose.map(str::to_string),
            repository_id: None,
        },
    )
    .context("failed to scaffold governance repository")?;

    let final_srsj = store
        .to_srsj_string()
        .context("failed to serialise store")?;
    std::fs::File::create(out_path)?.write_all(final_srsj.as_bytes())?;

    render::repo_created(output, title, &result.repository_id, purpose.is_some());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::GOVERNANCE_SEED;

    /// The vendored seed's decision-log DocumentView must carry the canonical authored
    /// default-hidden states (the whole point of #298 — regenerate the derived copy).
    #[test]
    fn seed_decision_log_view_is_type_query_with_excludes() {
        let seed: serde_json::Value =
            serde_json::from_str(GOVERNANCE_SEED).expect("embedded seed parses");
        let view = &seed["data"]["package/document-views/decision-log-b5c8d124.json"];
        assert!(!view.is_null(), "decision-log view present in seed");
        let source = &view["sections"][0]["source"];
        assert_eq!(
            source["type"], "type-query",
            "decision-log section must be a type-query (was the stale container-subset)"
        );
        let excludes: Vec<&str> = source["excludeLifecycleStates"]
            .as_array()
            .expect("excludeLifecycleStates array")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        // RFC-022 added the `abandoned` lifecycle state; the canonical view hides it by default.
        assert_eq!(excludes, vec!["superseded", "closed", "abandoned"]);
    }
}
