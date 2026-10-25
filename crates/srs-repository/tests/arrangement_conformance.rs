//! RFC-043 arrangement conformance runner (srs-rust#1477).
//!
//! RFC-043's "Conformance fixture" section binds acceptance to `conformance/arrangement/` in the
//! `srs` spec repo, with the shape of `conformance/discovery/` (see `discovery_conformance.rs`):
//! a `fixture-repo/` and a `scenarios.json`. Until this test existed nothing ran
//! `scenarios.json` against this implementation. The fixture is consumed as external test data,
//! never vendored (CLAUDE.md).
//!
//! Each scenario works on a fresh temp copy of `fixture-repo/` and is dispatched on its shape
//! through the real services (`container_service`, `render_service`,
//! `repository_navigation_service`). An unhandled shape or expectation key fails loudly; it is
//! never skipped. A failing scenario is a finding (engine or fixture), never "fixed" by editing
//! `scenarios.json`, which is spec-repo-owned: list it in [`EXPECTED_FAILURES`] with its issue.
//!
//! `storage-scenarios.json` (RFC-051) is run by the spec repo against the pinned CLI and is not
//! this test's concern.

use serde_json::Value;
use srs_core::arrangement;
use srs_core::types::container::ContainerEntry;
use srs_repository::container_service::{self, ContainerPatch};
use srs_repository::render_service::{render_composition, RenderCompositionOptions};
use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::store::FileStore;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Scenarios known to fail against this implementation, each with the issue that tracks the
/// finding. Empty: every scenario currently passes. Adding an entry here without an issue link
/// is not allowed; a listed scenario that starts passing fails the test so the entry is removed.
const EXPECTED_FAILURES: &[(&str, &str)] = &[];

/// Keys of a scenario that carry no expectation or operation (documentation only).
const DESCRIPTIVE_KEYS: &[&str] = &["name", "rules", "description"];

/// Root of the `conformance/arrangement` fixture in the `srs` spec repo, or `None` when it is
/// not available. Same resolution rule as `discovery_conformance.rs` (srs-rust#874, #922):
/// prefer `SRS_SPEC_DIR`; the sibling walk-up panics on a non-master checkout instead of
/// trusting a stale fixture.
fn conformance_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SRS_SPEC_DIR") {
        let p = PathBuf::from(dir).join("conformance/arrangement");
        if p.join("scenarios.json").exists() {
            return Some(p);
        }
        return None; // an explicit but unusable SRS_SPEC_DIR: skip, don't silently fall through
    }
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    loop {
        let sibling = dir.join("srs");
        let candidate = sibling.join("conformance/arrangement");
        if candidate.join("scenarios.json").exists() {
            if let Ok(out) = std::process::Command::new("git")
                .args([
                    "-C",
                    sibling.to_str().unwrap_or("."),
                    "rev-parse",
                    "--abbrev-ref",
                    "HEAD",
                ])
                .output()
            {
                if out.status.success() {
                    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if branch != "master" {
                        panic!(
                            "srs sibling checkout at {} is on branch '{branch}', not master: its \
                             conformance/arrangement fixture may be stale (srs-rust#874's \
                             false-green trap, srs-rust#922). Set SRS_SPEC_DIR to a fresh \
                             `origin/master` checkout instead.",
                            sibling.display()
                        );
                    }
                }
            }
            return Some(candidate);
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p.to_path_buf(),
            _ => break,
        }
    }
    None
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("scenario operation is missing string key '{key}': {v}"))
}

fn parse_entries(v: &Value) -> Result<Vec<ContainerEntry>, serde_json::Error> {
    serde_json::from_value(v.clone())
}

fn entries_json(entries: &[ContainerEntry]) -> Value {
    serde_json::to_value(entries).unwrap()
}

/// `# ` prefixed lines of rendered markdown, as `(level, title)`.
fn markdown_headings(rendered: &str) -> Vec<(usize, String)> {
    rendered
        .lines()
        .filter_map(|l| {
            let hashes = l.chars().take_while(|c| *c == '#').count();
            (hashes > 0 && l[hashes..].starts_with(' '))
                .then(|| (hashes, l[hashes..].trim().to_string()))
        })
        .collect()
}

/// Run one scenario against a fresh copy of the fixture. `Err` is a failure detail.
fn run_scenario(sc: &Value, fixture_repo: &Path) -> Result<(), String> {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    copy_dir(fixture_repo, &repo);
    let store = FileStore::new(&repo);

    // Reject any key we do not know how to evaluate: a skipped expectation is a false pass.
    const KNOWN: &[&str] = &[
        "operation",
        "container",
        "rootContainerId",
        "identityInstanceId",
        "exactMatch",
        "expectedDirectMembers",
        "expectedParents",
        "expectedHeadings",
        "expectedSections",
        "expectedOutcome",
        "expectedCode",
        "expectedMembers",
        "expectedPromoted",
        "expectedRemoved",
    ];
    for key in sc.as_object().unwrap().keys() {
        if !DESCRIPTIVE_KEYS.contains(&key.as_str()) && !KNOWN.contains(&key.as_str()) {
            return Err(format!("unhandled scenario key '{key}'"));
        }
    }

    // Read-only expectations over a container.
    if let Some(expected) = sc.get("expectedDirectMembers") {
        let container = s(sc, "container");
        let actual: BTreeSet<String> = container_service::get_arrangement(&store, container)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|e| e.instance_id)
            .collect();
        let expected: BTreeSet<String> = serde_json::from_value(expected.clone()).unwrap();
        if sc.get("exactMatch").and_then(Value::as_bool) != Some(true) {
            return Err("expectedDirectMembers without exactMatch: true".into());
        }
        return if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "direct members: expected {expected:?}, got {actual:?}"
            ))
        };
    }
    if let Some(expected) = sc.get("expectedParents") {
        let entries = container_service::get_arrangement(&store, s(sc, "container"))
            .map_err(|e| e.to_string())?;
        let actual: serde_json::Map<String, Value> = arrangement::outline(&entries)
            .into_iter()
            .map(|o| {
                (
                    o.instance_id,
                    o.parent_instance_id.map_or(Value::Null, Value::String),
                )
            })
            .collect();
        return if &Value::Object(actual.clone()) == expected {
            Ok(())
        } else {
            Err(format!(
                "parents: expected {expected}, got {}",
                Value::Object(actual)
            ))
        };
    }

    let op = sc
        .get("operation")
        .ok_or("scenario has neither operation nor a read expectation")?;

    // Navigation.
    if op.get("repo").and_then(Value::as_str) == Some("navigation") {
        let nav = repository_navigation(&store).map_err(|e| e.to_string())?;
        if nav.root_container_id != s(sc, "rootContainerId") {
            return Err(format!("root container: {}", nav.root_container_id));
        }
        let identity = nav.identity.as_ref().map(|n| n.instance_id.as_str());
        if identity != Some(s(sc, "identityInstanceId")) {
            return Err(format!("identity: {identity:?}"));
        }
        // [R12]: the root container's entries in order; depth-0 entries are sections and a
        // deeper entry nests under the nearest preceding shallower one. The engine reports every
        // entry with its `depth`, so the nesting is derived here from that, exactly as the
        // rule says a client may (`NavigationNode.children` is the part-of tree, a different
        // thing).
        let mut actual: Vec<Value> = Vec::new();
        let mut stack: Vec<(u32, String)> = Vec::new();
        for node in &nav.sections {
            while stack.last().is_some_and(|(d, _)| *d >= node.depth) {
                stack.pop();
            }
            if node.depth == 0 {
                actual.push(serde_json::json!({"instanceId": node.instance_id, "children": []}));
            } else if let Some((_, parent)) = stack.last() {
                if let Some(section) = actual
                    .iter_mut()
                    .find(|s| s["instanceId"] == Value::String(parent.clone()))
                {
                    section["children"]
                        .as_array_mut()
                        .unwrap()
                        .push(Value::String(node.instance_id.clone()));
                }
            }
            stack.push((node.depth, node.instance_id.clone()));
        }
        let expected = sc
            .get("expectedSections")
            .ok_or("navigation without expectedSections")?;
        return if &Value::Array(actual.clone()) == expected {
            Ok(())
        } else {
            Err(format!(
                "sections: expected {expected}, got {}",
                Value::Array(actual)
            ))
        };
    }

    // Render.
    if op.get("render").and_then(Value::as_str) == Some("composition") {
        let mut opts = RenderCompositionOptions::new(&store, s(op, "view"));
        opts.format = Some(s(op, "format"));
        opts.container_id = op.get("container").and_then(Value::as_str);
        let result = render_composition(opts);
        if sc.get("expectedOutcome").and_then(Value::as_str) == Some("error-diagnostic") {
            let r = result.map_err(|e| e.to_string())?;
            // "Renders nothing": no section content, only the document title may remain.
            let section_headings = markdown_headings(&r.rendered)
                .into_iter()
                .filter(|(level, _)| *level >= 2)
                .count();
            return if r.diagnostics.iter().any(|d| d.contains("error")) && section_headings == 0 {
                Ok(())
            } else {
                Err(format!(
                    "expected an error diagnostic and no output, got diagnostics {:?}, output {:?}",
                    r.diagnostics, r.rendered
                ))
            };
        }
        let r = result.map_err(|e| e.to_string())?;
        let expected = sc
            .get("expectedHeadings")
            .and_then(Value::as_array)
            .ok_or("render without expectedHeadings or expectedOutcome")?;
        let expected: Vec<(usize, String)> = expected
            .iter()
            .map(|h| {
                (
                    h["level"].as_u64().unwrap() as usize,
                    h["title"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        // The section heading itself is level 2; the entries are the headings below it.
        let actual: Vec<(usize, String)> = markdown_headings(&r.rendered)
            .into_iter()
            .filter(|(level, _)| *level >= 3)
            .collect();
        return if actual == expected {
            Ok(())
        } else {
            Err(format!("headings: expected {expected:?}, got {actual:?}"))
        };
    }

    // Container writes.
    let container_id = s(op, "containerId");
    let outcome: Result<(), String> = match s(op, "container") {
        "update" => {
            let entries = parse_entries(&op["memberInstanceIds"]).map_err(|e| e.to_string());
            match entries {
                // An entry carrying another property cannot even be parsed: refused at the type.
                Err(e) => Err(e),
                Ok(entries) => container_service::update_container(
                    &store,
                    container_id,
                    ContainerPatch {
                        member_instance_ids: Some(entries),
                        ..Default::default()
                    },
                )
                .map(|_| ())
                .map_err(|e| e.to_string()),
            }
        }
        "members-remove" => {
            match container_service::remove_member(&store, container_id, s(op, "instanceId")) {
                Ok(res) => {
                    if let Some(expected) = sc.get("expectedMembers") {
                        if &entries_json(&res.members) != expected {
                            return Err(format!(
                                "members: expected {expected}, got {}",
                                entries_json(&res.members)
                            ));
                        }
                    }
                    for (key, actual) in [
                        ("expectedPromoted", &res.promoted),
                        ("expectedRemoved", &res.removed),
                    ] {
                        if let Some(expected) = sc.get(key) {
                            let expected: Vec<String> =
                                serde_json::from_value(expected.clone()).unwrap();
                            if &expected != actual {
                                return Err(format!(
                                    "{key}: expected {expected:?}, got {actual:?}"
                                ));
                            }
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            }
        }
        other => return Err(format!("unhandled container operation '{other}'")),
    };

    match sc.get("expectedOutcome").and_then(Value::as_str) {
        Some("refused") => match outcome {
            Ok(()) => Err("expected the operation to be refused, but it succeeded".into()),
            Err(msg) => {
                // `expectedCode` may carry a parenthesised alternative for a different surface
                // ("instance not found (arrangement-unresolved on a stored container)"): the
                // primary code is the text before the parenthesis.
                match sc.get("expectedCode").and_then(Value::as_str) {
                    Some(code) => {
                        let primary = code.split(" (").next().unwrap_or(code);
                        let found = msg.contains(primary)
                            || (primary == "instance not found"
                                && msg.to_lowercase().contains("not found"));
                        if found {
                            Ok(())
                        } else {
                            Err(format!("refused, but not with '{primary}': {msg}"))
                        }
                    }
                    None => Ok(()),
                }
            }
        },
        None => outcome,
        Some(other) => Err(format!("unhandled expectedOutcome '{other}'")),
    }
}

#[test]
fn rfc_043_arrangement_fixture_scenarios() {
    let Some(dir) = conformance_dir() else {
        println!("Skipping: srs/conformance/arrangement fixture not found (isolated checkout)");
        return;
    };
    let scenarios_path = dir.join("scenarios.json");
    let fixture_repo = dir.join("fixture-repo");
    if !fixture_repo.join("manifest.json").exists() {
        println!("Skipping: srs/conformance/arrangement/fixture-repo not found");
        return;
    }

    // The fixture is at revision 11 (RFC-051 per-entry containers); an older fixture is awaiting
    // the corpus migration and cannot be loaded.
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_repo.join("manifest.json")).unwrap())
            .unwrap();
    let revision = manifest
        .get("dataModelRevision")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let current = srs_repository::field_type_migration_service::RFC051_CONTAINER_ENTRIES_REVISION;
    if revision < current {
        println!(
            "Skipping: arrangement fixture-repo is at dataModelRevision {revision} (< {current})"
        );
        return;
    }

    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(&scenarios_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", scenarios_path.display())),
    )
    .unwrap_or_else(|e| panic!("failed to parse {}: {e}", scenarios_path.display()));
    let scenarios = file["scenarios"].as_array().expect("scenarios array");
    // A loader that finds zero scenarios must not report success (CC-33).
    assert!(
        !scenarios.is_empty(),
        "{} has zero scenarios",
        scenarios_path.display()
    );

    let mut failures = Vec::new();
    let mut unexpected_passes = Vec::new();
    for sc in scenarios {
        let name = sc["name"].as_str().expect("scenario name");
        let expected_failure = EXPECTED_FAILURES.iter().find(|(n, _)| *n == name);
        match (run_scenario(sc, &fixture_repo), expected_failure) {
            (Ok(()), None) => {}
            (Ok(()), Some((n, issue))) => unexpected_passes.push(format!("{n} ({issue})")),
            (Err(_), Some(_)) => {}
            (Err(detail), None) => failures.push(format!("'{name}': {detail}")),
        }
    }

    assert!(
        failures.is_empty() && unexpected_passes.is_empty(),
        "{} of {} arrangement scenarios failed against {}:\n{}\nlisted in EXPECTED_FAILURES but now passing: {:?}",
        failures.len(),
        scenarios.len(),
        scenarios_path.display(),
        failures.join("\n"),
        unexpected_passes
    );
    println!(
        "{} of {} scenarios passed",
        scenarios.len(),
        scenarios.len()
    );
}
