use anyhow::{bail, Result};
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

        if let Some(def) = match_container(ct, title, &mut used_keys) {
            // Degrade gracefully: a single unreadable container must not abort the
            // whole top-level listing (matches the prior summary-derived count).
            let member_count = container_member_count(repo, &container_id).unwrap_or(0);
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

    // A find query is only needed when a runtime filter is active (exclusion, search, or
    // tag). With none active the authored member list is shown verbatim — preserving the
    // pre-#298 output (and keeping a container-subset view, which has no exclusion, identical).
    let need_find = !effective_excludes.is_empty() || search.is_some() || !tags.is_empty();
    let find_args = build_find_args(&container_id, &effective_excludes, search, tags);

    if explain {
        println!("# Underlying srs commands (resolve-view srs-rust#254, find #217):");
        run_srs(
            &["container", "resolve-view", &container_id],
            repo,
            true,
            false,
        )?;
        if need_find {
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
    let allowed: Option<HashSet<String>> = if need_find {
        let refs: Vec<&str> = find_args.iter().map(String::as_str).collect();
        let find_payload = run_srs(&refs, repo, false, false)?;
        let hits = find_payload["result"]["hits"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|h| h["instanceId"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Some(hits)
    } else {
        None
    };

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

/// Build the `srs find` argument vector that composes with the authored view: the global
/// `--container` scope, then the `find` subcommand with the effective lifecycle exclusions,
/// optional `--text` search, and repeated `--tag` filters. Returned as owned `String`s so the
/// caller can borrow `&str` slices for both `--explain` printing and execution.
fn build_find_args(
    container_id: &str,
    excludes: &[&str],
    search: Option<&str>,
    tags: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--container".into(),
        container_id.to_string(),
        "find".into(),
    ];
    for state in excludes {
        args.push("--exclude-lifecycle-state".into());
        args.push((*state).to_string());
    }
    if let Some(text) = search {
        args.push("--text".into());
        args.push(text.to_string());
    }
    for tag in tags {
        args.push("--tag".into());
        args.push(tag.clone());
    }
    args
}

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

/// Resolve a containerId for a governance container.
///
/// Matches on `containerType` first; when multiple containers share the same type
/// (both `articles` and `roles` are `"document"`), disambiguates by comparing the
/// container's title against `def.label` (case-insensitive).
fn resolve_container_id(def: &governance::ContainerTypeDef, repo: &str) -> Result<String> {
    let payload = run_srs(&["container", "list"], repo, false, false)?;
    let containers = payload["containers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no containers found in repo"))?;

    let by_type: Vec<&serde_json::Value> = containers
        .iter()
        .filter(|c| c["containerType"].as_str() == Some(def.container_type))
        .collect();

    let matched = if by_type.len() == 1 {
        by_type.into_iter().next()
    } else {
        // Disambiguate by title (case-insensitive match against def.label). If more
        // than one container of this type carries the same title, the choice is
        // ambiguous — fail loudly rather than silently picking the first match.
        let title_matches: Vec<&serde_json::Value> = by_type
            .into_iter()
            .filter(|c| {
                c["title"]
                    .as_str()
                    .map(|t| t.eq_ignore_ascii_case(def.label))
                    .unwrap_or(false)
            })
            .collect();
        if title_matches.len() > 1 {
            return Err(anyhow::anyhow!(
                "ambiguous container for key '{}': {} containers of type '{}' have title '{}' in {repo}",
                def.key,
                title_matches.len(),
                def.container_type,
                def.label
            ));
        }
        title_matches.into_iter().next()
    };

    matched
        .and_then(|c| c["containerId"].as_str())
        .map(String::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no container matching key '{}' (type '{}', label '{}') found in {repo}",
                def.key,
                def.container_type,
                def.label
            )
        })
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

    // 1. Stamp seed: fresh repositoryId + title.
    let mut seed: serde_json::Value =
        serde_json::from_str(GOVERNANCE_SEED).context("failed to parse embedded seed")?;
    let new_id = uuid::Uuid::new_v4().to_string();
    seed["manifest"]["repositoryId"] = serde_json::Value::String(new_id.clone());
    seed["manifest"]["title"] = serde_json::Value::String(title.to_string());
    let json = serde_json::to_string_pretty(&seed)?;
    std::fs::File::create(out_path)?.write_all(json.as_bytes())?;

    // 2. Articles container + charter article root.
    let articles_id = srs_create_container(output, "document", "Articles")?;
    let charter_text = purpose.unwrap_or("Add your organisation's purpose statement here.");
    let charter_fv = serde_json::json!({
        "fieldValues": [
            { "fieldId": "d7e82557-9045-5e92-a494-d99112bbec4a", "value": title },
            { "fieldId": "8aa3eba2-204b-5ebd-ba7a-be0f066027d6", "value": charter_text }
        ]
    });
    let charter_id = srs_create_record(output, "governance/article", &charter_fv.to_string())?;
    srs_roots_add(output, &articles_id, &charter_id)?;

    // 3. Decision Log container + decision_log root record.
    let dl_title = format!("{title} Decision Log");
    let dl_id = srs_create_container(output, "decision_log", &dl_title)?;
    let dl_fv = serde_json::json!({
        "fieldValues": [
            { "fieldId": "d7e82557-9045-5e92-a494-d99112bbec4a", "value": dl_title }
        ]
    });
    let dl_root_id = srs_create_record(output, "governance/decision_log", &dl_fv.to_string())?;
    srs_roots_add(output, &dl_id, &dl_root_id)?;

    // 4. Roles container (no root record — first role the org assigns serves as root).
    srs_create_container(output, "document", "Roles")?;

    render::repo_created(output, title, &new_id, purpose.is_some());
    Ok(())
}

fn srs_create_container(repo: &str, container_type: &str, title: &str) -> Result<String> {
    let input = serde_json::json!({ "containerType": container_type, "title": title });
    let payload = srs::run_srs_write(&["container", "create"], repo, &input.to_string())?;
    payload["container"]["containerId"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("container create returned no containerId"))
}

fn srs_create_record(repo: &str, type_ref: &str, field_values_json: &str) -> Result<String> {
    let payload = srs::run_srs_write(
        &["record", "create", "--type", type_ref],
        repo,
        field_values_json,
    )?;
    payload["record"]["instanceId"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("record create returned no instanceId"))
}

fn srs_roots_add(repo: &str, container_id: &str, instance_id: &str) -> Result<()> {
    srs::run_srs_write(
        &["container", "roots", "add", container_id, instance_id],
        repo,
        "",
    )?;
    Ok(())
}

use anyhow::Context;

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
        assert_eq!(excludes, vec!["superseded", "closed"]);
    }
}
