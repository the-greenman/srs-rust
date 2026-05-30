use crate::error::RepositoryError;
use crate::loader::load_tag_definition_relative;
use crate::manifest::load_manifest;
use crate::writer::{
    new_instance_id, upsert_tag_definition_index_entry, write_manifest_compat as write_manifest,
    write_tag_definition_path as write_tag_definition,
};
use srs_core::types::tag_definition::TagDefinition;
use srs_core::validation::tag_definition::validate_tag_definition;
use std::path::Path;

/// Summary for list operations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDefinitionSummary {
    pub instance_id: String,
    pub tag_key: String,
    pub label: Option<String>,
    pub roles: Option<Vec<String>>,
    pub status: Option<String>,
}

/// Result type for get_tag_definition_by_id
pub enum GetTagDefinitionResult {
    Found(Box<TagDefinition>),
    NotFound,
}

/// Result type for create_tag_definition
pub struct CreateTagDefinitionResult {
    pub tag_definition: TagDefinition,
}

/// Result type for update_tag_definition
pub struct UpdateTagDefinitionResult {
    pub tag_definition: TagDefinition,
}

/// Result type for delete_tag_definition
pub struct DeleteTagDefinitionResult {
    pub instance_id: String,
}

/// Convert a tag key to a filesystem-friendly slug.
/// Uses kebab-case: lowercase, spaces to hyphens, remove non-alphanumeric.
fn slugify_tag_key(tag_key: &str) -> String {
    tag_key
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// List all TagDefinitions in the repository.
/// Returns summaries for each definition.
pub fn list_tag_definitions(
    repo_root: &Path,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    let mut summaries = Vec::new();

    for entry in &manifest.instance_index {
        if !entry.is_tag_definition() {
            continue;
        }

        match load_tag_definition_relative(repo_root, &entry.path) {
            Ok(td) => {
                summaries.push(TagDefinitionSummary {
                    instance_id: td.instance_id,
                    tag_key: td.tag_key,
                    label: td.label,
                    roles: td.roles,
                    status: td.status,
                });
            }
            Err(_) => continue, // Skip invalid entries
        }
    }

    Ok(summaries)
}

/// List TagDefinitions filtered by role.
pub fn list_tag_definitions_by_role(
    repo_root: &Path,
    role: &str,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError> {
    let all = list_tag_definitions(repo_root)?;
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|td| {
            td.roles
                .as_ref()
                .map(|roles| roles.iter().any(|r| r == role))
                .unwrap_or(false)
        })
        .collect();
    Ok(filtered)
}

/// Get a TagDefinition by its instance ID.
pub fn get_tag_definition_by_id(
    repo_root: &Path,
    id: &str,
) -> Result<GetTagDefinitionResult, RepositoryError> {
    let manifest = load_manifest(repo_root)?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id && e.is_tag_definition());

    match entry {
        Some(entry) => {
            let td = load_tag_definition_relative(repo_root, &entry.path)?;
            Ok(GetTagDefinitionResult::Found(Box::new(td)))
        }
        None => Ok(GetTagDefinitionResult::NotFound),
    }
}

/// Get all foundation signal tags.
/// Returns tag_key strings for all TagDefinitions with role "foundation".
/// Returns Ok(vec![]) if no definitions exist — not an error.
pub fn get_foundation_signal_tags(repo_root: &Path) -> Result<Vec<String>, RepositoryError> {
    let foundation_defs = list_tag_definitions_by_role(repo_root, "foundation")?;
    let tag_keys: Vec<String> = foundation_defs.into_iter().map(|td| td.tag_key).collect();
    Ok(tag_keys)
}

/// Create a new TagDefinition.
/// Mints instance_id if empty, validates, writes to disk, updates manifest.
pub fn create_tag_definition(
    repo_root: &Path,
    mut tag_definition: TagDefinition,
) -> Result<CreateTagDefinitionResult, RepositoryError> {
    // Validate before minting ID
    validate_tag_definition(&tag_definition).map_err(|e| {
        RepositoryError::TagDefinitionValidation {
            path: repo_root.join("records/tag-definitions"),
            source: e,
        }
    })?;

    // Mint instance_id if empty
    if tag_definition.instance_id.is_empty() {
        tag_definition.instance_id = new_instance_id();
    }

    // Ensure the target directory exists
    let target_dir = repo_root.join("records/tag-definitions");
    std::fs::create_dir_all(&target_dir).map_err(|e| RepositoryError::Io {
        path: target_dir.clone(),
        source: e,
    })?;

    // Generate slug from tag_key for filename
    let slug = slugify_tag_key(&tag_definition.tag_key);
    let filename = format!("{}-{}.json", slug, &tag_definition.instance_id[..8]);
    let file_path = target_dir.join(&filename);

    // Write the TagDefinition
    write_tag_definition(&tag_definition, &file_path)?;

    // Calculate relative path for manifest
    let relative_path = file_path
        .strip_prefix(repo_root)
        .map_err(|_| RepositoryError::Io {
            path: file_path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "file path not within repo root",
            ),
        })?
        .to_string_lossy()
        .to_string();

    // Update manifest
    let mut manifest = load_manifest(repo_root)?;
    upsert_tag_definition_index_entry(&mut manifest, &tag_definition, &relative_path);
    write_manifest(&manifest)?;

    Ok(CreateTagDefinitionResult { tag_definition })
}

/// Service: Update an existing TagDefinition
/// Validates, writes to disk, and updates manifest
pub fn update_tag_definition(
    repo_root: &Path,
    tag_definition: TagDefinition,
) -> Result<UpdateTagDefinitionResult, RepositoryError> {
    // Validate before writing
    validate_tag_definition(&tag_definition).map_err(|e| {
        RepositoryError::TagDefinitionValidation {
            path: repo_root.join("records/tag-definitions"),
            source: e,
        }
    })?;

    let mut manifest = load_manifest(repo_root)?;

    // Find the existing entry
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == tag_definition.instance_id && e.is_tag_definition())
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: repo_root.join("records/tag-definitions"),
        })?;

    // Write the updated definition
    let full_path = repo_root.join(entry.path());
    write_tag_definition(&tag_definition, &full_path)?;

    // Update manifest
    upsert_tag_definition_index_entry(&mut manifest, &tag_definition, entry.path());
    write_manifest(&manifest)?;

    Ok(UpdateTagDefinitionResult { tag_definition })
}

/// Service: Delete a TagDefinition by ID
/// Removes the file and updates the manifest
pub fn delete_tag_definition(
    repo_root: &Path,
    id: &str,
) -> Result<DeleteTagDefinitionResult, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Find the entry in the manifest
    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == id && e.is_tag_definition())
        .ok_or_else(|| RepositoryError::NotFound {
            path: repo_root.join("records/tag-definitions"),
        })?;

    let entry = manifest.instance_index[entry_index].clone();
    let path = entry.path().to_string();

    // Remove the file
    let full_path = repo_root.join(&path);
    if full_path.exists() {
        std::fs::remove_file(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
    }

    // Remove from manifest
    manifest.instance_index.remove(entry_index);
    write_manifest(&manifest)?;

    Ok(DeleteTagDefinitionResult {
        instance_id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn create_minimal_manifest() -> serde_json::Value {
        json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": []
        })
    }

    fn create_test_td(tag_key: &str) -> TagDefinition {
        TagDefinition {
            instance_id: String::new(), // Will be minted
            tag_key: tag_key.to_string(),
            label: Some(format!("{} Label", tag_key)),
            description: Some(format!("Description for {}", tag_key)),
            roles: Some(vec!["foundation".to_string()]),
            aliases: None,
            status: Some("active".to_string()),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn list_tag_definitions_empty_repo() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        let result = list_tag_definitions(temp.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn create_tag_definition_writes_file_and_updates_manifest() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        let td = create_test_td("test-tag");
        let result = create_tag_definition(temp.path(), td).unwrap();

        // Check file was written (find it via the manifest index path)
        let manifest = load_manifest(temp.path()).unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == result.tag_definition.instance_id)
            .expect("manifest entry should exist");
        let file_path = temp.path().join(entry.path());
        assert!(file_path.exists());

        // Check instance_id was minted
        assert!(!result.tag_definition.instance_id.is_empty());

        // Check manifest was updated
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(temp.path().join("manifest.json")).unwrap())
                .unwrap();
        let index = manifest["instanceIndex"].as_array().unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0]["tier"], 3);
        assert_eq!(index[0]["instanceId"], result.tag_definition.instance_id);
    }

    #[test]
    fn get_tag_definition_by_id_finds_created() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        let td = create_test_td("test-tag");
        let created = create_tag_definition(temp.path(), td).unwrap();

        let result =
            get_tag_definition_by_id(temp.path(), &created.tag_definition.instance_id).unwrap();

        match result {
            GetTagDefinitionResult::Found(td) => {
                assert_eq!(td.tag_key, "test-tag");
            }
            GetTagDefinitionResult::NotFound => panic!("Should have found the tag definition"),
        }
    }

    #[test]
    fn get_tag_definition_by_id_not_found() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        let result =
            get_tag_definition_by_id(temp.path(), "00000000-0000-0000-0000-000000000000").unwrap();

        match result {
            GetTagDefinitionResult::Found(_) => panic!("Should not have found anything"),
            GetTagDefinitionResult::NotFound => (), // Expected
        }
    }

    #[test]
    fn list_tag_definitions_by_role_filters_correctly() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        // Create foundation tag
        let mut foundation_td = create_test_td("foundation-tag");
        foundation_td.roles = Some(vec!["foundation".to_string()]);
        create_tag_definition(temp.path(), foundation_td).unwrap();

        // Create navigation tag
        let mut nav_td = create_test_td("nav-tag");
        nav_td.roles = Some(vec!["navigation".to_string()]);
        create_tag_definition(temp.path(), nav_td).unwrap();

        // List foundation tags
        let foundation_results = list_tag_definitions_by_role(temp.path(), "foundation").unwrap();
        assert_eq!(foundation_results.len(), 1);
        assert_eq!(foundation_results[0].tag_key, "foundation-tag");

        // List navigation tags
        let nav_results = list_tag_definitions_by_role(temp.path(), "navigation").unwrap();
        assert_eq!(nav_results.len(), 1);
        assert_eq!(nav_results[0].tag_key, "nav-tag");
    }

    #[test]
    fn get_foundation_signal_tags_returns_tag_keys() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        // Create foundation tag
        let mut td = create_test_td("purpose");
        td.roles = Some(vec!["foundation".to_string()]);
        create_tag_definition(temp.path(), td).unwrap();

        let signal_tags = get_foundation_signal_tags(temp.path()).unwrap();
        assert_eq!(signal_tags, vec!["purpose"]);
    }

    #[test]
    fn get_foundation_signal_tags_empty_when_none_defined() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        let signal_tags = get_foundation_signal_tags(temp.path()).unwrap();
        assert!(signal_tags.is_empty());
    }

    #[test]
    fn slugify_tag_key_works() {
        assert_eq!(slugify_tag_key("Foundation"), "foundation");
        assert_eq!(slugify_tag_key("My Tag"), "my-tag");
        assert_eq!(slugify_tag_key("Complex!!!Tag"), "complextag");
    }

    // Acceptance Criteria Tests for Phase 2

    #[test]
    fn tag_update_rewrites_definition() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        // Create a tag
        let td = create_test_td("test-tag");
        let created = create_tag_definition(temp.path(), td).unwrap();
        let instance_id = created.tag_definition.instance_id.clone();

        // Update the tag
        let mut updated = created.tag_definition;
        updated.label = Some("Updated Label".to_string());

        let result = update_tag_definition(temp.path(), updated).unwrap();
        assert_eq!(
            result.tag_definition.label,
            Some("Updated Label".to_string())
        );

        // Verify file was rewritten (find it via the manifest index path)
        let manifest = load_manifest(temp.path()).unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == instance_id)
            .expect("manifest entry should exist");
        let file_path = temp.path().join(entry.path());
        let file_content = fs::read_to_string(&file_path).unwrap();
        let file_td: TagDefinition = serde_json::from_str(&file_content).unwrap();
        assert_eq!(file_td.label, Some("Updated Label".to_string()));

        // Verify can be retrieved
        let fetched = get_tag_definition_by_id(temp.path(), &instance_id).unwrap();
        match fetched {
            GetTagDefinitionResult::Found(td) => {
                assert_eq!(td.label, Some("Updated Label".to_string()));
            }
            _ => panic!("Should find updated tag"),
        }
    }

    #[test]
    fn tag_delete_removes_definition() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&create_minimal_manifest()).unwrap(),
        )
        .unwrap();

        // Create a tag
        let td = create_test_td("deletable-tag");
        let created = create_tag_definition(temp.path(), td).unwrap();
        let instance_id = created.tag_definition.instance_id.clone();
        // Get the file path from the manifest before deletion
        let manifest = load_manifest(temp.path()).unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == instance_id)
            .expect("manifest entry should exist");
        let file_path = temp.path().join(entry.path());

        // Verify it was created
        assert!(file_path.exists());

        // Delete the tag
        let result = delete_tag_definition(temp.path(), &instance_id).unwrap();
        assert_eq!(result.instance_id, instance_id);

        // Verify file was removed
        assert!(!file_path.exists());

        // Verify manifest was updated
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(temp.path().join("manifest.json")).unwrap())
                .unwrap();
        let index = manifest["instanceIndex"].as_array().unwrap();
        assert!(index.is_empty());

        // Verify it's no longer findable
        let fetched = get_tag_definition_by_id(temp.path(), &instance_id).unwrap();
        match fetched {
            GetTagDefinitionResult::NotFound => {} // Expected
            _ => panic!("Should not find deleted tag"),
        }
    }
}
