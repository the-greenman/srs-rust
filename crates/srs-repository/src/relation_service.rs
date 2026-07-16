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
use crate::record_store;
use crate::relation_graph;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::relation::{Relation, RelationsCollection};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use srs_schema::{SchemaRegistry, RELATIONS_COLLECTION_SCHEMA_ID};
use std::collections::HashSet;

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
    mut relation: Relation,
    definitions: &[RelationTypeDefinition],
) -> Result<CreateRelationResult, RepositoryError> {
    if relation.relation_id.trim().is_empty() {
        relation.relation_id = new_instance_id();
    }
    // Build owned context data
    let manifest = store.load_manifest()?;
    let known_instance_ids: HashSet<String> = manifest
        .instance_index
        .iter()
        .map(|e| e.instance_id().to_string())
        .collect();
    // Populate the semantic-type map so E4 (allowedSourceTypes / allowedTargetTypes /
    // requireSameSemanticObjectType) fires on the write path exactly as it does in
    // `repo validate` — previously this was an empty map, so E4 was dead on create (#556).
    let instance_semantic_types = crate::writer::build_instance_semantic_types(store, &manifest);
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

    // Schema validation of the updated collection before writing
    let collection_raw =
        serde_json::to_value(&collection).map_err(|e| RepositoryError::Serialize {
            path: std::path::PathBuf::from(&relative_path),
            source: e,
        })?;
    SchemaRegistry::global()
        .validate_by_id(RELATIONS_COLLECTION_SCHEMA_ID, &collection_raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from(&relative_path),
            message: e.to_string(),
        })?;

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

/// Ordered, de-duplicated list of relative paths to try when locating the
/// repository's relations file:
/// 1. `relationsPath` declared in `manifest.json`
/// 2. `relations/relations-collection.json` (default write path)
/// 3. `relations/relations.json` (alternate convention)
///
/// This is the single source of truth for the relations-file resolution order.
/// `load_relations_collection`, `resolve_relations_source`, and
/// `analysis::summarize_relations` all consume it, so the write path and every
/// read path (including `repo validate`) agree on which file is authoritative (#548).
///
/// A missing/unreadable manifest yields no `relationsPath` (the two defaults are
/// still returned); any other manifest error propagates.
pub(crate) fn relations_candidate_paths(
    store: &dyn RepositoryStore,
) -> Result<Vec<String>, RepositoryError> {
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

    let mut seen = HashSet::new();
    let candidates: Vec<String> = [
        manifest_path,
        Some("relations/relations-collection.json".to_string()),
        Some("relations/relations.json".to_string()),
    ]
    .into_iter()
    .flatten()
    .filter(|p| seen.insert(p.clone()))
    .collect();
    Ok(candidates)
}

/// Resolve the authoritative relations file and return `(relative_path, parsed_json)`,
/// or `None` when no relations file exists.
///
/// Reads via [`RepositoryStore::load_relations_json`] (the same method the write path
/// and `analysis::summarize_relations` use) so resolution works uniformly across
/// `FileStore`, `MemoryStore`, and `JsonStore` — the `.srsj`/WASM store behind srs-web.
/// `load_text_file` only surfaces `FileStore`'s on-disk text, so an object-backed store
/// would never find a relation written by `save_relations_json`, and `repo validate`
/// would silently skip every relation there (#548).
///
/// A missing file is skipped (`Io`/`NotFound`); a present-but-malformed file propagates
/// its error (`Serialize`) so the caller can surface it as a diagnostic rather than crash.
pub(crate) fn resolve_relations_source(
    store: &dyn RepositoryStore,
) -> Result<Option<(String, serde_json::Value)>, RepositoryError> {
    for relative_path in relations_candidate_paths(store)? {
        match store.load_relations_json(&relative_path) {
            Ok(value) => return Ok(Some((relative_path, value))),
            Err(RepositoryError::Io { .. } | RepositoryError::NotFound { .. }) => continue,
            Err(RepositoryError::Serialize { source, .. }) => {
                // A present-but-malformed file. The store reports an absolute path; re-attach
                // the relative candidate path so a caller (e.g. validate) can attribute the
                // diagnostic consistently with every other relative-path diagnostic.
                return Err(RepositoryError::Serialize {
                    path: std::path::PathBuf::from(&relative_path),
                    source,
                });
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Load the relations collection, returning (relative_path, collection).
///
/// Layers typed parsing over [`resolve_relations_source`] (the single resolver), so the
/// candidate order and store-read method are shared. Returns an empty collection at the
/// default write path if no file is found.
fn load_relations_collection(
    store: &dyn RepositoryStore,
) -> Result<(String, RelationsCollection), RepositoryError> {
    match resolve_relations_source(store)? {
        Some((relative_path, value)) => {
            let collection: RelationsCollection =
                serde_json::from_value(value).map_err(|e| RepositoryError::RecordLoad {
                    path: std::path::PathBuf::from(&relative_path),
                    source: e,
                })?;
            Ok((relative_path, collection))
        }
        None => {
            // Use the manifest's declared relationsPath as the write destination when
            // no file exists yet. relations_candidate_paths was already called by
            // resolve_relations_source above, so a second call cannot fail in any new
            // way; its first element is the declared path (or the default fallback if
            // no relationsPath is set).
            let write_path = relations_candidate_paths(store)?
                .into_iter()
                .next()
                .unwrap_or_else(|| "relations/relations-collection.json".to_string());
            Ok((
                write_path,
                RelationsCollection {
                    schema: Some(
                        "https://srs.semanticops.com/schema/2.0/relations-collection.json"
                            .to_string(),
                    ),
                    relations: Vec::new(),
                },
            ))
        }
    }
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

/// Input for ordering a set of instance IDs by their `precedes` relation chain.
#[derive(Debug, Clone)]
pub struct OrderByPrecedesInput {
    pub instance_ids: Vec<String>,
}

/// Result of ordering instance IDs by their `precedes` relation chain.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderByPrecedesResult {
    pub ordered_ids: Vec<String>,
}

/// Order a set of instance IDs by following the `precedes` relation chain.
///
/// Loads all relations from the store, resolves each ID to a record for
/// `created_at` fallback ordering, then delegates to
/// `relation_graph::sort_by_precedes_chain`. IDs that do not resolve to
/// a known record are still included — they sort last by instance ID.
///
/// Returns `OrderByPrecedesResult { ordered_ids }` in chain order.
pub fn order_by_precedes(
    store: &dyn RepositoryStore,
    input: OrderByPrecedesInput,
) -> Result<OrderByPrecedesResult, RepositoryError> {
    if input.instance_ids.len() <= 1 {
        return Ok(OrderByPrecedesResult {
            ordered_ids: input.instance_ids,
        });
    }

    let relations = load_relations(store)?;

    let entries: Vec<PrecedesEntry> = input
        .instance_ids
        .iter()
        .map(|id| {
            let created_at = record_store::get_record_by_id(store, id)
                .ok()
                .flatten()
                .and_then(|r| r.created_at);
            PrecedesEntry {
                instance_id: id.clone(),
                created_at,
            }
        })
        .collect();

    let sorted = relation_graph::sort_by_precedes_chain(entries, &relations);

    Ok(OrderByPrecedesResult {
        ordered_ids: sorted.into_iter().map(|e| e.instance_id).collect(),
    })
}

#[derive(Clone)]
struct PrecedesEntry {
    instance_id: String,
    created_at: Option<String>,
}

impl relation_graph::PrecedesSortable for PrecedesEntry {
    fn precedes_instance_id(&self) -> &str {
        &self.instance_id
    }
    fn precedes_created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
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
            key: "contains".to_string(),
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
            properties: None,
        }];
        let result = create_relation(&store, new_relation, &definitions).unwrap();
        assert_eq!(result.relation.relation_id, "r4");

        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 4);
    }

    /// A store with two Tier-2 instances carrying `semanticObjectType`, for E4-on-create tests (#556).
    fn make_store_with_typed_instances(src_type: &str, tgt_type: &str) -> MemoryStore {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        for id in ["src", "tgt"] {
            manifest
                .instance_index
                .push(crate::index::InstanceIndexEntry {
                    instance_id: id.to_string(),
                    tier: 2,
                    path: format!("records/{}.json", id),
                    title: None,
                    tags: None,
                });
        }
        store.save_manifest(&manifest).unwrap();
        store
            .save_instance_json(
                "records/src.json",
                &json!({ "instanceId": "src", "semanticObjectType": src_type }),
            )
            .unwrap();
        store
            .save_instance_json(
                "records/tgt.json",
                &json!({ "instanceId": "tgt", "semanticObjectType": tgt_type }),
            )
            .unwrap();
        store
    }

    /// A relation type definition keyed `com.test/links` with the given E4 constraints.
    fn links_def(
        allowed_source: Option<Vec<&str>>,
        allowed_target: Option<Vec<&str>>,
        require_same: Option<bool>,
    ) -> RelationTypeDefinition {
        use srs_core::types::relation_type_definition::RelationTypeCategory;
        RelationTypeDefinition {
            schema: None,
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            version: 1,
            key: "com.test/links".to_string(),
            namespace: "com.test".to_string(),
            label: "Links".to_string(),
            description: "source links to target".to_string(),
            category: RelationTypeCategory::Association,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            allowed_source_types: allowed_source
                .map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            allowed_target_types: allowed_target
                .map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            require_same_semantic_object_type: require_same,
            status: None,
            updated_at: None,
            properties: None,
        }
    }

    #[test]
    fn create_relation_enforces_e4_allowed_source_types() {
        // Regression for #556: E4 was dead on create because the semantic-type map was empty,
        // so a source whose semanticObjectType is not in allowedSourceTypes was accepted.
        let store = make_store_with_typed_instances("com.x/forbidden", "com.x/anything");
        let def = links_def(Some(vec!["com.x/allowed"]), None, None);
        let rel = make_relation("r-e4", "src", "tgt", "com.test/links");
        let err = create_relation(&store, rel, &[def]).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("E4"),
            "expected an E4 type-constraint error, got: {msg}"
        );
        // The offending relation must NOT have been persisted.
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert!(all.is_empty(), "relation must not be written on E4 failure");
    }

    #[test]
    fn create_relation_enforces_require_same_semantic_object_type() {
        let store = make_store_with_typed_instances("com.x/one", "com.x/two");
        let def = links_def(None, None, Some(true));
        let rel = make_relation("r-same", "src", "tgt", "com.test/links");
        let err = create_relation(&store, rel, &[def]).unwrap_err();
        assert!(format!("{err:?}").contains("E4"));
    }

    #[test]
    fn create_relation_allows_untyped_endpoints_under_constrained_type() {
        // Endpoints carry NO semanticObjectType, so E4 must stay a no-op even under a
        // constrained type — proves the fix doesn't over-reject (the `if let Some` guard holds).
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        for id in ["src", "tgt"] {
            manifest
                .instance_index
                .push(crate::index::InstanceIndexEntry {
                    instance_id: id.to_string(),
                    tier: 0,
                    path: format!("records/{}.json", id),
                    title: None,
                    tags: None,
                });
        }
        store.save_manifest(&manifest).unwrap();
        store
            .save_instance_json("records/src.json", &json!({ "instanceId": "src" }))
            .unwrap();
        store
            .save_instance_json("records/tgt.json", &json!({ "instanceId": "tgt" }))
            .unwrap();

        let def = links_def(Some(vec!["com.x/allowed"]), None, None);
        let rel = make_relation("r-untyped", "src", "tgt", "com.test/links");
        let result = create_relation(&store, rel, &[def]);
        assert!(
            result.is_ok(),
            "untyped endpoints must not trip E4: {result:?}"
        );
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
    fn create_relation_uses_manifest_relations_path_when_no_file_exists() {
        // Regression for #560: first create_relation must write to the manifest-declared
        // relationsPath, not the hardcoded default, when no relations file exists yet.
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("relationsPath".to_string(), json!("relations/custom.json"));
        for id in ["src-1", "tgt-1"] {
            manifest.instance_index.push(crate::index::InstanceIndexEntry {
                instance_id: id.to_string(),
                tier: 0,
                path: format!("records/{}.json", id),
                title: None,
                tags: None,
            });
        }
        store.save_manifest(&manifest).unwrap();

        let def = links_def(None, None, None);
        let rel = make_relation("r-new", "src-1", "tgt-1", "com.test/links");
        create_relation(&store, rel, &[def]).unwrap();

        assert!(
            store.load_relations_json("relations/custom.json").is_ok(),
            "relation must be written to the manifest-declared relationsPath"
        );
        assert!(
            store
                .load_relations_json("relations/relations-collection.json")
                .is_err(),
            "relation must not be written to the hardcoded default path"
        );
    }

    #[test]
    fn create_relation_no_relations_path_writes_to_default() {
        // Regression for #560: when no relationsPath is declared, first create_relation
        // must still write to the default "relations/relations-collection.json".
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        for id in ["src-1", "tgt-1"] {
            manifest.instance_index.push(crate::index::InstanceIndexEntry {
                instance_id: id.to_string(),
                tier: 0,
                path: format!("records/{}.json", id),
                title: None,
                tags: None,
            });
        }
        store.save_manifest(&manifest).unwrap();

        let def = links_def(None, None, None);
        let rel = make_relation("r-default", "src-1", "tgt-1", "com.test/links");
        create_relation(&store, rel, &[def]).unwrap();

        assert!(
            store
                .load_relations_json("relations/relations-collection.json")
                .is_ok(),
            "relation must be written to the default path when no relationsPath is declared"
        );
        assert!(
            store
                .load_relations_json("relations/relations.json")
                .is_err(),
            "relation must not be written to the legacy alternate path"
        );
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
        use crate::store::{RecordTier, RepositoryStore};

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
            fn ensure_fields_dir(&self, _: &str) -> Result<(), RepositoryError> {
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
            fn ensure_types_dir(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_relation_type_definition(
                &self,
                _: &str,
                _: &srs_core::types::relation_type_definition::RelationTypeDefinition,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_relation_type_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_relation_types_dir(&self, _: &str) -> Result<(), RepositoryError> {
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
            fn ensure_views_dir(&self, _: &str) -> Result<(), RepositoryError> {
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
            fn ensure_document_views_dir(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_blueprint(
                &self,
                _: &str,
                _: &srs_core::types::blueprint::Blueprint,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_blueprint_file(
                &self,
                _: &str,
                _: &srs_core::types::blueprint::Blueprint,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_blueprint_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_blueprints_dir(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_vocabulary(
                &self,
                _: &str,
                _: &srs_core::types::vocabulary::Vocabulary,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_vocabularies_dir(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_lifecycle(
                &self,
                _: &str,
                _: &srs_core::types::lifecycle::Lifecycle,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_lifecycles_dir(&self, _: &str) -> Result<(), RepositoryError> {
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
            fn record_tier_dir(&self, _tier: RecordTier) -> &'static str {
                unreachable!("record_tier_dir not expected in BrokenManifestStore tests")
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
            fn save_text_file(&self, _: &str, _: &str) -> Result<(), RepositoryError> {
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
            fn save_theme(
                &self,
                _: &str,
                _: &srs_core::types::theme::Theme,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_theme_file(
                &self,
                _: &str,
                _: &srs_core::types::theme::Theme,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_theme_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_themes_dir(&self, _: &str) -> Result<(), RepositoryError> {
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

    fn make_store_with_precedes() -> MemoryStore {
        let store = MemoryStore::default();
        let relations = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [
                {
                    "relationId": "rp1",
                    "relationType": "precedes",
                    "sourceInstanceId": "sec-a",
                    "targetInstanceId": "sec-b",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                {
                    "relationId": "rp2",
                    "relationType": "precedes",
                    "sourceInstanceId": "sec-b",
                    "targetInstanceId": "sec-c",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            ]
        });
        store
            .save_relations_json("relations/relations-collection.json", &relations)
            .unwrap();
        store
    }

    #[test]
    fn order_by_precedes_follows_chain() {
        let store = make_store_with_precedes();
        // Input is deliberately out of chain order
        let input = OrderByPrecedesInput {
            instance_ids: vec!["sec-c".into(), "sec-a".into(), "sec-b".into()],
        };
        let result = order_by_precedes(&store, input).unwrap();
        assert_eq!(result.ordered_ids, vec!["sec-a", "sec-b", "sec-c"]);
    }

    #[test]
    fn order_by_precedes_singleton_returns_unchanged() {
        let store = make_store_with_precedes();
        let input = OrderByPrecedesInput {
            instance_ids: vec!["sec-a".into()],
        };
        let result = order_by_precedes(&store, input).unwrap();
        assert_eq!(result.ordered_ids, vec!["sec-a"]);
    }

    #[test]
    fn order_by_precedes_empty_returns_empty() {
        let store = make_store_with_precedes();
        let input = OrderByPrecedesInput {
            instance_ids: vec![],
        };
        let result = order_by_precedes(&store, input).unwrap();
        assert!(result.ordered_ids.is_empty());
    }

    #[test]
    fn order_by_precedes_unknown_ids_included_not_dropped() {
        let store = make_store_with_precedes();
        // "orphan" has no precedes relations but must not be dropped.
        // No records in the store → created_at = None for all. Tiebreak is
        // instance_id ascending: "orphan" < "sec-a" < "sec-b", so orphan is
        // the first head, then sec-a starts the chain.
        let input = OrderByPrecedesInput {
            instance_ids: vec!["sec-b".into(), "sec-a".into(), "orphan".into()],
        };
        let result = order_by_precedes(&store, input).unwrap();
        assert_eq!(result.ordered_ids.len(), 3, "all IDs must be in output");
        // sec-a → sec-b is a chain; orphan is a standalone head
        assert!(result.ordered_ids.contains(&"sec-a".to_string()));
        assert!(result.ordered_ids.contains(&"sec-b".to_string()));
        assert!(result.ordered_ids.contains(&"orphan".to_string()));
        let a = result
            .ordered_ids
            .iter()
            .position(|x| x == "sec-a")
            .unwrap();
        let b = result
            .ordered_ids
            .iter()
            .position(|x| x == "sec-b")
            .unwrap();
        assert!(a < b, "sec-a must precede sec-b in the output");
    }
}
