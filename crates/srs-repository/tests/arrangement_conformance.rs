//! RFC-043 arrangement conformance runner (srs-rust#1477).
//!
//! RFC-043's "Conformance fixture" section binds acceptance to `conformance/arrangement/` in the
//! `srs` spec repo, with the shape of `conformance/discovery/` (see `discovery_conformance.rs`):
//! a `fixture-repo/` and a `scenarios.json`. Until this test existed nothing ran
//! `scenarios.json` against this implementation. The fixture is consumed as external test data,
//! never vendored (CLAUDE.md).
//!
//! Each scenario works on a fresh temp copy of `fixture-repo/` (plus any `given.files` written
//! over it) and is dispatched on its shape through the real services (`container_service`,
//! `render_service`, `repository_navigation_service`, `validation`). An unhandled shape or
//! expectation key fails loudly; it is
//! never skipped. A failing scenario is a finding (engine or fixture), never "fixed" by editing
//! `scenarios.json`, which is spec-repo-owned: list it in [`EXPECTED_FAILURES`] with its issue.
//!
//! `storage-scenarios.json` (RFC-051) is run by the spec repo against the pinned CLI and is not
//! this test's concern.

use serde_json::Value;
use srs_core::arrangement::{self, Placement, RelativeMove};
use srs_core::types::container::ContainerEntry;
use srs_repository::container_service::{self, ArrangementResult, ContainerPatch};
use srs_repository::error::RepositoryError;
use srs_repository::relation_service::{list_relations, ListRelationsFilter};
use srs_repository::render_service::{render_composition, RenderCompositionOptions};
use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use srs_repository::view_service::create_composition_normalized;
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

/// A refusal as the surfaces report it: the stable code first, then the message, so an
/// `expectedCode` matches on either.
fn refusal(e: RepositoryError) -> String {
    format!("{}: {e}", e.code())
}

/// The repository's validation diagnostics, for comparing two states.
fn validation_snapshot(store: &FileStore) -> Result<Vec<String>, String> {
    let report = validate_repository(store).map_err(|e| e.to_string())?;
    let mut d: Vec<String> = report
        .diagnostics
        .iter()
        .map(|d| format!("{d:?}"))
        .collect();
    d.sort();
    Ok(d)
}

/// Fail loudly on a key the scenario's code path never evaluates: accepting it would be a false pass.
fn reject_unevaluated(sc: &Value, keys: &[&str], path: &str) -> Result<(), String> {
    for k in keys {
        if sc.get(k).is_some() {
            return Err(format!("'{k}' is not evaluated for a {path} scenario"));
        }
    }
    Ok(())
}

/// An operation outcome judged against `expectedOutcome` / `expectedCode`.
fn judge(sc: &Value, outcome: Result<(), String>) -> Result<(), String> {
    if sc.get("expectedCode").is_some() && sc.get("expectedOutcome").is_none() {
        return Err("expectedCode without expectedOutcome".into());
    }
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

/// Check `expectedMembers` / `expectedPromoted` / `expectedRemoved` in `exp` against `res`.
fn check_arrangement(exp: &Value, res: &ArrangementResult) -> Result<(), String> {
    if let Some(expected) = exp.get("expectedMembers") {
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
        if let Some(expected) = exp.get(key) {
            let expected: Vec<String> = serde_json::from_value(expected.clone()).unwrap();
            if &expected != actual {
                return Err(format!("{key}: expected {expected:?}, got {actual:?}"));
            }
        }
    }
    Ok(())
}

/// Run one scenario against a fresh copy of the fixture. `Err` is a failure detail.
fn run_scenario(sc: &Value, fixture_repo: &Path) -> Result<(), String> {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    copy_dir(fixture_repo, &repo);
    let store = FileStore::new(&repo);

    // Reject any key we do not know how to evaluate: a skipped expectation is a false pass.
    const KNOWN: &[&str] = &[
        "given",
        "operation",
        "container",
        "rootContainerId",
        "identityInstanceId",
        "exactMatch",
        "expectedDirectMembers",
        "expectedParents",
        "expectedHeadings",
        "expectedRecords",
        "expectedSections",
        "expectedSectionContainers",
        "expectedDiagnostics",
        "expectedOutcome",
        "expectedCode",
        "expectedMembers",
        "expectedPromoted",
        "expectedRemoved",
        "expectedAnchorInstanceId",
        "expectedValidateCode",
        "expectedSecondRun",
        "expectedSectionOrder",
        "ignoredRelations",
        "expectedValidateUnchanged",
        "expectedDiagnosticContains",
    ];
    for key in sc.as_object().unwrap().keys() {
        if !DESCRIPTIVE_KEYS.contains(&key.as_str()) && !KNOWN.contains(&key.as_str()) {
            return Err(format!("unhandled scenario key '{key}'"));
        }
    }

    // `given.files`: repository-relative paths written over the fixture copy before the scenario.
    if let Some(given) = sc.get("given") {
        for key in given.as_object().ok_or("given is not an object")?.keys() {
            if key != "files" {
                return Err(format!("unhandled given key '{key}'"));
            }
        }
        for (rel, content) in given["files"]
            .as_object()
            .ok_or("given.files is not an object")?
        {
            let path = repo.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(content).unwrap()).unwrap();
        }
    }

    // Read-only expectations over a container.
    if let Some(expected) = sc.get("expectedDirectMembers") {
        reject_unevaluated(sc, &["operation", "expectedOutcome"], "direct-members")?;
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
        reject_unevaluated(sc, &["operation", "expectedOutcome"], "parents")?;
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
        reject_unevaluated(
            sc,
            &[
                "expectedOutcome",
                "expectedCode",
                "expectedHeadings",
                "expectedRecords",
                "expectedMembers",
                "expectedDiagnosticContains",
                "expectedValidateUnchanged",
            ],
            "navigation",
        )?;
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
        if ![
            "expectedSections",
            "expectedSectionOrder",
            "expectedSectionContainers",
            "expectedDiagnostics",
        ]
        .iter()
        .any(|k| sc.get(k).is_some())
        {
            return Err("navigation without an expectation".into());
        }
        if let Some(expected) = sc.get("expectedSections") {
            if &Value::Array(actual.clone()) != expected {
                return Err(format!(
                    "sections: expected {expected}, got {}",
                    Value::Array(actual)
                ));
            }
        }
        // [R12]: the root container's entries in order, as `{instanceId, depth?}` (depth omitted
        // at 0), whatever `precedes` relations say. `ignoredRelations` must exist in the fixture,
        // so the scenario cannot pass because the disagreeing relation was never there.
        if let Some(expected) = sc.get("expectedSectionOrder") {
            let order: Vec<Value> = nav
                .sections
                .iter()
                .map(|n| {
                    let mut o = serde_json::json!({"instanceId": n.instance_id});
                    if n.depth > 0 {
                        o["depth"] = n.depth.into();
                    }
                    o
                })
                .collect();
            if &Value::Array(order.clone()) != expected {
                return Err(format!(
                    "section order: expected {expected}, got {}",
                    Value::Array(order)
                ));
            }
        }
        if let Some(ignored) = sc.get("ignoredRelations") {
            let present = list_relations(&store, ListRelationsFilter::default())
                .map_err(|e| e.to_string())?;
            for rel in ignored
                .as_array()
                .ok_or("ignoredRelations is not an array")?
            {
                if !present.iter().any(|r| {
                    r.relation_type == s(rel, "relationType")
                        && r.source_id == s(rel, "sourceInstanceId")
                        && r.target_id == s(rel, "targetInstanceId")
                }) {
                    return Err(format!("ignoredRelations: {rel} is not in the fixture"));
                }
            }
        }
        // [R19]: each listed instance's section container (null = none).
        if let Some(expected) = sc.get("expectedSectionContainers") {
            for (id, want) in expected
                .as_object()
                .ok_or("expectedSectionContainers is not an object")?
            {
                let node = nav
                    .sections
                    .iter()
                    .find(|n| &n.instance_id == id)
                    .ok_or_else(|| format!("section container: {id} is not in navigation"))?;
                let got = node
                    .section_container_id
                    .clone()
                    .map_or(Value::Null, Value::String);
                if &got != want {
                    return Err(format!(
                        "section container of {id}: expected {want}, got {got}"
                    ));
                }
            }
        }
        if let Some(expected) = sc.get("expectedDiagnostics") {
            let expected: Vec<String> = serde_json::from_value(expected.clone()).unwrap();
            if expected != nav.diagnostics {
                return Err(format!(
                    "diagnostics: expected {expected:?}, got {:?}",
                    nav.diagnostics
                ));
            }
        }
        return Ok(());
    }

    // Render.
    if op.get("render").and_then(Value::as_str) == Some("composition") {
        reject_unevaluated(sc, &["expectedValidateUnchanged", "expectedCode"], "render")?;
        if let Some(o) = sc.get("expectedOutcome").and_then(Value::as_str) {
            if o != "error-diagnostic" {
                return Err(format!("unhandled expectedOutcome '{o}' for a render"));
            }
            reject_unevaluated(
                sc,
                &[
                    "expectedDiagnosticContains",
                    "expectedHeadings",
                    "expectedRecords",
                ],
                "render error-diagnostic",
            )?;
        }
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
        if let Some(needle) = sc.get("expectedDiagnosticContains").and_then(Value::as_str) {
            if !r.diagnostics.iter().any(|d| d.contains(needle)) {
                return Err(format!(
                    "no render diagnostic contains '{needle}': {:?}",
                    r.diagnostics
                ));
            }
        }
        // [R11] json: the records in outline order, each `{instanceId, title, depth?}`.
        if let Some(expected) = sc.get("expectedRecords") {
            let projection = serde_json::to_value(
                r.projection
                    .as_ref()
                    .ok_or("json render returned no projection")?,
            )
            .unwrap();
            let actual: Vec<Value> = projection["sections"]
                .as_array()
                .ok_or("json render has no sections")?
                .iter()
                .flat_map(|s| s["records"].as_array().cloned().unwrap_or_default())
                .map(|rec| {
                    let mut o = serde_json::json!({
                        "instanceId": rec["instanceId"],
                        "title": rec["fields"]["title"],
                    });
                    if let Some(d) = rec.get("depth") {
                        o["depth"] = d.clone();
                    }
                    o
                })
                .collect();
            return if &Value::Array(actual.clone()) == expected {
                Ok(())
            } else {
                Err(format!(
                    "records: expected {expected}, got {}",
                    Value::Array(actual)
                ))
            };
        }
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

    // Composition creation: refused where the rule is enforced, at the validator.
    if op.get("composition").and_then(Value::as_str) == Some("create") {
        reject_unevaluated(
            sc,
            &["expectedValidateUnchanged", "expectedMembers"],
            "composition create",
        )?;
        let outcome = create_composition_normalized(&store, op["definition"].clone(), None)
            .map(|_| ())
            .map_err(refusal);
        return judge(sc, outcome);
    }

    // Container writes.
    let validate_before =
        if sc.get("expectedValidateUnchanged").and_then(Value::as_bool) == Some(true) {
            Some(validation_snapshot(&store)?)
        } else {
            None
        };
    const OP_KEYS: &[&str] = &[
        "container",
        "containerId",
        "instanceId",
        "position",
        "depth",
        "before",
        "after",
        "into",
        "memberInstanceIds",
        "patch",
    ];
    for key in op.as_object().ok_or("operation is not an object")?.keys() {
        if !OP_KEYS.contains(&key.as_str()) {
            return Err(format!("unhandled operation key '{key}'"));
        }
    }
    let container_id = s(op, "containerId");
    // `before` / `after` / `into` name the entry to place against; `position` / `depth` are absolute.
    let relative = [
        ("before", Placement::Before),
        ("after", Placement::After),
        ("into", Placement::Into),
    ]
    .into_iter()
    .find_map(|(k, placement)| {
        op.get(k)
            .and_then(Value::as_str)
            .map(|target| (target.to_string(), placement))
    });
    let position = op
        .get("position")
        .and_then(Value::as_u64)
        .map(|p| p as usize);
    let depth = op.get("depth").and_then(Value::as_u64).map(|d| d as u32);
    let outcome: Result<(), String> = match s(op, "container") {
        "update" => {
            reject_unevaluated(
                sc,
                &[
                    "expectedPromoted",
                    "expectedRemoved",
                    "expectedAnchorInstanceId",
                    "expectedValidateCode",
                    "expectedSecondRun",
                ],
                "container update",
            )?;
            // A raw `patch` is parsed as the CLI parses it: an unknown key is `invalid-input`.
            let patch = match op.get("patch") {
                Some(p) => serde_json::from_value::<ContainerPatch>(p.clone())
                    .map_err(|e| format!("invalid-input: {e}")),
                // An entry carrying another property cannot even be parsed: refused at the type.
                None => op
                    .get("memberInstanceIds")
                    .ok_or_else(|| "update has neither patch nor memberInstanceIds".to_string())
                    .and_then(|m| parse_entries(m).map_err(|e| e.to_string()))
                    .map(|entries| ContainerPatch {
                        member_instance_ids: Some(entries),
                        ..Default::default()
                    })
                    .map_err(|e| e.to_string()),
            };
            patch.and_then(|patch| {
                container_service::update_container(&store, container_id, patch)
                    .map_err(refusal)?;
                if let Some(expected) = sc.get("expectedMembers") {
                    let members = container_service::get_arrangement(&store, container_id)
                        .map_err(refusal)?;
                    if &entries_json(&members) != expected {
                        return Err(format!(
                            "members: expected {expected}, got {}",
                            entries_json(&members)
                        ));
                    }
                }
                Ok(())
            })
        }
        "members-remove" => {
            match container_service::remove_member(&store, container_id, s(op, "instanceId")) {
                Ok(res) => check_arrangement(sc, &res),
                Err(e) => Err(refusal(e)),
            }
        }
        // [R7]: a move carries the entry's run; `after` / `before` / `into` place it against an
        // entry, `depth` / `position` set it absolutely.
        "members-move" => {
            let id = s(op, "instanceId");
            match &relative {
                Some((target, placement)) => container_service::move_member_relative(
                    &store,
                    container_id,
                    id,
                    &RelativeMove::Place {
                        target: target.clone(),
                        placement: *placement,
                    },
                ),
                None => container_service::move_member(&store, container_id, id, position, depth),
            }
            .map_err(refusal)
            .and_then(|res| check_arrangement(sc, &res))
        }
        "members-add" => {
            let id = s(op, "instanceId");
            match &relative {
                Some((target, placement)) => container_service::add_member_relative(
                    &store,
                    container_id,
                    id,
                    target,
                    *placement,
                ),
                None => container_service::add_member(&store, container_id, id, position, depth),
            }
            .map_err(refusal)
            .and_then(|res| check_arrangement(sc, &res))
        }
        // [R20]: repair removes unresolved entries by [R7], leaves the pointers alone, and is
        // idempotent.
        "members-repair" => {
            let res = container_service::repair_members(&store, container_id).map_err(refusal)?;
            check_arrangement(sc, &res)?;
            if let Some(expected) = sc.get("expectedAnchorInstanceId") {
                let anchor = store
                    .load_container_unchecked(container_id)
                    .map_err(refusal)?
                    .anchor_instance_id
                    .map_or(Value::Null, Value::String);
                if &anchor != expected {
                    return Err(format!("anchor: expected {expected}, got {anchor}"));
                }
            }
            if let Some(code) = sc.get("expectedValidateCode").and_then(Value::as_str) {
                let report = validate_repository(&store).map_err(refusal)?;
                if !report
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_deref() == Some(code))
                {
                    return Err(format!(
                        "validate: expected a '{code}' diagnostic, got {:?}",
                        report.diagnostics
                    ));
                }
            }
            if let Some(second) = sc.get("expectedSecondRun") {
                let res =
                    container_service::repair_members(&store, container_id).map_err(refusal)?;
                check_arrangement(second, &res).map_err(|e| format!("second run: {e}"))?;
            }
            Ok(())
        }
        other => return Err(format!("unhandled container operation '{other}'")),
    };

    // [R13]: the operation leaves `repo validate` exactly as it was.
    if let (Some(before), Ok(())) = (&validate_before, &outcome) {
        let after = validation_snapshot(&store)?;
        if before != &after {
            return Err(format!(
                "validate changed: before {before:?}, after {after:?}"
            ));
        }
    }

    judge(sc, outcome)
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
