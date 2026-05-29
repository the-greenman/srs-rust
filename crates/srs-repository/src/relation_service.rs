use crate::error::RepositoryError;
use crate::manifest::load_manifest;
use srs_core::types::relation::{Relation, RelationsCollection};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Summary for relation list operations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationSummary {
    pub relation_id: String,
    pub relation_type: String,
    pub source_id: String,
    pub target_id: String,
}

/// Result for get_relation_by_id
#[derive(Debug, Clone)]
pub enum GetRelationResult {
    Found(Box<Relation>),
    NotFound,
}

/// Result for create_relation
#[derive(Debug, Clone)]
pub struct CreateRelationResult {
    pub relation: Relation,
}

/// Result for delete_relation
#[derive(Debug, Clone)]
pub struct DeleteRelationResult {
    pub relation_id: String,
}

/// Filter options for listing relations
#[derive(Debug, Clone, Default)]
pub struct ListRelationsFilter {
    pub source: Option<String>,
    pub target: Option<String>,
    pub relation_type: Option<String>,
}

/// List relations from the relations-collection.json file with optional filtering
pub fn list_relations(
    repo_root: &Path,
    filter: ListRelationsFilter,
) -> Result<Vec<RelationSummary>, RepositoryError> {
    let relations = load_relations(repo_root)?;

    let filtered: Vec<_> = relations
        .into_iter()
        .filter(|r| {
            if let Some(ref source_filter) = filter.source {
                if &r.source_instance_id != source_filter {
                    return false;
                }
            }
            if let Some(ref target_filter) = filter.target {
                if &r.target_instance_id != target_filter {
                    return false;
                }
            }
            if let Some(ref type_filter) = filter.relation_type {
                if &r.relation_type != type_filter {
                    return false;
                }
            }
            true
        })
        .map(|r| RelationSummary {
            relation_id: r.relation_id.clone(),
            relation_type: r.relation_type.clone(),
            source_id: r.source_instance_id.clone(),
            target_id: r.target_instance_id.clone(),
        })
        .collect();

    Ok(filtered)
}

/// Get a relation by its relation ID
pub fn get_relation_by_id(
    repo_root: &Path,
    id: &str,
) -> Result<GetRelationResult, RepositoryError> {
    let relations = load_relations(repo_root)?;

    match relations.into_iter().find(|r| r.relation_id == id) {
        Some(relation) => Ok(GetRelationResult::Found(Box::new(relation))),
        None => Ok(GetRelationResult::NotFound),
    }
}

/// Create a new relation with E1-E4 validation
pub fn create_relation(
    repo_root: &Path,
    relation: Relation,
    definitions: &[RelationTypeDefinition],
) -> Result<CreateRelationResult, RepositoryError> {
    // Build owned context data
    let manifest = load_manifest(repo_root)?;
    let known_instance_ids: HashSet<String> = manifest
        .instance_index
        .iter()
        .map(|e| e.instance_id().to_string())
        .collect();
    let instance_semantic_types: HashMap<String, String> = HashMap::new();
    let ctx = RelationValidationContext {
        definitions,
        known_instance_ids: &known_instance_ids,
        instance_semantic_types: &instance_semantic_types,
    };

    // Validate the relation (E1-E4 checks)
    validate_relation(&relation, &ctx, true).map_err(|errors| {
        RepositoryError::RelationValidation {
            relation_id: relation.relation_id.clone(),
            message: errors
                .iter()
                .map(|e| format!("{:?}: {}", e.code, e.message))
                .collect::<Vec<_>>()
                .join(", "),
        }
    })?;

    // Load existing collection
    let mut collection = load_relations_collection(repo_root)?;

    // Check for duplicate relation_id
    if collection
        .relations
        .iter()
        .any(|r| r.relation_id == relation.relation_id)
    {
        return Err(RepositoryError::RelationValidation {
            relation_id: relation.relation_id.clone(),
            message: format!("Relation with id '{}' already exists", relation.relation_id),
        });
    }

    // Add the new relation
    collection.relations.push(relation.clone());

    // Write back to file
    write_relations_collection(repo_root, &collection)?;

    Ok(CreateRelationResult { relation })
}

/// Delete a relation by its relation ID
pub fn delete_relation(
    repo_root: &Path,
    relation_id: &str,
) -> Result<DeleteRelationResult, RepositoryError> {
    let mut collection = load_relations_collection(repo_root)?;

    // Find and remove the relation
    let pos = collection
        .relations
        .iter()
        .position(|r| r.relation_id == relation_id)
        .ok_or_else(|| RepositoryError::NotFound {
            path: repo_root.join("relations/relations-collection.json"),
        })?;

    collection.relations.remove(pos);

    // Write back to file
    write_relations_collection(repo_root, &collection)?;

    Ok(DeleteRelationResult {
        relation_id: relation_id.to_string(),
    })
}

/// Load all relations from the relations-collection.json file
pub(crate) fn load_relations(repo_root: &Path) -> Result<Vec<Relation>, RepositoryError> {
    let collection = load_relations_collection(repo_root)?;
    Ok(collection.relations)
}

/// Load the relations collection file
fn load_relations_collection(repo_root: &Path) -> Result<RelationsCollection, RepositoryError> {
    let relations_path = repo_root.join("relations/relations-collection.json");

    if !relations_path.exists() {
        // Return empty collection if file doesn't exist
        return Ok(RelationsCollection {
            schema: Some(
                "https://srs.semanticops.com/schema/2.0/relations-collection.json".to_string(),
            ),
            relations: Vec::new(),
        });
    }

    let content = std::fs::read_to_string(&relations_path).map_err(|e| RepositoryError::Io {
        path: relations_path.clone(),
        source: e,
    })?;

    let collection: RelationsCollection =
        serde_json::from_str(&content).map_err(|e| RepositoryError::RecordLoad {
            path: relations_path.clone(),
            source: e,
        })?;

    Ok(collection)
}

/// Write the relations collection to file
fn write_relations_collection(
    repo_root: &Path,
    collection: &RelationsCollection,
) -> Result<(), RepositoryError> {
    let relations_dir = repo_root.join("relations");
    let relations_path = relations_dir.join("relations-collection.json");

    // Ensure directory exists
    std::fs::create_dir_all(&relations_dir).map_err(|e| RepositoryError::Io {
        path: relations_dir.clone(),
        source: e,
    })?;

    let json =
        serde_json::to_string_pretty(collection).map_err(|e| RepositoryError::Serialize {
            path: relations_path.clone(),
            source: e,
        })?;

    std::fs::write(&relations_path, json).map_err(|e| RepositoryError::Io {
        path: relations_path.clone(),
        source: e,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_temp_repo_with_relations(temp: &TempDir) {
        std::fs::create_dir(temp.path().join("relations")).unwrap();

        // Create a minimal manifest
        let manifest = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": [
                {"instanceId": "note-1", "tier": 0, "path": "records/notes/note1.json"},
                {"instanceId": "note-2", "tier": 0, "path": "records/notes/note2.json"},
                {"instanceId": "note-3", "tier": 0, "path": "records/notes/note3.json"},
                {"instanceId": "note-4", "tier": 0, "path": "records/notes/note4.json"}
            ]
        });
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let relations = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [
                {
                    "relationId": "r1",
                    "relationType": "contains",
                    "sourceInstanceId": "note-1",
                    "targetInstanceId": "note-2",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                {
                    "relationId": "r2",
                    "relationType": "references",
                    "sourceInstanceId": "note-2",
                    "targetInstanceId": "note-3",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                {
                    "relationId": "r3",
                    "relationType": "contains",
                    "sourceInstanceId": "note-1",
                    "targetInstanceId": "note-4",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            ]
        });

        std::fs::write(
            temp.path().join("relations/relations-collection.json"),
            serde_json::to_string_pretty(&relations).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_relations_returns_all() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let result = list_relations(temp.path(), ListRelationsFilter::default()).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn list_relations_filters_by_source() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let filter = ListRelationsFilter {
            source: Some("note-1".to_string()),
            target: None,
            relation_type: None,
        };
        let result = list_relations(temp.path(), filter).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.source_id == "note-1"));
    }

    #[test]
    fn list_relations_filters_by_target() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let filter = ListRelationsFilter {
            source: None,
            target: Some("note-2".to_string()),
            relation_type: None,
        };
        let result = list_relations(temp.path(), filter).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].relation_id, "r1");
    }

    #[test]
    fn list_relations_filters_by_type() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let filter = ListRelationsFilter {
            source: None,
            target: None,
            relation_type: Some("contains".to_string()),
        };
        let result = list_relations(temp.path(), filter).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.relation_type == "contains"));
    }

    #[test]
    fn get_relation_by_id_finds_relation() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let result = get_relation_by_id(temp.path(), "r2").unwrap();
        match result {
            GetRelationResult::Found(relation) => {
                assert_eq!(relation.relation_type, "references");
                assert_eq!(relation.source_instance_id, "note-2");
            }
            GetRelationResult::NotFound => panic!("Should have found relation"),
        }
    }

    #[test]
    fn get_relation_by_id_not_found() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let result = get_relation_by_id(temp.path(), "nonexistent").unwrap();
        match result {
            GetRelationResult::Found(_) => panic!("Should not have found relation"),
            GetRelationResult::NotFound => (), // Expected
        }
    }

    #[test]
    fn list_relations_empty_when_no_file() {
        let temp = TempDir::new().unwrap();
        // Create manifest for consistency
        let manifest = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": []
        });
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        // Don't create relations-collection.json

        let result = list_relations(temp.path(), ListRelationsFilter::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn relation_create_appends_to_relations_file() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let new_relation = Relation {
            relation_id: "r4".to_string(),
            relation_type: "contains".to_string(),
            source_instance_id: "note-3".to_string(),
            target_instance_id: "note-4".to_string(),
            asserted_by: None,
            confidence: None,
            created_at: Some("2026-01-02T00:00:00Z".to_string()),
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        };

        let definitions = vec![RelationTypeDefinition {
            schema: None,
            id: "00000000-0000-0000-0000-000000000099".to_string(),
            version: 1,
            relation_type: "contains".to_string(),
            namespace: "com.test".to_string(),
            label: "Contains".to_string(),
            description: "A contains B".to_string(),
            category: srs_core::types::relation_type_definition::RelationTypeCategory::Composition,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: None,
            updated_at: None,
        }];
        let result = create_relation(temp.path(), new_relation, &definitions).unwrap();
        assert_eq!(result.relation.relation_id, "r4");

        // Verify it was written to file
        let collection: RelationsCollection = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("relations/relations-collection.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(collection.relations.len(), 4);
        assert!(collection.relations.iter().any(|r| r.relation_id == "r4"));
    }

    #[test]
    fn relation_delete_removes_from_relations_file() {
        let temp = TempDir::new().unwrap();
        create_temp_repo_with_relations(&temp);

        let result = delete_relation(temp.path(), "r2").unwrap();
        assert_eq!(result.relation_id, "r2");

        // Verify it was removed from file
        let collection: RelationsCollection = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("relations/relations-collection.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(collection.relations.len(), 2);
        assert!(!collection.relations.iter().any(|r| r.relation_id == "r2"));
    }
}
