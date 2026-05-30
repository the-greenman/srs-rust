use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use serde::{Deserialize, Serialize};
use srs_core::types::container::Container;
use srs_core::validation::container::validate_container;
use std::collections::HashSet;

/// Internal index entry mapping a container ID to its file path within the repository.
/// This type is an srs-repository implementation detail and must not be exposed to callers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContainerIndexEntry {
    pub container_id: String,
    pub title: String,
    pub path: String,
}

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

fn load_container_index(
    store: &dyn RepositoryStore,
) -> Result<Vec<ContainerIndexEntry>, RepositoryError> {
    let manifest = store.load_manifest()?;
    Ok(manifest
        .extra
        .get("containerIndex")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

fn save_container_index(
    store: &dyn RepositoryStore,
    index: Vec<ContainerIndexEntry>,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    let value = serde_json::to_value(index).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from("manifest.json"),
        source,
    })?;
    manifest.extra.insert("containerIndex".to_string(), value);
    store.save_manifest(&manifest)
}

fn load_container(
    store: &dyn RepositoryStore,
    relative_path: &str,
) -> Result<Container, RepositoryError> {
    let value = store.load_container_json(relative_path)?;
    serde_json::from_value(value).map_err(|source| RepositoryError::ManifestParse {
        path: std::path::PathBuf::from(relative_path),
        source,
    })
}

fn write_container(
    store: &dyn RepositoryStore,
    container: &Container,
    relative_path: &str,
) -> Result<(), RepositoryError> {
    let value = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source,
    })?;
    store.save_container_json(relative_path, &value)
}

fn slugify_title(title: &str) -> String {
    let slug = title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-");
    if slug.is_empty() {
        "container".to_string()
    } else {
        slug
    }
}

fn find_container_entry(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<ContainerIndexEntry, RepositoryError> {
    let index = load_container_index(store)?;
    index
        .into_iter()
        .find(|e| e.container_id == container_id)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })
}

pub fn list_containers(
    store: &dyn RepositoryStore,
    container_type: Option<&str>,
    member_instance_id: Option<&str>,
    root_instance_id: Option<&str>,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    let index = load_container_index(store)?;
    let mut summaries = Vec::new();

    for entry in index {
        let container = load_container(store, &entry.path)?;
        if let Some(filter) = container_type {
            if container.container_type.as_deref() != Some(filter) {
                continue;
            }
        }
        if let Some(member_filter) = member_instance_id {
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
        if let Some(root_filter) = root_instance_id {
            let in_roots = container
                .root_instance_ids
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id == root_filter));
            if !in_roots {
                continue;
            }
        }
        summaries.push(ContainerSummary {
            container_id: entry.container_id,
            title: entry.title,
            container_type: container.container_type,
        });
    }

    Ok(summaries)
}

pub fn containers_for_instance(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    list_containers(store, None, Some(instance_id), None)
}

pub fn create_container(
    store: &dyn RepositoryStore,
    mut container: Container,
) -> Result<Container, RepositoryError> {
    if container.container_id.is_empty() {
        container.container_id = new_instance_id();
    }
    validate_container(&container)
        .map_err(|source| RepositoryError::ContainerValidation { source })?;

    let slug = slugify_title(&container.title);
    let id_prefix = &container.container_id[..container.container_id.len().min(8)];
    let filename = format!("{}-{}.json", slug, id_prefix);
    let relative_path = format!("containers/{}", filename);

    store.ensure_containers_dir()?;
    write_container(store, &container, &relative_path)?;

    let mut index = load_container_index(store)?;
    index.push(ContainerIndexEntry {
        container_id: container.container_id.clone(),
        title: container.title.clone(),
        path: relative_path,
    });
    save_container_index(store, index)?;
    Ok(container)
}

pub fn get_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Container, RepositoryError> {
    let entry = find_container_entry(store, container_id)?;
    load_container(store, &entry.path)
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
    validate_container(&container)
        .map_err(|source| RepositoryError::ContainerValidation { source })?;

    let mut index = load_container_index(store)?;
    let pos = index
        .iter()
        .position(|e| e.container_id == container_id)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })?;
    let path = index[pos].path.clone();
    write_container(store, &container, &path)?;
    index[pos].title = container.title.clone();
    save_container_index(store, index)?;
    Ok(container)
}

pub fn delete_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<String, RepositoryError> {
    let mut index = load_container_index(store)?;
    let pos = index
        .iter()
        .position(|e| e.container_id == container_id)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })?;
    let entry = index.remove(pos);
    save_container_index(store, index)?;

    if let Err(err) = store.delete_container_file(&entry.path) {
        eprintln!(
            "warning: failed to remove container file '{}': {}",
            entry.path, err
        );
    }
    Ok(container_id.to_string())
}

pub fn list_members(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(store, container_id)?;
    Ok(container.member_instance_ids.unwrap_or_default())
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

    let entry = find_container_entry(store, container_id)?;
    write_container(store, &container, &entry.path)?;
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

    let entry = find_container_entry(store, container_id)?;
    write_container(store, &container, &entry.path)?;
    Ok(members)
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

    let entry = find_container_entry(store, container_id)?;
    write_container(store, &container, &entry.path)?;
    Ok(roots)
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

    let entry = find_container_entry(store, container_id)?;
    write_container(store, &container, &entry.path)?;
    Ok(roots)
}

pub fn is_member(
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
        let listed = list_containers(&store, None, None, None).unwrap();
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
        let listed = list_containers(&store, None, None, None).unwrap();
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
        let listed = list_containers(&store, None, None, None).unwrap();
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
        let listed = list_containers(&store, None, None, None).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_container_file_is_absent_after_delete() {
        let store = make_store();
        let created = create_container(
            &store,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Delete"),
        )
        .unwrap();
        let index = load_container_index(&store).unwrap();
        let path = index[0].path.clone();
        assert!(store.load_container_json(&path).is_ok());
        delete_container(&store, &created.container_id).unwrap();
        assert!(store.load_container_json(&path).is_err());
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
        let out = list_containers(&store, None, None, Some(id)).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].container_id, a.container_id);
    }

    #[test]
    fn create_container_mints_full_uuid_prefix_filename_safely() {
        let store = make_store();
        let out = create_container(&store, minimal_container("", "Sprint")).unwrap();
        let index = load_container_index(&store).unwrap();
        assert_eq!(index.len(), 1);
        assert!(index[0].path.contains(&out.container_id[..8]));
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
    fn save_and_load_container_index_roundtrip() {
        let store = make_store();
        let idx = vec![ContainerIndexEntry {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "A".to_string(),
            path: "containers/a-550e8400.json".to_string(),
        }];
        save_container_index(&store, idx).unwrap();
        let loaded = load_container_index(&store).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "A");
    }
}
