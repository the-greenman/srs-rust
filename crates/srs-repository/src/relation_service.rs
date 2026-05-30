//! # Relation Service
//!
//! Public API for relation operations. This module is the sole entry point for
//! all relation logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, container orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: RelationListFilter = RelationListFilter { container_id: ctx.container_id };
//! let result = relation_service::list_relations(store, input)?;
//! output::ok("relation list", result)
//! ```

use crate::container_service;
use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::relation::{Relation, RelationsCollection};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use std::collections::{HashMap, HashSet};

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
    /// If Some, only return relations where BOTH source AND target are members of this container.
    pub container_id: Option<String>,
}

/// List relations from the relations-collection.json file with optional filtering
pub fn list_relations(
    store: &dyn RepositoryStore,
    filter: ListRelationsFilter,
) -> Result<Vec<RelationSummary>, RepositoryError> {
    // Resolve container members once if container filter is set
    let member_ids: Option<HashSet<String>> = if let Some(ref cid) = filter.container_id {
        let members = container_service::list_members(store, cid)?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    let relations = load_relations(store)?;

    let filtered: Vec<_> = relations
        .into_iter()
        .filter(|r| {
            // Container filter: both source AND target must be members
            if let Some(ref member_set) = member_ids {
                if !member_set.contains(&r.source_instance_id)
                    || !member_set.contains(&r.target_instance_id)
                {
                    return false;
                }
            }
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

/// Create a relation, loading relation type definitions internally from the package.
///
/// This variant does not require the caller to supply definitions — the service
/// resolves them from the package. Use this from service-layer callers.
pub fn create_relation_auto(
    store: &dyn RepositoryStore,
    relation: Relation,
) -> Result<CreateRelationResult, RepositoryError> {
    let package = store.load_package()?;
    create_relation(store, relation, &package.relation_type_definitions)
}

/// Get a relation by its relation ID
pub fn get_relation_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetRelationResult, RepositoryError> {
    let relations = load_relations(store)?;

    match relations.into_iter().find(|r| r.relation_id == id) {
        Some(relation) => Ok(GetRelationResult::Found(Box::new(relation))),
        None => Ok(GetRelationResult::NotFound),
    }
}

/// Create a new relation with E1-E4 validation
pub fn create_relation(
    store: &dyn RepositoryStore,
    relation: Relation,
    definitions: &[RelationTypeDefinition],
) -> Result<CreateRelationResult, RepositoryError> {
    // Build owned context data
    let manifest = store.load_manifest()?;
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
    let (relative_path, mut collection) = load_relations_collection(store)?;

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

    // Write back
    write_relations_collection(store, &relative_path, &collection)?;

    Ok(CreateRelationResult { relation })
}

/// Delete a relation by its relation ID
pub fn delete_relation(
    store: &dyn RepositoryStore,
    relation_id: &str,
) -> Result<DeleteRelationResult, RepositoryError> {
    let (relative_path, mut collection) = load_relations_collection(store)?;

    // Find and remove the relation
    let pos = collection
        .relations
        .iter()
        .position(|r| r.relation_id == relation_id)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(&relative_path),
        })?;

    collection.relations.remove(pos);

    // Write back
    write_relations_collection(store, &relative_path, &collection)?;

    Ok(DeleteRelationResult {
        relation_id: relation_id.to_string(),
    })
}

/// Load all relations from the relations collection file.
pub(crate) fn load_relations(
    store: &dyn RepositoryStore,
) -> Result<Vec<Relation>, RepositoryError> {
    let (_, collection) = load_relations_collection(store)?;
    Ok(collection.relations)
}

/// Load the relations collection, returning (relative_path, collection).
///
/// Path resolution order:
/// 1. `relationsPath` declared in `manifest.json`
/// 2. `relations/relations-collection.json` (legacy default)
/// 3. `relations/relations.json` (alternate convention)
///
/// Returns an empty collection (at the default write path) if no file is found.
fn load_relations_collection(
    store: &dyn RepositoryStore,
) -> Result<(String, RelationsCollection), RepositoryError> {
    let default_write_path = "relations/relations-collection.json".to_string();
    let empty = || RelationsCollection {
        schema: Some(
            "https://srs.semanticops.com/schema/2.0/relations-collection.json".to_string(),
        ),
        relations: Vec::new(),
    };

    // Determine the path to try first from the manifest's relationsPath field.
    // Only suppress NotFound/Io errors (no manifest yet); propagate all other errors.
    let manifest_path = match store.load_manifest() {
        Ok(m) => m
            .extra
            .get("relationsPath")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        Err(
            RepositoryError::Io { .. }
            | RepositoryError::NotFound { .. }
            | RepositoryError::ManifestMissing { .. },
        ) => None,
        Err(e) => return Err(e),
    };

    // Build candidate list: manifest path first, then the two defaults.
    let candidates: Vec<String> = [
        manifest_path,
        Some("relations/relations-collection.json".to_string()),
        Some("relations/relations.json".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();

    for relative_path in &candidates {
        match store.load_relations_json(relative_path) {
            Ok(value) => {
                let collection: RelationsCollection =
                    serde_json::from_value(value).map_err(|e| RepositoryError::RecordLoad {
                        path: std::path::PathBuf::from(relative_path),
                        source: e,
                    })?;
                return Ok((relative_path.clone(), collection));
            }
            Err(RepositoryError::Io { .. } | RepositoryError::NotFound { .. }) => continue,
            Err(e) => return Err(e),
        }
    }

    Ok((default_write_path, empty()))
}

/// Write the relations collection to the store.
fn write_relations_collection(
    store: &dyn RepositoryStore,
    relative_path: &str,
    collection: &RelationsCollection,
) -> Result<(), RepositoryError> {
    let dir = relative_path
        .rfind('/')
        .map(|i| &relative_path[..i])
        .unwrap_or("relations");
    store.ensure_relations_dir(dir)?;

    let value = serde_json::to_value(collection).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })?;
    store.save_relations_json(relative_path, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;

    fn make_store_with_relations() -> MemoryStore {
        let store = MemoryStore::default();
        // Add instance index to manifest
        let mut manifest = store.load_manifest().unwrap();
        for id in ["note-1", "note-2", "note-3", "note-4"] {
            manifest
                .instance_index
                .push(crate::index::InstanceIndexEntry {
                    instance_id: id.to_string(),
                    tier: 0,
                    path: format!("records/notes/{}.json", id),
                    title: None,
                    tags: None,
                });
        }
        store.save_manifest(&manifest).unwrap();

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
        store
            .save_relations_json("relations/relations-collection.json", &relations)
            .unwrap();
        store
    }

    fn make_relation(id: &str, src: &str, tgt: &str, rel_type: &str) -> Relation {
        Relation {
            relation_id: id.to_string(),
            relation_type: rel_type.to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
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
        }
    }

    #[test]
    fn list_relations_returns_all() {
        let store = make_store_with_relations();
        let result = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn list_relations_filters_by_source() {
        let store = make_store_with_relations();
        let filter = ListRelationsFilter {
            source: Some("note-1".to_string()),
            target: None,
            relation_type: None,
            container_id: None,
        };
        let result = list_relations(&store, filter).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.source_id == "note-1"));
    }

    #[test]
    fn list_relations_filters_by_target() {
        let store = make_store_with_relations();
        let filter = ListRelationsFilter {
            source: None,
            target: Some("note-2".to_string()),
            relation_type: None,
            container_id: None,
        };
        let result = list_relations(&store, filter).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].relation_id, "r1");
    }

    #[test]
    fn list_relations_filters_by_type() {
        let store = make_store_with_relations();
        let filter = ListRelationsFilter {
            source: None,
            target: None,
            relation_type: Some("contains".to_string()),
            container_id: None,
        };
        let result = list_relations(&store, filter).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.relation_type == "contains"));
    }

    #[test]
    fn get_relation_by_id_finds_relation() {
        let store = make_store_with_relations();
        let result = get_relation_by_id(&store, "r2").unwrap();
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
        let store = make_store_with_relations();
        let result = get_relation_by_id(&store, "nonexistent").unwrap();
        match result {
            GetRelationResult::Found(_) => panic!("Should not have found relation"),
            GetRelationResult::NotFound => (),
        }
    }

    #[test]
    fn list_relations_empty_when_no_file() {
        let store = MemoryStore::default();
        let result = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn relation_create_appends() {
        let store = make_store_with_relations();
        let new_relation = make_relation("r4", "note-3", "note-4", "contains");
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
        let result = create_relation(&store, new_relation, &definitions).unwrap();
        assert_eq!(result.relation.relation_id, "r4");

        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn load_relations_respects_manifest_relations_path() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("relationsPath".to_string(), json!("relations/custom.json"));
        store.save_manifest(&manifest).unwrap();

        let relations = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [
                {
                    "relationId": "rc1",
                    "relationType": "precedes",
                    "sourceInstanceId": "a",
                    "targetInstanceId": "b",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            ]
        });
        store
            .save_relations_json("relations/custom.json", &relations)
            .unwrap();

        let result = load_relations(&store).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].relation_id, "rc1");
    }

    #[test]
    fn load_relations_returns_empty_when_no_file() {
        let store = MemoryStore::default();
        let result = load_relations(&store).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn relation_delete_removes() {
        let store = make_store_with_relations();
        let result = delete_relation(&store, "r2").unwrap();
        assert_eq!(result.relation_id, "r2");

        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert!(!all.iter().any(|r| r.relation_id == "r2"));
    }

    #[test]
    fn load_relations_propagates_manifest_parse_error() {
        // A broken manifest (ManifestParse) must not be silently swallowed —
        // load_relations should return the error rather than falling through
        // to default candidate paths.
        use crate::error::RepositoryError;
        use crate::manifest::Manifest;
        use crate::package::Package;
        use crate::repository_lifecycle::{CreateRepositoryResult, InitializeRepositoryInput};
        use crate::store::RepositoryStore;

        struct BrokenManifestStore;

        impl RepositoryStore for BrokenManifestStore {
            fn repository_root(&self) -> std::path::PathBuf {
                unimplemented!()
            }
            fn repository_exists(&self) -> Result<bool, RepositoryError> {
                unimplemented!()
            }
            fn initialize_repository(
                &self,
                _: &InitializeRepositoryInput,
            ) -> Result<CreateRepositoryResult, RepositoryError> {
                unimplemented!()
            }
            fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
                Err(RepositoryError::ManifestParse {
                    path: std::path::PathBuf::from("manifest.json"),
                    source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
                })
            }
            fn save_manifest(&self, _: &Manifest) -> Result<(), RepositoryError> {
                Ok(())
            }
            fn load_package(&self) -> Result<Package, RepositoryError> {
                unimplemented!()
            }
            fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
                unimplemented!()
            }
            fn save_package_json(&self, _: &serde_json::Value) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_field(
                &self,
                _: &str,
                _: &srs_core::types::field::Field,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_field_file(
                &self,
                _: &str,
                _: &srs_core::types::field::Field,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_field_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_fields_dir(&self) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_type(
                &self,
                _: &str,
                _: &srs_core::types::record_type::RecordType,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_type_file(
                &self,
                _: &str,
                _: &srs_core::types::record_type::RecordType,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_type_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_types_dir(&self) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_relation_type_definition(
                &self,
                _: &str,
                _: &srs_core::types::relation_type_definition::RelationTypeDefinition,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_relation_types_dir(&self) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_view(
                &self,
                _: &str,
                _: &srs_core::types::view::View,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_view_file(
                &self,
                _: &str,
                _: &srs_core::types::view::View,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_view_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_views_dir(&self) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_document_view(
                &self,
                _: &str,
                _: &srs_core::types::view::DocumentView,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_document_view_file(
                &self,
                _: &str,
                _: &srs_core::types::view::DocumentView,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_document_view_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_document_views_dir(&self) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn load_instance_json(&self, _: &str) -> Result<serde_json::Value, RepositoryError> {
                unimplemented!()
            }
            fn save_instance_json(
                &self,
                _: &str,
                _: &serde_json::Value,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_instance_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_instance_dir(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn list_instance_files(&self, _: &str) -> Result<Vec<String>, RepositoryError> {
                unimplemented!()
            }
            fn load_relations_json(&self, _: &str) -> Result<serde_json::Value, RepositoryError> {
                unimplemented!()
            }
            fn save_relations_json(
                &self,
                _: &str,
                _: &serde_json::Value,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_relations_dir(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn load_container(
                &self,
                _: &str,
            ) -> Result<srs_core::types::container::Container, RepositoryError> {
                unimplemented!()
            }
            fn save_container(
                &self,
                _: &srs_core::types::container::Container,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_container(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
                unimplemented!()
            }
            #[allow(deprecated)]
            fn load_container_json(&self, _: &str) -> Result<serde_json::Value, RepositoryError> {
                unimplemented!()
            }
            #[allow(deprecated)]
            fn save_container_json(
                &self,
                _: &str,
                _: &serde_json::Value,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            #[allow(deprecated)]
            fn delete_container_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            #[allow(deprecated)]
            fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn list_files_recursive(&self, _: &str) -> Vec<String> {
                unimplemented!()
            }
            fn load_text_file(&self, _: &str) -> Result<String, RepositoryError> {
                unimplemented!()
            }
            fn validate_package_ref_path(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn list_package_boundaries(
                &self,
            ) -> Result<Vec<crate::package_types::PackageBoundary>, RepositoryError> {
                unimplemented!()
            }
            fn load_package_boundary(
                &self,
                _: &crate::package_types::PackageSelector,
            ) -> Result<crate::package_types::PackageBoundary, RepositoryError> {
                unimplemented!()
            }
            fn save_package_boundary_metadata(
                &self,
                _: &crate::package_types::PackageBoundary,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn register_package_boundary(
                &self,
                _: &crate::package_types::PackageSelector,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn add_definition_to_boundary(
                &self,
                _: &crate::package_types::PackageSelector,
                _: crate::package_types::DefinitionKind,
                _: &str,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn remove_definition_from_boundary(
                &self,
                _: &crate::package_types::PackageSelector,
                _: crate::package_types::DefinitionKind,
                _: &str,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn resolve_definition_owner(
                &self,
                _: &str,
                _: crate::package_types::DefinitionKind,
            ) -> Result<crate::package_types::PackageSelector, RepositoryError> {
                unimplemented!()
            }
        }

        let result = load_relations(&BrokenManifestStore);
        assert!(
            matches!(result, Err(RepositoryError::ManifestParse { .. })),
            "expected ManifestParse to propagate, got {:?}",
            result
        );
    }
}
