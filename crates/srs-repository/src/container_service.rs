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
use serde::{Deserialize, Serialize};
use srs_core::types::container::Container;
use srs_core::validation::container::validate_container;
use srs_schema::{SchemaRegistry, CONTAINER_SCHEMA_ID};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSummary {
    pub container_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerPatch {
    pub title: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub container_type: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta: Option<serde_json::Value>,
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
    let summaries_raw = store.list_container_summaries()?;
    let mut summaries = Vec::new();

    for (container_id, _title) in summaries_raw {
        let container = store.load_container(&container_id)?;
        if let Some(ref ct) = filter.container_type {
            if container.container_type.as_deref() != Some(ct.as_str()) {
                continue;
            }
        }
        if let Some(ref member_filter) = filter.member_instance_id {
            let in_members = container
                .member_instance_ids
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id == member_filter));
            let in_roots = container
                .root_instance_ids
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id == member_filter));
            if !in_members && !in_roots {
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

    store.save_container(&container)?;
    Ok(container)
}

pub fn get_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Container, RepositoryError> {
    store.load_container(container_id)
}

pub fn update_container(
    store: &dyn RepositoryStore,
    container_id: &str,
    patch: ContainerPatch,
) -> Result<Container, RepositoryError> {
    let mut container = get_container(store, container_id)?;
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

    // Schema validation at service boundary (after patch application)
    let raw = serde_json::to_value(&container).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(format!("containers/{container_id}.json")),
        source: e,
    })?;
    SchemaRegistry::global()
        .validate_by_id(CONTAINER_SCHEMA_ID, &raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from(format!("containers/{container_id}.json")),
            message: e.to_string(),
        })?;

    validate_container(&container)
        .map_err(|source| RepositoryError::ContainerValidation { source })?;

    store.save_container(&container)?;
    Ok(container)
}

pub fn delete_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<String, RepositoryError> {
    store.delete_container(container_id)?;
    Ok(container_id.to_string())
}

pub(crate) fn list_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(store, container_id)?;
    let roots = container.root_instance_ids.unwrap_or_default();
    let members = container.member_instance_ids.unwrap_or_default();
    let mut combined: Vec<String> = roots;
    for id in members {
        if !combined.contains(&id) {
            combined.push(id);
        }
    }
    Ok(combined)
}

pub fn add_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(store, container_id)?;
    let mut members = container.member_instance_ids.unwrap_or_default();
    if members.iter().any(|id| id == instance_id) {
        return Ok(members);
    }
    members.push(instance_id.to_string());
    members.sort();
    container.member_instance_ids = Some(members.clone());
    store.save_container(&container)?;
    Ok(members)
}

pub fn remove_member(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(store, container_id)?;
    let mut members = container.member_instance_ids.unwrap_or_default();
    members.retain(|id| id != instance_id);
    if members.is_empty() {
        container.member_instance_ids = None;
    } else {
        container.member_instance_ids = Some(members.clone());
    }
    store.save_container(&container)?;
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
    let mut container = get_container(store, container_id)?;
    let mut roots = container.root_instance_ids.unwrap_or_default();
    if roots.iter().any(|id| id == instance_id) {
        return Ok(roots);
    }
    roots.push(instance_id.to_string());
    roots.sort();
    container.root_instance_ids = Some(roots.clone());
    store.save_container(&container)?;
    Ok(container.root_instance_ids.unwrap_or_default())
}

pub fn remove_root(
    store: &dyn RepositoryStore,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(store, container_id)?;
    let mut roots = container.root_instance_ids.unwrap_or_default();
    roots.retain(|id| id != instance_id);
    if roots.is_empty() {
        container.root_instance_ids = None;
    } else {
        container.root_instance_ids = Some(roots.clone());
    }
    store.save_container(&container)?;
    Ok(container.root_instance_ids.unwrap_or_default())
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
    let container = get_container(store, container_id)?;
    let mut errors = Vec::new();
    if let Err(err) = validate_container(&container) {
        errors.push(err.to_string());
    }

    let manifest = store.load_manifest()?;
    let known_ids: HashSet<String> = manifest
        .instance_index
        .iter()
        .map(|e| e.instance_id().to_string())
        .collect();

    if let Some(ref ids) = container.member_instance_ids {
        if ids.iter().any(|id| id == &container.container_id) {
            errors.push("containerId must not appear in memberInstanceIds".to_string());
        }
        for id in ids {
            if !known_ids.contains(id) {
                errors.push(format!(
                    "memberInstanceId '{}' not found in instanceIndex",
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
                    "rootInstanceId '{}' not found in instanceIndex",
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
        MemoryStore::default()
    }

    fn minimal_container(id: &str, title: &str) -> Container {
        Container {
            container_id: id.to_string(),
            title: title.to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::HashMap::new(),
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
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            meta: None,
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
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            meta: None,
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
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            meta: None,
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

    #[test]
    fn validate_invariants_fails_invalid_member_id() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Invalid"),
        )
        .unwrap();
        add_member(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_invalid_root_id() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Invalid"),
        )
        .unwrap();
        add_root(
            &store,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_container_id_in_member_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let created = create_container(&store, minimal_container(id, "Invalid")).unwrap();
        add_member(&store, &created.container_id, id).unwrap();
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_container_id_in_root_ids() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let created = create_container(&store, minimal_container(id, "Invalid")).unwrap();
        add_root(&store, &created.container_id, id).unwrap();
        let report = validate_container_invariants(&store, &created.container_id).unwrap();
        assert!(!report.ok);
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
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            meta: None,
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

    #[test]
    fn validate_container_invariants_unchanged() {
        let store = make_store();
        let id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, minimal_container(id, "Test")).unwrap();
        // Clean container passes
        let report = validate_container_invariants(&store, id).unwrap();
        assert!(report.ok);
        // Adding the container's own ID as member fails
        add_member(&store, id, id).unwrap();
        let report = validate_container_invariants(&store, id).unwrap();
        assert!(!report.ok);
    }
}
