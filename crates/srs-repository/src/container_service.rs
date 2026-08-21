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
use crate::writer::{new_instance_id, write_manifest};
use serde::{Deserialize, Serialize};
use srs_core::types::container::Container;
use srs_core::types::relation::Relation;
use srs_core::validation::container::validate_container;
use srs_schema::{SchemaRegistry, CONTAINER_SCHEMA_ID};
use std::collections::{HashSet, VecDeque};

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
    pub identity_instance_id: Option<String>,
    pub root_instance_ids: Option<Vec<String>>,
    pub member_instance_ids: Option<Vec<String>>,
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
    pub root_instance_id: Option<String>,
}

pub fn list_containers(
    store: &dyn RepositoryStore,
    filter: &ContainerListFilter,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    let mut summaries_raw = store.list_container_summaries()?;

    // Include manifest.container embed root if not already in containerIndex (RFC-013).
    let manifest = store.load_manifest()?;
    if let Some(ref embed) = manifest.container {
        if !summaries_raw
            .iter()
            .any(|(id, _)| id == &embed.container_id)
        {
            summaries_raw.insert(0, (embed.container_id.clone(), embed.title.clone()));
        }
    }

    // Loaded once, only when the member filter is in play: I-66 condition 3 is a
    // relation traversal, and re-reading the relations file per container would
    // turn a list into an O(n) file scan.
    let membership_relations: Vec<Relation> = if filter.member_instance_id.is_some() {
        crate::relation_service::load_relations(store)?
    } else {
        Vec::new()
    };

    let mut summaries = Vec::new();
    for (container_id, _title) in summaries_raw {
        let (container, _) = load_container_with_embed_fallback(store, &container_id)?;
        if let Some(ref ct) = filter.container_type {
            if container.container_type.as_deref() != Some(ct.as_str()) {
                continue;
            }
        }
        if let Some(ref member_filter) = filter.member_instance_id {
            // I-66 membership, not a second two-condition reading of it.
            if !member_ids(&container, &membership_relations)
                .iter()
                .any(|id| id == member_filter)
            {
                continue;
            }
        }
        if let Some(ref root_filter) = filter.root_instance_id {
            let in_roots = container
                .root_instance_ids
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id == root_filter));
            if !in_roots {
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

pub fn create_container(
    store: &dyn RepositoryStore,
    mut container: Container,
) -> Result<Container, RepositoryError> {
    if container.container_id.is_empty() {
        container.container_id = new_instance_id();
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

    store.save_container(&container)?;
    Ok(container)
}

pub fn get_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Container, RepositoryError> {
    let (container, _) = load_container_with_embed_fallback(store, container_id)?;
    Ok(container)
}

/// Resolve the repository's root container declared by `manifest.container`.
///
/// Resolution order (RFC-013):
/// 1. A materialised container in the container store (`containerIndex` / `containers/`)
///    whose id matches the embed's `containerId` — richest source when present
///    (e.g. repos scaffolded by srs-gov or `repo create`).
/// 2. The `manifest.container` embed itself. The embed is documented as the canonical
///    source of truth for the repository's identity, so an embed-only root (as written
///    by `repo set-root-container`, or by migrations of pre-RFC-013 repos) must resolve
///    without a container file existing.
///
/// Returns `Ok(None)` when the manifest declares no root container at all.
pub fn resolve_root_container(
    store: &dyn RepositoryStore,
    manifest: &crate::manifest::Manifest,
) -> Result<Option<Container>, RepositoryError> {
    let Some(embed) = manifest.container.as_ref() else {
        return Ok(None);
    };
    match store.load_container(&embed.container_id) {
        Ok(container) => Ok(Some(container)),
        Err(RepositoryError::ContainerNotFound { .. }) => Ok(Some(embed.clone())),
        Err(e) => Err(e),
    }
}

fn load_container_with_embed_fallback(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<(Container, bool), RepositoryError> {
    match store.load_container(container_id) {
        Ok(c) => Ok((c, false)),
        Err(RepositoryError::ContainerNotFound { .. }) => {
            let manifest = store.load_manifest()?;
            match resolve_root_container(store, &manifest)? {
                Some(c) if c.container_id == container_id => Ok((c, true)),
                _ => Err(RepositoryError::ContainerNotFound {
                    container_id: container_id.to_string(),
                }),
            }
        }
        Err(e) => Err(e),
    }
}

/// [`load_container_with_embed_fallback`] for a **repair** operation (ADR-045).
///
/// Two deliberate differences, both required for the caller to be able to act on
/// a repository whose catalog build is fatal under [R24]:
/// - the file-backed lookup goes through `load_container_unchecked`;
/// - the embed fallback reads `manifest.container` directly rather than calling
///   `resolve_root_container`, which routes through the **checked**
///   `store.load_container` and so would re-raise the very error being repaired.
pub(crate) fn load_container_for_repair(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<(Container, bool), RepositoryError> {
    match store.load_container_unchecked(container_id) {
        Ok(c) => Ok((c, false)),
        Err(RepositoryError::ContainerNotFound { .. }) => match store.load_manifest()?.container {
            Some(c) if c.container_id == container_id => Ok((c, true)),
            _ => Err(RepositoryError::ContainerNotFound {
                container_id: container_id.to_string(),
            }),
        },
        Err(e) => Err(e),
    }
}

/// [`save_container_syncing_embed`] for a **repair** operation (ADR-045) — the
/// write half of [`load_container_for_repair`]. Mirrors the
/// `sync_file_backed_root = false` behaviour of its checked counterpart.
fn save_container_for_repair(
    store: &dyn RepositoryStore,
    container: &Container,
    is_embed_only: bool,
) -> Result<(), RepositoryError> {
    if is_embed_only {
        let mut manifest = store.load_manifest()?;
        if manifest
            .container
            .as_ref()
            .map(|mc| mc.container_id.as_str())
            == Some(container.container_id.as_str())
        {
            manifest.container = Some(container.clone());
            write_manifest(store, &manifest)?;
        }
        return Ok(());
    }
    store.save_container_unchecked(container)
}

/// Save a container, syncing `manifest.container` when appropriate.
///
/// `sync_file_backed_root`: when true and the container is a file-backed root, do a
/// dual write (file + manifest) under the batch seam (ADR-041 G6). When false, only
/// the file is written — callers that manage manifest.container themselves (e.g.
/// `migrate_identity`) pass false to avoid overwriting their own manifest writes.
fn save_container_syncing_embed(
    store: &dyn RepositoryStore,
    container: &Container,
    is_embed_only: bool,
    sync_file_backed_root: bool,
) -> Result<(), RepositoryError> {
    if is_embed_only {
        // Caller guarantees is_embed_only=true only when container_id matches manifest.container
        // (load_container_with_embed_fallback enforces this). If the ID somehow doesn't match,
        // assert loudly rather than silently returning Ok without writing.
        let mut manifest = store.load_manifest()?;
        debug_assert_eq!(
            manifest.container.as_ref().map(|mc| mc.container_id.as_str()),
            Some(container.container_id.as_str()),
            "save_container_syncing_embed: is_embed_only=true but container_id does not match manifest.container"
        );
        if manifest
            .container
            .as_ref()
            .map(|mc| mc.container_id.as_str())
            == Some(container.container_id.as_str())
        {
            manifest.container = Some(container.clone());
            write_manifest(store, &manifest)?;
        }
        return Ok(());
    }
    if sync_file_backed_root {
        let mut manifest = store.load_manifest()?;
        let is_root = manifest
            .container
            .as_ref()
            .map(|mc| mc.container_id.as_str())
            == Some(container.container_id.as_str());
        if is_root {
            manifest.container = Some(container.clone());
            store.begin_batch();
            if let Err(e) = store.save_container(container) {
                store.abort_batch();
                return Err(e);
            }
            if let Err(e) = write_manifest(store, &manifest) {
                store.abort_batch();
                return Err(e);
            }
            if let Err(e) = store.commit_batch() {
                store.abort_batch();
                return Err(e);
            }
            return Ok(());
        }
    }
    store.save_container(container)?;
    Ok(())
}

pub fn update_container(
    store: &dyn RepositoryStore,
    container_id: &str,
    patch: ContainerPatch,
) -> Result<Container, RepositoryError> {
    let (mut container, is_embed_only) = load_container_with_embed_fallback(store, container_id)?;
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
    if let Some(ref v) = patch.identity_instance_id {
        container.identity_instance_id = Some(v.clone());
    }
    if let Some(mut v) = patch.root_instance_ids {
        v.sort();
        v.dedup();
        container.root_instance_ids = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(mut v) = patch.member_instance_ids {
        v.sort();
        v.dedup();
        container.member_instance_ids = if v.is_empty() { None } else { Some(v) };
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

    save_container_syncing_embed(store, &container, is_embed_only, true)?;
    Ok(container)
}

pub fn delete_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<String, RepositoryError> {
    // RFC-038 Change F: the [R22] cascade analogue for containers. A containerId
    // must never appear as a Relation endpoint (spec invariant), but a legacy or
    // hand-edited repo may carry such edges — remove them with the container so
    // the delete never leaves dangling endpoints behind.
    store.begin_batch();
    let write_result = crate::relation_service::delete_relations_incident_to(store, container_id)
        .and_then(|_| store.delete_container(container_id));
    match write_result {
        Ok(()) => store.commit_batch()?,
        Err(e) => {
            store.abort_batch();
            return Err(e);
        }
    }
    Ok(container_id.to_string())
}

/// The **one** membership operation (I-66, I-118) — the union of all three
/// conditions: `rootInstanceIds` ∪ `memberInstanceIds` ∪ everything reachable by
/// transitive `contains` traversal from `rootInstanceIds`. Every consumer that
/// asks "is this a member" routes through here (`find --container`, `container
/// resolve-view`, the MCP container resource, `containers_for_instance`).
///
/// `doctor_service`'s reachability check reads `memberInstanceIds` /
/// `rootInstanceIds` / `identityInstanceId` directly and deliberately does not
/// route through this: it answers a different question — "does any container
/// *declare* a reference to this id", the [R13] dangling-reference question —
/// for which a traversal-reachable instance is not a reference at all.
///
/// Pure so `list_containers` can filter a whole index against one relation load.
///
/// Order is `rootInstanceIds` in declared order, then `memberInstanceIds`, then
/// the traversal in breadth-first order with each node's outgoing `contains`
/// edges taken in the canonical `(createdAt, targetInstanceId)` tiebreak — a
/// total order, so the result is identical however the relations file is
/// ordered (RFC-038 [R14]). The visited set makes a `contains` cycle terminate.
fn member_ids(container: &Container, relations: &[Relation]) -> Vec<String> {
    let mut combined: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for id in container
        .root_instance_ids
        .iter()
        .chain(container.member_instance_ids.iter())
        .flatten()
    {
        if seen.insert(id.as_str()) {
            combined.push(id.clone());
        }
    }

    // I-66 condition 3: transitive `contains` from the roots (not from
    // `memberInstanceIds` — the spec anchors the traversal on roots alone).
    //
    // `traversed` is deliberately separate from `seen`: a node can already be
    // in the output (a root, or a declared member that a root also contains)
    // and still need walking, or everything it in turn contains is lost.
    let mut traversed: HashSet<&str> = HashSet::new();
    let mut frontier: VecDeque<&str> = VecDeque::new();
    for id in container.root_instance_ids.iter().flatten() {
        if traversed.insert(id.as_str()) {
            frontier.push_back(id.as_str());
        }
    }
    while let Some(source) = frontier.pop_front() {
        let mut children: Vec<&Relation> = relations
            .iter()
            .filter(|r| r.relation_type == "contains" && r.source_instance_id == source)
            .collect();
        children.sort_by(|a, b| {
            a.created_at
                .as_deref()
                .unwrap_or("")
                .cmp(b.created_at.as_deref().unwrap_or(""))
                .then_with(|| a.target_instance_id.cmp(&b.target_instance_id))
        });
        for rel in children {
            let target = rel.target_instance_id.as_str();
            if seen.insert(target) {
                combined.push(rel.target_instance_id.clone());
            }
            if traversed.insert(target) {
                frontier.push_back(target);
            }
        }
    }

    combined
}

pub(crate) fn list_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(store, container_id)?;
    let relations = crate::relation_service::load_relations(store)?;
    Ok(member_ids(&container, &relations))
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
/// a container reference in the [R13] reference set, and its dangling case is
/// srs-rust#837's separate question.
fn declared_membership(container: &Container) -> impl Iterator<Item = &str> {
    container
        .root_instance_ids
        .iter()
        .chain(container.member_instance_ids.iter())
        .flatten()
        .map(String::as_str)
}

pub fn add_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    require_resolvable_instances(store, [instance_id])?;
    let (mut container, is_embed_only) = load_container_with_embed_fallback(store, container_id)?;
    let mut members = container.member_instance_ids.unwrap_or_default();
    if members.iter().any(|id| id == instance_id) {
        return Ok(members);
    }
    members.push(instance_id.to_string());
    members.sort();
    container.member_instance_ids = Some(members.clone());
    save_container_syncing_embed(store, &container, is_embed_only, false)?;
    Ok(members)
}

/// Remove a member — a **repair** operation (ADR-045), so it reads and writes
/// through the unchecked catalog. Dropping a membership entry can only reduce
/// incoherence, and it is the only way back out of a repository bricked by a
/// dangling container reference (srs-rust#841).
pub fn remove_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let (mut container, is_embed_only) = load_container_for_repair(store, container_id)?;
    let mut members = container.member_instance_ids.unwrap_or_default();
    members.retain(|id| id != instance_id);
    if members.is_empty() {
        container.member_instance_ids = None;
    } else {
        container.member_instance_ids = Some(members.clone());
    }
    save_container_for_repair(store, &container, is_embed_only)?;
    Ok(container.member_instance_ids.unwrap_or_default())
}

pub fn list_roots(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(store, container_id)?;
    Ok(container.root_instance_ids.unwrap_or_default())
}

pub fn add_root(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    require_resolvable_instances(store, [instance_id])?;
    let (mut container, is_embed_only) = load_container_with_embed_fallback(store, container_id)?;
    let mut roots = container.root_instance_ids.unwrap_or_default();
    if roots.iter().any(|id| id == instance_id) {
        return Ok(roots);
    }
    roots.push(instance_id.to_string());
    roots.sort();
    container.root_instance_ids = Some(roots.clone());
    save_container_syncing_embed(store, &container, is_embed_only, false)?;
    Ok(container.root_instance_ids.unwrap_or_default())
}

/// Remove a root — a **repair** operation on the same terms as
/// [`remove_member`] (ADR-045).
pub fn remove_root(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let (mut container, is_embed_only) = load_container_for_repair(store, container_id)?;
    let mut roots = container.root_instance_ids.unwrap_or_default();
    roots.retain(|id| id != instance_id);
    if roots.is_empty() {
        container.root_instance_ids = None;
    } else {
        container.root_instance_ids = Some(roots.clone());
    }
    save_container_for_repair(store, &container, is_embed_only)?;
    Ok(container.root_instance_ids.unwrap_or_default())
}

/// RFC-038 Change F: remove `instance_id` from `memberInstanceIds` and
/// `rootInstanceIds` of every Container that names it. Called from the instance
/// delete paths (srs-rust#834) so a delete never leaves a dangling container
/// reference — `SRS038-R13-DANGLING-REFERENCE`, which [R24] makes fatal, i.e. a
/// successful delete would render the repository unloadable.
///
/// [R22] forbids a routine delete from writing `manifest.json`, from writing
/// `manifest.container`, and from writing "any object other than its own target"
/// — but all three prohibitions are conditioned on the same qualifier: "A
/// routine instance create, update, or delete **that is not an explicit
/// container-membership operation**…". Change F classifies exactly this case out
/// of that qualifier: "Deleting a root-container member is therefore **not** a
/// routine unscoped operation: it is an explicit container-membership operation,
/// and it writes the manifest." So none of the three apply here, which is what
/// permits both the inline-root manifest write and the file-backed container
/// writes below.
///
/// `containers_for_instance` matches `memberInstanceIds` *and* `rootInstanceIds`
/// across file-backed containers and the inline root alike — exactly the set the
/// catalog draws its container references from.
///
/// `identityInstanceId` is cleared in the same edit when it names the deleted
/// instance. It is not an [R13] reference — the catalog does not resolve it — but
/// leaving it behind only trades the fatal diagnostic for an invalid repository:
/// `validate` reports **I-81 as an error** ("identityInstanceId is not in
/// rootInstanceIds or memberInstanceIds") and `repository_navigation` fails
/// outright on the unresolvable identity. RFC-029 states that a root container
/// with no `identityInstanceId` is valid, so clearing is the state that stays
/// valid; `srs container update` re-points an identity when a successor exists
/// (clearing one deliberately has no encoding — srs-rust#837).
///
/// Two consequences worth knowing, neither introduced here:
/// - a root container left with no identity *and* no roots has no navigation —
///   `repository_navigation` returns `NotFound`, which is what `repo create`'s
///   scaffolded shape becomes once its sole purpose record is deleted;
/// - when roots do remain, navigation silently promotes the first one to the
///   identity node and drops it from `sections`. That fallback predates this
///   cascade and fires for any identity-less root container — srs-rust#838.
///
/// Only containers that list the instance are visited, so an identity naming a
/// non-member is not reached. RFC-013 requires an identity to be a member, and
/// I-81 enforces it on the root container, so that shape is already invalid.
///
/// The edits are applied to one loaded container and written once rather than
/// through `remove_member`/`remove_root`: they must land in a single write (the
/// inline root's is a non-atomic `manifest.json` truncate), and no existing
/// helper can clear an identity.
pub(crate) fn remove_instance_from_all_containers(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<(), RepositoryError> {
    for summary in containers_for_instance(store, instance_id)? {
        let (mut container, is_embed_only) =
            load_container_with_embed_fallback(store, &summary.container_id)?;
        let mut changed = false;
        for ids in [
            &mut container.member_instance_ids,
            &mut container.root_instance_ids,
        ] {
            if let Some(v) = ids.as_mut() {
                let before = v.len();
                v.retain(|id| id != instance_id);
                changed |= v.len() != before;
                if v.is_empty() {
                    *ids = None;
                }
            }
        }
        if container.identity_instance_id.as_deref() == Some(instance_id) {
            container.identity_instance_id = None;
            changed = true;
        }
        // Since I-66 condition 3 landed, `containers_for_instance` also returns
        // containers that reach the instance only through `contains` traversal.
        // Those declare nothing to remove, and [R22] does not license writing a
        // container this delete does not actually change.
        if !changed {
            continue;
        }
        save_container_syncing_embed(store, &container, is_embed_only, false)?;
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

/// Add a member to a container — public entry point for membership management commands.
pub fn add_container_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    add_member(store, container_id, instance_id)
}

/// Remove a member from a container — public entry point for membership management commands.
pub fn remove_container_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    remove_member(store, container_id, instance_id)
}

/// List members of a container — public entry point for membership inspection commands.
pub fn list_container_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    list_members(store, container_id)
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
        Some(crate::catalog::ROOT_CONTAINER_LOCATOR) => store
            .load_manifest()?
            .container
            .ok_or_else(|| RepositoryError::ContainerNotFound {
                container_id: container_id.to_string(),
            })?,
        Some(path) => {
            serde_json::from_value(store.load_instance_json(path)?).map_err(|source| {
                RepositoryError::ManifestParse {
                    path: std::path::PathBuf::from(path),
                    source,
                }
            })?
        }
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
    let known_ids: HashSet<String> = cat.instances.into_iter().map(|e| e.id).collect();

    if let Some(ref ids) = container.member_instance_ids {
        if ids.iter().any(|id| id == &container.container_id) {
            errors.push("containerId must not appear in memberInstanceIds".to_string());
        }
        for id in ids {
            if !known_ids.contains(id) {
                errors.push(format!(
                    "memberInstanceId '{}' not found in the instance set",
                    id
                ));
            }
        }
    }
    if let Some(ref ids) = container.root_instance_ids {
        if ids.iter().any(|id| id == &container.container_id) {
            errors.push("containerId must not appear in rootInstanceIds".to_string());
        }
        for id in ids {
            if !known_ids.contains(id) {
                errors.push(format!(
                    "rootInstanceId '{}' not found in the instance set",
                    id
                ));
            }
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
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        }
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
        let updated = update_container(&store, &created.container_id, patch).unwrap();
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
                relation_id: "de000001-0000-4000-a000-000000000001".to_string(),
                relation_type: "contains".to_string(),
                source_instance_id: created.container_id.clone(),
                target_instance_id: "some-instance".to_string(),
                asserted_by: None,
                confidence: None,
                created_at: None,
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
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
        )
        .unwrap();
        assert_eq!(out.len(), 1);
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
        )
        .unwrap();
        let out = add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert_eq!(out.len(), 1);
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
        )
        .unwrap();
        let out = remove_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.is_empty());
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
        assert!(out.is_empty());
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

    #[test]
    fn add_root_adds_id() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Roots"),
        )
        .unwrap();
        let out = add_root(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn remove_root_removes_id() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Roots"),
        )
        .unwrap();
        add_root(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let out = remove_root(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.is_empty());
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
        let own = |v: &[&str]| -> Option<Vec<String>> {
            (!v.is_empty()).then(|| v.iter().map(|s| s.to_string()).collect())
        };
        c.root_instance_ids = own(roots);
        c.member_instance_ids = own(members);
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

    // ---- I-66 condition 3: transitive `contains` traversal (srs-rust#863) ----

    fn contains_rel(id: &str, src: &str, tgt: &str, created_at: &str) -> Relation {
        Relation {
            relation_id: id.to_string(),
            relation_type: "contains".to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            asserted_by: None,
            confidence: None,
            created_at: Some(created_at.to_string()),
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        }
    }

    fn container_with_root(root: &str) -> Container {
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Doc");
        c.root_instance_ids = Some(vec![root.to_string()]);
        c
    }

    /// I-66 condition 3: an instance reachable only by `contains` traversal from
    /// a root is a member. Two hops deep, so this is transitive, not one-level.
    #[test]
    fn member_ids_includes_transitive_contains_from_roots() {
        let c = container_with_root("root");
        let rels = vec![
            contains_rel("r1", "root", "child", "2026-01-01T00:00:00Z"),
            contains_rel("r2", "child", "grandchild", "2026-01-01T00:00:00Z"),
        ];
        assert_eq!(
            member_ids(&c, &rels),
            vec![
                "root".to_string(),
                "child".to_string(),
                "grandchild".to_string()
            ]
        );
    }

    /// RFC-038 [R14]: the traversal order must not depend on the order the
    /// relations file happens to list its edges.
    #[test]
    fn member_ids_traversal_is_relation_order_independent() {
        let c = container_with_root("root");
        let base = vec![
            contains_rel("r1", "root", "b", "2026-01-01T00:00:00Z"),
            contains_rel("r2", "root", "a", "2026-01-01T00:00:00Z"),
            contains_rel("r3", "b", "c", "2026-01-01T00:00:00Z"),
        ];
        let expected = vec![
            "root".to_string(),
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ];
        for rotation in 0..base.len() {
            let mut rels = base.clone();
            rels.rotate_left(rotation);
            assert_eq!(member_ids(&c, &rels), expected, "rotation {rotation}");
        }
    }

    /// A `contains` cycle must terminate, not hang or duplicate.
    #[test]
    fn member_ids_terminates_on_contains_cycle() {
        let c = container_with_root("root");
        let rels = vec![
            contains_rel("r1", "root", "a", "2026-01-01T00:00:00Z"),
            contains_rel("r2", "a", "root", "2026-01-01T00:00:00Z"),
        ];
        assert_eq!(
            member_ids(&c, &rels),
            vec!["root".to_string(), "a".to_string()]
        );
    }

    /// A `contains` child that is *also* a declared member must still be
    /// walked through — otherwise everything it contains disappears. The
    /// output dedup and the traversal visited-set are separate for this reason.
    #[test]
    fn member_ids_traverses_through_a_declared_member() {
        let mut c = container_with_root("root");
        c.member_instance_ids = Some(vec!["child".to_string()]);
        let rels = vec![
            contains_rel("r1", "root", "child", "2026-01-01T00:00:00Z"),
            contains_rel("r2", "child", "grandchild", "2026-01-01T00:00:00Z"),
        ];
        assert_eq!(
            member_ids(&c, &rels),
            vec![
                "root".to_string(),
                "child".to_string(),
                "grandchild".to_string()
            ]
        );
    }

    /// I-66 anchors condition 3 on `rootInstanceIds` only — a `contains` edge
    /// out of a plain `memberInstanceIds` entry does not pull its target in.
    #[test]
    fn member_ids_does_not_traverse_from_plain_members() {
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Doc");
        c.member_instance_ids = Some(vec!["m".to_string()]);
        let rels = vec![contains_rel("r1", "m", "x", "2026-01-01T00:00:00Z")];
        assert_eq!(member_ids(&c, &rels), vec!["m".to_string()]);
    }

    /// Only `contains` traverses; another relation type off a root is not
    /// membership.
    #[test]
    fn member_ids_ignores_non_contains_relations() {
        let c = container_with_root("root");
        let mut rel = contains_rel("r1", "root", "x", "2026-01-01T00:00:00Z");
        rel.relation_type = "refines".to_string();
        assert_eq!(member_ids(&c, &[rel]), vec!["root".to_string()]);
    }

    /// The store-level surface: `list_members` (the one function every consumer
    /// routes through — `find --container`, `container resolve-view`, the MCP
    /// container resource) reports the `contains`-only member.
    #[test]
    fn list_members_includes_contains_only_member() {
        let store = MemoryStore::default();
        for id in ["root-note", "child-note"] {
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
                    "sourceInstanceId": "root-note",
                    "targetInstanceId": "child-note",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            }),
        );
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Doc");
        c.root_instance_ids = Some(vec!["root-note".to_string()]);
        let created = create_container(&store, c).unwrap();

        assert_eq!(
            list_members(&store, &created.container_id).unwrap(),
            vec!["root-note".to_string(), "child-note".to_string()]
        );
        assert!(is_member(&store, &created.container_id, "child-note").unwrap());
        // I-66's own operation: the container is returned for the
        // `contains`-only member too.
        let hits = containers_for_instance(&store, "child-note").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].container_id, created.container_id);
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
        add_member(&store, &a.container_id, member).unwrap();
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
        add_root(&store, &a.container_id, member).unwrap();
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
        add_root(&store, &a.container_id, id).unwrap();
        add_member(&store, &b.container_id, id).unwrap();
        let out = list_containers(
            &store,
            &ContainerListFilter {
                root_instance_id: Some(id.to_string()),
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
        add_member(&store, &created.container_id, id).unwrap();
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
        let updated = update_container(&store, id, patch).unwrap();
        assert_eq!(updated.title, "Updated");
    }

    #[test]
    fn container_membership_unchanged() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Test")).unwrap();
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, id, member).unwrap();
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
    fn add_root_succeeds_with_resolvable_instance_id() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        let member = "11111111-1111-4111-8111-111111111111";
        assert_eq!(add_root(&store, &cid, member).unwrap(), vec![member]);
        // idempotent
        assert_eq!(add_root(&store, &cid, member).unwrap(), vec![member]);
    }

    #[test]
    fn add_root_rejects_blank_instance_id() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        for blank in ["", "   "] {
            assert!(matches!(
                add_root(&store, &cid, blank),
                Err(RepositoryError::InvalidInput { .. })
            ));
        }
        assert!(get_container(&store, &cid)
            .unwrap()
            .root_instance_ids
            .is_none());
    }

    /// The blank-id message must survive an already-bricked repository — that is
    /// where it matters most, and it is the one diagnostic the catalog cannot
    /// give. The check therefore runs ahead of the catalog build.
    #[test]
    fn blank_instance_id_is_reported_even_on_a_bricked_repository() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container_with_membership(&store, id, &[UNRESOLVABLE], &[]);
        assert!(store.catalog().is_err(), "repository should be bricked");

        assert!(
            matches!(
                add_member(&store, id, "  "),
                Err(RepositoryError::InvalidInput { .. })
            ),
            "a blank id must be named as such, not buried under a CatalogLoad"
        );
    }

    #[test]
    fn add_root_rejects_unresolvable_instance_id() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        assert!(matches!(
            add_root(&store, &cid, UNRESOLVABLE),
            Err(RepositoryError::InstanceNotFound { .. })
        ));
        assert!(get_container(&store, &cid)
            .unwrap()
            .root_instance_ids
            .is_none());
    }

    #[test]
    fn add_member_rejects_blank_instance_id() {
        let store = make_store();
        let cid = seeded_container(&store, "550e8400-e29b-41d4-a716-446655440000");
        for blank in ["", "   "] {
            assert!(matches!(
                add_member(&store, &cid, blank),
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
            add_member(&store, &cid, UNRESOLVABLE),
            Err(RepositoryError::InstanceNotFound { .. })
        ));
        assert!(get_container(&store, &cid)
            .unwrap()
            .member_instance_ids
            .is_none());
    }

    /// The repair path (ADR-045) on a file-backed container: a dangling root
    /// makes the checked catalog fatal, and `remove_root` is the way back out.
    #[test]
    fn remove_root_repairs_bricked_file_backed_container() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container_with_membership(&store, id, &[UNRESOLVABLE], &[]);
        assert!(store.catalog().is_err(), "container should be bricked");

        remove_root(&store, id, UNRESOLVABLE).unwrap();

        store
            .catalog()
            .expect("repository loads again after repair");
        assert!(get_container(&store, id)
            .unwrap()
            .root_instance_ids
            .is_none());
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
    #[test]
    fn remove_root_repairs_bricked_embed_root_container() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut root = minimal_container(id, "Root");
        root.root_instance_ids = Some(vec![UNRESOLVABLE.to_string()]);
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(root);
        store.save_manifest(&manifest).unwrap();
        assert!(store.catalog().is_err(), "repository should be bricked");

        remove_root(&store, id, UNRESOLVABLE).unwrap();

        store
            .catalog()
            .expect("repository loads again after repair");
        assert!(store
            .load_manifest()
            .unwrap()
            .container
            .unwrap()
            .root_instance_ids
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
        manifest.container = Some(minimal_container(container_id, "Root"));
        store.save_manifest(&manifest).unwrap();

        let patch = ContainerPatch {
            identity_instance_id: Some("new-identity-id".to_string()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, container_id, patch).unwrap();
        assert_eq!(
            updated.identity_instance_id,
            Some("new-identity-id".to_string())
        );

        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.container.unwrap().identity_instance_id,
            Some("new-identity-id".to_string())
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
        manifest.container = Some(minimal_container(root_id, "Root"));
        store.save_manifest(&manifest).unwrap();

        // Patch OTHER container's identity_instance_id — manifest should not change
        let patch = ContainerPatch {
            identity_instance_id: Some("should-not-sync".to_string()),
            ..ContainerPatch::default()
        };
        update_container(&store, other_id, patch).unwrap();

        let manifest = store.load_manifest().unwrap();
        assert_eq!(manifest.container.unwrap().identity_instance_id, None);
    }

    #[test]
    fn update_container_patches_root_instance_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Root")).unwrap();
        let patch = ContainerPatch {
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap();
        assert_eq!(
            updated.root_instance_ids,
            Some(vec!["11111111-1111-4111-8111-111111111111".to_string()])
        );
        let reloaded = get_container(&store, id).unwrap();
        assert_eq!(
            reloaded.root_instance_ids,
            Some(vec!["11111111-1111-4111-8111-111111111111".to_string()])
        );
    }

    #[test]
    fn update_container_patches_member_instance_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Container")).unwrap();
        let patch = ContainerPatch {
            member_instance_ids: Some(vec!["22222222-2222-4222-8222-222222222222".to_string()]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap();
        assert_eq!(
            updated.member_instance_ids,
            Some(vec!["22222222-2222-4222-8222-222222222222".to_string()])
        );
        let reloaded = get_container(&store, id).unwrap();
        assert_eq!(
            reloaded.member_instance_ids,
            Some(vec!["22222222-2222-4222-8222-222222222222".to_string()])
        );
    }

    #[test]
    fn update_container_with_empty_root_instance_ids_clears_field() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut c = minimal_container(id, "Container");
        c.root_instance_ids = Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]);
        create_container(&store, c).unwrap();
        let patch = ContainerPatch {
            root_instance_ids: Some(vec![]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap();
        assert!(updated.root_instance_ids.is_none());
        let reloaded = get_container(&store, id).unwrap();
        assert!(reloaded.root_instance_ids.is_none());
    }

    #[test]
    fn update_container_with_empty_member_instance_ids_clears_field() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let mut c = minimal_container(id, "Container");
        c.member_instance_ids = Some(vec!["22222222-2222-4222-8222-222222222222".to_string()]);
        create_container(&store, c).unwrap();
        let patch = ContainerPatch {
            member_instance_ids: Some(vec![]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap();
        assert!(updated.member_instance_ids.is_none());
        let reloaded = get_container(&store, id).unwrap();
        assert!(reloaded.member_instance_ids.is_none());
    }

    #[test]
    fn update_container_sorts_patched_root_and_member_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Container")).unwrap();
        seed_instance(&store, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        seed_instance(&store, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let patch = ContainerPatch {
            root_instance_ids: Some(vec![
                "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            ]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, id, patch).unwrap();
        assert_eq!(
            updated.root_instance_ids,
            Some(vec![
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
                "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
            ])
        );
    }

    #[test]
    fn update_container_combined_patch_on_root_container_syncs_all_fields() {
        let store = make_store();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";
        // Embed-only root ([R1]): a containers/*.json file sharing the embed's
        // id is a fatal SRS038-R12-DUPLICATE-ID under the catalog.
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(minimal_container(container_id, "Root"));
        store.save_manifest(&manifest).unwrap();

        let patch = ContainerPatch {
            identity_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            member_instance_ids: Some(vec!["22222222-2222-4222-8222-222222222222".to_string()]),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, container_id, patch).unwrap();
        assert_eq!(
            updated.identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        assert_eq!(
            updated.root_instance_ids,
            Some(vec!["11111111-1111-4111-8111-111111111111".to_string()])
        );
        assert_eq!(
            updated.member_instance_ids,
            Some(vec!["22222222-2222-4222-8222-222222222222".to_string()])
        );
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.container.unwrap().identity_instance_id,
            Some("11111111-1111-4111-8111-111111111111".to_string())
        );
        let reloaded = get_container(&store, container_id).unwrap();
        assert_eq!(
            reloaded.root_instance_ids,
            Some(vec!["11111111-1111-4111-8111-111111111111".to_string()])
        );
        assert_eq!(
            reloaded.member_instance_ids,
            Some(vec!["22222222-2222-4222-8222-222222222222".to_string()])
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
        manifest.container = Some(minimal_container(embed_id, title));
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
        std::fs::write(temp.path().join("manifest.json"), &manifest_json).unwrap();
        let store = crate::FileStore::new(temp.path());
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
        add_member(&store, embed_id, member).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = manifest.container.unwrap();
        assert!(embed
            .member_instance_ids
            .as_ref()
            .is_some_and(|ids| ids.contains(&member.to_string())));
    }

    #[test]
    fn embed_only_remove_member_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&store, embed_id, member).unwrap();
        remove_member(&store, embed_id, member).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = manifest.container.unwrap();
        assert!(embed
            .member_instance_ids
            .as_ref()
            .is_none_or(|ids| !ids.contains(&member.to_string())));
    }

    #[test]
    fn embed_only_add_root_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let root = "11111111-1111-4111-8111-111111111111";
        add_root(&store, embed_id, root).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = manifest.container.unwrap();
        assert!(embed
            .root_instance_ids
            .as_ref()
            .is_some_and(|ids| ids.contains(&root.to_string())));
    }

    #[test]
    fn embed_only_remove_root_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let root = "11111111-1111-4111-8111-111111111111";
        add_root(&store, embed_id, root).unwrap();
        remove_root(&store, embed_id, root).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = manifest.container.unwrap();
        assert!(embed
            .root_instance_ids
            .as_ref()
            .is_none_or(|ids| !ids.contains(&root.to_string())));
    }

    #[test]
    fn embed_only_update_container_title_updates_manifest() {
        let embed_id = "aaa00000-0000-4000-8000-000000000001";
        let store = embed_only_store(embed_id, "Root");
        let patch = ContainerPatch {
            title: Some("Updated Root".to_string()),
            ..ContainerPatch::default()
        };
        let updated = update_container(&store, embed_id, patch).unwrap();
        assert_eq!(updated.title, "Updated Root");
        let manifest = store.load_manifest().unwrap();
        assert_eq!(manifest.container.unwrap().title, "Updated Root");
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
        manifest.container = Some(minimal_container(embed_id, "Root"));
        store.save_manifest(&manifest).unwrap();
        let patch = ContainerPatch {
            title: Some("New Title".to_string()),
            description: Some("A description".to_string()),
            ..ContainerPatch::default()
        };
        update_container(&store, embed_id, patch).unwrap();
        let manifest = store.load_manifest().unwrap();
        let embed = manifest.container.unwrap();
        assert_eq!(embed.title, "New Title");
        assert_eq!(embed.description.as_deref(), Some("A description"));
    }
}
