use crate::error::RepositoryError;
use crate::manifest::{load_manifest, Manifest};
use crate::writer::{new_instance_id, write_manifest};
use serde::{Deserialize, Serialize};
use srs_core::types::container::{Container, ContainerIndexEntry};
use srs_core::validation::container::validate_container;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSummary {
    pub container_id: String,
    pub title: String,
    pub path: String,
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

fn load_container_index(manifest: &Manifest) -> Vec<ContainerIndexEntry> {
    manifest
        .extra
        .get("containerIndex")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn save_container_index(
    manifest: &mut Manifest,
    index: Vec<ContainerIndexEntry>,
) -> Result<(), RepositoryError> {
    let value = serde_json::to_value(index).map_err(|source| RepositoryError::Serialize {
        path: manifest.root.join("manifest.json"),
        source,
    })?;
    manifest.extra.insert("containerIndex".to_string(), value);
    Ok(())
}

fn load_container_file(path: &Path) -> Result<Container, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|source| RepositoryError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    serde_json::from_str(&content).map_err(|source| RepositoryError::ManifestParse {
        path: path.to_path_buf(),
        source,
    })
}

fn write_container_file(container: &Container, path: &Path) -> Result<(), RepositoryError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| RepositoryError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let content =
        serde_json::to_string_pretty(container).map_err(|source| RepositoryError::Serialize {
            path: path.to_path_buf(),
            source,
        })?;
    std::fs::write(path, content).map_err(|source| RepositoryError::Io {
        path: path.to_path_buf(),
        source,
    })
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
    repo_root: &Path,
    container_id: &str,
) -> Result<(Manifest, Vec<ContainerIndexEntry>, ContainerIndexEntry), RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    let index = load_container_index(&manifest);
    let entry = index
        .iter()
        .find(|e| e.container_id == container_id)
        .cloned()
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })?;
    Ok((manifest, index, entry))
}

pub fn list_containers(
    repo_root: &Path,
    container_type: Option<&str>,
    member_instance_id: Option<&str>,
    root_instance_id: Option<&str>,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    let index = load_container_index(&manifest);
    let mut summaries = Vec::new();

    for entry in index {
        let container = load_container_file(&repo_root.join(&entry.path))?;
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
            path: entry.path,
            container_type: container.container_type,
        });
    }

    Ok(summaries)
}

pub fn containers_for_instance(
    repo_root: &Path,
    instance_id: &str,
) -> Result<Vec<ContainerSummary>, RepositoryError> {
    list_containers(repo_root, None, Some(instance_id), None)
}

pub fn create_container(
    repo_root: &Path,
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
    let file_path = repo_root.join("containers").join(filename);
    write_container_file(&container, &file_path)?;

    let relative_path = file_path
        .strip_prefix(repo_root)
        .map_err(|_| RepositoryError::Io {
            path: file_path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "container path not within repo root",
            ),
        })?
        .to_string_lossy()
        .to_string();

    let mut manifest = load_manifest(repo_root)?;
    let mut index = load_container_index(&manifest);
    index.push(ContainerIndexEntry {
        container_id: container.container_id.clone(),
        title: container.title.clone(),
        path: relative_path,
    });
    save_container_index(&mut manifest, index)?;
    write_manifest(&manifest)?;
    Ok(container)
}

pub fn get_container(repo_root: &Path, container_id: &str) -> Result<Container, RepositoryError> {
    let (_, _, entry) = find_container_entry(repo_root, container_id)?;
    load_container_file(&repo_root.join(entry.path))
}

pub fn update_container(
    repo_root: &Path,
    container_id: &str,
    patch: ContainerPatch,
) -> Result<Container, RepositoryError> {
    let mut container = get_container(repo_root, container_id)?;
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

    let mut manifest = load_manifest(repo_root)?;
    let mut index = load_container_index(&manifest);
    let pos = index
        .iter()
        .position(|e| e.container_id == container_id)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })?;
    let path = index[pos].path.clone();
    write_container_file(&container, &repo_root.join(&path))?;
    index[pos].title = container.title.clone();
    save_container_index(&mut manifest, index)?;
    write_manifest(&manifest)?;
    Ok(container)
}

pub fn delete_container(repo_root: &Path, container_id: &str) -> Result<String, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;
    let mut index = load_container_index(&manifest);
    let pos = index
        .iter()
        .position(|e| e.container_id == container_id)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })?;
    let entry = index.remove(pos);

    save_container_index(&mut manifest, index)?;
    write_manifest(&manifest)?;

    let full_path = repo_root.join(&entry.path);
    if let Err(err) = std::fs::remove_file(&full_path) {
        eprintln!(
            "warning: failed to remove container file {:?}: {}",
            full_path, err
        );
    }
    Ok(container_id.to_string())
}

pub fn list_members(repo_root: &Path, container_id: &str) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(repo_root, container_id)?;
    Ok(container.member_instance_ids.unwrap_or_default())
}

pub fn add_member(
    repo_root: &Path,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(repo_root, container_id)?;
    let mut members = container.member_instance_ids.unwrap_or_default();
    if members.iter().any(|id| id == instance_id) {
        return Ok(members);
    }
    members.push(instance_id.to_string());
    members.sort();
    container.member_instance_ids = Some(members.clone());

    let (_, _, entry) = find_container_entry(repo_root, container_id)?;
    write_container_file(&container, &repo_root.join(entry.path))?;
    Ok(members)
}

pub fn remove_member(
    repo_root: &Path,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(repo_root, container_id)?;
    let mut members = container.member_instance_ids.unwrap_or_default();
    members.retain(|id| id != instance_id);
    if members.is_empty() {
        container.member_instance_ids = None;
    } else {
        container.member_instance_ids = Some(members.clone());
    }

    let (_, _, entry) = find_container_entry(repo_root, container_id)?;
    write_container_file(&container, &repo_root.join(entry.path))?;
    Ok(members)
}

pub fn list_roots(repo_root: &Path, container_id: &str) -> Result<Vec<String>, RepositoryError> {
    let container = get_container(repo_root, container_id)?;
    Ok(container.root_instance_ids.unwrap_or_default())
}

pub fn add_root(
    repo_root: &Path,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(repo_root, container_id)?;
    let mut roots = container.root_instance_ids.unwrap_or_default();
    if roots.iter().any(|id| id == instance_id) {
        return Ok(roots);
    }
    roots.push(instance_id.to_string());
    roots.sort();
    container.root_instance_ids = Some(roots.clone());

    let (_, _, entry) = find_container_entry(repo_root, container_id)?;
    write_container_file(&container, &repo_root.join(entry.path))?;
    Ok(roots)
}

pub fn remove_root(
    repo_root: &Path,
    container_id: &str,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut container = get_container(repo_root, container_id)?;
    let mut roots = container.root_instance_ids.unwrap_or_default();
    roots.retain(|id| id != instance_id);
    if roots.is_empty() {
        container.root_instance_ids = None;
    } else {
        container.root_instance_ids = Some(roots.clone());
    }

    let (_, _, entry) = find_container_entry(repo_root, container_id)?;
    write_container_file(&container, &repo_root.join(entry.path))?;
    Ok(roots)
}

pub fn is_member(
    repo_root: &Path,
    container_id: &str,
    instance_id: &str,
) -> Result<bool, RepositoryError> {
    let members = list_members(repo_root, container_id)?;
    Ok(members.iter().any(|id| id == instance_id))
}

pub fn validate_container_invariants(
    repo_root: &Path,
    container_id: &str,
) -> Result<ContainerValidationReport, RepositoryError> {
    let container = get_container(repo_root, container_id)?;
    let mut errors = Vec::new();
    if let Err(err) = validate_container(&container) {
        errors.push(err.to_string());
    }

    let manifest = load_manifest(repo_root)?;
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
    use crate::manifest::load_manifest;
    use tempfile::TempDir;

    fn make_minimal_container_repo(temp: &TempDir) -> std::path::PathBuf {
        std::fs::create_dir(temp.path().join(".srs")).unwrap();
        let manifest = serde_json::json!({
            "instanceIndex": [],
            "containerIndex": []
        });
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        temp.path().to_path_buf()
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
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Sprint 1");
        let out = create_container(&repo, c).unwrap();
        assert_eq!(out.title, "Sprint 1");
        let listed = list_containers(&repo, None, None, None).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn get_container_missing_returns_error() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let err = get_container(&repo, "missing").unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ContainerNotFound { container_id } if container_id == "missing"
        ));
    }

    #[test]
    fn create_container_mints_id_if_empty() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let out = create_container(&repo, minimal_container("", "Sprint 1")).unwrap();
        assert!(uuid::Uuid::parse_str(&out.container_id).is_ok());
    }

    #[test]
    fn list_containers_returns_all() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440001", "B"),
        )
        .unwrap();
        let listed = list_containers(&repo, None, None, None).unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn get_container_returns_container() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Sprint 1"),
        )
        .unwrap();
        let got = get_container(&repo, &created.container_id).unwrap();
        assert_eq!(got.title, "Sprint 1");
    }

    #[test]
    fn update_container_patches_title() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
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
        let updated = update_container(&repo, &created.container_id, patch).unwrap();
        assert_eq!(updated.title, "New");
    }

    #[test]
    fn update_container_list_shows_updated_title() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
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
        update_container(&repo, &created.container_id, patch).unwrap();
        let listed = list_containers(&repo, None, None, None).unwrap();
        assert_eq!(listed[0].title, "New");
    }

    #[test]
    fn update_container_preserves_other_fields() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440000", "Old");
        c.description = Some("keep".to_string());
        let created = create_container(&repo, c).unwrap();
        let patch = ContainerPatch {
            title: Some("New".to_string()),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            meta: None,
        };
        update_container(&repo, &created.container_id, patch).unwrap();
        let got = get_container(&repo, &created.container_id).unwrap();
        assert_eq!(got.description.as_deref(), Some("keep"));
    }

    #[test]
    fn delete_container_removes_index_entry() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Delete"),
        )
        .unwrap();
        delete_container(&repo, &created.container_id).unwrap();
        let listed = list_containers(&repo, None, None, None).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_container_file_is_absent_after_delete() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Delete"),
        )
        .unwrap();
        let listed = list_containers(&repo, None, None, None).unwrap();
        let path = repo.join(&listed[0].path);
        assert!(path.exists());
        delete_container(&repo, &created.container_id).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn delete_container_missing_returns_error() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let err = delete_container(&repo, "missing").unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ContainerNotFound { container_id } if container_id == "missing"
        ));
    }

    #[test]
    fn add_member_adds_id() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        let out = add_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn add_member_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        add_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let out = add_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn remove_member_removes_id() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        add_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let out = remove_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn remove_member_noop_when_absent() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        let out = remove_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn remove_member_clears_field_when_list_empty() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        add_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        remove_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let got = get_container(&repo, &created.container_id).unwrap();
        assert!(got.member_instance_ids.is_none());
    }

    #[test]
    fn add_root_adds_id() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Roots"),
        )
        .unwrap();
        let out = add_root(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn remove_root_removes_id() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Roots"),
        )
        .unwrap();
        add_root(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let out = remove_root(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn validate_invariants_passes_clean() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Clean"),
        )
        .unwrap();
        let report = validate_container_invariants(&repo, &created.container_id).unwrap();
        assert!(report.ok);
    }

    #[test]
    fn validate_invariants_fails_invalid_member_id() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Invalid"),
        )
        .unwrap();
        add_member(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let report = validate_container_invariants(&repo, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_invalid_root_id() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Invalid"),
        )
        .unwrap();
        add_root(
            &repo,
            &created.container_id,
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let report = validate_container_invariants(&repo, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_container_id_in_member_ids() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let created = create_container(&repo, minimal_container(id, "Invalid")).unwrap();
        add_member(&repo, &created.container_id, id).unwrap();
        let report = validate_container_invariants(&repo, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn validate_invariants_fails_container_id_in_root_ids() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let created = create_container(&repo, minimal_container(id, "Invalid")).unwrap();
        add_root(&repo, &created.container_id, id).unwrap();
        let report = validate_container_invariants(&repo, &created.container_id).unwrap();
        assert!(!report.ok);
    }

    #[test]
    fn containers_for_instance_returns_matching_containers() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let a = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let _b = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440001", "B"),
        )
        .unwrap();
        let member = "11111111-1111-4111-8111-111111111111";
        add_member(&repo, &a.container_id, member).unwrap();
        let out = containers_for_instance(&repo, member).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].container_id, a.container_id);
    }

    #[test]
    fn containers_for_instance_includes_root_role() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let a = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let member = "11111111-1111-4111-8111-111111111111";
        add_root(&repo, &a.container_id, member).unwrap();
        let out = containers_for_instance(&repo, member).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn containers_for_instance_returns_empty_when_no_match() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let out = containers_for_instance(&repo, "11111111-1111-4111-8111-111111111111").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn list_containers_root_filter_matches_root_only() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let a = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "A"),
        )
        .unwrap();
        let b = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440001", "B"),
        )
        .unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        add_root(&repo, &a.container_id, id).unwrap();
        add_member(&repo, &b.container_id, id).unwrap();
        let out = list_containers(&repo, None, None, Some(id)).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].container_id, a.container_id);
    }

    #[test]
    fn create_container_mints_full_uuid_prefix_filename_safely() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let out = create_container(&repo, minimal_container("", "Sprint")).unwrap();
        let listed = list_containers(&repo, None, None, None).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].path.contains(&out.container_id[..8]));
    }

    #[test]
    fn is_member_true_and_false() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let created = create_container(
            &repo,
            minimal_container("550e8400-e29b-41d4-a716-446655440000", "Members"),
        )
        .unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        assert!(!is_member(&repo, &created.container_id, id).unwrap());
        add_member(&repo, &created.container_id, id).unwrap();
        assert!(is_member(&repo, &created.container_id, id).unwrap());
    }

    #[test]
    fn save_and_load_container_index_roundtrip() {
        let temp = TempDir::new().unwrap();
        let repo = make_minimal_container_repo(&temp);
        let mut manifest = load_manifest(&repo).unwrap();
        let idx = vec![ContainerIndexEntry {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "A".to_string(),
            path: "containers/a-550e8400.json".to_string(),
        }];
        save_container_index(&mut manifest, idx).unwrap();
        let loaded = load_container_index(&manifest);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "A");
    }
}
