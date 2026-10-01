//! `rfc043-container-entries` migration (srs-rust#1141, RFC-043 Change I / [R17]) — data-model
//! revision 7 -> 8.
//!
//! ## What it does
//!
//! 1. Every Container (`manifest.container` and each `containers/*.json`): the member set is
//!    `rootInstanceIds` plus `memberInstanceIds`, deduplicated, written as flat
//!    `{instanceId}` entries (depth 0) in the order in effect today — ids a Composition
//!    section's `memberOrder` names for that container first, in listed order, then the rest
//!    in Rule [N+12] order ([`relation_graph::sort_by_precedes_chain`] — the one function the
//!    renderer and navigation call). In the root container the identity entry is first and the
//!    non-identity members follow in today's navigation order.
//! 2. `anchorInstanceId` is set to `rootInstanceIds[0]` where absent (the I-145 fallback, so no
//!    typing check starts firing that did not fire before); `rootInstanceIds` is removed.
//! 3. Every Composition (the primary package and each local sub-package): on each
//!    `container-subset` section with a `memberOrder`, remove it and set
//!    `ordering.source: "arranged"`. A `memberOrder` on any other section is stripped and
//!    reported (it was ignored with a diagnostic under [N+29]).
//! 4. Every `memberOrder` id dropped because it is not a member is reported
//!    (`migration-memberorder-dropped`).
//! 5. `dataModelRevision` 8 is stamped on the manifest and on every package manifest.
//!
//! ## Why it reads the raw tree
//!
//! An unmigrated manifest or container fails the revision-8 schemas on load, so — like
//! `graduated-at-cleanup` — this entry never goes through the checked catalog or the typed
//! `Container`; it is the sole sanctioned reader of the revision-7 container and Composition
//! shapes (deleted with the entry under srs-rust#1138). The binary's general code paths never
//! interpret them ([R16]).
//!
//! ## All-or-nothing, and the gate
//!
//! The whole plan — every new container, every rewritten Composition — is computed in memory and
//! gated before the first write; any refusal (`migration-memberorder-conflict`) writes nothing.
//! The gate re-derives, with literal copies of the revision-7 rules, (a) the member order each
//! `memberOrder` section rendered and (b) the root container's navigation order, and requires the
//! migrated container to yield the same sequence through the revision-8 reading; and it requires
//! each record's section-container link to be unchanged except where it was ambiguous before.
//! ponytail: the gate compares orderings (what a render depends on), not rendered bytes — the
//! revision-7 renderer is not kept. Byte-identical renders are proved at corpus level by
//! rendering with the last revision-7 build before and this build after (PR notes).

use crate::error::RepositoryError;
use crate::relation_graph::{self, PrecedesSortable};
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::{json, Value};
use srs_core::types::relation::Relation;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MIGRATION_ID: &str = "rfc043-container-entries";
pub const RFC043_REVISION: u64 = 8;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rfc043Result {
    pub from_revision: u64,
    pub to_revision: u64,
    pub containers_migrated: usize,
    pub entries_written: usize,
    pub sections_flipped_to_arranged: usize,
    pub member_order_stripped_not_container_subset: usize,
    /// Containers whose `anchorInstanceId` was set from `rootInstanceIds[0]`.
    pub anchors_set: Vec<String>,
    /// Containers that had several roots (only the first becomes the anchor).
    pub multi_root_containers: Vec<String>,
    pub packages_stamped: usize,
    /// `migration-memberorder-dropped` / other non-fatal notes.
    pub diagnostics: Vec<String>,
}

fn refuse(code: &str, msg: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::InvalidSnapshotData {
        message: format!("rfc043-container-entries migration refused ({code}): {msg}"),
    }
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn raw_manifest(store: &dyn RepositoryStore) -> Result<Value, RepositoryError> {
    let text = store.load_manifest_raw_text()?;
    serde_json::from_str(&text).map_err(|source| RepositoryError::ManifestParse {
        path: std::path::PathBuf::from("manifest.json"),
        source,
    })
}

/// A container is still revision 7 when it carries `rootInstanceIds` or string members.
pub(crate) fn container_value_is_legacy(c: &Value) -> bool {
    container_is_legacy(c)
}

fn container_is_legacy(c: &Value) -> bool {
    c.get("rootInstanceIds").is_some()
        || c.get("memberInstanceIds")
            .and_then(|m| m.as_array())
            .is_some_and(|a| a.iter().any(|e| e.is_string()))
}

/// `true` when the raw manifest's root container is revision-7 shaped. Used by the load gate
/// (`manifest::rfc038::check_manifest`) to refuse naming RFC-043 ([R16]).
pub(crate) fn raw_manifest_container_is_legacy(raw: &Value) -> bool {
    raw.get("container").is_some_and(container_is_legacy)
}

fn package_roots(raw_manifest: &Value) -> Vec<String> {
    let mut roots = vec!["package".to_string()];
    for r in raw_manifest
        .get("packageRefs")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        if r.get("mode").and_then(|m| m.as_str()) == Some("local") {
            if let Some(path) = r.get("path").and_then(|p| p.as_str()) {
                roots.push(path.to_string());
            }
        }
    }
    roots
}

fn container_files(store: &dyn RepositoryStore) -> Vec<String> {
    let mut files: Vec<String> = store
        .list_files_recursive("containers")
        .into_iter()
        .filter(|p| p.ends_with(".json"))
        .collect();
    files.sort();
    files
}

/// Composition file paths of one package root, as the package index lists them.
fn composition_paths(store: &dyn RepositoryStore, root: &str) -> Vec<String> {
    match store.load_instance_json(&format!("{root}/package.json")) {
        Ok(idx) => str_list(&idx, "compositions")
            .into_iter()
            .map(|rel| format!("{root}/{rel}"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn composition_needs_migration(doc: &Value) -> bool {
    doc.get("sections")
        .and_then(|s| s.as_array())
        .is_some_and(|secs| {
            secs.iter().any(|s| {
                s.get("ordering")
                    .is_some_and(|o| o.get("memberOrder").is_some())
            })
        })
}

/// `Needed` when the revision stamp is below 8, or any container / Composition still has a
/// revision-7 shape. Reads raw JSON only.
pub fn migration_needed(store: &dyn RepositoryStore) -> bool {
    let Ok(raw) = raw_manifest(store) else {
        return false;
    };
    if raw
        .get(crate::field_type_migration_service::DATA_MODEL_REVISION_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        < RFC043_REVISION
    {
        return true;
    }
    if raw_manifest_container_is_legacy(&raw) {
        return true;
    }
    for path in container_files(store) {
        if store
            .load_instance_json(&path)
            .is_ok_and(|c| container_is_legacy(&c))
        {
            return true;
        }
    }
    for root in package_roots(&raw) {
        for path in composition_paths(store, &root) {
            if store
                .load_instance_json(&path)
                .is_ok_and(|d| composition_needs_migration(&d))
            {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Revision-7 reading helpers (the private raw-tree reader — deleted with the entry, #1138)
// ---------------------------------------------------------------------------

/// A member awaiting a `precedes` sort — id and `createdAt` only.
#[derive(Clone)]
struct Member {
    id: String,
    created_at: Option<String>,
}

impl PrecedesSortable for Member {
    fn precedes_instance_id(&self) -> &str {
        &self.id
    }
    fn precedes_created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
}

struct Corpus {
    /// instance id -> `createdAt`
    created_at: HashMap<String, Option<String>>,
    relations: Vec<Relation>,
}

fn load_corpus(store: &dyn RepositoryStore) -> Result<Corpus, RepositoryError> {
    // The catalog cannot be used: its builder loads the typed manifest, which a revision-7
    // root container does not parse as (RFC-043 [R16]). Walk the trees directly.
    let mut created_at = HashMap::new();
    let mut paths = store.list_files_recursive("records");
    paths.sort();
    for path in paths.into_iter().filter(|p| p.ends_with(".json")) {
        let Ok(v) = store.load_instance_json(&path) else {
            continue;
        };
        if let Some(id) = v.get("instanceId").and_then(|i| i.as_str()) {
            let created = v
                .get("createdAt")
                .and_then(|c| c.as_str())
                .map(str::to_string);
            created_at.insert(id.to_string(), created);
        }
    }
    let mut relations = Vec::new();
    let mut paths = store.list_files_recursive("relations");
    paths.sort();
    for path in paths.into_iter().filter(|p| p.ends_with(".json")) {
        let Ok(mut v) = store.load_instance_json(&path) else {
            continue;
        };
        // The pinned `$schema` is not part of the Relation shape (RFC-038 Change E).
        if let Some(o) = v.as_object_mut() {
            o.remove("$schema");
        }
        // A relation that does not parse would silently change a `precedes` order the
        // migration freezes, so it refuses rather than skips.
        let r = serde_json::from_value::<Relation>(v).map_err(|e| {
            refuse(
                "migration-relation-unreadable",
                format!("{path} is not a relation ({e}); nothing was written"),
            )
        })?;
        relations.push(r);
    }
    Ok(Corpus {
        created_at,
        relations,
    })
}

impl Corpus {
    fn member(&self, id: &str) -> Member {
        Member {
            id: id.to_string(),
            created_at: self.created_at.get(id).cloned().flatten(),
        }
    }

    /// Rule [N+12] order over `ids` (the function the renderer and navigation call).
    fn rule_order(&self, ids: &[String]) -> Vec<String> {
        let members: Vec<Member> = ids.iter().map(|i| self.member(i)).collect();
        relation_graph::sort_by_precedes_chain(members, &self.relations)
            .into_iter()
            .map(|m| m.id)
            .collect()
    }

    /// Members that are not the `contains`-target of another member (the render filter,
    /// srs#682): the section's roots.
    fn contains_roots(&self, ids: &[String]) -> Vec<String> {
        let set: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let non_root: HashSet<&str> = self
            .relations
            .iter()
            .filter(|r| {
                r.relation_type == "contains"
                    && set.contains(r.source_instance_id.as_str())
                    && set.contains(r.target_instance_id.as_str())
            })
            .map(|r| r.target_instance_id.as_str())
            .collect();
        ids.iter()
            .filter(|i| !non_root.contains(i.as_str()))
            .cloned()
            .collect()
    }

    /// Revision-7 `memberOrder` application ([N+29]/[N+30], srs-rust#1130 root filtering),
    /// literally: listed ids that are roots, in listed order; then the remaining roots in
    /// Rule [N+12] order.
    fn old_member_order_render(&self, members: &[String], listed: &[String]) -> Vec<String> {
        let roots = self.contains_roots(members);
        let root_set: HashSet<&str> = roots.iter().map(String::as_str).collect();
        let mut out: Vec<String> = Vec::new();
        let mut taken: HashSet<&str> = HashSet::new();
        for id in listed {
            if root_set.contains(id.as_str()) && taken.insert(id.as_str()) {
                out.push(id.clone());
            }
        }
        let rest: Vec<String> = roots
            .iter()
            .filter(|i| !taken.contains(i.as_str()))
            .cloned()
            .collect();
        out.extend(self.rule_order(&rest));
        out
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

struct ContainerPlan {
    /// `None` = the manifest-embedded root container.
    locator: Option<String>,
    id: String,
    old: Value,
    new: Value,
}

struct CompositionPlan {
    path: String,
    doc: Value,
}

fn is_container_subset(section: &Value) -> bool {
    section
        .get("source")
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        == Some("container-subset")
}

pub fn migrate_rfc043_container_entries(
    store: &dyn RepositoryStore,
) -> Result<Rfc043Result, RepositoryError> {
    let from_revision = raw_manifest(store)?
        .get(crate::field_type_migration_service::DATA_MODEL_REVISION_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let required = crate::field_type_migration_service::DISCOVERY_QUERY_CUTOVER_REVISION;
    if from_revision < required {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "rfc043-container-entries migration requires data-model revision >= {required} \
                 (found {from_revision}): migrate to revision {required} with the last build \
                 before RFC-043 (srs-rust build.NNN before #1141) first"
            ),
        });
    }

    let mut result = Rfc043Result {
        from_revision,
        to_revision: RFC043_REVISION,
        ..Default::default()
    };
    let mut manifest_raw = raw_manifest(store)?;
    let corpus = load_corpus(store)?;

    // ---- Compositions: collect memberOrder per container, plan the rewrite.
    let mut compositions: Vec<CompositionPlan> = Vec::new();
    // container id -> (list, "composition path#section")
    let mut listed_by_container: BTreeMap<String, (Vec<String>, String)> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    for root in package_roots(&manifest_raw) {
        for path in composition_paths(store, &root) {
            let mut doc = match store.load_instance_json(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let mut changed = false;
            if let Some(sections) = doc.get_mut("sections").and_then(|s| s.as_array_mut()) {
                for section in sections.iter_mut() {
                    let Some(list_val) = section
                        .get("ordering")
                        .and_then(|o| o.get("memberOrder"))
                        .cloned()
                    else {
                        continue;
                    };
                    let listed: Vec<String> = list_val
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let sid = section
                        .get("sectionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let here = format!("{path}#{sid}");
                    let subset = is_container_subset(section);
                    let subtree = section
                        .get("source")
                        .and_then(|s| s.get("containerScope"))
                        .and_then(|c| c.as_str())
                        == Some("subtree");
                    if subset && subtree {
                        return Err(refuse(
                            "migration-memberorder-conflict",
                            format!(
                                "{here} combines memberOrder with containerScope 'subtree' \
                                 (invalid under RFC-043 [R10]); nothing was written"
                            ),
                        ));
                    }
                    if let Some(o) = section.get_mut("ordering").and_then(|o| o.as_object_mut()) {
                        o.shift_remove("memberOrder");
                    }
                    if subset {
                        if let Some(o) = section.get_mut("ordering").and_then(|o| o.as_object_mut())
                        {
                            o.insert("source".to_string(), json!("arranged"));
                        }
                        result.sections_flipped_to_arranged += 1;
                        if let Some(cid) = section
                            .get("source")
                            .and_then(|s| s.get("containerId"))
                            .and_then(|c| c.as_str())
                        {
                            match listed_by_container.get(cid) {
                                Some((prev, prev_at)) if *prev != listed => {
                                    conflicts.push(format!(
                                        "container {cid} is named by two sections with different \
                                     memberOrder lists: {prev_at} and {here}"
                                    ))
                                }
                                Some(_) => {}
                                None => {
                                    listed_by_container
                                        .insert(cid.to_string(), (listed, here.clone()));
                                }
                            }
                        }
                    } else {
                        result.member_order_stripped_not_container_subset += 1;
                        result.diagnostics.push(format!(
                            "migration-memberorder-dropped: {here} carried memberOrder on a \
                             section that is not container-subset (ignored under [N+29]); stripped"
                        ));
                    }
                    changed = true;
                }
            }
            if changed {
                compositions.push(CompositionPlan { path, doc });
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(refuse(
            "migration-memberorder-conflict",
            format!("{}; nothing was written", conflicts.join("; ")),
        ));
    }

    // ---- Containers.
    let mut docs: Vec<(Option<String>, Value)> = Vec::new();
    if let Some(c) = manifest_raw.get("container") {
        docs.push((None, c.clone()));
    }
    for path in container_files(store) {
        if let Ok(v) = store.load_instance_json(&path) {
            docs.push((Some(path), v));
        }
    }
    let root_id = manifest_raw
        .get("container")
        .and_then(|c| c.get("containerId"))
        .and_then(|i| i.as_str())
        .map(str::to_string);
    let mut plans: Vec<ContainerPlan> = Vec::new();
    // Section-container link before/after for the gate.
    let mut roots_before: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut anchors_after: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut anchor_diff_from_first_root: HashSet<String> = HashSet::new();

    for (locator, old) in docs {
        let cid = old
            .get("containerId")
            .and_then(|i| i.as_str())
            .unwrap_or_default()
            .to_string();
        let roots = str_list(&old, "rootInstanceIds");
        let members = str_list(&old, "memberInstanceIds");
        for r in &roots {
            roots_before.entry(r.clone()).or_default().push(cid.clone());
        }
        let legacy = container_is_legacy(&old);
        // Anchor (step 2).
        let declared_anchor = old
            .get("anchorInstanceId")
            .and_then(|a| a.as_str())
            .map(str::to_string);
        let anchor = declared_anchor.clone().or_else(|| roots.first().cloned());
        if let Some(a) = &anchor {
            anchors_after
                .entry(a.clone())
                .or_default()
                .push(cid.clone());
            if roots.first().is_some_and(|r| r != a) {
                anchor_diff_from_first_root.insert(a.clone());
            }
        }
        if !legacy {
            continue;
        }

        // Member set (step 1): roots first, then members, deduplicated.
        let mut set: Vec<String> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for id in roots.iter().chain(members.iter()) {
            if seen.insert(id.as_str()) {
                set.push(id.clone());
            }
        }
        let identity = old
            .get("identityInstanceId")
            .and_then(|i| i.as_str())
            .map(str::to_string);
        let listed = listed_by_container.get(&cid).map(|(l, _)| l.clone());
        let is_root = root_id.as_deref() == Some(cid.as_str());

        let ordered: Vec<String> = if is_root {
            // Today's navigation order: identity first, then the non-identity members by the
            // [N+12] order. A memberOrder naming the root container that disagrees is refused.
            let non_identity: Vec<String> = set
                .iter()
                .filter(|i| Some(i.as_str()) != identity.as_deref())
                .cloned()
                .collect();
            let nav = corpus.rule_order(&non_identity);
            if let Some(l) = &listed {
                let nav_listed: Vec<String> =
                    nav.iter().filter(|i| l.contains(i)).cloned().collect();
                let want: Vec<String> = l.iter().filter(|i| nav.contains(i)).cloned().collect();
                if nav_listed != want {
                    return Err(refuse(
                        "migration-memberorder-conflict",
                        format!(
                            "a memberOrder list names the root container {cid} and disagrees with \
                             its navigation order; nothing was written"
                        ),
                    ));
                }
            }
            identity
                .iter()
                .filter(|i| set.contains(i))
                .cloned()
                .chain(nav)
                .collect()
        } else {
            let l = listed.clone().unwrap_or_default();
            let member_set: HashSet<&str> = set.iter().map(String::as_str).collect();
            let mut out: Vec<String> = Vec::new();
            let mut taken: HashSet<String> = HashSet::new();
            for id in &l {
                if member_set.contains(id.as_str()) {
                    if taken.insert(id.clone()) {
                        out.push(id.clone());
                    }
                } else {
                    result.diagnostics.push(format!(
                        "migration-memberorder-dropped: memberOrder id {id} is not a member of \
                         container {cid}; dropped"
                    ));
                }
            }
            let rest: Vec<String> = set
                .iter()
                .filter(|i| !taken.contains(*i))
                .cloned()
                .collect();
            out.extend(corpus.rule_order(&rest));
            out
        };

        // Gate (a): a memberOrder section must see the same sequence it saw before.
        if let Some(l) = &listed {
            let before = corpus.old_member_order_render(&set, l);
            let roots_after = corpus.contains_roots(&ordered);
            if before != roots_after {
                return Err(refuse(
                    "migration-memberorder-conflict",
                    format!(
                        "gate: the migrated order of container {cid} would change what its \
                         memberOrder section rendered (before: {before:?}, after: {roots_after:?}); \
                         nothing was written"
                    ),
                ));
            }
        }

        // Build the new container (raw edit).
        let mut new = old.clone();
        let obj = new.as_object_mut().ok_or_else(|| {
            refuse(
                "migration-container-shape",
                format!("container {cid} is not an object"),
            )
        })?;
        obj.shift_remove("rootInstanceIds");
        let had_members_key = old.get("memberInstanceIds").is_some();
        if ordered.is_empty() && !had_members_key {
            // nothing to write
        } else {
            obj.insert(
                "memberInstanceIds".to_string(),
                Value::Array(
                    ordered
                        .iter()
                        .map(|id| json!({ "instanceId": id }))
                        .collect(),
                ),
            );
        }
        if declared_anchor.is_none() {
            if let Some(a) = &anchor {
                obj.insert("anchorInstanceId".to_string(), json!(a));
                result.anchors_set.push(cid.clone());
                if roots.len() > 1 {
                    result.multi_root_containers.push(cid.clone());
                }
            }
        }
        result.containers_migrated += 1;
        result.entries_written += ordered.len();
        plans.push(ContainerPlan {
            locator,
            id: cid,
            old,
            new,
        });
    }

    // Gate (c): the section-container link. Unambiguous before (one container), must be the same
    // container after; ambiguity before (several containers, or an anchor that differs from
    // rootInstanceIds[0]) is tolerated and reported.
    for (root, before) in &roots_before {
        if before.len() > 1 || anchor_diff_from_first_root.contains(root) {
            result.diagnostics.push(format!(
                "migration-section-link: record {root} was a root of {} containers or anchored \
                 differently from rootInstanceIds[0]; its section-container link now follows \
                 anchorInstanceId (RFC-043 [R19])",
                before.len()
            ));
            continue;
        }
        let after = anchors_after.get(root).cloned().unwrap_or_default();
        if after != *before {
            // a non-first root of a multi-root container loses the link (anchor is single).
            let owner_roots_gt1 = plans
                .iter()
                .any(|p| p.id == before[0] && str_list(&p.old, "rootInstanceIds").len() > 1);
            if owner_roots_gt1 {
                result.diagnostics.push(format!(
                    "migration-section-link: record {root} was a non-first root of multi-root \
                     container {}; it keeps no section-container link (single anchor, RFC-043 [R19])",
                    before[0]
                ));
            } else {
                return Err(refuse(
                    "migration-memberorder-conflict",
                    format!(
                        "gate: the section-container link of {root} would change \
                         ({before:?} -> {after:?}); nothing was written"
                    ),
                ));
            }
        }
    }

    // ---- Apply (every refusal above was decided before this first write).
    store.begin_batch();
    let applied = (|| -> Result<(), RepositoryError> {
        for plan in &plans {
            match &plan.locator {
                Some(path) => write_like(store, path, &plan.new)?,
                None => {
                    manifest_raw["container"] = plan.new.clone();
                }
            }
        }
        for c in &compositions {
            write_like(store, &c.path, &c.doc)?;
        }
        // Stamp the manifest and every package manifest.
        let key = crate::field_type_migration_service::DATA_MODEL_REVISION_KEY;
        let pkg_roots = package_roots(&manifest_raw);
        manifest_raw[key] = json!(RFC043_REVISION);
        write_manifest_raw(store, &manifest_raw)?;
        for root in pkg_roots {
            let path = format!("{root}/package.json");
            if let Ok(mut idx) = store.load_instance_json(&path) {
                if idx.get(key).is_some() {
                    idx[key] = json!(RFC043_REVISION);
                    write_like(store, &path, &idx)?;
                    result.packages_stamped += 1;
                }
            }
        }
        Ok(())
    })();
    match applied {
        Ok(()) => store.commit_batch()?,
        Err(e) => {
            let _ = store.abort_batch();
            return Err(e);
        }
    }
    Ok(result)
}

/// Write `value` to `path` in the file's own style: 2-space pretty JSON, with a trailing newline
/// exactly when the original had one — so the migration's diff is the change, not the framing.
fn write_like(
    store: &dyn RepositoryStore,
    path: &str,
    value: &Value,
) -> Result<(), RepositoryError> {
    let had_newline = store
        .load_text_file(path)
        .map(|t| t.ends_with('\n'))
        .unwrap_or(false);
    let mut text =
        serde_json::to_string_pretty(value).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from(path),
            source,
        })?;
    if had_newline {
        text.push('\n');
    }
    store.save_text_file(path, &text)
}

/// Write the raw manifest back. A file tree takes the raw write (a revision-7 root container
/// cannot be represented by the typed manifest); `MemoryStore`, whose manifest is typed and
/// already revision-8 shaped, takes the typed stamp.
fn write_manifest_raw(
    store: &dyn RepositoryStore,
    manifest: &Value,
) -> Result<(), RepositoryError> {
    if store.is_file_tree_store() {
        return write_like(store, "manifest.json", manifest);
    }
    let mut typed = store.load_manifest()?;
    typed.extra.insert(
        crate::field_type_migration_service::DATA_MODEL_REVISION_KEY.to_string(),
        manifest[crate::field_type_migration_service::DATA_MODEL_REVISION_KEY].clone(),
    );
    store.save_manifest(&typed)
}

/// Pre-load transformer for a `.srsj` archive (RFC-043 Change I): the same outcome as the
/// registry entry, applied to the bundle's text before any store is built. Bundle forms stay out
/// of the registry (ADR-032's scope rule, the `migrate_rfc014` precedent). Idempotent.
pub fn migrate_srsj_str(content: &str) -> Result<(String, Rfc043Result), RepositoryError> {
    let store = crate::srsj::open_srsj(content)?.with_rfc038_exemption();
    let result = migrate_rfc043_container_entries(&store)?;
    Ok((crate::srsj::to_srsj_string(&store)?, result))
}

/// Pre-load transformer for a `package-bundle` / `.srspkg` JSON value: retire every
/// Composition section's `ordering.memberOrder` (a `container-subset` section becomes
/// `ordering.source: "arranged"`) and stamp revision 8. A bundle carries no containers, so the
/// order a `memberOrder` named cannot be frozen here — that is stated, not hidden: the number of
/// sections whose list was dropped is returned so the caller can report it.
pub fn migrate_package_bundle_value(bundle: &mut Value) -> usize {
    fn walk(v: &mut Value, dropped: &mut usize) {
        match v {
            Value::Object(o) => {
                if let Some(ordering) = o.get_mut("ordering").and_then(|x| x.as_object_mut()) {
                    if ordering.shift_remove("memberOrder").is_some() {
                        *dropped += 1;
                        ordering.insert("source".to_string(), json!("arranged"));
                    }
                }
                for x in o.values_mut() {
                    walk(x, dropped);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, dropped)),
            _ => {}
        }
    }
    let mut dropped = 0;
    walk(bundle, &mut dropped);
    if let Some(o) = bundle.as_object_mut() {
        if o.contains_key(crate::field_type_migration_service::DATA_MODEL_REVISION_KEY) {
            o.insert(
                crate::field_type_migration_service::DATA_MODEL_REVISION_KEY.to_string(),
                json!(RFC043_REVISION),
            );
        }
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use std::path::Path;

    const ROOT: &str = "11111111-0000-4000-8000-000000000001";
    const IDENT: &str = "22222222-0000-4000-8000-000000000001";
    const SEC_A: &str = "22222222-0000-4000-8000-0000000000aa";
    const SEC_B: &str = "22222222-0000-4000-8000-0000000000bb";
    const PART: &str = "11111111-0000-4000-8000-0000000000c1";
    const M1: &str = "22222222-0000-4000-8000-000000000011";
    const M2: &str = "22222222-0000-4000-8000-000000000012";
    const M3: &str = "22222222-0000-4000-8000-000000000013";

    fn w(dir: &Path, rel: &str, v: &Value) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(v).unwrap()).unwrap();
    }

    fn note(id: &str, created: &str) -> Value {
        json!({"instanceId": id, "title": id, "sections": [], "createdAt": created})
    }

    /// A revision-7 corpus: a root container (identity + two sections, roots and members,
    /// precedes B -> A so nav is B, A), a Part container (root M1; members M1, M2, M3) with a
    /// memberOrder section naming [M3, M1] and a precedes edge M1 -> M2.
    fn rev7_repo(dir: &Path, member_order: Value) {
        w(dir, ".srs/marker", &json!({}));
        w(
            dir,
            "manifest.json",
            &json!({
                "srsVersion": "2.0-draft", "dataModelRevision": 7,
                "repositoryId": "00000000-0000-4000-8000-0000000000f0",
                "namespace": "com.test", "packageRef": {"mode": "local", "path": "package"},
                "container": {
                    "containerId": ROOT, "title": "Root", "identityInstanceId": IDENT,
                    "anchorInstanceId": IDENT,
                    "rootInstanceIds": [IDENT],
                    "memberInstanceIds": [SEC_A, SEC_B]
                }
            }),
        );
        w(
            dir,
            "package/package.json",
            &json!({"id": "00000000-0000-4000-8000-00000000bbbb", "namespace": "com.test",
                "name": "p", "version": "1.0.0", "dataModelRevision": 7,
                "fields": [], "types": [], "compositions": ["compositions/doc.json"]}),
        );
        w(
            dir,
            "package/compositions/doc.json",
            &json!({"id": "00000000-0000-4000-8000-00000000cccc", "namespace": "com.test",
                "name": "doc", "version": 1, "description": "d",
                "sections": [
                    {"sectionId": "s1", "order": 0,
                     "source": {"type": "container-subset", "containerId": PART},
                     "ordering": {"memberOrder": [M3, M1]}},
                    {"sectionId": "s2", "order": 1,
                     "source": {"type": "container-subset", "containerId": PART}, "ordering": {"memberOrder": member_order}}
                ], "createdAt": "2026-01-01T00:00:00Z"}),
        );
        for (id, c) in [
            (IDENT, "2026-01-01T00:00:00Z"),
            (SEC_A, "2026-01-02T00:00:00Z"),
            (SEC_B, "2026-01-03T00:00:00Z"),
            (M1, "2026-01-04T00:00:00Z"),
            (M2, "2026-01-05T00:00:00Z"),
            (M3, "2026-01-06T00:00:00Z"),
        ] {
            w(dir, &format!("records/notes/note-{id}.json"), &note(id, c));
        }
        w(
            dir,
            "relations/aaaaaaaa-0000-4000-8000-000000000001.json",
            &json!({"relationId": "aaaaaaaa-0000-4000-8000-000000000001", "relationType": "precedes",
                "sourceInstanceId": SEC_B, "targetInstanceId": SEC_A, "createdAt": "2026-01-01T00:00:00Z"}),
        );
        w(
            dir,
            "relations/aaaaaaaa-0000-4000-8000-000000000002.json",
            &json!({"relationId": "aaaaaaaa-0000-4000-8000-000000000002", "relationType": "precedes",
                "sourceInstanceId": M1, "targetInstanceId": M2, "createdAt": "2026-01-01T00:00:00Z"}),
        );
        w(
            dir,
            "containers/part.json",
            &json!({"containerId": PART, "title": "Part", "rootInstanceIds": [M1],
                "memberInstanceIds": [M1, M2, M3]}),
        );
    }

    #[test]
    fn migrates_containers_compositions_and_stamps_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        rev7_repo(tmp.path(), json!([M3, M1]));
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        assert!(migration_needed(&store));

        let r = migrate_rfc043_container_entries(&store).unwrap();
        assert_eq!(r.containers_migrated, 2);
        assert_eq!(r.sections_flipped_to_arranged, 2);

        let m: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(m["dataModelRevision"], 8);
        let root = &m["container"];
        assert!(root.get("rootInstanceIds").is_none());
        // identity first, then the nav order: precedes B -> A means B, A
        let ids: Vec<&str> = root["memberInstanceIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["instanceId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![IDENT, SEC_B, SEC_A]);

        let part: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("containers/part.json")).unwrap(),
        )
        .unwrap();
        let ids: Vec<&str> = part["memberInstanceIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["instanceId"].as_str().unwrap())
            .collect();
        // listed first (M3, M1), then the rest (M2)
        assert_eq!(ids, vec![M3, M1, M2]);
        assert_eq!(
            part["anchorInstanceId"], M1,
            "anchor folded from rootInstanceIds[0]"
        );
        assert!(part.get("rootInstanceIds").is_none());

        let comp: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("package/compositions/doc.json")).unwrap(),
        )
        .unwrap();
        assert!(comp["sections"][0]["ordering"].get("memberOrder").is_none());
        assert_eq!(comp["sections"][0]["ordering"]["source"], "arranged");

        // alreadyApplied, and a second apply changes nothing.
        assert!(!migration_needed(&store));
        let before = std::fs::read_to_string(tmp.path().join("containers/part.json")).unwrap();
        migrate_rfc043_container_entries(&store).unwrap();
        assert_eq!(
            before,
            std::fs::read_to_string(tmp.path().join("containers/part.json")).unwrap()
        );
    }

    #[test]
    fn conflicting_member_order_lists_refuse_with_no_write() {
        let tmp = tempfile::tempdir().unwrap();
        rev7_repo(tmp.path(), json!([M1, M3]));
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let before = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
        let err = migrate_rfc043_container_entries(&store)
            .unwrap_err()
            .to_string();
        assert!(err.contains("migration-memberorder-conflict"), "{err}");
        assert_eq!(
            before,
            std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap(),
            "all-or-nothing: nothing written"
        );
        let part = std::fs::read_to_string(tmp.path().join("containers/part.json")).unwrap();
        assert!(part.contains("rootInstanceIds"), "container untouched");
    }

    #[test]
    fn dropped_member_order_ids_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        rev7_repo(tmp.path(), json!([M3, M1]));
        // memberOrder naming a non-member on both sections (same list: no conflict)
        let mut doc: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("package/compositions/doc.json")).unwrap(),
        )
        .unwrap();
        let ghost = "33333333-0000-4000-8000-0000000000ff";
        for s in doc["sections"].as_array_mut().unwrap() {
            s["ordering"]["memberOrder"] = json!([M3, ghost, M1]);
        }
        w(tmp.path(), "package/compositions/doc.json", &doc);
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let r = migrate_rfc043_container_entries(&store).unwrap();
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.starts_with("migration-memberorder-dropped") && d.contains(ghost)),
            "{:?}",
            r.diagnostics
        );
    }

    #[test]
    fn srsj_transformer_migrates_a_revision_7_archive_and_is_idempotent() {
        let srsj = json!({
            "srsj": "2",
            "manifest": {
                "srsVersion": "2.0-draft", "dataModelRevision": 7,
                "repositoryId": "00000000-0000-4000-8000-0000000000f0",
                "packageRef": {"mode": "local", "path": "package"},
                "container": {"containerId": ROOT, "title": "Root", "identityInstanceId": IDENT,
                    "rootInstanceIds": [IDENT], "memberInstanceIds": [SEC_A]}
            },
            "data": {
                "package/package.json": {"id": "00000000-0000-4000-8000-00000000bbbb",
                    "namespace": "com.test", "name": "p", "version": "1.0.0",
                    "dataModelRevision": 7, "fields": [], "types": []},
                "records/notes/note-a.json": note(IDENT, "2026-01-01T00:00:00Z"),
                "records/notes/note-b.json": note(SEC_A, "2026-01-02T00:00:00Z"),
            }
        })
        .to_string();
        let (out, r) = migrate_srsj_str(&srsj).unwrap();
        assert_eq!(r.containers_migrated, 1);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["manifest"]["dataModelRevision"], 8);
        assert!(v["manifest"]["container"].get("rootInstanceIds").is_none());
        assert_eq!(
            v["manifest"]["container"]["memberInstanceIds"],
            json!([{"instanceId": IDENT}, {"instanceId": SEC_A}])
        );
        let (again, r2) = migrate_srsj_str(&out).unwrap();
        assert_eq!(r2.containers_migrated, 0);
        assert_eq!(
            serde_json::from_str::<Value>(&again).unwrap(),
            serde_json::from_str::<Value>(&out).unwrap()
        );
    }

    #[test]
    fn package_bundle_transformer_retires_member_order() {
        let mut bundle = json!({"dataModelRevision": 7, "compositions": [{"sections": [
            {"sectionId": "s", "source": {"type": "container-subset", "containerId": PART},
             "ordering": {"memberOrder": [M1, M2], "direction": "asc"}}]}]});
        assert_eq!(migrate_package_bundle_value(&mut bundle), 1);
        let o = &bundle["compositions"][0]["sections"][0]["ordering"];
        assert!(o.get("memberOrder").is_none());
        assert_eq!(o["source"], "arranged");
        assert_eq!(bundle["dataModelRevision"], 8);
    }

    /// [R16]: a revision-8 binary does not interpret revision-7 container shapes — the ordinary
    /// (non-exempt) reader refuses naming RFC-043 and the migration, and `repo migrations` still
    /// reports it `needed` through the exempt reader.
    #[test]
    fn revision_8_reader_refuses_a_revision_7_corpus_naming_the_migration() {
        let tmp = tempfile::tempdir().unwrap();
        rev7_repo(tmp.path(), json!([M3, M1]));
        let strict = FileStore::new(tmp.path());
        let err = strict.load_manifest().unwrap_err();
        assert!(
            matches!(err, RepositoryError::Rfc043MigrationNeeded),
            "{err:?}"
        );
        assert!(err.to_string().contains("rfc043-container-entries"));
        let exempt = FileStore::new(tmp.path()).with_rfc038_exemption();
        let listed = crate::migration_registry_service::list_migrations(&exempt).unwrap();
        let m = listed.iter().find(|m| m.id == MIGRATION_ID).unwrap();
        assert_eq!(
            m.status,
            crate::migration_registry_service::MigrationStatus::Needed
        );
    }
}
