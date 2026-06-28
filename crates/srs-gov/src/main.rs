use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashSet;

mod governance;
mod render;
mod srs;

use governance::{by_key, match_container, GOVERNANCE_CONTAINERS};
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
    #[arg(long, default_value = ".")]
    repo: String,

    /// Print the underlying srs command(s) instead of running them
    #[arg(long)]
    explain: bool,

    /// Print raw srs JSON envelopes instead of friendly output
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List members of a governance container (view-on-container)
    #[command(name = "list")]
    List {
        /// Container key (e.g. decision_log, articles, roles)
        key: String,
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
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => cmd_top(&cli.repo, cli.explain, cli.json),
        Some(Commands::List { key }) => cmd_list(&key, &cli.repo, cli.explain, cli.json),
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
    }
}

// ---------------------------------------------------------------------------
// Top-level: list governance containers
// ---------------------------------------------------------------------------

fn cmd_top(repo: &str, explain: bool, json: bool) -> Result<()> {
    if explain {
        println!("# Underlying srs command:");
        run_srs(&["container", "list"], repo, true, false)?;
        return Ok(());
    }

    let payload = run_srs(&["container", "list"], repo, false, json)?;
    if json {
        return Ok(());
    }

    let containers = payload["containers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("unexpected container list payload"))?;

    let mut used_keys: HashSet<&'static str> = HashSet::new();
    let mut rows: Vec<ContainerRow> = Vec::new();

    for c in containers {
        let ct = c["containerType"].as_str();
        let title = c["title"].as_str().unwrap_or("");
        let container_id = c["containerId"].as_str().unwrap_or("").to_string();
        let member_count = c["memberInstanceIds"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);

        if let Some(def) = match_container(ct, title, &mut used_keys) {
            rows.push(ContainerRow {
                icon: def.icon,
                key: def.key.to_string(),
                container_type: def.container_type.to_string(),
                member_count,
                container_id,
            });
        }
    }

    if rows.is_empty() {
        println!("No governance containers found in {repo}");
        println!("(Expected containerType values: document, decision_log)");
        return Ok(());
    }

    container_list(&rows);
    Ok(())
}

// ---------------------------------------------------------------------------
// <key> list — render view on container
// ---------------------------------------------------------------------------

fn cmd_list(key: &str, repo: &str, explain: bool, json: bool) -> Result<()> {
    let def = by_key(key)
        .ok_or_else(|| anyhow::anyhow!("unknown key '{key}'. Known: {}", known_keys()))?;

    // 1. Find the container id
    let container_id = resolve_container_id(def.container_type, repo)?;

    if explain {
        println!("# Underlying srs commands:");
        println!("# 1. resolve container id (container list)");
        println!("#    container_id = {container_id}");
        println!("# 2. find applicable document-view:");
        run_srs(
            &["document-view", "list-for-container", &container_id],
            repo,
            true,
            false,
        )?;
        println!("# 3. render (example — actual viewId resolved at runtime):");
        println!("  srs --repo {repo} --format json render document-view --view <viewId> --container {container_id} --view-format text");
        println!("# 4. member id index:");
        run_srs(
            &["container", "members", "list", &container_id],
            repo,
            true,
            false,
        )?;
        return Ok(());
    }

    // 2. Find applicable document-view
    let views_payload = run_srs(
        &["document-view", "list-for-container", &container_id],
        repo,
        false,
        false,
    )?;
    let views = views_payload["documentViews"].as_array();
    let view_id = views
        .and_then(|vs| vs.first())
        .and_then(|v| v["id"].as_str())
        .map(|s| s.to_string());

    if let Some(ref vid) = view_id {
        // 3. Render view
        let render_payload = run_srs(
            &[
                "render",
                "document-view",
                "--view",
                vid,
                "--container",
                &container_id,
                "--view-format",
                "text",
            ],
            repo,
            false,
            json,
        )?;

        if !json {
            if let Some(rendered) = render_payload["rendered"].as_str() {
                println!();
                println!("{rendered}");
            }
        }
    } else {
        println!("(No document-view found for this container — falling back to member list)");
    }

    // 4. Member id index (always shown, even when we have a rendered view)
    if !json {
        let members_payload = run_srs(
            &["container", "members", "list", &container_id],
            repo,
            false,
            false,
        )?;
        // Exclude root instances
        let container_payload = run_srs(&["container", "get", &container_id], repo, false, false)?;
        let roots: HashSet<String> = container_payload["container"]["rootInstanceIds"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let members: Vec<&str> = members_payload["memberInstanceIds"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let non_root: Vec<&&str> = members.iter().filter(|id| !roots.contains(**id)).collect();

        if !non_root.is_empty() {
            section("Member IDs  (use with: srs-gov get)");
            for id in &non_root {
                println!("  {id}");
            }
            println!();
        }
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

    let container_id = resolve_container_id(def.container_type, repo)?;

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
    let mut fv_entries: Vec<String> = Vec::new();
    for (_, name, fid, req) in &fields {
        let placeholder = match name.as_str() {
            "title" => title.unwrap_or("<TITLE>"),
            "decision_statement" => statement.unwrap_or("<DECISION STATEMENT>"),
            _ if *req => "<REQUIRED>",
            _ => continue,
        };
        fv_entries.push(format!(
            r#"    {{ "fieldId": "{fid}", "value": "{placeholder}" }}"#
        ));
    }

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
    println!("{{");
    println!("  \"fieldValues\": [");
    println!("{}", fv_entries.join(",\n"));
    println!("  ]");
    println!("}}");
    println!("EOF");
    println!();

    if explain {
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

/// Resolve a containerId for a given containerType by calling `srs container list`.
fn resolve_container_id(container_type: &str, repo: &str) -> Result<String> {
    let payload = run_srs(&["container", "list"], repo, false, false)?;
    let containers = payload["containers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no containers found in repo"))?;

    containers
        .iter()
        .find(|c| c["containerType"].as_str() == Some(container_type))
        .and_then(|c| c["containerId"].as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("no container with type '{container_type}' found in {repo}"))
}
