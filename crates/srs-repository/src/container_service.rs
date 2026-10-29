//! # Container Service
//!
//! Public API for container operations. This module is the sole entry point for
//! all container logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, container orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!   Specifically: `list_members`, `add_member`, `remove_member`, `is_member` are
//!   `pub(crate)` so that CLI and API handlers cannot call them directly — container
//!   scoping is the service's responsibility, not the caller's.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: ContainerPatch = serde_json::from_reader(io::stdin())?;
//! let result = container_service::update_container(store, id, input)?;
//! output::ok("container update", result)
//! ```

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use serde::{Deserialize, Deserializer, Serialize};
use srs_core::arrangement;
use srs_core::types::container::{Container, ContainerEntry};
use srs_core::validation::container::validate_container;
use srs_schema::{SchemaRegistry, CONTAINER_SCHEMA_ID};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSummary {
    pub container_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerPatch {
    pub title: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub container_type: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta: Option<serde_json::Value>,
    /// Double-`Option` (srs-rust#1293): `None` means the key was omitted — leave the field
    /// untouched; `Some(None)` means the key was sent as JSON `null` — clear the field.
    /// A single `Option<String>` cannot distinguish those two cases.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub identity_instance_id: Option<Option<String>>,
    pub anchor_instance_id: Option<String>,
    /// RFC-043: the whole ordered outline (replaces the arrangement; order is data, never sorted).
    pub member_instance_ids: Option<Vec<ContainerEntry>>,
    pub child_container_ids: Option<Vec<String>>,
}

/// Deserializes a present field (including JSON `null`) as `Some(_)`, so the caller can tell
/// "key omitted" (`None`, via `#[serde(default)]`) apart from "key sent as `null`" (`Some(None)`).
fn deserialize_some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerValidationReport {
    pub ok: bool,
    pub errors: Vec<String>,
}

/// Filter parameters for [`list_containers`]. No serde — this is a service contract, not a wire shape.
#[derive(Debug, Clone, Default)]
pub struct ContainerListFilter {
    pub container_type: Option<String>,
    pub member_instance_id: Option<String>,
    pub anchor_instance_id: Option<String>,
}

pub fn list_containers(
    store: &dyn RepositoryStore,
    filter: &ContainerListFilter,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    // RFC-052: the root container is an ordinary member of the container set. It is listed
    // first, as it was while the manifest embedded it.
    let mut summaries_raw = store.list_container_summaries()?;
    if let Some(root_id) = store.load_manifest()?.root_container_id() {
        if let Some(i) = summaries_raw.iter().position(|(id, _)| id == root_id) {
            let root = summaries_raw.remove(i);
            summaries_raw.insert(0, root);
        }
    }

    let mut summaries = Vec::new();
    for (container_id, _title) in summaries_raw {
        let container = store.load_container(&container_id)?;
        if let Some(ref ct) = filter.container_type {
            if container.container_type.as_deref() != Some(ct.as_str()) {
                continue;
            }
        }
        if let Some(ref member_filter) = filter.member_instance_id {
            // RFC-034 [R5]: the same effective(C) every other membership
            // consumer uses, not a second reading of it.
            if !effective_member_ids(store, &container)?
                .iter()
                .any(|id| id == member_filter)
            {
                continue;
            }
        }
        if let Some(ref root_filter) = filter.anchor_instance_id {
            // RFC-043 [R4]: roots are gone; "root" now means the declared anchor entry.
            if container.anchor_instance_id.as_deref() != Some(root_filter.as_str()) {
                continue;
            }
        }
        summaries.push(ContainerSummary {
            container_id: container.container_id.clone(),
            title: container.title.clone(),
            container_type: container.container_type,
        });
    }

    Ok(summaries)
}

/// Declared membership, inverted: instance id -> ids of the containers that list it directly
/// (RFC-034: declared, never derived). One pass over every container.
pub fn membership_index(
    store: &dyn RepositoryStore,
) -> Result<std::collections::HashMap<String, Vec<String>>, RepositoryError> {
    let mut index: std::collections::HashMap<String, Vec<String>> = Default::default();
    for summary in list_containers(store, &ContainerListFilter::default())? {
        let container = store.load_container(&summary.container_id)?;
        for id in direct_member_ids(&container) {
            index
                .entry(id)
                .or_default()
                .push(summary.container_id.clone());
        }
    }
    Ok(index)
}

pub fn containers_for_instance(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    list_containers(
        store,
        &ContainerListFilter {
            member_instance_id: Some(instance_id.to_string()),
            ..Default::default()
        },
    )
}

/// The one `container_create` input contract (RFC-043 revision-8 shape, no `rootInstanceIds`),
/// shared by the CLI, the WASM binding and the MCP tool so every adapter applies the same
/// rules: unknown keys are rejected, `containerId` is minted when omitted. It carries every
/// authorable property of `container.json` (only the `$schema` marker is left out), so no
/// adapter can set less than another.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
pub struct ContainerCreateInput {
    pub container_id: Option<String>,
    pub title: String,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub container_type: Option<String>,
    pub anchor_instance_id: Option<String>,
    pub identity_instance_id: Option<String>,
    pub member_instance_ids: Option<Vec<ContainerEntryInput>>,
    /// RFC-034 Change B: container ids of directly nested child scopes.
    pub child_container_ids: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Implementation-local metadata; cross-system keys should be namespaced.
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
}

/// One outline entry of [`ContainerCreateInput`].
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
pub struct ContainerEntryInput {
    pub instance_id: String,
    pub depth: Option<u32>,
}

impl From<ContainerCreateInput> for Container {
    fn from(i: ContainerCreateInput) -> Self {
        Container {
            container_id: i.container_id.unwrap_or_default(),
            title: i.title,
            namespace: i.namespace,
            name: i.name,
            description: i.description,
            container_type: i.container_type,
            identity_instance_id: i.identity_instance_id,
            anchor_instance_id: i.anchor_instance_id,
            member_instance_ids: i.member_instance_ids.map(|v| {
                v.into_iter()
                    .map(|e| ContainerEntry {
                        instance_id: e.instance_id,
                        depth: e.depth,
                    })
                    .collect()
            }),
            child_container_ids: i.child_container_ids,
            tags: i.tags,
            created_at: i.created_at,
            updated_at: i.updated_at,
            meta: i.meta.map(serde_json::Value::Object),
            extra: Default::default(),
        }
    }
}

pub fn create_container(
    store: &dyn RepositoryStore,
    mut container: Container,
) -> Result<Container, RepositoryError> {
    if container.container_id.is_empty() {
        container.container_id = new_instance_id();
    }

    // srs-rust#1167: create is a create, not an upsert — an id that already resolves
    // is refused.
    // Callers that mean "replace" go through `update_container`.
    match store.load_container(&container.container_id) {
        Ok(_) => {
            return Err(RepositoryError::ContainerAlreadyExists {
                container_id: container.container_id.clone(),
            })
        }
        Err(RepositoryError::ContainerNotFound { .. }) => {}
        Err(e) => return Err(e),
    }

    // Schema validation at service boundary
    let raw = serde_json::to_value(&container).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from("<stdin>"),
        source: e,
    })?;
    SchemaRegistry::global()
        .validate_by_id(CONTAINER_SCHEMA_ID, &raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from("<stdin>"),
            message: e.to_string(),
        })?;

    validate_container(&container)
        .map_err(|source| RepositoryError::ContainerValidation { source })?;

    require_resolvable_instances(store, declared_membership(&container))?;
    require_valid_arrangement(store, &container)?;
    require_resolvable_identity(store, &container)?;
    require_valid_child_containers(
        store,
        &container.container_id,
        container.child_container_ids.as_deref().unwrap_or(&[]),
    )?;

    store.save_container(&container)?;
    Ok(container)
}

pub fn get_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Container, RepositoryError> {
    let container = store.load_container(container_id)?;
    Ok(container)
}

/// Resolve the repository's root container: the container
/// `manifest.rootContainerId` names, read from the container set like any
/// other (RFC-052 Change A). Returns `Ok(None)` when the manifest names no root
/// container (a rootless legacy corpus below revision 10; at revision 10 the
/// load gate refuses that manifest, [R1]).
pub fn resolve_root_container(
    store: &dyn RepositoryStore,
    manifest: &crate::manifest::Manifest,
) -> Result<Option<Container>, RepositoryError> {
    manifest
        .root_container_id()
        .map(|id| store.load_container(id))
        .transpose()
}

/// Make `container` the repository's root container (RFC-052): write it to the container
/// set (at `containers/<containerId>.json` when it has no file yet) and point
/// `manifest.rootContainerId` at it. Updates `manifest` in place; the caller saves it.
pub fn set_root_container(
    store: &dyn RepositoryStore,
    manifest: &mut crate::manifest::Manifest,
    container: &Container,
) -> Result<(), RepositoryError> {
    crate::store::save_root_container(store, container)?;
    manifest.set_root_container_id(Some(container.container_id.clone()));
    Ok(())
}

/// Result of [`update_container`]: the updated container plus non-fatal
/// diagnostics (srs-rust#1026) reporting anything the whole-object replace
/// dropped that the caller may not have intended to drop.
#[derive(Debug, Clone)]
pub struct ContainerUpdateResult {
    pub container: Container,
    pub diagnostics: Vec<String>,
}

/// Diagnose a wholesale-replace patch field: names, in an informational
/// message, any id present in `before` but absent from `after`. Membership
/// replace semantics are unchanged by this — it is reporting only
/// (srs-rust#1026, muDemocracy.org#155): `container update` silently dropped
/// ids a sibling writer had added, because nothing treated "a container lost
/// members" as reportable.
fn diagnose_removed_ids(field: &str, before: &[String], after: &[String]) -> Option<String> {
    let after_set: HashSet<&String> = after.iter().collect();
    let mut removed: Vec<&String> = before.iter().filter(|id| !after_set.contains(id)).collect();
    if removed.is_empty() {
        return None;
    }
    removed.sort();
    let total = removed.len();
    const CAP: usize = 20;
    let listed: Vec<&str> = removed.iter().take(CAP).map(|s| s.as_str()).collect();
    let suffix = if total > CAP {
        format!(", and {} more", total - CAP)
    } else {
        String::new()
    };
    Some(format!(
        "container update replaced {field} and removed {total} id(s) not present in the patch: {}{suffix}",
        listed.join(", ")
    ))
}

pub fn update_container(
    store: &dyn RepositoryStore,
    container_id: &str,
    patch: ContainerPatch,
) -> Result<ContainerUpdateResult, RepositoryError> {
    let mut container = store.load_container(container_id)?;
    let before_members = container.member_ids();
    let before_children = container.child_container_ids.clone().unwrap_or_default();
    if let Some(v) = patch.title {
        container.title = v;
    }
    if let Some(v) = patch.namespace {
        container.namespace = Some(v);
    }
    if let Some(v) = patch.name {
        container.name = Some(v);
    }
    if let Some(v) = patch.description {
        container.description = Some(v);
    }
    if let Some(v) = patch.container_type {
        container.container_type = Some(v);
    }
    if let Some(v) = patch.tags {
        container.tags = Some(v);
    }
    if let Some(v) = patch.meta {
        container.meta = Some(v);
    }
    if let Some(v) = patch.identity_instance_id {
        container.identity_instance_id = v;
    }
    if let Some(ref v) = patch.anchor_instance_id {
        container.anchor_instance_id = Some(v.clone());
    }
    let mut diagnostics: Vec<String> = Vec::new();
    if let Some(v) = patch.member_instance_ids {
        // RFC-043 [R6]: order is data — the replacement is stored exactly as given.
        let after: Vec<String> = v.iter().map(|e| e.instance_id.clone()).collect();
        if let Some(d) = diagnose_removed_ids("memberInstanceIds", &before_members, &after) {
            diagnostics.push(d);
        }
        container.member_instance_ids = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(mut v) = patch.child_container_ids {
        v.sort();
        v.dedup();
        if let Some(d) = diagnose_removed_ids("childContainerIds", &before_children, &v) {
            diagnostics.push(d);
        }
        container.child_container_ids = if v.is_empty() { None } else { Some(v) };
    }

    // Schema validation at service boundary (after patch application)
    let raw = serde_json::to_value(&container).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(container_id),
        source: e,
    })?;
    SchemaRegistry::global()
        .validate_by_id(CONTAINER_SCHEMA_ID, &raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from(container_id),
            message: e.to_string(),
        })?;

    validate_container(&container)
        .map_err(|source| RepositoryError::ContainerValidation { source })?;

    require_resolvable_instances(store, declared_membership(&container))?;
    require_valid_arrangement(store, &container)?;
    require_resolvable_identity(store, &container)?;
    require_valid_child_containers(
        store,
        &container.container_id,
        container.child_container_ids.as_deref().unwrap_or(&[]),
    )?;

    store.save_container(&container)?;
    Ok(ContainerUpdateResult {
        container,
        diagnostics,
    })
}

pub fn delete_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<String, RepositoryError> {
    // RFC-052 [R6]: the container `manifest.rootContainerId` names is the repository's
    // identity object (owner ruling srs-rust#742); deleting it is refused, changing nothing.
    let manifest = store.load_manifest()?;
    if manifest.root_container_id() == Some(container_id) {
        return Err(RepositoryError::ContainerRootDelete {
            container_id: container_id.to_string(),
        });
    }

    // RFC-038 Change F: the [R22] cascade analogue for containers. A containerId
    // must never appear as a Relation endpoint (spec invariant), but a legacy or
    // hand-edited repo may carry such edges — remove them with the container so
    // the delete never leaves dangling endpoints behind.
    store.begin_batch();
    // RFC-034 [R7] (I-150): no `childContainerIds` entry may name a missing Container, so the
    // parents that list this container drop it in the same batch (srs-rust#1442).
    let write_result = crate::relation_service::delete_relations_incident_to(store, container_id)
        .and_then(|_| unlink_child_container(store, container_id))
        .and_then(|_| store.delete_container(container_id));
    match write_result {
        Ok(()) => store.commit_batch()?,
        Err(e) => {
            let _ = store.abort_batch();
            return Err(e);
        }
    }
    Ok(container_id.to_string())
}

/// Remove `child_id` from every container's `childContainerIds` (the field becomes `None` when
/// the list empties, as `update_container` stores it).
fn unlink_child_container(
    store: &dyn RepositoryStore,
    child_id: &str,
) -> Result<(), RepositoryError> {
    for (parent_id, _title) in store.list_container_summaries()? {
        if parent_id == child_id {
            continue;
        }
        let mut parent = store.load_container(&parent_id)?;
        let Some(children) = parent.child_container_ids.as_mut() else {
            continue;
        };
        if !children.iter().any(|c| c == child_id) {
            continue;
        }
        children.retain(|c| c != child_id);
        if children.is_empty() {
            parent.child_container_ids = None;
        }
        store.save_container(&parent)?;
    }
    Ok(())
}

/// Why a member of a container being deleted stays (srs-rust#1436).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeptReason {
    /// Another container (not being deleted) lists it.
    HeldElsewhere,
    /// A record that stays is `derived-from` it, directly or through other members.
    Lineage,
}

/// A member of a container being deleted that is kept, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptInstance {
    pub instance_id: String,
    pub reason: KeptReason,
}

/// What deleting some containers takes with them (srs-rust#1436).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerDeletePlan {
    /// The requested containers that exist: deleted.
    pub container_ids: Vec<String>,
    /// Requested ids naming no container: nothing to delete.
    pub missing_container_ids: Vec<String>,
    /// The instances only these containers hold (declared membership, RFC-034), in member order:
    /// deleted with them.
    pub instance_ids: Vec<String>,
    /// Their other members, which stay.
    pub kept: Vec<KeptInstance>,
}

/// **delete plan** (srs-rust#1436): what deleting `container_ids` takes with it, without writing.
/// A member goes with them when no other container lists it and no record that stays is
/// `derived-from` it, transitively (keeping a record keeps its lineage, which may keep more).
/// Refuses the RFC-013 root container, as [`delete_container`] does.
pub fn container_delete_plan(
    store: &dyn RepositoryStore,
    container_ids: &[String],
) -> Result<ContainerDeletePlan, RepositoryError> {
    let existing: HashSet<String> = list_containers(store, &ContainerListFilter::default())?
        .into_iter()
        .map(|c| c.container_id)
        .collect();
    let (ours, missing): (Vec<String>, Vec<String>) = container_ids
        .iter()
        .cloned()
        .partition(|id| existing.contains(id) || is_root_container(store, id));
    // RFC-052 [R6]: the root container is never deleted.
    if let Some(root) = ours.iter().find(|id| is_root_container(store, id)) {
        return Err(RepositoryError::ContainerRootDelete {
            container_id: root.clone(),
        });
    }

    let mut members: Vec<String> = Vec::new();
    for id in &ours {
        let container = store.load_container(id)?;
        for m in direct_member_ids(&container) {
            if !members.contains(&m) {
                members.push(m);
            }
        }
    }

    let index = membership_index(store)?;
    let mut kept = Vec::new();
    let mut going: Vec<String> = Vec::new();
    for id in members {
        let elsewhere = index
            .get(&id)
            .is_some_and(|cs| cs.iter().any(|c| !ours.contains(c)));
        if elsewhere {
            kept.push(KeptInstance {
                instance_id: id,
                reason: KeptReason::HeldElsewhere,
            });
        } else {
            going.push(id);
        }
    }

    // Lineage, to a fixed point: a record outside the going set that derives from a going one
    // keeps it, and keeping it may keep what that one derives from.
    let derived: Vec<_> = store
        .list_relations()?
        .into_iter()
        .filter(|r| r.relation_type == "derived-from")
        .collect();
    loop {
        let set: HashSet<&String> = going.iter().collect();
        let (stay, lineage): (Vec<String>, Vec<String>) = going.iter().cloned().partition(|id| {
            !derived
                .iter()
                .any(|r| &r.target_instance_id == id && !set.contains(&r.source_instance_id))
        });
        if lineage.is_empty() {
            break;
        }
        kept.extend(lineage.into_iter().map(|instance_id| KeptInstance {
            instance_id,
            reason: KeptReason::Lineage,
        }));
        going = stay;
    }

    Ok(ContainerDeletePlan {
        container_ids: ours,
        missing_container_ids: missing,
        instance_ids: going,
        kept,
    })
}

/// **delete with records** (srs-rust#1436): carry out [`container_delete_plan`]. The instances
/// go first (each through its own delete, incident relations cascaded), then the containers, so
/// a failure part-way leaves the containers and a retry plans again; it never leaves an
/// instance that belongs to nothing because its container went first.
pub fn delete_containers_with_records(
    store: &dyn RepositoryStore,
    container_ids: &[String],
) -> Result<ContainerDeletePlan, RepositoryError> {
    let plan = container_delete_plan(store, container_ids)?;
    for id in &plan.instance_ids {
        match store.find_instance(id)?.map(|r| r.tier) {
            Some(0) => {
                crate::services::delete_note(store, id, true)?;
            }
            Some(_) => {
                crate::record_store::delete_record(store, id, true)?;
            }
            None => {}
        }
    }
    for id in &plan.container_ids {
        delete_container(store, id)?;
    }
    Ok(plan)
}

/// The **one** typing-anchor resolution (RFC-009 Change A, amended by srs#446/I-145 and
/// RFC-043 [R4]): the declared `anchorInstanceId`, nothing else. The transitional
/// `rootInstanceIds[0]` fallback is withdrawn with `rootInstanceIds`. Pure — no I/O.
pub fn typing_anchor_instance_id(container: &Container) -> Option<String> {
    container.anchor_instance_id.clone()
}

/// RFC-042 Revision 5 [R22]'s anchor resolution (RFC-043 [R9]: the single-root fallback is
/// removed) — the declared anchor, as for typing.
pub(crate) fn r22_position_anchor_instance_id(container: &Container) -> Option<String> {
    container.anchor_instance_id.clone()
}

/// RFC-034 [R1]: a Container's **direct membership** — `rootInstanceIds` ∪
/// `memberInstanceIds`, in declared order, deduplicated. No traversal of any
/// kind: a `contains` Relation never adds a member (RFC-034 [R4]), and nesting
/// only ever follows a declared `childContainerIds` edge (see
/// [`effective_member_ids`]), never membership-array overlap ([R2]).
///
/// This is RFC-011 `containerScope: "explicit"`'s own scope (`direct(C)`) —
/// the shallow half; `"subtree"` and every other membership consumer wants
/// [`effective_member_ids`] instead.
fn direct_member_ids(container: &Container) -> Vec<String> {
    container.member_ids()
}

/// RFC-034 [R1] direct membership, indexed by each container's declared root(s),
/// as extra part-of children for `tree_service`/navigation (srs-rust#1096).
///
/// `container.json` declares direct membership as `rootInstanceIds ∪
/// memberInstanceIds`, but the part-of tree only ever descended `contains`
/// Relations — a container whose members are declared solely through the
/// membership arrays was therefore unreachable from `repo navigation`/`tree`
/// even though `repo validate` reported it healthy. This gives the tree
/// walker, keyed by a node's own id, the other direct members of every
/// container that names that node as a root — the node's own id is excluded
/// so a container isn't wired as its own child.
pub(crate) fn direct_children_by_root(
    store: &dyn RepositoryStore,
) -> Result<HashMap<String, Vec<String>>, RepositoryError> {
    let mut by_root: HashMap<String, Vec<String>> = HashMap::new();
    for summary in list_containers(store, &ContainerListFilter::default())? {
        let container = get_container(store, &summary.container_id)?;
        // RFC-043 [R19]: the section container of a record is the Container whose
        // `anchorInstanceId` equals it.
        let Some(root_id) = container.anchor_instance_id.clone() else {
            continue;
        };
        let entry = by_root.entry(root_id.clone()).or_default();
        for id in direct_member_ids(&container) {
            if id != root_id && !entry.contains(&id) {
                entry.push(id);
            }
        }
    }
    Ok(by_root)
}

/// RFC-034 [R3]/Change B: a Container's **effective membership** — the least
/// fixed point `effective(C) = direct(C) ∪ ⋃ effective(child)` for every
/// `child` admitted through `C.childContainerIds`. The **one** membership
/// operation every consumer that asks "is this a member" routes through
/// (`find --container`, `container resolve-view`, the MCP container resource,
/// `containers_for_instance`, RFC-012 `containerId` filtering).
///
/// Follows only the declared `childContainerIds` edge — never a `contains`
/// Relation (RFC-034 Change C) and never membership-array overlap (RFC-034
/// [R2]). [R7] requires every `childContainerIds` entry to resolve to an
/// existing, distinct, acyclically-reachable Container; `require_valid_child_containers`
/// enforces that at write time, so the `visited_containers` guard here is a
/// defensive backstop, not the primary cycle defence.
///
/// [R3] states the result is unordered and duplicate-free and explicitly
/// disclaims any ordering from either membership array or from
/// `childContainerIds` order — insertion order (this container's own
/// `direct(C)`, then each declared child's `effective` set in listed order)
/// is deterministic but carries no semantic meaning; `precedes` and
/// Composition-owned ordering are the applicable ordering mechanisms.
///
/// `doctor_service`'s reachability check reads `memberInstanceIds` /
/// `rootInstanceIds` / `identityInstanceId` directly and deliberately does not
/// route through this: it answers a different question — "does any container
/// *declare* a reference to this id", the [R13] dangling-reference question —
/// for which an effective-only (nested) member is not a reference at all.
fn effective_member_ids(
    store: &dyn RepositoryStore,
    container: &Container,
) -> Result<Vec<String>, RepositoryError> {
    let mut combined = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut visited_containers: HashSet<String> = HashSet::new();
    effective_member_ids_into(
        store,
        container,
        &mut combined,
        &mut seen,
        &mut visited_containers,
    )?;
    Ok(combined)
}

fn effective_member_ids_into(
    store: &dyn RepositoryStore,
    container: &Container,
    combined: &mut Vec<String>,
    seen: &mut HashSet<String>,
    visited_containers: &mut HashSet<String>,
) -> Result<(), RepositoryError> {
    if !visited_containers.insert(container.container_id.clone()) {
        return Ok(());
    }
    for id in direct_member_ids(container) {
        if seen.insert(id.clone()) {
            combined.push(id);
        }
    }
    for child_id in container.child_container_ids.iter().flatten() {
        let child = store.load_container(child_id)?;
        effective_member_ids_into(store, &child, combined, seen, visited_containers)?;
    }
    Ok(())
}

/// RFC-034 [R7]: every `childContainerIds` entry MUST reference an existing,
/// distinct Container, and the whole `childContainerIds` graph MUST be
/// acyclic. Checked at write time (`create_container`/`update_container`) so a
/// bad edit is rejected before it can leave any Container's `effective(C)`
/// undefined ([R7]: an operation requiring `effective(C)` over a broken graph
/// MUST fail with a diagnostic, never return a partial set).
fn require_valid_child_containers(
    store: &dyn RepositoryStore,
    container_id: &str,
    child_ids: &[String],
) -> Result<(), RepositoryError> {
    for child_id in child_ids {
        if child_id == container_id {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "childContainerIds: '{child_id}' cannot be its own child (self-reference)"
                ),
            });
        }
    }
    // Walk the declared graph reachable from the proposed children, failing on
    // a missing target (a `containerId` that isn't a Container at all — e.g.
    // an instance id) or a path back to `container_id` (a cycle).
    let mut stack: Vec<String> = child_ids.to_vec();
    let mut visited: HashSet<String> = HashSet::new();
    while let Some(id) = stack.pop() {
        if id == container_id {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "childContainerIds graph contains a cycle back to '{container_id}'"
                ),
            });
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        let child = store.load_container(&id).map_err(|e| match e {
            RepositoryError::ContainerNotFound { container_id } => RepositoryError::InvalidInput {
                message: format!(
                    "childContainerIds: '{container_id}' does not resolve to an existing Container"
                ),
            },
            other => other,
        })?;
        for grandchild in child.child_container_ids.iter().flatten() {
            stack.push(grandchild.clone());
        }
    }
    Ok(())
}

/// RFC-034 [R7] / I-150 at rest: every `childContainerIds` entry MUST name an
/// existing, distinct Container and the graph MUST be acyclic. The write-time
/// twin is `require_valid_child_containers`; this one reads a whole container
/// set, so `repo validate` (CLI, MCP, WASM alike) reports a dangling id that
/// reached the tree by another route (delete, merge, hand edit, older tool).
/// Each message names the parent container and the offending child id; a cycle
/// is reported once, on the edge that closes it.
pub fn child_container_diagnostics(containers: &[Container]) -> Vec<String> {
    use std::collections::HashMap;
    let by_id: HashMap<&str, &Container> = containers
        .iter()
        .map(|c| (c.container_id.as_str(), c))
        .collect();
    let mut out: Vec<String> = Vec::new();
    for parent in containers {
        for child in parent.child_container_ids.iter().flatten() {
            if child == &parent.container_id {
                out.push(format!(
                    "RFC-034 [R7] I-150: container '{}' lists itself in childContainerIds (self-reference)",
                    parent.container_id
                ));
            } else if !by_id.contains_key(child.as_str()) {
                out.push(format!(
                    "RFC-034 [R7] I-150: container '{}' childContainerIds entry '{}' does not name an existing Container",
                    parent.container_id, child
                ));
            }
        }
    }
    // Cycles of length >= 2: depth-first, three-colour; a back edge closes one.
    let mut state: HashMap<&str, u8> = HashMap::new(); // 1 = on path, 2 = done
    let mut ids: Vec<&str> = by_id.keys().copied().collect();
    ids.sort_unstable();
    for start in ids {
        if state.contains_key(start) {
            continue;
        }
        let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
        state.insert(start, 1);
        while let Some(&(id, i)) = stack.last() {
            let kids: &[String] = by_id[id].child_container_ids.as_deref().unwrap_or(&[]);
            if i >= kids.len() {
                state.insert(id, 2);
                stack.pop();
                continue;
            }
            stack.last_mut().unwrap().1 += 1;
            let kid = kids[i].as_str();
            if kid == id || !by_id.contains_key(kid) {
                continue;
            }
            match state.get(kid) {
                Some(1) => out.push(format!(
                    "RFC-034 [R7] I-150: container '{id}' childContainerIds entry '{kid}' closes a cycle in the childContainerIds graph"
                )),
                Some(_) => {}
                None => {
                    state.insert(kid, 1);
                    stack.push((kid, 0));
                }
            }
        }
    }
    out
}

/// Effective membership (RFC-034 [R3]) — the deep, recursive scope. Every
/// membership-listing consumer wants this: `find --container`, `container
/// resolve-view`, the MCP container resource, `containers_for_instance`,
/// RFC-012 `containerId` filtering (via `list_records_filtered`). RFC-011
/// `containerScope: "explicit"` wants [`list_direct_members`] instead.
pub fn list_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(store, container_id)?;
    effective_member_ids(store, &container)
}

/// `container_id` plus every container reachable through `childContainerIds`
/// (the containers whose members make up `list_members`).
pub fn container_closure(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![container_id.to_string()];
    while let Some(id) = stack.pop() {
        if out.contains(&id) {
            continue;
        }
        let c = store.load_container(&id)?;
        stack.extend(c.child_container_ids.into_iter().flatten());
        out.push(id);
    }
    Ok(out)
}

/// Direct membership (RFC-034 [R1]) — this container's own `rootInstanceIds`
/// ∪ `memberInstanceIds`, with no recursion into `childContainerIds`. RFC-011
/// `containerScope: "explicit"`'s scope; every other membership consumer
/// wants [`list_members`] (effective) instead.
pub(crate) fn list_direct_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(store, container_id)?;
    Ok(direct_member_ids(&container))
}

/// Membership writes may only name instances that actually exist.
///
/// A container reference that resolves to nothing is a fatal [R13] catalog
/// diagnostic, so persisting one turns the repository into something no command
/// can load — including the `remove` that would undo it. The check therefore
/// belongs here, ahead of any write, in the one path the CLI, MCP and WASM
/// adapters all route through (ADR-010; srs-rust#841). Blank ids get their own
/// message: `InstanceNotFound { id: "" }` reads as a lookup failure when the real
/// problem is an empty argument.
///
/// This is the *single* guard for every membership-writing entry point: the
/// incremental writers `add_member`/`add_root` pass one id, the wholesale
/// writers `create_container`/`update_container` pass their entire membership
/// list (srs-rust#845). An empty list loads no catalog, so a container with no
/// membership needs nothing from this function.
///
/// Blank ids are rejected in a first pass, **before** the catalog is built:
/// their message is the whole point of separating them, and on a repository
/// whose catalog build is already fatal it is the only way the caller hears
/// "you passed an empty argument" instead of a `CatalogLoad` about unrelated
/// damage.
fn require_resolvable_instances<'a>(
    store: &dyn RepositoryStore,
    instance_ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), RepositoryError> {
    let ids: Vec<&str> = instance_ids.into_iter().collect();
    for instance_id in &ids {
        if instance_id.trim().is_empty() {
            return Err(RepositoryError::InvalidInput {
                message: "instance_id must not be empty".to_string(),
            });
        }
    }
    if ids.is_empty() {
        return Ok(());
    }
    let catalog = store.catalog()?;
    for instance_id in ids {
        if !catalog.instances.iter().any(|e| e.id == instance_id) {
            return Err(RepositoryError::InstanceNotFound {
                id: instance_id.to_string(),
            });
        }
    }
    Ok(())
}

/// Every membership id a container declares — `rootInstanceIds` ∪
/// `memberInstanceIds`. `identityInstanceId` is deliberately excluded: it is not
/// a container reference in the [R13] reference set (srs-rust#837 leaves that
/// question — whether a dangling identity should become a catalog/[R13]
/// diagnostic — open; see `require_resolvable_identity` for the write-time
/// check that does apply to it).
fn declared_membership(container: &Container) -> impl Iterator<Item = &str> {
    container
        .member_instance_ids
        .iter()
        .flatten()
        .map(|e| e.instance_id.as_str())
}

/// RFC-043 [R2] write-time arrangement check: depth rules, exactly-once, and (root
/// container) the identity entry at depth 0 with no descendants.
fn require_valid_arrangement(
    store: &dyn RepositoryStore,
    container: &Container,
) -> Result<(), RepositoryError> {
    let Some(entries) = &container.member_instance_ids else {
        return Ok(());
    };
    let identity = root_identity(store, &container.container_id, container);
    match arrangement::check_entries(entries, identity)
        .into_iter()
        .next()
    {
        Some(v) => Err(arrangement_error(v)),
        None => Ok(()),
    }
}

fn arrangement_error(v: arrangement::Violation) -> RepositoryError {
    RepositoryError::InvalidInput {
        message: v.to_string(),
    }
}

/// `identityInstanceId` must name an instance that actually exists.
///
/// Not membership (see `declared_membership`'s doc) and not folded into
/// `require_resolvable_instances`'s caller list: an unresolvable identity is
/// checked, but doesn't need the empty-list short-circuit or the blank-id
/// distinction membership writes want, since a container legitimately carries
/// no identity at all (`None` is skipped here).
///
/// srs-rust#837: before this check, `container update`/`container create`
/// accepted any string here with no existence check, so a typo or a stale id
/// silently produced a dangling `identityInstanceId` — invisible to `repo
/// validate` at [R13]-fatal severity (it's just the RFC-013 I-81 warning) and
/// fatal to `repo navigation`, which hard-fails instead of degrading the way
/// it does for a *cleared* identity (srs-rust#843).
fn require_resolvable_identity(
    store: &dyn RepositoryStore,
    container: &Container,
) -> Result<(), RepositoryError> {
    if let Some(ref id) = container.identity_instance_id {
        require_resolvable_instances(store, [id.as_str()])?;
    }
    Ok(())
}

/// Result of a member operation (RFC-043 Change D): the container's arrangement after the
/// operation, the ids whose depth was lowered by a promoting removal ([R7]: the operation
/// reports them), and the ids it removed.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArrangementResult {
    pub members: Vec<ContainerEntry>,
    #[serde(default)]
    pub promoted: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    /// RFC-051 [R11]: the detached entries (orphans and loop breaks) repair re-attached at
    /// the end of the top level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reattached: Vec<String>,
}

pub(crate) fn is_root_container(store: &dyn RepositoryStore, container_id: &str) -> bool {
    store
        .load_manifest()
        .ok()
        .is_some_and(|m| m.root_container_id() == Some(container_id))
}

fn pointer_error(container: &Container, instance_id: &str, what: &str) -> RepositoryError {
    arrangement_error(arrangement::Violation {
        code: arrangement::CODE_POINTER,
        instance_id: instance_id.to_string(),
        message: format!(
            "{} of container {} names this entry; move the pointer to another member first",
            what, container.container_id
        ),
    })
}

fn require_not_pointer(container: &Container, instance_id: &str) -> Result<(), RepositoryError> {
    if container.identity_instance_id.as_deref() == Some(instance_id) {
        return Err(pointer_error(container, instance_id, "identityInstanceId"));
    }
    if container.anchor_instance_id.as_deref() == Some(instance_id) {
        return Err(pointer_error(container, instance_id, "anchorInstanceId"));
    }
    Ok(())
}

/// Flat arrangement read ([R15]): the container's entries with depth, in order.
pub fn get_arrangement(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<ContainerEntry>, RepositoryError> {
    Ok(get_container(store, container_id)?
        .member_instance_ids
        .unwrap_or_default())
}

fn store_entries(container: &mut Container, entries: Vec<ContainerEntry>) {
    container.member_instance_ids = if entries.is_empty() {
        None
    } else {
        Some(entries)
    };
}

/// **add** / **insert** (Change D). With no `position` the entry is appended at `depth`
/// (default 0). An id that is already a member is returned unchanged unless a position or
/// depth was asked for (use [`move_member`]).
pub fn add_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
    position: Option<usize>,
    depth: Option<u32>,
) -> Result<ArrangementResult, RepositoryError> {
    require_resolvable_instances(store, [instance_id])?;
    let mut container = store.load_container(container_id)?;
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    if entries.iter().any(|e| e.instance_id == instance_id) {
        if position.is_some() || depth.is_some() {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "{instance_id} is already a member of {container_id}; use move to reposition it"
                ),
            });
        }
        return Ok(ArrangementResult {
            members: entries,
            ..Default::default()
        });
    }
    let identity = root_identity(store, container_id, &container);
    let out = arrangement::insert(
        &entries,
        ContainerEntry::at(instance_id, depth.unwrap_or(0)),
        position,
        identity,
    )
    .map_err(arrangement_error)?;
    store_entries(&mut container, out.clone());
    store.save_container(&container)?;
    Ok(ArrangementResult {
        members: out,
        ..Default::default()
    })
}

/// **remove** (Change D, [R7]) — the promoting removal. A **repair**-seam operation
/// (ADR-045): it reads and writes through the unchecked catalog so it still works on a
/// repository bricked by a dangling reference (srs-rust#841). Rejected with
/// `arrangement-pointer` when the entry is the container's identity or anchor.
pub fn remove_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<ArrangementResult, RepositoryError> {
    let mut container = store.load_container_unchecked(container_id)?;
    require_not_pointer(&container, instance_id)?;
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    let Some((out, promoted)) = arrangement::remove_promoting(&entries, instance_id) else {
        return Ok(ArrangementResult {
            members: entries,
            ..Default::default()
        });
    };
    store_entries(&mut container, out.clone());
    store.save_container_unchecked(&container)?;
    Ok(ArrangementResult {
        members: out,
        promoted,
        removed: vec![instance_id.to_string()],
        ..Default::default()
    })
}

/// **move** / **set depth** (Change D): move the run of `instance_id` to `position`
/// (against the list without the run; default: where it is) taking `depth` (default:
/// its current depth).
pub fn move_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
    position: Option<usize>,
    depth: Option<u32>,
) -> Result<ArrangementResult, RepositoryError> {
    rearrange(
        store,
        container_id,
        Some(instance_id),
        |entries, identity| {
            arrangement::move_run(entries, instance_id, position, depth, identity)
                .map_err(arrangement_error)
        },
    )
}

/// The identity entry the [R2] identity rule pins: only the root container has one.
fn root_identity<'a>(
    store: &dyn RepositoryStore,
    container_id: &str,
    container: &'a Container,
) -> Option<&'a str> {
    is_root_container(store, container_id)
        .then_some(container.identity_instance_id.as_deref())
        .flatten()
}

/// The shared write path of the arrangement edits: load the container, apply `op` to its
/// entries with the root container's identity (for the [R2] identity rule), persist the result.
/// `moved` is the entry a move operation moves (RFC-051 [R8]: it is written even where its
/// parent is unchanged).
fn rearrange(
    store: &dyn RepositoryStore,
    container_id: &str,
    moved: Option<&str>,
    op: impl FnOnce(&[ContainerEntry], Option<&str>) -> Result<Vec<ContainerEntry>, RepositoryError>,
) -> Result<ArrangementResult, RepositoryError> {
    let mut container = store.load_container(container_id)?;
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    let identity = root_identity(store, container_id, &container);
    let out = op(&entries, identity)?;
    store_entries(&mut container, out.clone());
    store.save_container_moving(&container, moved)?;
    Ok(ArrangementResult {
        members: out,
        ..Default::default()
    })
}

/// **move relative** (Change D, issue #1156): `before` / `after` / `into` another entry,
/// `indent` / `outdent`, or `up` / `down` one sibling step — each resolved to the one
/// `move_run` validity path in `srs_core::arrangement`.
pub fn move_member_relative(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
    mv: &arrangement::RelativeMove,
) -> Result<ArrangementResult, RepositoryError> {
    rearrange(
        store,
        container_id,
        Some(instance_id),
        |entries, identity| {
            mv.apply(entries, instance_id, identity)
                .map_err(arrangement_error)
        },
    )
}

/// **add relative**: add a new member and place it `before` / `after` / `into` `target` in one
/// write. Rejected (nothing written) when `instance_id` is already a member.
pub fn add_member_relative(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
    target: &str,
    placement: arrangement::Placement,
) -> Result<ArrangementResult, RepositoryError> {
    require_resolvable_instances(store, [instance_id])?;
    rearrange(store, container_id, None, |entries, identity| {
        if entries.iter().any(|e| e.instance_id == instance_id) {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "{instance_id} is already a member of {container_id}; use move to reposition it"
                ),
            });
        }
        arrangement::insert(entries, ContainerEntry::new(instance_id), None, identity)
            .and_then(|e| arrangement::place(&e, instance_id, target, placement, identity))
            .map_err(arrangement_error)
    })
}

/// **replace** (srs-rust#1136): swap member ids in place, keeping each entry's position and
/// depth, in ONE write. Each `old` must be a member and not the identity/anchor pointer; each
/// `new` must not already be a member. The fork service's swap.
pub fn replace_members(
    store: &dyn RepositoryStore,
    container_id: &str,
    swaps: &[(String, String)],
) -> Result<ArrangementResult, RepositoryError> {
    let new_ids: Vec<&str> = swaps.iter().map(|(_, n)| n.as_str()).collect();
    require_resolvable_instances(store, new_ids.iter().copied())?;
    let mut container = store.load_container(container_id)?;
    let mut entries = container.member_instance_ids.clone().unwrap_or_default();
    for (old, new) in swaps {
        require_not_pointer(&container, old)?;
        if entries.iter().any(|e| e.instance_id == *new) {
            return Err(RepositoryError::InvalidInput {
                message: format!("{new} is already a member of {container_id}"),
            });
        }
        let e = entries
            .iter_mut()
            .find(|e| e.instance_id == *old)
            .ok_or_else(|| RepositoryError::InvalidInput {
                message: format!("{old} is not a member of {container_id}"),
            })?;
        e.instance_id = new.clone();
    }
    store_entries(&mut container, entries.clone());
    store.save_container(&container)?;
    Ok(ArrangementResult {
        members: entries,
        ..Default::default()
    })
}

/// Input of [`copy_container`]. Both keys optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
pub struct ContainerCopyInput {
    /// Default: `"<source title> (copy)"`.
    pub title: Option<String>,
    /// Default: minted. An existing id is refused.
    pub container_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerCopyResult {
    pub container: Container,
    /// The anchor fork (`derived-from` its original) when the source had an anchor.
    pub forks: Vec<crate::fork_service::ForkPair>,
    pub relations: Vec<srs_core::types::relation::Relation>,
}

/// **copy** (srs-rust#1136): a new container that SHARES the source's member records
/// (arrangement copied verbatim, `childContainerIds` shared by reference). The one exception
/// is the anchor (title) record: it is forked through the fork core and the new container's
/// anchor entry / `anchorInstanceId` / `identityInstanceId` (if it named the anchor) point at
/// the fork, so no two containers share an anchor. No member record is duplicated. An
/// `identityInstanceId` naming a record other than the anchor stays shared (v1).
pub fn copy_container(
    store: &dyn RepositoryStore,
    source_id: &str,
    input: ContainerCopyInput,
) -> Result<ContainerCopyResult, RepositoryError> {
    if is_root_container(store, source_id) {
        return Err(RepositoryError::ContainerIsRepositoryRoot {
            container_id: source_id.to_string(),
        });
    }
    let source = get_container(store, source_id)?;
    let mut copy = source.clone();
    copy.container_id = input.container_id.unwrap_or_else(new_instance_id);
    copy.title = input
        .title
        .unwrap_or_else(|| format!("{} (copy)", source.title));
    let now = chrono::Utc::now().to_rfc3339();
    copy.created_at = Some(now.clone());
    copy.updated_at = Some(now);
    // Fail early on an existing id, before forking anything.
    match store.load_container(&copy.container_id) {
        Ok(_) => {
            return Err(RepositoryError::ContainerAlreadyExists {
                container_id: copy.container_id,
            })
        }
        Err(RepositoryError::ContainerNotFound { .. }) => {}
        Err(e) => return Err(e),
    }
    let (forks, relations) = match source.anchor_instance_id.clone() {
        Some(anchor) => crate::fork_service::fork_records(store, &[anchor])?,
        None => (vec![], vec![]),
    };
    if let Some(pair) = forks.first() {
        let swap = |id: &mut String| {
            if *id == pair.original_id {
                *id = pair.fork_id.clone();
            }
        };
        copy.anchor_instance_id.as_mut().map(swap);
        copy.identity_instance_id.as_mut().map(swap);
        for e in copy.member_instance_ids.iter_mut().flatten() {
            swap(&mut e.instance_id);
        }
    }
    let swapped = forks
        .first()
        .map(|p| (p.original_id.clone(), p.fork_id.clone()));
    match create_container(store, copy).and_then(|container| {
        // RFC-051 [R1]: a copy keeps every entry's `parent` and `key`.
        use crate::container_store::{copy_entries, has_head};
        if has_head(store, source_id) && has_head(store, &container.container_id) {
            copy_entries(
                store,
                source_id,
                &container.container_id,
                |id| match &swapped {
                    Some((from, to)) if from == id => to.clone(),
                    _ => id.to_string(),
                },
            )?;
        }
        Ok(container)
    }) {
        Ok(container) => Ok(ContainerCopyResult {
            container,
            forks,
            relations,
        }),
        Err(e) => {
            for p in &forks {
                crate::record_store::attempt_rollback_delete(store, &p.fork_id);
            }
            Err(e)
        }
    }
}

/// The structured outline read ([R15], issue #1156): the whole arrangement with derived
/// parent / run (`entries`) and the document `body` — the entries that remain once the
/// container's anchor and identity entries are set aside by the promoting removal ([R7]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerOutline {
    pub container_id: String,
    pub anchor_instance_id: Option<String>,
    pub identity_instance_id: Option<String>,
    pub entries: Vec<arrangement::OutlineEntry>,
    pub body: Vec<arrangement::OutlineEntry>,
}

/// Read a container's outline. The body rule is the renderer's: the anchor is the section
/// lead and never a plain member ([`r22_position_anchor_instance_id`]); the identity entry
/// is the container's identity record, which navigation excludes (`identityInstanceId`).
pub fn get_outline(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<ContainerOutline, RepositoryError> {
    let container = get_container(store, container_id)?;
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    let anchor = r22_position_anchor_instance_id(&container);
    let identity = container.identity_instance_id.clone();
    let (body, _, _) = arrangement::retain_promoting(&entries, |id| {
        anchor.as_deref() != Some(id) && identity.as_deref() != Some(id)
    });
    Ok(ContainerOutline {
        container_id: container.container_id.clone(),
        anchor_instance_id: anchor,
        identity_instance_id: identity,
        entries: arrangement::outline(&entries),
        body: arrangement::outline(&body),
    })
}

/// **repair** (Change D, [R20]) — remove every entry whose `instanceId` does not resolve,
/// each by the promoting removal; report each. Exempt from the pointer guard; never touches
/// `identityInstanceId` or `anchorInstanceId` (a dangling pointer stays a reported
/// validation error). Idempotent. Repair seam: reads through the unchecked catalog.
///
/// RFC-051 [R11]: in a stored directory it also re-attaches every detached entry (orphan or
/// loop break) at the end of the top level, never removing one; and a headless directory
/// named by `container_id` is removed whole, its entry ids reported as removed.
pub fn repair_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<ArrangementResult, RepositoryError> {
    let mut container = match store.load_container_unchecked(container_id) {
        Err(e @ RepositoryError::ContainerNotFound { .. }) => {
            return match crate::container_store::remove_headless(store, container_id)? {
                Some(removed) => Ok(ArrangementResult {
                    removed,
                    ..Default::default()
                }),
                None => Err(e),
            };
        }
        other => other?,
    };
    let known: HashSet<String> = store
        .catalog_unchecked()?
        .instances
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    let (out, removed, promoted) = arrangement::retain_promoting(&entries, |id| known.contains(id));
    let reattached: Vec<String> = crate::container_store::detached(store, container_id)?
        .into_iter()
        .filter(|id| known.contains(id))
        .collect();
    if removed.is_empty() && reattached.is_empty() {
        return Ok(ArrangementResult {
            members: entries,
            ..Default::default()
        });
    }
    store_entries(&mut container, out.clone());
    store.repair_container_unchecked(&container)?;
    Ok(ArrangementResult {
        members: out,
        promoted,
        removed,
        reattached,
    })
}

pub fn list_roots(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    // RFC-043 [R4]: `rootInstanceIds` is removed; a container's root is its anchor entry.
    let container = get_container(store, container_id)?;
    Ok(container.anchor_instance_id.into_iter().collect())
}

/// RFC-038 Change F: remove `instance_id` from `memberInstanceIds` and
/// `rootInstanceIds` of every Container that names it. Called from the instance
/// delete paths (srs-rust#834) so a delete never leaves a dangling container
/// reference — `SRS038-R13-DANGLING-REFERENCE`, which [R24] makes fatal, i.e. a
/// successful delete would render the repository unloadable.
///
/// RFC-052 [R4] (amending RFC-038 [R22]): a routine instance delete must not modify
/// `manifest.json`, and a record delete's cascade must remove that record's entries from the
/// root container as from any other. Since the root is stored in the container set, every
/// write here is a container file; the manifest is never written.
///
/// `containers_for_instance` matches `memberInstanceIds` across every container, the root
/// included — exactly the set the catalog draws its container references from.
///
/// Revision 8 (RFC-043 [R7]): each container drops the entry by the promoting removal
/// (descendants move up one level). An entry named by `identityInstanceId` or
/// `anchorInstanceId` is never silently cleared; the cascade is rejected whole
/// (`arrangement-pointer`) before any container is written, so the caller must re-point or
/// clear the pointer first.
///
/// The edits are applied to each loaded container and written once, rather than through
/// [`remove_member`], so the pointer check covers every container before the first write.
pub(crate) fn remove_instance_from_all_containers(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<(), RepositoryError> {
    // RFC-043 [R7]: a cascade that would remove the entry named by `identityInstanceId` or
    // `anchorInstanceId` is rejected whole, before any container is written.
    let mut loaded = Vec::new();
    for summary in containers_for_instance(store, instance_id)? {
        let container = store.load_container(&summary.container_id)?;
        if container.has_member(instance_id) {
            require_not_pointer(&container, instance_id)?;
            loaded.push(container);
        }
    }
    for mut container in loaded {
        let entries = container.member_instance_ids.clone().unwrap_or_default();
        let Some((out, _)) = arrangement::remove_promoting(&entries, instance_id) else {
            continue;
        };
        store_entries(&mut container, out);
        store.save_container(&container)?;
    }
    Ok(())
}

pub(crate) fn is_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<bool, RepositoryError> {
    let members = list_members(store, container_id)?;
    Ok(members.iter().any(|id| id == instance_id))
}

pub fn validate_container_invariants(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<ContainerValidationReport, RepositoryError> {
    // One catalog snapshot, via the unchecked builder — not `store.catalog()`
    // ([R24] fatal) and not `get_container` (which routes through it): this
    // function's entire purpose is to *report* an invalid container (e.g. a
    // dangling member/root reference) as a validation report, mirroring
    // `repo validate`'s [R24] exemption. The container we are validating is
    // by construction already persisted, so its own dangling reference would
    // otherwise fail the fatal catalog build before this function ever got
    // to describe the problem.
    let cat = store.catalog_unchecked()?;
    let container: Container = match cat
        .containers
        .iter()
        .find(|e| e.id == container_id)
        .and_then(|e| e.locator.as_deref())
    {
        Some(path) => serde_json::from_value(crate::container_store::logical_value(store, path)?)
            .map_err(|source| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from(path),
            source,
        })?,
        None => {
            return Err(RepositoryError::ContainerNotFound {
                container_id: container_id.to_string(),
            })
        }
    };
    let mut errors = Vec::new();
    if let Err(err) = validate_container(&container) {
        errors.push(err.to_string());
    }

    // RFC-013 [R6]/[R9] as amended by RFC-038 [R25]: resolved against the
    // same snapshot's instance set, not `manifest.instanceIndex`.
    let known_ids: HashSet<String> = cat.instances.iter().map(|e| e.id.clone()).collect();

    if let Some(ref entries) = container.member_instance_ids {
        for e in entries {
            if e.instance_id == container.container_id {
                errors.push("containerId must not appear in memberInstanceIds".to_string());
            }
            if !known_ids.contains(&e.instance_id) {
                errors.push(format!(
                    "{}: memberInstanceId '{}' not found in the instance set",
                    arrangement::CODE_UNRESOLVED,
                    e.instance_id
                ));
            }
        }
        for v in arrangement::check_entries(entries, root_identity(store, container_id, &container))
        {
            errors.push(v.to_string());
        }
    }

    Ok(ContainerValidationReport {
        ok: errors.is_empty(),
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;

    fn make_store() -> MemoryStore {
        let store = MemoryStore::default();
        // RFC-038 [R13]: a container member/root reference must resolve to a
        // real instance or the fatal catalog build rejects every subsequent
        // read. Pre-seed the two well-known ids this module's tests use as
        // members/roots. Tests needing a genuinely-dangling id use
        // dddddddd-dddd-4ddd-8ddd-dddddddddddd (never seeded).
        seed_instance(&store, "11111111-1111-4111-8111-111111111111");
        seed_instance(&store, "22222222-2222-4222-8222-222222222222");
        store
    }

    /// Persist a minimal Tier-0 note under `id` so it resolves as a real
    /// instance in the catalog's instance set (RFC-038 [R13]) — needed
    /// whenever a test uses `id` as a container member/root, since the
    /// catalog now fatally rejects a dangling memberInstanceIds/
    /// rootInstanceIds reference.
    fn seed_instance(store: &MemoryStore, id: &str) {
        store
            .save_note(&srs_core::types::note::Note {
                created_by: None,
                instance_id: id.to_string(),
                title: None,
                tags: None,
                sections: vec![],
                graduated_at: None,
                source_refs: None,
                created_at: None,
                updated_at: None,
                meta: None,
            })
            .unwrap();
    }

    fn minimal_container(id: &str, title: &str) -> Container {
        Container {
            container_id: id.to_string(),
            title: title.to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            member_instance_ids: None,
            child_container_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn typing_anchor_is_the_declared_anchor_and_nothing_else() {
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Sprint 1");
        // RFC-043 [R4]: no rootInstanceIds[0] fallback — members do not imply an anchor.
        c.member_instance_ids = Some(srs_core::types::container::entries([
            "11111111-1111-4111-8111-111111111111",
        ]));
        assert_eq!(typing_anchor_instance_id(&c), None);
        assert_eq!(r22_position_anchor_instance_id(&c), None);
        c.anchor_instance_id = Some("22222222-2222-4222-8222-222222222222".to_string());
        assert_eq!(
            typing_anchor_instance_id(&c),
            Some("22222222-2222-4222-8222-222222222222".to_string())
        );
        assert_eq!(
            r22_position_anchor_instance_id(&c),
            Some("22222222-2222-4222-8222-222222222222".to_string())
        );
    }

    #[test]
    fn create_container_writes_file_and_index() {
        let store = make_store();
        let c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Sprint 1");
        let out = create_container(&store, c).unwrap();
        assert_eq!(out.title, "Sprint 1");
        let listed = list_containers(&store, &ContainerListFilter::default()).unwrap();
        assert_eq!(listed.len(), 1);
    }

    /// srs-rust#1167: create is a create, not an upsert — a second `create_container`
    /// call against an id that already resolves must refuse rather than silently
    /// replacing the existing container (including its membership).
    #[test]
    fn create_container_refuses_existing_id() {
        let store = make_store();
        let c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Sprint 1");
        create_container(&store, c.clone()).unwrap();

        let mut replacement =
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Overwritten title");
        replacement.member_instance_ids = Some(srs_core::types::container::entries([
            "11111111-1111-4111-8111-111111111111",
        ]));
        let err = create_container(&store, replacement).unwrap_err();
        assert!(
            matches!(
                &err,
                RepositoryError::ContainerAlreadyExists { container_id }
                    if container_id == "550e8400-e29b-41d4-a716-446655440000"
            ),
            "expected ContainerAlreadyExists, got {err:?}"
        );

        // The original container survives untouched.
        let loaded = get_container(&store, "550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(loaded.title, "Sprint 1");
        assert_eq!(loaded.member_instance_ids, None);
    }

    /// srs-rust#1167: the same refusal applies to the manifest's embedded root
    /// container even before any `containers/*.json` file for it exists — the id
    /// is already claimed, so `create` must still refuse it. `update_container` is
    /// the path for materialising the root's full definition.
    #[test]
    fn create_container_refuses_manifest_root_id_with_no_file() {
        let root_id = "650e8400-e29b-41d4-a716-446655440000";
        let store = make_store();
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(
            &store,
            &mut manifest,
            &minimal_container(root_id, ""),
        )
        .unwrap();
        crate::writer::write_manifest(&store, &manifest).unwrap();

        let err = create_container(&store, minimal_container(root_id, "Root")).unwrap_err();
        assert!(
            matches!(
                &err,
                RepositoryError::ContainerAlreadyExists { container_id }
                    if container_id == root_id
            ),
            "expected ContainerAlreadyExists, got {err:?}"
        );
    }

    #[test]
    fn get_container_missing_returns_error() {
        let store = make_store();
        let err = get_container(&store, "missing").unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ContainerNotFound { container_id } if container_id == "missing"
        ));
    }

    #[test]
    fn create_container_mints_id_if_empty() {
        let store = make_store();
        let out = create_container(&store, minimal_container("", "Sprint 1")).unwrap();
        assert!(uuid::Uuid::parse_str(&out.container_id).is_ok());
    }

    #[test]
    fn list_containers_returns_all() {
        let store = make_store();
        create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440001", "B"),
        )
        .unwrap();
        let listed = list_containers(&store, &ContainerListFilter::default()).unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn get_container_returns_container() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Sprint 1"),
        )
        .unwrap();
        let got = get_container(&store, &created.container_id).unwrap();
        assert_eq!(got.title, "Sprint 1");
    }

    #[test]
    fn update_container_patches_title() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Old"),
        )
        .unwrap();
        let patch = ContainerPatch {
            title: Some("New".to_string()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, &created.container_id, patch)
            .unwrap()
            .container;
        assert_eq!(updated.title, "New");
    }

    #[test]
    fn update_container_list_shows_updated_title() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Old"),
        )
        .unwrap();
        let patch = ContainerPatch {
            title: Some("New".to_string()),
            ..ContainerPatch::default()
        };
        update_container(&store, &created.container_id, patch).unwrap();
        let listed = list_containers(&store, &ContainerListFilter::default()).unwrap();
        assert_eq!(listed[0].title, "New");
    }

    #[test]
    fn update_container_preserves_other_fields() {
        let store = make_store();
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Old");
        c.description = Some("keep".to_string());
        let created = create_container(&store, c).unwrap();
        let patch = ContainerPatch {
            title: Some("New".to_string()),
            ..ContainerPatch::default()
        };
        update_container(&store, &created.container_id, patch).unwrap();
        let got = get_container(&store, &created.container_id).unwrap();
        assert_eq!(got.description.as_deref(), Some("keep"));
    }

    #[test]
    fn delete_container_removes_index_entry() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Delete"),
        )
        .unwrap();
        delete_container(&store, &created.container_id).unwrap();
        let listed = list_containers(&store, &ContainerListFilter::default()).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_container_cascades_incident_relations() {
        // RFC-038 Change F: the [R22] cascade analogue for containers — a legacy
        // edge naming the containerId as an endpoint is removed with the container.
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Delete"),
        )
        .unwrap();
        store
            .save_relation(&srs_core::types::relation::Relation {
                created_by: None,
                relation_id: "de000001-0000-4000-a000-000000000001".to_string(),
                relation_type: "contains".to_string(),
                source_instance_id: created.container_id.clone(),
                target_instance_id: "some-instance".to_string(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            })
            .unwrap();

        delete_container(&store, &created.container_id).unwrap();

        assert!(
            crate::relation_service::load_relations(&store)
                .unwrap()
                .is_empty(),
            "legacy container-endpoint edge must be cascaded"
        );
    }

    fn with_children(id: &str, kids: &[&str]) -> Container {
        let mut c = minimal_container(id, "C");
        c.child_container_ids = Some(kids.iter().map(|k| k.to_string()).collect());
        c
    }

    #[test]
    fn child_container_diagnostics_reports_missing_self_and_cycle() {
        // RFC-034 [R7] / I-150 at rest (srs-rust#1445).
        let (p, c, x) = ("p-1", "c-1", "x-1");
        assert!(
            child_container_diagnostics(&[with_children(p, &[c]), minimal_container(c, "C")])
                .is_empty()
        );

        let missing = child_container_diagnostics(&[with_children(p, &[c])]);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("'p-1'") && missing[0].contains("'c-1'"));

        let selfref = child_container_diagnostics(&[with_children(p, &[p])]);
        assert_eq!(selfref.len(), 1);
        assert!(selfref[0].contains("itself"));

        let cycle = child_container_diagnostics(&[with_children(p, &[c]), with_children(c, &[p])]);
        assert_eq!(cycle.len(), 1, "{cycle:?}");
        assert!(cycle[0].contains("cycle"));

        // A diamond is not a cycle.
        let diamond = child_container_diagnostics(&[
            with_children(p, &[c, x]),
            with_children(c, &["d-1"]),
            with_children(x, &["d-1"]),
            minimal_container("d-1", "D"),
        ]);
        assert!(diamond.is_empty(), "{diamond:?}");
    }

    #[test]
    fn child_container_diagnostics_reads_a_dangling_id_left_by_a_store_write() {
        // MemoryStore round trip: a dangling id saved past the write-time check is reported.
        let store = make_store();
        let (p, c) = (
            "00000000-0000-4000-8000-0000000000a9",
            "00000000-0000-4000-8000-0000000000c9",
        );
        store.save_container(&with_children(p, &[c])).unwrap();
        let all = vec![store.load_container(p).unwrap()];
        let d = child_container_diagnostics(&all);
        assert_eq!(d.len(), 1);
        assert!(d[0].contains(p) && d[0].contains(c));
    }

    #[test]
    fn delete_container_unlinks_it_from_parent_child_container_ids() {
        // RFC-034 [R7] / I-150 (srs-rust#1442): deleting a listed child must not leave the
        // parent's `childContainerIds` dangling.
        let store = make_store();
        let child = "00000000-0000-4000-8000-0000000000c1";
        let keep = "00000000-0000-4000-8000-0000000000c2";
        let parent = "00000000-0000-4000-8000-0000000000a1";
        create_container(&store, minimal_container(child, "Child")).unwrap();
        create_container(&store, minimal_container(keep, "Keep")).unwrap();
        let mut p = minimal_container(parent, "Parent");
        p.child_container_ids = Some(vec![child.to_string(), keep.to_string()]);
        create_container(&store, p).unwrap();

        delete_container(&store, child).unwrap();

        let after = store.load_container(parent).unwrap();
        assert_eq!(after.child_container_ids, Some(vec![keep.to_string()]));
        assert!(list_members(&store, parent).is_ok());

        delete_container(&store, keep).unwrap();
        let after = store.load_container(parent).unwrap();
        assert_eq!(after.child_container_ids, None);
        assert!(list_members(&store, parent).is_ok());
    }

    #[test]
    fn delete_containers_with_records_unlinks_deleted_child_from_parent() {
        let store = make_store();
        let child = "00000000-0000-4000-8000-0000000000c1";
        let parent = "00000000-0000-4000-8000-0000000000a1";
        create_container(&store, minimal_container(child, "Child")).unwrap();
        let mut p = minimal_container(parent, "Parent");
        p.child_container_ids = Some(vec![child.to_string()]);
        create_container(&store, p).unwrap();

        delete_containers_with_records(&store, &[child.to_string()]).unwrap();

        assert_eq!(
            store.load_container(parent).unwrap().child_container_ids,
            None
        );
        assert!(list_members(&store, parent).is_ok());
    }

    #[test]
    fn delete_container_makes_container_unreachable() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Delete"),
        )
        .unwrap();
        delete_container(&store, &created.container_id).unwrap();
        let err = store.load_container(&created.container_id).unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));
    }

    #[test]
    fn delete_container_missing_returns_error() {
        let store = make_store();
        let err = delete_container(&store, "missing").unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ContainerNotFound { container_id } if container_id == "missing"
        ));
    }

    // --- Owner ruling srs-rust#742: `container delete` refuses the RFC-013 root ---

    /// A page container (P) and another (O): `only` is P's alone; `shared` is in both; `kept`
    /// (in O) derives from `mid` (P's), which derives from `base` (P's), so both stay (srs-rust#1436).
    fn delete_plan_fixture() -> (MemoryStore, [&'static str; 5]) {
        let store = make_store();
        let ids @ [only, shared, mid, base, kept] = [
            "a1000000-0000-4000-8000-000000000001",
            "a1000000-0000-4000-8000-000000000002",
            "a1000000-0000-4000-8000-000000000003",
            "a1000000-0000-4000-8000-000000000004",
            "a1000000-0000-4000-8000-000000000005",
        ];
        for id in ids {
            seed_instance(&store, id);
        }
        let with = |id: &str, title: &str, members: &[&str]| {
            let mut c = minimal_container(id, title);
            c.member_instance_ids = Some(srs_core::types::container::entries(
                members.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
            ));
            create_container(&store, c).unwrap();
        };
        with(
            "c1000000-0000-4000-8000-00000000000a",
            "Page",
            &[only, shared, mid, base],
        );
        with(
            "c1000000-0000-4000-8000-00000000000b",
            "Other",
            &[shared, kept],
        );
        for (n, (source, target)) in [(kept, mid), (mid, base)].into_iter().enumerate() {
            store
                .save_relation(&srs_core::types::relation::Relation {
                    created_by: None,
                    relation_id: format!("de100000-0000-4000-a000-00000000000{n}"),
                    relation_type: "derived-from".to_string(),
                    source_instance_id: source.to_string(),
                    target_instance_id: target.to_string(),
                    created_at: None,
                    notes: None,
                    source_refs: None,
                    meta: None,
                })
                .unwrap();
        }
        (store, ids)
    }

    #[test]
    fn delete_plan_keeps_shared_members_and_the_whole_lineage_of_a_kept_record() {
        let (store, [only, shared, mid, base, _]) = delete_plan_fixture();
        let missing = "c1000000-0000-4000-8000-0000000000ff".to_string();
        let plan = container_delete_plan(
            &store,
            &[
                "c1000000-0000-4000-8000-00000000000a".to_string(),
                missing.clone(),
            ],
        )
        .unwrap();
        assert_eq!(plan.container_ids, ["c1000000-0000-4000-8000-00000000000a"]);
        assert_eq!(plan.missing_container_ids, [missing]);
        assert_eq!(plan.instance_ids, [only]);
        let kept = |id: &str| {
            plan.kept
                .iter()
                .find(|k| k.instance_id == id)
                .map(|k| k.reason)
        };
        assert_eq!(kept(shared), Some(KeptReason::HeldElsewhere));
        assert_eq!(kept(mid), Some(KeptReason::Lineage));
        assert_eq!(
            kept(base),
            Some(KeptReason::Lineage),
            "kept transitively, whatever the order"
        );
        // a plan writes nothing
        assert!(store.find_instance(only).unwrap().is_some());
    }

    #[test]
    fn delete_plan_refuses_the_repository_root() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let err = container_delete_plan(&store, &[embed_id.to_string()]).unwrap_err();
        assert!(
            matches!(err, RepositoryError::ContainerRootDelete { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn delete_with_records_deletes_what_only_the_container_holds() {
        let (store, [only, shared, mid, base, kept]) = delete_plan_fixture();
        let page = "c1000000-0000-4000-8000-00000000000a";
        let plan = delete_containers_with_records(&store, &[page.to_string()]).unwrap();
        assert_eq!(plan.instance_ids, [only]);
        assert!(store.find_instance(only).unwrap().is_none(), "deleted");
        for id in [shared, mid, base, kept] {
            assert!(store.find_instance(id).unwrap().is_some(), "{id} stays");
        }
        assert!(get_container(&store, page).is_err(), "container deleted");
        assert_eq!(
            crate::relation_service::load_relations(&store)
                .unwrap()
                .len(),
            2,
            "the lineage is intact"
        );
    }

    #[test]
    fn delete_container_refuses_embed_only_root() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let err = delete_container(&store, embed_id).unwrap_err();
        assert!(
            matches!(
                err,
                RepositoryError::ContainerRootDelete { ref container_id } if container_id == embed_id
            ),
            "expected ContainerRootDelete, got {err:?}"
        );
        // The refusal must not have deleted anything.
        assert_eq!(
            get_container(&store, embed_id).unwrap().container_id,
            embed_id
        );
    }

    #[test]
    fn delete_container_refuses_file_backed_root() {
        // A root container whose id is *also* materialised under `containers/`
        // (a legacy/repair-worthy shape — RFC-038 [R12] makes the two
        // co-existing a fatal duplicate under the checked catalog, which is
        // exactly why the guard below reads `manifest.container` directly
        // rather than routing through it). The refusal must fire, and it
        // must not touch the file the checked catalog can no longer load.
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Root"),
        )
        .unwrap();
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(&store, &mut manifest, &created.clone())
            .unwrap();
        store.save_manifest(&manifest).unwrap();

        let err = delete_container(&store, &created.container_id).unwrap_err();
        assert!(
            matches!(
                err,
                RepositoryError::ContainerRootDelete { ref container_id }
                    if container_id == &created.container_id
            ),
            "expected ContainerRootDelete, got {err:?}"
        );
        // Still resolvable via the unchecked (repair) seam — the refusal did
        // not delete the file-backed container.
        assert!(store
            .load_container_unchecked(&created.container_id)
            .is_ok());
    }

    #[test]
    fn delete_container_still_deletes_a_non_root_container() {
        // The root-container guard must not block deletion of an ordinary
        // container when a *different* container is the declared root.
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let other = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Other"),
        )
        .unwrap();

        delete_container(&store, &other.container_id).unwrap();

        let err = store.load_container(&other.container_id).unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));
    }

    #[test]
    fn add_member_adds_id() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        let out = add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
            None,
            None,
        )
        .unwrap();
        assert_eq!(out.members.len(), 1);
    }

    #[test]
    fn add_member_is_idempotent() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
            None,
            None,
        )
        .unwrap();
        let out = add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
            None,
            None,
        )
        .unwrap();
        assert_eq!(out.members.len(), 1);
    }

    #[test]
    fn remove_member_removes_id() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
            None,
            None,
        )
        .unwrap();
        let out = remove_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.members.is_empty());
    }

    #[test]
    fn remove_member_noop_when_absent() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        let out = remove_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.members.is_empty());
    }

    #[test]
    fn remove_member_clears_field_when_list_empty() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
            None,
            None,
        )
        .unwrap();
        remove_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let got = get_container(&store, &created.container_id).unwrap();
        assert!(got.member_instance_ids.is_none());
    }

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const B: &str = "22222222-2222-4222-8222-222222222222";

    fn ids(r: &ArrangementResult) -> Vec<(String, u32)> {
        r.members
            .iter()
            .map(|e| (e.instance_id.clone(), e.depth()))
            .collect()
    }

    #[test]
    fn add_insert_move_and_set_depth_follow_the_rfc_043_rules() {
        let store = make_store();
        let c = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Outline"),
        )
        .unwrap();
        add_member(&store, &c.container_id, A, None, None).unwrap();
        // depth may rise by one: B at depth 1 under A
        let r = add_member(&store, &c.container_id, B, None, Some(1)).unwrap();
        assert_eq!(ids(&r), vec![(A.into(), 0), (B.into(), 1)]);
        // a depth jump is rejected whole and changes nothing
        let err = move_member(&store, &c.container_id, B, None, Some(2)).unwrap_err();
        assert_eq!(err.code(), "invalid-input"); // generic bucket code: the message check below is the discriminator
        assert!(err.to_string().contains("arrangement-depth"), "{err}");
        assert_eq!(get_arrangement(&store, &c.container_id).unwrap().len(), 2);
        // outdent B to 0, move it first
        let r = move_member(&store, &c.container_id, B, Some(0), Some(0)).unwrap();
        assert_eq!(ids(&r), vec![(B.into(), 0), (A.into(), 0)]);
        // promoting removal: A under B, remove B, A is promoted
        move_member(&store, &c.container_id, A, None, Some(1)).unwrap();
        let r = remove_member(&store, &c.container_id, B).unwrap();
        assert_eq!(ids(&r), vec![(A.into(), 0)]);
        assert_eq!(r.promoted, vec![A.to_string()]);
        assert_eq!(r.removed, vec![B.to_string()]);
    }

    #[test]
    fn relative_moves_and_outline_read() {
        use srs_core::arrangement::{Placement, RelativeMove, Shift};
        const C: &str = "33333333-3333-4333-8333-333333333333";
        let store = make_store();
        seed_instance(&store, C);
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Rel");
        c.member_instance_ids = Some(srs_core::types::container::entries([A, B]));
        c.anchor_instance_id = Some(A.to_string());
        let c = create_container(&store, c).unwrap();
        let id = &c.container_id;
        // anchor A leads; body is [B]
        let o = get_outline(&store, id).unwrap();
        assert_eq!(o.entries.len(), 2);
        assert_eq!(o.body.len(), 1);
        assert_eq!(o.body[0].instance_id, B);
        // indent B under A (A is first entry), then outdent back
        let r = move_member_relative(&store, id, B, &RelativeMove::Shift(Shift::Indent)).unwrap();
        assert_eq!(ids(&r), vec![(A.into(), 0), (B.into(), 1)]);
        let o = get_outline(&store, id).unwrap();
        assert_eq!(o.entries[1].parent_instance_id.as_deref(), Some(A));
        assert_eq!(o.body[0].depth, 0); // anchor set aside: B promoted in the body view
        let r = move_member_relative(&store, id, B, &RelativeMove::Shift(Shift::Outdent)).unwrap();
        assert_eq!(ids(&r), vec![(A.into(), 0), (B.into(), 0)]);
        let place = |t: &str, p| RelativeMove::Place {
            target: t.into(),
            placement: p,
        };
        let err = move_member_relative(&store, id, A, &place(A, Placement::Into)).unwrap_err();
        assert_eq!(err.code(), "invalid-input"); // generic bucket code: the message check below is the discriminator
        assert!(err.to_string().contains("arrangement-target"), "{err}");
        let r = move_member_relative(&store, id, B, &place(A, Placement::Before)).unwrap();
        assert_eq!(ids(&r), vec![(B.into(), 0), (A.into(), 0)]);
        assert!(add_member_relative(&store, id, B, A, Placement::After).is_err());
        // add relative: C into B, atomic and persisted
        let r = add_member_relative(&store, id, C, B, Placement::Into).unwrap();
        assert_eq!(ids(&r), vec![(B.into(), 0), (C.into(), 1), (A.into(), 0)]);
        assert_eq!(get_arrangement(&store, id).unwrap(), r.members);
    }

    #[test]
    fn removing_the_identity_or_anchor_entry_is_rejected() {
        let store = make_store();
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Pointers");
        c.member_instance_ids = Some(srs_core::types::container::entries([A, B]));
        c.anchor_instance_id = Some(A.to_string());
        let c = create_container(&store, c).unwrap();
        let err = remove_member(&store, &c.container_id, A).unwrap_err();
        assert_eq!(err.code(), "invalid-input"); // generic bucket code: the message check below is the discriminator
        assert!(err.to_string().contains("arrangement-pointer"), "{err}");
        remove_member(&store, &c.container_id, B).unwrap();
    }

    #[test]
    fn validate_invariants_passes_clean() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Clean"),
        )
        .unwrap();
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(report.ok);
    }

    /// An incoherent container, built the only way still open to it.
    ///
    /// Every membership-writing service entry point now rejects an id that
    /// resolves to nothing — the incremental `add_member`/`add_root`
    /// (srs-rust#841) and the wholesale `create_container`/`update_container`
    /// (srs-rust#845). Tests whose whole subject is an *already*-incoherent
    /// container therefore write through the ADR-045 repair seam, which is what
    /// that seam is for: it is the only surface in the codebase that can express
    /// a state the service layer will no longer produce.
    fn create_container_with_membership(
        store: &dyn RepositoryStore,
        id: &str,
        roots: &[&str],
        members: &[&str],
    ) -> Container {
        let mut c = minimal_container(id, "Invalid");
        // RFC-043: roots are gone; the first former root becomes the anchor and every id is an
        // ordinary entry.
        let mut all: Vec<&str> = Vec::new();
        for id in roots.iter().chain(members.iter()) {
            if !all.contains(id) {
                all.push(id);
            }
        }
        c.anchor_instance_id = roots.first().map(|s| s.to_string());
        c.member_instance_ids =
            (!all.is_empty()).then(|| srs_core::types::container::entries(all.iter().copied()));
        store.save_container_unchecked(&c).unwrap();
        c
    }

    #[test]
    fn validate_invariants_fails_invalid_member_id() {
        let store = make_store();
        let created = create_container_with_membership(
            &store,
            "550e8400-e29b-41d4-a716-446655440000",
            &[],
            &["dddddddd-dddd-4ddd-8ddd-dddddddddddd"],
        );
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_invalid_root_id() {
        let store = make_store();
        let created = create_container_with_membership(
            &store,
            "550e8400-e29b-41d4-a716-446655440000",
            &["dddddddd-dddd-4ddd-8ddd-dddddddddddd"],
            &[],
        );
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_container_id_in_member_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let created = create_container_with_membership(&store, id, &[], &[id]);
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_container_id_in_root_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let created = create_container_with_membership(&store, id, &[id], &[]);
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    // ---- RFC-034: declared nesting and effective membership (srs-rust#970) ----

    /// RFC-034 Testability "Direct membership": no `childContainerIds`, so
    /// `direct(A) == effective(A)`.
    #[test]
    fn effective_membership_with_no_children_equals_direct() {
        let store = make_store();
        seed_instance(&store, "aaaaaaaa-0000-4000-8000-00000000000a");
        seed_instance(&store, "bbbbbbbb-0000-4000-8000-00000000000b");
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "A");
        c.anchor_instance_id = Some("aaaaaaaa-0000-4000-8000-00000000000a".to_string());
        c.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
        ]));
        let created = create_container(&store, c).unwrap();
        assert_eq!(
            list_members(&store, &created.container_id).unwrap(),
            vec![
                "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
                "bbbbbbbb-0000-4000-8000-00000000000b".to_string()
            ]
        );
    }

    /// RFC-034 Testability "Recursive nesting": `A -> B -> C` via declared
    /// `childContainerIds`, two hops deep, so this is transitive, not one-level.
    #[test]
    fn effective_membership_recurses_through_declared_children() {
        let store = make_store();
        seed_instance(&store, "aaaaaaaa-0000-4000-8000-00000000000a");
        seed_instance(&store, "bbbbbbbb-0000-4000-8000-00000000000b");
        seed_instance(&store, "cccccccc-0000-4000-8000-00000000000c");

        let mut c = minimal_container("00000000-0000-4000-8000-00000000000c", "C");
        c.anchor_instance_id = Some("bbbbbbbb-0000-4000-8000-00000000000b".to_string());
        c.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
            "cccccccc-0000-4000-8000-00000000000c".to_string(),
        ]));
        create_container(&store, c).unwrap();

        let mut b = minimal_container("00000000-0000-4000-8000-00000000000b", "B");
        b.anchor_instance_id = Some("aaaaaaaa-0000-4000-8000-00000000000a".to_string());
        b.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
        ]));
        b.child_container_ids = Some(vec!["00000000-0000-4000-8000-00000000000c".to_string()]);
        create_container(&store, b).unwrap();

        let mut a = minimal_container("00000000-0000-4000-8000-00000000000a", "A");
        a.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
        ]));
        a.child_container_ids = Some(vec!["00000000-0000-4000-8000-00000000000b".to_string()]);
        create_container(&store, a).unwrap();

        let expected_abc = vec![
            "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
            "cccccccc-0000-4000-8000-00000000000c".to_string(),
        ];
        assert_eq!(
            list_members(&store, "00000000-0000-4000-8000-00000000000a").unwrap(),
            expected_abc
        );
        assert_eq!(
            list_members(&store, "00000000-0000-4000-8000-00000000000b").unwrap(),
            expected_abc
        );
        assert_eq!(
            list_members(&store, "00000000-0000-4000-8000-00000000000c").unwrap(),
            vec![
                "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
                "cccccccc-0000-4000-8000-00000000000c".to_string()
            ]
        );
        // Effective reverse lookup (RFC-034 Testability): containers_for_instance(c)
        // includes every ancestor reached through the declared chain.
        let hits = containers_for_instance(&store, "cccccccc-0000-4000-8000-00000000000c").unwrap();
        let mut hit_ids: Vec<String> = hits.into_iter().map(|h| h.container_id).collect();
        hit_ids.sort();
        assert_eq!(
            hit_ids,
            vec![
                "00000000-0000-4000-8000-00000000000a".to_string(),
                "00000000-0000-4000-8000-00000000000b".to_string(),
                "00000000-0000-4000-8000-00000000000c".to_string(),
            ]
        );
    }

    /// RFC-034 Testability "Overlap is not nesting": a Container rooted at an
    /// instance that appears in another Container's membership acquires no
    /// relationship to it unless the edge is declared in `childContainerIds`.
    #[test]
    fn effective_membership_ignores_undeclared_root_overlap() {
        let store = make_store();
        seed_instance(&store, "aaaaaaaa-0000-4000-8000-00000000000a");
        seed_instance(&store, "bbbbbbbb-0000-4000-8000-00000000000b");
        seed_instance(&store, "eeeeeeee-0000-4000-8000-00000000000e");

        let mut area = minimal_container("00000000-0000-4000-8000-0000000000a1", "Area");
        area.anchor_instance_id = Some("aaaaaaaa-0000-4000-8000-00000000000a".to_string());
        area.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
        ]));
        let area = create_container(&store, area).unwrap();

        // `Proj` roots at `b`, a member of `Area` — but `Area` never declares
        // `Proj` as a child, so no admission occurs.
        let mut proj = minimal_container("00000000-0000-4000-8000-0000000000a2", "Proj");
        proj.anchor_instance_id = Some("bbbbbbbb-0000-4000-8000-00000000000b".to_string());
        proj.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
            "eeeeeeee-0000-4000-8000-00000000000e".to_string(),
        ]));
        create_container(&store, proj).unwrap();

        assert_eq!(
            list_members(&store, &area.container_id).unwrap(),
            vec![
                "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
                "bbbbbbbb-0000-4000-8000-00000000000b".to_string()
            ],
            "effective(Area) must be unchanged by the creation of an undeclared Proj"
        );
    }

    /// RFC-034 Testability "Rootless declared child": a child Container with
    /// no roots is still admitted, and its members still contribute.
    #[test]
    fn effective_membership_includes_rootless_declared_child() {
        let store = make_store();
        seed_instance(&store, "bbbbbbbb-0000-4000-8000-00000000000b");

        let mut child = minimal_container("00000000-0000-4000-8000-0000000000b1", "B");
        child.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
        ]));
        create_container(&store, child).unwrap();

        let mut a = minimal_container("00000000-0000-4000-8000-0000000000a1", "A");
        a.child_container_ids = Some(vec!["00000000-0000-4000-8000-0000000000b1".to_string()]);
        let a = create_container(&store, a).unwrap();

        assert_eq!(
            list_members(&store, &a.container_id).unwrap(),
            vec!["bbbbbbbb-0000-4000-8000-00000000000b".to_string()]
        );
    }

    /// RFC-034 Change C / Testability "`contains` independence": a `contains`
    /// Relation MUST NOT add an instance to a Container's membership, direct or
    /// effective — this is the behavior the pre-RFC-034 I-66 condition 3
    /// traversal implemented and RFC-034 explicitly retires.
    #[test]
    fn effective_membership_ignores_contains_relations() {
        let store = MemoryStore::default();
        for id in [
            "00000000-0000-4000-8000-0000000a0006",
            "00000000-0000-4000-8000-0000000a0007",
        ] {
            store
                .save_instance_json(
                    &format!("records/notes/{id}.json"),
                    &serde_json::json!({"instanceId": id, "sections": []}),
                )
                .unwrap();
        }
        crate::store::write_relations_standalone_for_test(
            &store,
            &serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                "relations": [{
                    "relationId": "aaaaaaaa-0000-4000-8000-000000000001",
                    "relationType": "contains",
                    "sourceInstanceId": "00000000-0000-4000-8000-0000000a0006",
                    "targetInstanceId": "00000000-0000-4000-8000-0000000a0007",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            }),
        );
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Doc");
        c.anchor_instance_id = Some("00000000-0000-4000-8000-0000000a0006".to_string());
        c.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "00000000-0000-4000-8000-0000000a0006".to_string(),
        ]));
        let created = create_container(&store, c).unwrap();

        assert_eq!(
            list_members(&store, &created.container_id).unwrap(),
            vec!["00000000-0000-4000-8000-0000000a0006".to_string()],
            "a contains Relation must never add a member (RFC-034 [R4])"
        );
        assert!(!is_member(
            &store,
            &created.container_id,
            "00000000-0000-4000-8000-0000000a0007"
        )
        .unwrap());
        assert!(
            containers_for_instance(&store, "00000000-0000-4000-8000-0000000a0007")
                .unwrap()
                .is_empty()
        );
    }

    /// RFC-034 [R8]: `containerScope: "explicit"` (`list_direct_members`) stays
    /// shallow even when the container has admitted children; `"subtree"`
    /// (`list_members`, effective) is the scope that descends.
    #[test]
    fn list_direct_members_excludes_declared_children() {
        let store = make_store();
        seed_instance(&store, "aaaaaaaa-0000-4000-8000-00000000000a");
        seed_instance(&store, "bbbbbbbb-0000-4000-8000-00000000000b");

        let mut child = minimal_container("00000000-0000-4000-8000-0000000000b1", "B");
        child.anchor_instance_id = Some("bbbbbbbb-0000-4000-8000-00000000000b".to_string());
        child.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "bbbbbbbb-0000-4000-8000-00000000000b".to_string(),
        ]));
        create_container(&store, child).unwrap();

        let mut a = minimal_container("00000000-0000-4000-8000-0000000000a1", "A");
        a.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
        ]));
        a.child_container_ids = Some(vec!["00000000-0000-4000-8000-0000000000b1".to_string()]);
        let a = create_container(&store, a).unwrap();

        assert_eq!(
            list_direct_members(&store, &a.container_id).unwrap(),
            vec!["aaaaaaaa-0000-4000-8000-00000000000a".to_string()]
        );
        assert_eq!(
            list_members(&store, &a.container_id).unwrap(),
            vec![
                "aaaaaaaa-0000-4000-8000-00000000000a".to_string(),
                "bbbbbbbb-0000-4000-8000-00000000000b".to_string()
            ]
        );
    }

    /// RFC-034 [R7]: a self-referential `childContainerIds` entry is rejected.
    #[test]
    fn create_container_rejects_self_referential_child() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut c = minimal_container(id, "Self");
        c.child_container_ids = Some(vec![id.to_string()]);
        // Caught by the core `validate_container` self-reference check
        // (which every write already runs) before `require_valid_child_containers`
        // (the graph-existence/acyclicity check) is ever reached.
        assert!(matches!(
            create_container(&store, c),
            Err(RepositoryError::ContainerValidation { .. })
        ));
    }

    /// RFC-034 [R7]: every `childContainerIds` entry must resolve to an
    /// existing, distinct Container — an id that doesn't resolve to a
    /// Container at all (e.g. an instance id) is rejected before it can leave
    /// `effective(C)` undefined for anyone reading this container later.
    #[test]
    fn create_container_rejects_missing_child_target() {
        let store = make_store();
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "A");
        c.child_container_ids = Some(vec!["dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string()]);
        assert!(matches!(
            create_container(&store, c),
            Err(RepositoryError::InvalidInput { .. })
        ));
    }

    /// RFC-034 [R7]: the `childContainerIds` graph MUST be acyclic. Proves the
    /// guard actually rejects the violation it exists to catch (Protocol
    /// "Prove" stage) — red before the cycle, and the base graph stays valid
    /// (green) both before the cyclic edit is attempted and after.
    #[test]
    fn update_container_rejects_child_cycle() {
        let store = make_store();
        let b_id = "550e8400-e29b-41d4-a716-446655440001";
        create_container(&store, minimal_container(b_id, "B")).unwrap();
        let mut a = minimal_container("550e8400-e29b-41d4-a716-446655440000", "A");
        a.child_container_ids = Some(vec![b_id.to_string()]);
        let a = create_container(&store, a).unwrap();

        // Green before: B has no children, so no cycle exists yet.
        assert_eq!(list_members(&store, b_id).unwrap(), Vec::<String>::new());

        // Red: B -> A would close the cycle A -> B -> A.
        let patch = ContainerPatch {
            child_container_ids: Some(vec![a.container_id.clone()]),
            ..ContainerPatch::default()
        };
        assert!(
            matches!(
                update_container(&store, b_id, patch),
                Err(RepositoryError::InvalidInput { .. })
            ),
            "a childContainerIds cycle must be rejected, not silently accepted"
        );

        // Green after: the rejected edit must not have been persisted.
        assert_eq!(
            get_container(&store, b_id).unwrap().child_container_ids,
            None
        );
    }

    /// Cross-store roundtrip (memory -> file) per CLAUDE.md Storage Boundary
    /// Rules: declared `childContainerIds` nesting, not just flat membership,
    /// must round-trip and recompute identically on `FileStore`.
    #[test]
    fn effective_membership_via_declared_children_roundtrips_via_filestore() {
        let store = MemoryStore::default();
        for id in [
            "00000000-0000-4000-8000-0000000a0006",
            "00000000-0000-4000-8000-0000000a0007",
        ] {
            store
                .save_instance_json(
                    &format!("records/notes/{id}.json"),
                    &serde_json::json!({"instanceId": id, "sections": []}),
                )
                .unwrap();
        }
        let mut child = minimal_container("550e8400-e29b-41d4-a716-446655440001", "Child");
        child.anchor_instance_id = Some("00000000-0000-4000-8000-0000000a0007".to_string());
        child.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "00000000-0000-4000-8000-0000000a0007".to_string(),
        ]));
        create_container(&store, child).unwrap();

        let mut root = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Root");
        root.anchor_instance_id = Some("00000000-0000-4000-8000-0000000a0006".to_string());
        root.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "00000000-0000-4000-8000-0000000a0006".to_string(),
        ]));
        root.child_container_ids = Some(vec!["550e8400-e29b-41d4-a716-446655440001".to_string()]);
        let created = create_container(&store, root).unwrap();

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();

        assert_eq!(
            list_members(&file_store, &created.container_id).unwrap(),
            vec![
                "00000000-0000-4000-8000-0000000a0006".to_string(),
                "00000000-0000-4000-8000-0000000a0007".to_string()
            ]
        );
    }

    /// RFC-043 [R6]: order is data — a copy (memory -> file) keeps entry order and depth.
    #[test]
    fn copy_preserves_entry_order_and_depth() {
        let store = make_store();
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Outline");
        // Deliberately not id-sorted, with nesting.
        c.member_instance_ids = Some(vec![ContainerEntry::new(B), ContainerEntry::at(A, 1)]);
        create_container(&store, c).unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let got = get_arrangement(&file_store, "550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(got, vec![ContainerEntry::new(B), ContainerEntry::at(A, 1)]);
    }

    #[test]
    fn containers_for_instance_returns_matching_containers() {
        let store = make_store();
        let a = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let _b = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440001", "B"),
        )
        .unwrap();
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, &a.container_id, member, None, None).unwrap();
        let out = containers_for_instance(&store, member).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].container_id, a.container_id);
    }

    #[test]
    fn containers_for_instance_includes_root_role() {
        let store = make_store();
        let a = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, &a.container_id, member, None, None).unwrap();
        let out = containers_for_instance(&store, member).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn containers_for_instance_returns_empty_when_no_match() {
        let store = make_store();
        create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let out = containers_for_instance(&store, "11111111-1111-4111-8111-111111111111").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn list_containers_root_filter_matches_root_only() {
        let store = make_store();
        let a = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let b = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440001", "B"),
        )
        .unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        // RFC-043: "root" is the anchor entry.
        add_member(&store, &a.container_id, id, None, None).unwrap();
        update_container(
            &store,
            &a.container_id,
            ContainerPatch {
                anchor_instance_id: Some(id.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        add_member(&store, &b.container_id, id, None, None).unwrap();
        let out = list_containers(
            &store,
            &ContainerListFilter {
                anchor_instance_id: Some(id.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].container_id, a.container_id);
    }

    #[test]
    fn create_container_mints_full_uuid_prefix_filename_safely() {
        let store = make_store();
        let out = create_container(&store, minimal_container("", "Sprint")).unwrap();
        assert!(!out.container_id.is_empty());
        assert!(uuid::Uuid::parse_str(&out.container_id).is_ok());
    }

    #[test]
    fn is_member_true_and_false() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        assert!(!is_member(&store, &created.container_id, id).unwrap());
        add_member(&store, &created.container_id, id, None, None).unwrap();
        assert!(is_member(&store, &created.container_id, id).unwrap());
    }

    #[test]
    fn create_container_uses_logical_id_boundary() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let out = create_container(&store, minimal_container(id, "Test")).unwrap();
        // Service creates through container ID, not path
        assert_eq!(out.container_id, id);
        let loaded = store.load_container(id).unwrap();
        assert_eq!(loaded.container_id, id);
    }

    #[test]
    fn update_container_does_not_require_path_lookup_in_service() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Original")).unwrap();
        let patch = ContainerPatch {
            title: Some("Updated".to_string()),
            ..ContainerPatch::default()
        };
        // Path lookup is adapter-owned; service only needs the container ID
        let updated = update_container(&store, id, patch).unwrap().container;
        assert_eq!(updated.title, "Updated");
    }

    #[test]
    fn container_membership_unchanged() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Test")).unwrap();
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, id, member, None, None).unwrap();
        assert!(is_member(&store, id, member).unwrap());
        remove_member(&store, id, member).unwrap();
        assert!(!is_member(&store, id, member).unwrap());
    }

    // --- srs-rust#841: membership writes may not brick the repository ---

    const UNRESOLVABLE: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";

    fn seeded_container(store: &MemoryStore, id: &str) -> String {
        create_container(store, minimal_container(id, "Guarded"))
            .unwrap()
            .container_id
    }

    #[test]
    fn add_member_succeeds_with_resolvable_instance_id_and_is_idempotent() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        let member = "11111111-1111-4111-8111-111111111111";
        assert_eq!(
            add_member(&store, &cid, member, None, None)
                .unwrap()
                .members,
            srs_core::types::container::entries([member])
        );
        // idempotent
        assert_eq!(
            add_member(&store, &cid, member, None, None)
                .unwrap()
                .members,
            srs_core::types::container::entries([member])
        );
    }

    #[test]
    fn blank_instance_id_is_reported_even_on_a_bricked_repository() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container_with_membership(&store, id, &[UNRESOLVABLE], &[]);
        assert!(store.catalog().is_err(), "repository should be bricked");

        assert!(
            matches!(
                add_member(&store, id, "  ", None, None),
                Err(RepositoryError::InvalidInput { .. })
            ),
            "a blank id must be named as such, not buried under a CatalogLoad"
        );
    }

    #[test]
    fn add_member_rejects_blank_instance_id() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        for blank in ["", "   "] {
            assert!(matches!(
                add_member(&store, &cid, blank, None, None),
                Err(RepositoryError::InvalidInput { .. })
            ));
        }
        assert!(get_container(&store, &cid)
            .unwrap()
            .member_instance_ids
            .is_none());
    }

    #[test]
    fn add_member_rejects_unresolvable_instance_id() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        assert!(matches!(
            add_member(&store, &cid, UNRESOLVABLE, None, None),
            Err(RepositoryError::InstanceNotFound { .. })
        ));
        assert!(get_container(&store, &cid)
            .unwrap()
            .member_instance_ids
            .is_none());
    }

    /// The repair path (ADR-045) on a file-backed container: a dangling root
    /// makes the checked catalog fatal, and `remove_root` is the way back out.
    /// The repair path (ADR-045) on a file-backed container: a dangling member makes the
    /// checked catalog fatal, and `repair_members` is the way back out (RFC-043 [R20]).
    #[test]
    fn repair_members_unbricks_a_file_backed_container_and_is_idempotent() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container_with_membership(&store, id, &[], &[A, UNRESOLVABLE, B]);
        assert!(store.catalog().is_err(), "container should be bricked");

        let r = repair_members(&store, id).unwrap();
        assert_eq!(r.removed, vec![UNRESOLVABLE.to_string()]);
        store
            .catalog()
            .expect("repository loads again after repair");
        assert_eq!(get_arrangement(&store, id).unwrap().len(), 2);
        // idempotent
        let again = repair_members(&store, id).unwrap();
        assert!(again.removed.is_empty());
    }

    /// Repair promotes the dangling entry's descendants and leaves a dangling pointer alone
    /// (re-pointing it is a semantic choice, [R20]).
    #[test]
    fn repair_promotes_descendants_and_leaves_dangling_pointers() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut c = minimal_container(id, "Outline");
        c.member_instance_ids = Some(vec![
            srs_core::types::container::ContainerEntry::new(UNRESOLVABLE),
            srs_core::types::container::ContainerEntry::at(A, 1),
            srs_core::types::container::ContainerEntry::new(B),
        ]);
        c.anchor_instance_id = Some(UNRESOLVABLE.to_string());
        store.save_container_unchecked(&c).unwrap();
        let r = repair_members(&store, id).unwrap();
        assert_eq!(ids(&r), vec![(A.into(), 0), (B.into(), 0)]);
        assert_eq!(r.promoted, vec![A.to_string()]);
        let got = store.load_container_unchecked(id).unwrap();
        assert_eq!(got.anchor_instance_id.as_deref(), Some(UNRESOLVABLE));
    }

    #[test]
    fn remove_member_repairs_bricked_file_backed_container() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container_with_membership(&store, id, &[], &[UNRESOLVABLE]);
        assert!(store.catalog().is_err(), "container should be bricked");

        remove_member(&store, id, UNRESOLVABLE).unwrap();

        store
            .catalog()
            .expect("repository loads again after repair");
    }

    /// The same repair on the embed-only root container ([R1]) — the shape the
    /// #841 reproduction actually hits, and the one whose fallback must not
    /// route through the checked `resolve_root_container`.
    /// The same repair on the embed-only root container ([R1]) — the shape the #841
    /// reproduction actually hits, and the one whose fallback must not route through the
    /// checked `resolve_root_container`.
    #[test]
    fn repair_members_unbricks_the_embed_root_container() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut root = minimal_container(id, "Root");
        root.member_instance_ids = Some(srs_core::types::container::entries([UNRESOLVABLE]));
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(&store, &mut manifest, &root).unwrap();
        store.save_manifest(&manifest).unwrap();
        assert!(store.catalog().is_err(), "repository should be bricked");

        repair_members(&store, id).unwrap();

        store
            .catalog()
            .expect("repository loads again after repair");
        assert!(crate::container_service::resolve_root_container(
            &store,
            &store.load_manifest().unwrap()
        )
        .unwrap()
        .unwrap()
        .member_instance_ids
        .is_none());
    }

    #[test]
    fn validate_container_invariants_unchanged() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Test")).unwrap();
        // Clean container passes
        let report = validate_container_invariants(&store, id).unwrap();
        assert!(report.ok);
        // The container's own ID as a member fails. No service writer accepts it
        // any more (a containerId is not an instance — srs-rust#841/#845), so the
        // invalid state is planted through the ADR-045 repair seam.
        create_container_with_membership(&store, id, &[], &[id]);
        let report = validate_container_invariants(&store, id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn patch_identity_instance_id_on_root_container_syncs_manifest() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";
        // Embed-only root ([R1]): a containers/*.json file sharing the embed's
        // id is a fatal SRS038-R12-DUPLICATE-ID under the catalog.
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(
            &store,
            &mut manifest,
            &minimal_container(container_id, "Root"),
        )
        .unwrap();
        store.save_manifest(&manifest).unwrap();

        let patch = ContainerPatch {
            identity_instance_id: Some(Some("11111111-1111-4111-8111-111111111111".to_string())),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, container_id, patch)
            .unwrap()
            .container;
        assert_eq!(
            updated.identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );

        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            crate::container_service::resolve_root_container(&store, &manifest)
                .unwrap()
                .unwrap()
                .identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
    }

    #[test]
    fn patch_identity_instance_id_on_non_root_container_does_not_touch_manifest() {
        let store = make_store();
        let root_id = "550e8400-e29b-41d4-a716-446655440000";
        let other_id = "550e8400-e29b-41d4-a716-446655440001";
        // Embed-only root ([R1]); only the non-root container is file-backed.
        create_container(&store, minimal_container(other_id, "Other")).unwrap();

        // Set manifest.container to root_id with no identity pointer
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(
            &store,
            &mut manifest,
            &minimal_container(root_id, "Root"),
        )
        .unwrap();
        store.save_manifest(&manifest).unwrap();

        // Patch OTHER container's identity_instance_id — manifest should not change
        let patch = ContainerPatch {
            identity_instance_id: Some(Some("22222222-2222-4222-8222-222222222222".to_string())),
            ..ContainerPatch::default()
        };
        update_container(&store, other_id, patch).unwrap();

        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            crate::container_service::resolve_root_container(&store, &manifest)
                .unwrap()
                .unwrap()
                .identity_instance_id,
            None
        );
    }

    /// srs-rust#837: before `require_resolvable_identity`, `container update`
    /// accepted a nonexistent `identityInstanceId` with no existence check,
    /// producing a genuinely dangling pointer — not caught by the [R13]
    /// catalog fatal (identity is deliberately outside that reference set),
    /// only surfaced later as a non-fatal `repo validate` I-81 warning, and
    /// fatal to `repo navigation` (`get_record_by_id` -> `NotFound`).
    #[test]
    fn update_container_rejects_nonexistent_identity_instance_id() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(container_id, "Root")).unwrap();

        let patch = ContainerPatch {
            identity_instance_id: Some(Some("dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string())),
            ..ContainerPatch::default()
        };
        let err = update_container(&store, container_id, patch).unwrap_err();
        assert!(
            matches!(
                err,
                RepositoryError::InstanceNotFound { ref id }
                if id == "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
            ),
            "expected InstanceNotFound, got: {err:?}"
        );

        // The rejected patch must not have been written.
        let container = store.load_container(container_id).unwrap();
        assert_eq!(container.identity_instance_id, None);
    }

    #[test]
    fn create_container_rejects_nonexistent_identity_instance_id() {
        let store = make_store();
        let mut container = minimal_container("550e8400-e29b-41d4-a716-446655440002", "Root");
        container.identity_instance_id = Some("dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string());

        let err = create_container(&store, container).unwrap_err();
        assert!(
            matches!(
                err,
                RepositoryError::InstanceNotFound { ref id }
                if id == "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
            ),
            "expected InstanceNotFound, got: {err:?}"
        );
    }

    /// A container with no identity at all must still create/update cleanly —
    /// `require_resolvable_identity` must not treat `None` as a failure.
    #[test]
    fn update_container_allows_clearing_or_omitting_identity() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440003";
        create_container(&store, minimal_container(container_id, "Root")).unwrap();

        let patch = ContainerPatch {
            title: Some("Renamed".to_string()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, container_id, patch)
            .unwrap()
            .container;
        assert_eq!(updated.identity_instance_id, None);
        assert_eq!(updated.title, "Renamed");
    }

    /// srs-rust#1293: `{"identityInstanceId": null}` must clear the field, not no-op. Goes
    /// through `serde_json::from_str` (not a Rust struct literal) because the bug was in how
    /// JSON `null` round-trips through `ContainerPatch`'s deserialization, not in the service.
    #[test]
    fn update_container_null_identity_instance_id_clears_field() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440004";
        let mut container = minimal_container(container_id, "Root");
        container.identity_instance_id = Some("11111111-1111-4111-8111-111111111111".to_string());
        create_container(&store, container).unwrap();

        let patch: ContainerPatch =
            serde_json::from_str(r#"{"identityInstanceId": null}"#).unwrap();
        let updated = update_container(&store, container_id, patch)
            .unwrap()
            .container;
        assert_eq!(updated.identity_instance_id, None);
    }

    /// Sibling case: omitting the key entirely must still leave the field untouched — the two
    /// JSON shapes (`null` vs. absent) must deserialize to different `ContainerPatch` values.
    #[test]
    fn update_container_omitted_identity_instance_id_leaves_untouched() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440005";
        let mut container = minimal_container(container_id, "Root");
        container.identity_instance_id = Some("11111111-1111-4111-8111-111111111111".to_string());
        create_container(&store, container).unwrap();

        let patch: ContainerPatch = serde_json::from_str(r#"{"title": "Renamed"}"#).unwrap();
        let updated = update_container(&store, container_id, patch)
            .unwrap()
            .container;
        assert_eq!(
            updated.identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        assert_eq!(updated.title, "Renamed");
    }

    #[test]
    fn update_container_patches_anchor_instance_id() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Root")).unwrap();
        let patch = ContainerPatch {
            anchor_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap().container;
        assert_eq!(
            updated.anchor_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        let reloaded = get_container(&store, id).unwrap();
        assert_eq!(
            reloaded.anchor_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
    }

    #[test]
    fn update_container_patches_member_instance_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Container")).unwrap();
        let patch = ContainerPatch {
            member_instance_ids: Some(srs_core::types::container::entries(vec![
                "22222222-2222-4222-8222-222222222222".to_string(),
            ])),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap().container;
        assert_eq!(
            updated.member_instance_ids,
            Some(srs_core::types::container::entries([
                "22222222-2222-4222-8222-222222222222"
            ]))
        );
        let reloaded = get_container(&store, id).unwrap();
        assert_eq!(
            reloaded.member_instance_ids,
            Some(srs_core::types::container::entries([
                "22222222-2222-4222-8222-222222222222"
            ]))
        );
    }

    #[test]
    fn update_container_removing_members_reports_diagnostic() {
        // srs-rust#1026: a patch that replaces memberInstanceIds with a
        // subset must still succeed (replace semantics unchanged) but must
        // name what it dropped instead of losing it silently.
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        seed_instance(&store, "33333333-3333-4333-8333-333333333333");
        let mut c = minimal_container(id, "Container");
        c.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "11111111-1111-4111-8111-111111111111".to_string(),
            "22222222-2222-4222-8222-222222222222".to_string(),
            "33333333-3333-4333-8333-333333333333".to_string(),
        ]));
        create_container(&store, c).unwrap();

        let patch = ContainerPatch {
            member_instance_ids: Some(srs_core::types::container::entries(vec![
                "11111111-1111-4111-8111-111111111111".to_string(),
            ])),
            ..ContainerPatch::default()
        };
        let result = update_container(&store, id, patch).unwrap();

        // Replace semantics unchanged: the operation succeeded and the
        // stored membership is exactly the patch's list.
        assert_eq!(
            result.container.member_instance_ids,
            Some(srs_core::types::container::entries([
                "11111111-1111-4111-8111-111111111111"
            ]))
        );
        let reloaded = get_container(&store, id).unwrap();
        assert_eq!(
            reloaded.member_instance_ids,
            Some(srs_core::types::container::entries([
                "11111111-1111-4111-8111-111111111111"
            ]))
        );

        // The removal is now visible.
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        let msg = &result.diagnostics[0];
        assert!(msg.contains("memberInstanceIds"), "{msg}");
        assert!(msg.contains('2'), "{msg}");
        assert!(
            msg.contains("22222222-2222-4222-8222-222222222222"),
            "{msg}"
        );
        assert!(
            msg.contains("33333333-3333-4333-8333-333333333333"),
            "{msg}"
        );
    }

    #[test]
    fn update_container_keeping_all_members_emits_no_diagnostic() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut c = minimal_container(id, "Container");
        c.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "11111111-1111-4111-8111-111111111111".to_string(),
            "22222222-2222-4222-8222-222222222222".to_string(),
        ]));
        create_container(&store, c).unwrap();

        // Same set, reordered — a no-op removal-wise.
        let patch = ContainerPatch {
            member_instance_ids: Some(srs_core::types::container::entries(vec![
                "22222222-2222-4222-8222-222222222222".to_string(),
                "11111111-1111-4111-8111-111111111111".to_string(),
            ])),
            ..ContainerPatch::default()
        };
        let result = update_container(&store, id, patch).unwrap();
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn update_container_with_empty_member_instance_ids_clears_field() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut c = minimal_container(id, "Container");
        c.member_instance_ids = Some(srs_core::types::container::entries(vec![
            "22222222-2222-4222-8222-222222222222".to_string(),
        ]));
        create_container(&store, c).unwrap();
        let patch = ContainerPatch {
            member_instance_ids: Some(Vec::new()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap().container;
        assert!(updated.member_instance_ids.is_none());
        let reloaded = get_container(&store, id).unwrap();
        assert!(reloaded.member_instance_ids.is_none());
    }

    /// RFC-043 [R6]: order is data — a patched arrangement is stored exactly as given.
    #[test]
    fn update_container_preserves_patched_entry_order_and_depth() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Container")).unwrap();
        let (a, b) = (
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        );
        seed_instance(&store, a);
        seed_instance(&store, b);
        let patch = ContainerPatch {
            member_instance_ids: Some(vec![
                srs_core::types::container::ContainerEntry::new(a),
                srs_core::types::container::ContainerEntry::at(b, 1),
            ]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap().container;
        assert_eq!(
            updated.member_instance_ids,
            Some(vec![
                srs_core::types::container::ContainerEntry::new(a),
                srs_core::types::container::ContainerEntry::at(b, 1),
            ])
        );
        let reloaded = get_container(&store, id).unwrap();
        assert_eq!(reloaded.member_ids(), vec![a.to_string(), b.to_string()]);
    }

    #[test]
    fn update_container_combined_patch_on_root_container_syncs_all_fields() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";
        // Embed-only root ([R1]): a containers/*.json file sharing the embed's
        // id is a fatal SRS038-R12-DUPLICATE-ID under the catalog.
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(
            &store,
            &mut manifest,
            &minimal_container(container_id, "Root"),
        )
        .unwrap();
        store.save_manifest(&manifest).unwrap();

        let patch = ContainerPatch {
            identity_instance_id: Some(Some("11111111-1111-4111-8111-111111111111".to_string())),
            anchor_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            member_instance_ids: Some(srs_core::types::container::entries(vec![
                "22222222-2222-4222-8222-222222222222".to_string(),
            ])),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, container_id, patch)
            .unwrap()
            .container;
        assert_eq!(
            updated.identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        assert_eq!(
            updated.anchor_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        assert_eq!(
            updated.member_instance_ids,
            Some(srs_core::types::container::entries([
                "22222222-2222-4222-8222-222222222222"
            ]))
        );
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            crate::container_service::resolve_root_container(&store, &manifest)
                .unwrap()
                .unwrap()
                .identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        let reloaded = get_container(&store, container_id).unwrap();
        assert_eq!(
            reloaded.anchor_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        assert_eq!(
            reloaded.member_instance_ids,
            Some(srs_core::types::container::entries([
                "22222222-2222-4222-8222-222222222222"
            ]))
        );
    }

    #[test]
    fn container_patch_unknown_field_fails_deserialization() {
        let result: Result<ContainerPatch, _> = serde_json::from_str(r#"{"unknownField": "x"}"#);
        assert!(
            result.is_err(),
            "unknown fields in ContainerPatch must fail deserialization, not silently drop"
        );
    }

    // --- Phase 1: embed-only read path ---

    fn embed_only_store(embed_id: &str, title: &str) -> MemoryStore {
        let store = MemoryStore::default();
        seed_instance(&store, "11111111-1111-4111-8111-111111111111");
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(
            &store,
            &mut manifest,
            &minimal_container(embed_id, title),
        )
        .unwrap();
        store.save_manifest(&manifest).unwrap();
        store
    }

    #[test]
    fn embed_only_get_container_returns_embed() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let c = get_container(&store, embed_id).unwrap();
        assert_eq!(c.container_id, embed_id);
        assert_eq!(c.title, "Root");
    }

    #[test]
    fn embed_only_get_container_not_found_for_unknown_id() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let err = get_container(&store, "00000000-0000-4000-8000-000000000099").unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));
    }

    #[test]
    fn embed_only_list_includes_root() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let listed = list_containers(&store, &ContainerListFilter::default()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].container_id, embed_id);
    }

    /// srs-rust#988: a `containers/*.json` file sharing the embed's id is not
    /// a "richest source" `resolve_root_container` can prefer — no write path
    /// (`save_container`/`save_container_unchecked`) ever produces this shape,
    /// but a hand-edited or pre-RFC-038 legacy repo can. This state is fatal
    /// under [R12] (`SRS038-R12-DUPLICATE-ID`) because the embed unconditionally
    /// contributes its own catalog entry alongside the file's, and
    /// `resolve_root_container` must surface that as a propagated error, never
    /// as `Ok(container)` from the file-backed copy.
    #[test]
    fn resolve_root_container_rejects_embed_and_file_backed_coexistence() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        // Bypass the dedup in `save_container_at` (which would fold a write
        // under this id back into the embed) by writing the file directly —
        // this is exactly the "hand-edited outside the sanctioned write path"
        // shape the finding describes.
        let duplicate = minimal_container(embed_id, "Root (duplicate file)");
        let value = serde_json::to_value(&duplicate).unwrap();
        store.ensure_instance_dir("containers").unwrap();
        store
            .save_instance_json("containers/duplicate-root.json", &value)
            .unwrap();

        let manifest = store.load_manifest().unwrap();
        let err = resolve_root_container(&store, &manifest).unwrap_err();
        match err {
            RepositoryError::CatalogLoad { diagnostics, .. } => {
                // RFC-051: at revision 11 the root is a directory named by its id, so a second
                // file declaring that id can only be a flat file, refused as
                // `container-migration-needed` — still fatal, still propagated.
                assert!(
                    diagnostics
                        .iter()
                        .any(|d| d.code == crate::catalog::codes::DUPLICATE_ID
                            || d.code == crate::container_store::codes::MIGRATION_NEEDED),
                    "expected a fatal duplicate diagnostic, got: {diagnostics:?}"
                );
            }
            other => panic!("expected RepositoryError::CatalogLoad, got: {other:?}"),
        }
    }

    // list_no_duplicate_when_root_in_index retired by RFC-038 Phase 3
    // (srs-rust#783): its scenario — the root container declared by BOTH
    // manifest.container and a containers/*.json file — is now a fatal
    // SRS038-R12-DUPLICATE-ID at catalog build, so the "don't list it twice"
    // guarantee is enforced upstream by construction. Embed listing is
    // covered by embed_only_list_containers_includes_embed.

    #[test]
    fn embed_only_filestore_get_container_returns_embed() {
        use tempfile::TempDir;
        let temp = TempDir::new().unwrap();
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let manifest_json = format!(
            r#"{{"srsVersion":"2.0-draft","repositoryId":"test","dataModelRevision":2,"container":{{"containerId":"{embed_id}","title":"Root"}}}}"#
        );
        crate::rfc052_root_container_migration_service::write_fixture_manifest(
            temp.path(),
            &serde_json::from_str(&manifest_json).unwrap(),
        )
        .unwrap();
        let store = crate::validation::fixture_ladder::store(temp.path());
        let c = get_container(&store, embed_id).unwrap();
        assert_eq!(c.container_id, embed_id);
        assert_eq!(c.title, "Root");
    }

    // --- Phase 2: embed-only and dual-write path ---

    #[test]
    fn embed_only_add_member_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, embed_id, member, None, None).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = crate::container_service::resolve_root_container(&store, &manifest)
            .unwrap()
            .unwrap();
        assert!(embed
            .member_instance_ids
            .as_ref()
            .is_some_and(|ids| ids.iter().any(|e| e.instance_id == member)));
    }

    #[test]
    fn embed_only_remove_member_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, embed_id, member, None, None).unwrap();
        remove_member(&store, embed_id, member).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = crate::container_service::resolve_root_container(&store, &manifest)
            .unwrap()
            .unwrap();
        assert!(embed
            .member_instance_ids
            .as_ref()
            .is_none_or(|ids| !ids.iter().any(|e| e.instance_id == member)));
    }

    #[test]
    fn embed_only_move_member_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        seed_instance(&store, "22222222-2222-4222-8222-222222222222");
        add_member(
            &store,
            embed_id,
            "11111111-1111-4111-8111-111111111111",
            None,
            None,
        )
        .unwrap();
        add_member(
            &store,
            embed_id,
            "22222222-2222-4222-8222-222222222222",
            None,
            None,
        )
        .unwrap();
        move_member(
            &store,
            embed_id,
            "22222222-2222-4222-8222-222222222222",
            Some(0),
            None,
        )
        .unwrap();
        let embed = crate::container_service::resolve_root_container(
            &store,
            &store.load_manifest().unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            embed.member_ids(),
            vec![
                "22222222-2222-4222-8222-222222222222".to_string(),
                "11111111-1111-4111-8111-111111111111".to_string()
            ]
        );
    }

    #[test]
    fn embed_only_update_container_title_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let patch = ContainerPatch {
            title: Some("Updated Root".to_string()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, embed_id, patch).unwrap().container;
        assert_eq!(updated.title, "Updated Root");
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            crate::container_service::resolve_root_container(&store, &manifest)
                .unwrap()
                .unwrap()
                .title,
            "Updated Root"
        );
    }

    // file_backed_root_add_member_updates_file retired by RFC-038 Phase 3
    // (srs-rust#783): the "file-backed root" it exercised — the root container
    // in BOTH manifest.container and a containers/*.json file — is now a
    // fatal SRS038-R12-DUPLICATE-ID at catalog build ([R1]: the embed is the
    // only authoritative root form). Root membership writes are covered by
    // embed_only_add_member_updates_manifest.

    #[test]
    fn update_container_all_fields_sync_to_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        // Embed-only root ([R1]): a containers/*.json file sharing the embed's
        // id is a fatal SRS038-R12-DUPLICATE-ID under the catalog.
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        crate::container_service::set_root_container(
            &store,
            &mut manifest,
            &minimal_container(embed_id, "Root"),
        )
        .unwrap();
        store.save_manifest(&manifest).unwrap();
        let patch = ContainerPatch {
            title: Some("New Title".to_string()),
            description: Some("A description".to_string()),
            ..ContainerPatch::default()
        };
        update_container(&store, embed_id, patch).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = crate::container_service::resolve_root_container(&store, &manifest)
            .unwrap()
            .unwrap();
        assert_eq!(embed.title, "New Title");
        assert_eq!(embed.description.as_deref(), Some("A description"));
    }
}
