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
use crate::store::{relation_object_path, RepositoryStore};
use crate::writer::new_instance_id;
use srs_core::types::relation::{Relation, RelationsCollection};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use srs_schema::{SchemaRegistry, RELATIONS_COLLECTION_SCHEMA_ID};
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
    pub path: String,
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

/// List all relations (standalone objects + transitional collection entries)
/// with optional filtering
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

/// Build the owned data needed to construct a `RelationValidationContext`.
/// Shared by `create_relation` and `rebuild_precedes_chain` to avoid duplicating
/// the catalog-fetch + instance-set + semantic-type-map construction. One
/// catalog snapshot for both values (RFC-038: no manifest.instanceIndex).
fn load_validation_data(
    store: &dyn RepositoryStore,
) -> Result<(HashSet<String>, HashMap<String, String>), RepositoryError> {
    let cat = store.catalog()?;
    // Populate the type-id map so E4 (requireSameType) fires on the write path
    // exactly as it does in `repo validate` — previously this was an empty map,
    // so E4 was dead on create (#556).
    Ok(crate::writer::known_instances_and_type_ids(store, &cat))
}

/// Create a new relation with E1-E4 validation.
///
/// Writes exactly one standalone relation object at `relations/<relationId>.json`
/// (RFC-038 Change E) — never the transitional collection file.
pub fn create_relation(
    store: &dyn RepositoryStore,
    mut relation: Relation,
    definitions: &[RelationTypeDefinition],
) -> Result<CreateRelationResult, RepositoryError> {
    if relation.relation_id.trim().is_empty() {
        relation.relation_id = new_instance_id();
    }
    let (known_instance_ids, instance_type_ids) = load_validation_data(store)?;
    let ctx = RelationValidationContext {
        definitions,
        known_instance_ids: &known_instance_ids,
        instance_type_ids: &instance_type_ids,
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

    // Duplicate relationId check across every discovered relation object —
    // standalone files and transitional collection entries ([R12]).
    if load_relations_with_locators(store)?
        .iter()
        .any(|lr| lr.relation.relation_id == relation.relation_id)
    {
        return Err(RepositoryError::RelationValidation {
            relation_id: relation.relation_id.clone(),
            message: format!("Relation with id '{}' already exists", relation.relation_id),
        });
    }

    // JSON-schema gate preserved from the collection era: validate the relation
    // shape against the relations-collection schema's item definition. The
    // standalone relation.json mirror schema is registered in srs-schema
    // PR; until then this is the schema-level check available in srs-schema.
    schema_validate_relation(&relation)?;

    // Write exactly one object — the relation's own file.
    store.save_relation(&relation)?;

    Ok(CreateRelationResult { relation })
}

/// Delete a relation by its relation ID.
///
/// A standalone relation object (`relations/<relationId>.json`) is deleted by
/// removing only its own file. Transitional: a relation that still lives in the
/// collection file is removed by rewriting that collection (removed at the
/// RFC-038 Phase-6 flip, when the collection form is retired).
pub fn delete_relation(
    store: &dyn RepositoryStore,
    relation_id: &str,
) -> Result<DeleteRelationResult, RepositoryError> {
    // [R12] fail-fast, consistent with load_relations: a cross-form duplicate
    // (same id as a standalone object AND a collection entry) is a diagnosable
    // error naming every locator — deleting through it by precedence would
    // silently leave the surviving copy re-enumerating.
    load_relations_with_locators(store)?;

    match store.load_relation(relation_id) {
        Ok(_) => {
            store.delete_relation(relation_id)?;
            return Ok(DeleteRelationResult {
                relation_id: relation_id.to_string(),
                path: relation_object_path(relation_id),
            });
        }
        // InvalidRelationId: legacy collection entries may carry non-UUID ids —
        // they cannot exist as standalone objects, so fall through to the
        // collection fallback below (exempt migration surface only, [R11]).
        Err(
            RepositoryError::RelationNotFound { .. } | RepositoryError::InvalidRelationId { .. },
        ) => {}
        Err(e) => return Err(e),
    }

    // Transitional collection fallback.
    let (relative_path, mut collection) = load_relations_collection(store)?;
    let pos = collection
        .relations
        .iter()
        .position(|r| r.relation_id == relation_id)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(&relative_path),
        })?;
    collection.relations.remove(pos);
    write_relations_collection(store, &relative_path, &collection)?;

    Ok(DeleteRelationResult {
        relation_id: relation_id.to_string(),
        path: relative_path,
    })
}

/// A discovered relation and the locator it was read from.
///
/// Transitional dual-read bookkeeping (RFC-038 Phase 2): locators are
/// `relations/<relationId>.json` for standalone objects and
/// `<collection-path>#relations[<i>]` for collection entries.
pub(crate) struct LocatedRelation {
    pub locator: String,
    pub relation: Relation,
}

/// Load every relation in the repository with its locator, enforcing [R12]
/// (duplicate `relationId` is an error naming every locator).
///
/// Since the Phase-6 flip, standalone `relations/<relationId>.json` objects
/// are the only live form ([R11]) — a collection file is an error for every
/// normal reader. The [R21]-exempt migration surface still merges collection
/// entries (their stored order) with standalone objects, because it reads the
/// pre-migration state by definition. Enumeration order carries no meaning;
/// `precedes` is the only ordering semantics.
// phase-3: route discovery and duplicate detection via RepositoryCatalog.
pub(crate) fn load_relations_with_locators(
    store: &dyn RepositoryStore,
) -> Result<Vec<LocatedRelation>, RepositoryError> {
    let (collection_path, collection) = load_relations_collection(store)?;
    let mut out: Vec<LocatedRelation> = collection
        .relations
        .into_iter()
        .enumerate()
        .map(|(i, relation)| LocatedRelation {
            locator: format!("{collection_path}#relations[{i}]"),
            relation,
        })
        .collect();
    for relation in store.list_relations()? {
        out.push(LocatedRelation {
            locator: relation_object_path(&relation.relation_id),
            relation,
        });
    }

    // [R12] duplicate detection across all discovered relation objects.
    let mut by_id: HashMap<&str, Vec<&str>> = HashMap::new();
    for lr in &out {
        by_id
            .entry(lr.relation.relation_id.as_str())
            .or_default()
            .push(lr.locator.as_str());
    }
    if let Some((id, locators)) = by_id.iter().find(|(_, l)| l.len() > 1) {
        return Err(RepositoryError::DuplicateRelationId {
            relation_id: (*id).to_string(),
            locators: locators.iter().map(|s| s.to_string()).collect(),
        });
    }

    Ok(out)
}

/// Load all relations in the repository (dual read: transitional collection
/// entries + standalone relation objects). See [`load_relations_with_locators`].
pub(crate) fn load_relations(
    store: &dyn RepositoryStore,
) -> Result<Vec<Relation>, RepositoryError> {
    Ok(load_relations_with_locators(store)?
        .into_iter()
        .map(|lr| lr.relation)
        .collect())
}

/// Remove every relation matching `pred`, across both storage forms.
///
/// Standalone objects are deleted file-by-file; collection residents are
/// removed via one collection rewrite (transitional). Returns the removed
/// relations. Batch scoping (`begin_batch`/`commit_batch`) is the caller's
/// responsibility so a cascade can group this with its other writes.
pub(crate) fn remove_relations_where(
    store: &dyn RepositoryStore,
    pred: impl Fn(&Relation) -> bool,
) -> Result<Vec<Relation>, RepositoryError> {
    let mut removed = Vec::new();

    let (collection_path, mut collection) = load_relations_collection(store)?;
    let before = collection.relations.len();
    collection.relations.retain(|r| {
        if pred(r) {
            removed.push(r.clone());
            false
        } else {
            true
        }
    });
    if collection.relations.len() != before {
        write_relations_collection(store, &collection_path, &collection)?;
    }

    for relation in store.list_relations()? {
        if pred(&relation) {
            store.delete_relation(&relation.relation_id)?;
            removed.push(relation);
        }
    }

    Ok(removed)
}

/// RFC-038 Change F / [R22] scoped cascade: remove every Relation incident to
/// `instance_id` (as source or target). Called from instance/container delete
/// paths so a delete never leaves dangling relation endpoints behind. The
/// incident relation files are declared targets of the same delete under
/// [R22]'s explicit cascade exception. Returns the removed relation ids.
///
/// Batch scoping is the caller's responsibility (group with the instance
/// delete under `begin_batch`/`commit_batch`).
pub(crate) fn delete_relations_incident_to(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let removed = remove_relations_where(store, |r| {
        r.source_instance_id == instance_id || r.target_instance_id == instance_id
    })?;
    Ok(removed.into_iter().map(|r| r.relation_id).collect())
}

/// Validate one relation's JSON shape against the relations-collection schema's
/// item contract (wrapped in a synthetic single-entry collection).
fn schema_validate_relation(relation: &Relation) -> Result<(), RepositoryError> {
    let path = relation_object_path(&relation.relation_id);
    let value = serde_json::to_value(relation).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(&path),
        source: e,
    })?;
    let wrapped = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
        "relations": [value]
    });
    SchemaRegistry::global()
        .validate_by_id(RELATIONS_COLLECTION_SCHEMA_ID, &wrapped)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from(&path),
            message: e.to_string(),
        })
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
/// `FileStore`, `MemoryStore`, and `FileStore` — the `.srsj`/WASM store behind srs-web.
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

/// Load the transitional relations collection, returning (relative_path, collection).
///
/// Layers typed parsing over [`resolve_relations_source`] (the single resolver), so the
/// candidate order and store-read method are shared. Returns an empty collection at the
/// default write path if no file is found.
///
/// Post-flip: readable only through the [R21]-exempt migration surface ([R11]);
/// read-and-shrink only — new relations are always standalone objects.
fn load_relations_collection(
    store: &dyn RepositoryStore,
) -> Result<(String, RelationsCollection), RepositoryError> {
    match resolve_relations_source(store)? {
        Some((relative_path, value)) => {
            // [R11], since the Phase-6 flip: only the [R21]-exempt migration
            // surface may still read a collection file — a normal reader
            // errors, matching the catalog's SRS038-R11-COLLECTION-RETIRED.
            if !store.rfc038_exempt() {
                return Err(RepositoryError::InvalidSnapshotData {
                    message: format!(
                        "{relative_path}: relations-collection files are retired at \
                         dataModelRevision >= 2 ([R11]) — run the rfc038-storage migration"
                    ),
                });
            }
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
            // no relationsPath is set). The vec is never empty (the default path is
            // an unconditional entry), so .next().expect() is safe.
            let write_path = relations_candidate_paths(store)?
                .into_iter()
                .next()
                .expect("relations_candidate_paths always returns at least one path");
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildPrecedesChainInput {
    /// Desired linear order — edges created as instance_ids[0]→[1]→…→[n-1].
    pub instance_ids: Vec<String>,
    /// IDs whose existing `precedes` edges (source OR target) are deleted first.
    pub clear_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildPrecedesChainResult {
    pub created: Vec<RelationSummary>,
}

/// Rebuild a linear `precedes` chain as one batch operation.
///
/// Deletes all `precedes` edges where source OR target is in `clear_ids`
/// (standalone objects file-by-file, transitional collection residents via one
/// collection rewrite), then creates `n-1` new standalone relation objects
/// connecting `instance_ids[0]→[1]→…→[n-1]`. Validation (E1–E4) runs before any
/// write; the writes are grouped under the store's batch seam (a no-op on
/// FileStore until #813 — see RFC-038 Disagreement 8).
pub fn rebuild_precedes_chain(
    store: &dyn RepositoryStore,
    input: RebuildPrecedesChainInput,
) -> Result<RebuildPrecedesChainResult, RepositoryError> {
    let clear_ids_set: HashSet<String> = input.clear_ids.into_iter().collect();

    // Validate every new edge before touching anything.
    let new_relations: Vec<Relation> = if input.instance_ids.len() < 2 {
        Vec::new()
    } else {
        let package = store.load_package()?;
        let (known_instance_ids, instance_type_ids) = load_validation_data(store)?;
        let ctx = RelationValidationContext {
            definitions: &package.relation_type_definitions,
            known_instance_ids: &known_instance_ids,
            instance_type_ids: &instance_type_ids,
        };
        let mut rels = Vec::with_capacity(input.instance_ids.len() - 1);
        for window in input.instance_ids.windows(2) {
            let relation = Relation {
                relation_id: new_instance_id(),
                relation_type: "precedes".to_string(),
                source_instance_id: window[0].clone(),
                target_instance_id: window[1].clone(),
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
            };
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
            schema_validate_relation(&relation)?;
            rels.push(relation);
        }
        rels
    };

    store.begin_batch();
    let write_result = (|| {
        remove_relations_where(store, |r| {
            r.relation_type == "precedes"
                && (clear_ids_set.contains(&r.source_instance_id)
                    || clear_ids_set.contains(&r.target_instance_id))
        })?;
        for rel in &new_relations {
            store.save_relation(rel)?;
        }
        Ok(())
    })();
    match write_result {
        Ok(()) => store.commit_batch()?,
        Err(e) => {
            store.abort_batch();
            return Err(e);
        }
    }

    let created = new_relations
        .iter()
        .map(|r| RelationSummary {
            relation_id: r.relation_id.clone(),
            relation_type: r.relation_type.clone(),
            source_id: r.source_instance_id.clone(),
            target_id: r.target_instance_id.clone(),
        })
        .collect();
    Ok(RebuildPrecedesChainResult { created })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;

    fn make_store_with_relations() -> MemoryStore {
        let store = MemoryStore::default();
        // RFC-038 [R1]: membership comes from the tree — write real, minimal
        // Note files rather than a manifest.instanceIndex entry, so the
        // catalog's instance set (and [R13] reference resolution) sees them.
        for id in ["note-1", "note-2", "note-3", "note-4"] {
            store
                .save_instance_json(
                    &format!("records/notes/{id}.json"),
                    &json!({"instanceId": id, "sections": []}),
                )
                .unwrap();
        }

        let relations = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [
                {
                    "relationId": "aaaaaaaa-0000-4000-8000-000000000001",
                    "relationType": "contains",
                    "sourceInstanceId": "note-1",
                    "targetInstanceId": "note-2",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                {
                    "relationId": "aaaaaaaa-0000-4000-8000-000000000002",
                    "relationType": "references",
                    "sourceInstanceId": "note-2",
                    "targetInstanceId": "note-3",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                {
                    "relationId": "aaaaaaaa-0000-4000-8000-000000000003",
                    "relationType": "contains",
                    "sourceInstanceId": "note-1",
                    "targetInstanceId": "note-4",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            ]
        });
        crate::store::write_relations_standalone_for_test(&store, &relations);
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
        assert_eq!(
            result[0].relation_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
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
        let result = get_relation_by_id(&store, "aaaaaaaa-0000-4000-8000-000000000002").unwrap();
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
        let new_relation = make_relation(
            "d0000004-0000-4000-a000-000000000004",
            "note-3",
            "note-4",
            "contains",
        );
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
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        }];
        let result = create_relation(&store, new_relation, &definitions).unwrap();
        assert_eq!(
            result.relation.relation_id,
            "d0000004-0000-4000-a000-000000000004"
        );

        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 4);
    }

    /// A store with two Tier-2 Records bound to the given Type ids, for E4-on-create
    /// tests (#556, re-keyed onto the Type system by srs-rust#910's semanticObjectType
    /// collapse — srs#372/#383/#524). Written through the typed `save_record` seam
    /// (ADR-042) rather than raw JSON, so the fixtures are real catalog-valid Records.
    fn make_store_with_typed_instances(src_type_id: &str, tgt_type_id: &str) -> MemoryStore {
        use srs_core::types::record::{FieldValues, Record};
        let store = MemoryStore::default();
        store
            .save_record(&Record {
                field_meta: None,
                instance_id: "00000000-0000-4000-8000-00000000005c".to_string(),
                type_id: src_type_id.to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "src-type".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                extra: std::collections::BTreeMap::new(),
            })
            .unwrap();
        store
            .save_record(&Record {
                field_meta: None,
                instance_id: "00000000-0000-4000-8000-00000000007c".to_string(),
                type_id: tgt_type_id.to_string(),
                type_version: 1,
                type_namespace: "com.test".to_string(),
                type_name: "tgt-type".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: None,
                tags: None,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                updated_at: None,
                extra: std::collections::BTreeMap::new(),
            })
            .unwrap();
        store
    }

    /// A relation type definition keyed `com.test/links` with the given `requireSameType`.
    fn links_def(require_same: Option<bool>) -> RelationTypeDefinition {
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
            require_same_type: require_same,
            status: None,
            updated_at: None,
            meta: None,
        }
    }

    #[test]
    fn create_relation_enforces_e4_require_same_type() {
        // Regression for #556 (re-keyed by srs-rust#910): E4 was dead on create because
        // the type-id map was empty, so a mismatched-type relation was accepted.
        let store = make_store_with_typed_instances("type-one", "type-two");
        let def = links_def(Some(true));
        let rel = make_relation(
            "d0000006-0000-4000-a000-000000000006",
            "00000000-0000-4000-8000-00000000005c",
            "00000000-0000-4000-8000-00000000007c",
            "com.test/links",
        );
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
    fn create_relation_allows_same_type_under_require_same_type() {
        let store = make_store_with_typed_instances("type-one", "type-one");
        let def = links_def(Some(true));
        let rel = make_relation(
            "d0000007-0000-4000-a000-000000000007",
            "00000000-0000-4000-8000-00000000005c",
            "00000000-0000-4000-8000-00000000007c",
            "com.test/links",
        );
        let result = create_relation(&store, rel, &[def]);
        assert!(
            result.is_ok(),
            "matching types must not trip E4: {result:?}"
        );
    }

    #[test]
    fn create_relation_allows_untyped_endpoints_under_constrained_type() {
        // Endpoints carry NO typeId (Tier-0 Notes), so E4 must stay a no-op even under a
        // constrained type — proves the fix doesn't over-reject (the `if let Some` guard holds).
        let store = MemoryStore::default();
        store
            .save_instance_json(
                "records/notes/src.json",
                &json!({ "instanceId": "00000005-5c00-4000-8000-000000000005", "sections": [] }),
            )
            .unwrap();
        store
            .save_instance_json(
                "records/notes/tgt.json",
                &json!({ "instanceId": "00000005-7960-4000-8000-000000000005", "sections": [] }),
            )
            .unwrap();

        let def = links_def(Some(true));
        let rel = make_relation(
            "d0000005-0000-4000-a000-000000000005",
            "00000005-5c00-4000-8000-000000000005",
            "00000005-7960-4000-8000-000000000005",
            "com.test/links",
        );
        let result = create_relation(&store, rel, &[def]);
        assert!(
            result.is_ok(),
            "untyped endpoints must not trip E4: {result:?}"
        );
    }

    #[test]
    fn load_relations_denies_declared_relations_path_collection() {
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

        // Post-flip: a declared relationsPath collection is denied for a
        // normal reader ([R11]); only the exempt migration surface reads it.
        let err = load_relations(&store).unwrap_err();
        assert!(
            err.to_string().contains("[R11]"),
            "declared-path collection denied: {err}"
        );
    }

    #[test]
    fn create_relation_writes_standalone_object_never_the_collection() {
        // RFC-038 Change E: create writes exactly one standalone relation object,
        // never the transitional collection — even when a manifest relationsPath
        // is declared (supersedes the #560 collection-era behavior).
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("relationsPath".to_string(), json!("relations/custom.json"));
        store.save_manifest(&manifest).unwrap();
        // RFC-038 [R1]/[R13]: relation endpoints must resolve to real discovered
        // instances — write minimal Note files rather than a manifest index entry.
        for id in ["src-1", "tgt-1"] {
            store
                .save_instance_json(
                    &format!("records/notes/{id}.json"),
                    &json!({"instanceId": id, "sections": []}),
                )
                .unwrap();
        }

        let def = links_def(None);
        let rel = make_relation(
            "da000001-0000-4000-a000-000000000001",
            "src-1",
            "tgt-1",
            "com.test/links",
        );
        create_relation(&store, rel, &[def]).unwrap();

        let raw = store
            .load_relations_json("relations/da000001-0000-4000-a000-000000000001.json")
            .expect("relation must be written as a standalone object");
        assert_eq!(
            raw.get("$schema").and_then(|v| v.as_str()),
            Some(crate::store::RELATION_OBJECT_SCHEMA_URL),
            "standalone object must carry the pinned $schema"
        );
        assert!(
            store.load_relations_json("relations/custom.json").is_err(),
            "create must not write the manifest-declared collection"
        );
        assert!(
            store
                .load_relations_json("relations/relations-collection.json")
                .is_err(),
            "create must not write the default collection"
        );
    }

    #[test]
    fn create_relation_writes_only_its_own_file() {
        // Pre-existing standalone relation files are left byte-for-byte
        // untouched by create ([R11]: one object per file, no shared writes).
        let store = make_store_with_relations();
        let sibling = "relations/aaaaaaaa-0000-4000-8000-000000000001.json";
        let before = store.load_relations_json(sibling).unwrap();

        let def = links_def(None);
        let rel = make_relation(
            "da000002-0000-4000-a000-000000000002",
            "note-1",
            "note-3",
            "com.test/links",
        );
        create_relation(&store, rel, &[def]).unwrap();

        let after = store.load_relations_json(sibling).unwrap();
        assert_eq!(before, after, "sibling relation file untouched by create");
        assert!(store
            .load_relations_json("relations/da000002-0000-4000-a000-000000000002.json")
            .is_ok());

        // Dual read enumerates both forms.
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 4);
        assert!(all
            .iter()
            .any(|r| r.relation_id == "da000002-0000-4000-a000-000000000002"));
    }

    #[test]
    fn delete_relation_standalone_touches_only_its_file() {
        let store = make_store_with_relations();
        let def = links_def(None);
        create_relation(
            &store,
            make_relation(
                "da000002-0000-4000-a000-000000000002",
                "note-1",
                "note-3",
                "com.test/links",
            ),
            &[def],
        )
        .unwrap();
        let sibling = "relations/aaaaaaaa-0000-4000-8000-000000000001.json";
        let before = store.load_relations_json(sibling).unwrap();

        let result = delete_relation(&store, "da000002-0000-4000-a000-000000000002").unwrap();
        assert_eq!(
            result.path,
            "relations/da000002-0000-4000-a000-000000000002.json"
        );
        let after = store.load_relations_json(sibling).unwrap();
        assert_eq!(
            before, after,
            "sibling relation file untouched by standalone delete"
        );
        assert!(store
            .load_relations_json("relations/da000002-0000-4000-a000-000000000002.json")
            .is_err());
    }

    #[test]
    fn duplicate_relation_id_names_all_locators() {
        // [R12] across forms survives only on the [R21]-exempt migration
        // surface — a normal reader is denied at the collection itself ([R11]).
        let dup_id = "dd00d0d0-0000-4000-a000-00000000dddd";
        let doc = serde_json::json!({
            "srsj": "2",
            "manifest": { "dataModelRevision": 2 },
            "data": {
                "relations/relations-collection.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                    "relations": [{
                        "relationId": dup_id,
                        "relationType": "contains",
                        "sourceInstanceId": "note-1",
                        "targetInstanceId": "note-2",
                        "createdAt": "2026-01-01T00:00:00Z"
                    }]
                }
            }
        });
        let exempt = crate::srsj::open_srsj(&doc.to_string())
            .unwrap()
            .with_rfc038_exemption();
        exempt
            .save_relation(&make_relation(dup_id, "note-1", "note-2", "contains"))
            .unwrap();

        // Exempt surface: the duplicate is detected and names both locators.
        let err = load_relations(&exempt).unwrap_err();
        match &err {
            RepositoryError::DuplicateRelationId {
                relation_id,
                locators,
            } => {
                assert_eq!(relation_id, dup_id);
                assert_eq!(locators.len(), 2);
                assert!(locators
                    .iter()
                    .any(|l| l == "relations/relations-collection.json#relations[0]"));
                assert!(locators
                    .iter()
                    .any(|l| l == &format!("relations/{dup_id}.json")));
            }
            other => panic!("expected DuplicateRelationId, got {other:?}"),
        }

        // Normal reader: the collection itself is the error ([R11]).
        let normal =
            crate::srsj::open_srsj(&crate::srsj::to_srsj_string(&exempt).unwrap()).unwrap();
        let err = load_relations(&normal).unwrap_err();
        assert!(
            err.to_string().contains("[R11]"),
            "normal reader is denied at the collection: {err}"
        );
    }
    #[test]
    fn create_relation_refuses_duplicate_id() {
        let store = make_store_with_relations();
        let def = links_def(None);
        create_relation(
            &store,
            make_relation(
                "da000003-0000-4000-a000-000000000003",
                "note-1",
                "note-2",
                "com.test/links",
            ),
            std::slice::from_ref(&def),
        )
        .unwrap();
        let err = create_relation(
            &store,
            make_relation(
                "da000003-0000-4000-a000-000000000003",
                "note-2",
                "note-3",
                "com.test/links",
            ),
            &[def],
        )
        .unwrap_err();
        assert!(
            matches!(&err, RepositoryError::RelationValidation { relation_id, message }
                if relation_id == "da000003-0000-4000-a000-000000000003" && message.contains("already exists")),
            "expected duplicate-id refusal, got {err:?}"
        );
    }

    #[test]
    fn two_branch_independent_relation_merge_is_clean() {
        // RFC-038 acceptance test 2 (two-store simulation): two branches from a
        // common base each create one relation; a git merge unions the two
        // standalone files with no textual conflict. Simulated by replaying each
        // branch's file into the base store and asserting both enumerate cleanly.
        let def = links_def(None);

        let branch_a = make_store_with_relations();
        create_relation(
            &branch_a,
            make_relation(
                "da00000a-0000-4000-a000-00000000000a",
                "note-1",
                "note-3",
                "com.test/links",
            ),
            std::slice::from_ref(&def),
        )
        .unwrap();

        let branch_b = make_store_with_relations();
        create_relation(
            &branch_b,
            make_relation(
                "da00000b-0000-4000-a000-00000000000b",
                "note-2",
                "note-4",
                "com.test/links",
            ),
            &[def],
        )
        .unwrap();

        // "Merge": union of the two branches' relation files on top of the base.
        let merged = make_store_with_relations();
        for (branch, path) in [
            (
                &branch_a,
                "relations/da00000a-0000-4000-a000-00000000000a.json",
            ),
            (
                &branch_b,
                "relations/da00000b-0000-4000-a000-00000000000b.json",
            ),
        ] {
            let raw = branch.load_relations_json(path).unwrap();
            merged.save_relations_json(path, &raw).unwrap();
        }

        let all = list_relations(&merged, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 5, "3 base + 1 from each branch");
        assert!(all
            .iter()
            .any(|r| r.relation_id == "da00000a-0000-4000-a000-00000000000a"));
        assert!(all
            .iter()
            .any(|r| r.relation_id == "da00000b-0000-4000-a000-00000000000b"));
        // No duplicate-id error, and the base relation files are untouched.
        let base = make_store_with_relations();
        for id in [
            "aaaaaaaa-0000-4000-8000-000000000001",
            "aaaaaaaa-0000-4000-8000-000000000002",
            "aaaaaaaa-0000-4000-8000-000000000003",
        ] {
            let path = format!("relations/{id}.json");
            assert_eq!(
                base.load_relations_json(&path).unwrap(),
                merged.load_relations_json(&path).unwrap(),
                "merge touched no shared file"
            );
        }
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
        let result = delete_relation(&store, "aaaaaaaa-0000-4000-8000-000000000002").unwrap();
        assert_eq!(result.relation_id, "aaaaaaaa-0000-4000-8000-000000000002");
        assert_eq!(
            result.path,
            "relations/aaaaaaaa-0000-4000-8000-000000000002.json"
        );

        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert!(!all
            .iter()
            .any(|r| r.relation_id == "aaaaaaaa-0000-4000-8000-000000000002"));
    }

    #[test]
    fn relation_delete_denies_declared_relations_path_collection() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("relationsPath".to_string(), json!("relations/custom.json"));
        store.save_manifest(&manifest).unwrap();

        let relations = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [{
                "relationId": "r-custom",
                "relationType": "precedes",
                "sourceInstanceId": "a",
                "targetInstanceId": "b",
                "createdAt": "2026-01-01T00:00:00Z"
            }]
        });
        store
            .save_relations_json("relations/custom.json", &relations)
            .unwrap();

        // Post-flip the declared-path collection is denied ([R11]) — the
        // delete fails on the collection rather than rewriting the retired
        // form.
        let err = delete_relation(&store, "r-custom").unwrap_err();
        assert!(
            err.to_string().contains("[R11]"),
            "collection delete path denied: {err}"
        );
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
            fn save_composition(
                &self,
                _: &str,
                _: &srs_core::types::view::Composition,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn update_composition_file(
                &self,
                _: &str,
                _: &srs_core::types::view::Composition,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn delete_composition_file(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn ensure_compositions_dir(&self, _: &str) -> Result<(), RepositoryError> {
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
            fn save_record(
                &self,
                _: &srs_core::types::record::Record,
            ) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn save_note(&self, _: &srs_core::types::note::Note) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn load_record_by_id(
                &self,
                _: &str,
            ) -> Result<srs_core::types::record::Record, RepositoryError> {
                unimplemented!()
            }
            fn load_note_by_id(
                &self,
                _: &str,
            ) -> Result<srs_core::types::note::Note, RepositoryError> {
                unimplemented!()
            }
            fn delete_instance(&self, _: &str) -> Result<(), RepositoryError> {
                unimplemented!()
            }
            fn find_instance(
                &self,
                _: &str,
            ) -> Result<Option<crate::index::InstanceRef>, RepositoryError> {
                unimplemented!()
            }
            fn list_instances(
                &self,
                _: &crate::index::InstanceQuery,
            ) -> Result<Vec<crate::index::InstanceRef>, RepositoryError> {
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
            fn delete_relations_json(&self, _: &str) -> Result<(), RepositoryError> {
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
            fn load_binary_file(&self, _: &str) -> Result<Vec<u8>, RepositoryError> {
                unimplemented!()
            }
            fn save_binary_file(&self, _: &str, _: &[u8]) -> Result<(), RepositoryError> {
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
                    "relationId": "eeeeeeee-0000-4000-8000-000000000051",
                    "relationType": "precedes",
                    "sourceInstanceId": "sec-a",
                    "targetInstanceId": "sec-b",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                {
                    "relationId": "eeeeeeee-0000-4000-8000-000000000052",
                    "relationType": "precedes",
                    "sourceInstanceId": "sec-b",
                    "targetInstanceId": "sec-c",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            ]
        });
        crate::store::write_relations_standalone_for_test(&store, &relations);
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

    fn make_store_for_rebuild_chain(ids: &[&str]) -> MemoryStore {
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };
        let store = MemoryStore::default();
        // RFC-038 [R1]/[R13]: relation endpoints must resolve to real discovered
        // instances — write minimal Note files rather than a manifest index entry.
        for id in ids {
            store
                .save_instance_json(
                    &format!("records/notes/{id}.json"),
                    &json!({"instanceId": id, "sections": []}),
                )
                .unwrap();
        }
        store
            .save_relation_type_definition(
                "package/relation-types/precedes.json",
                &RelationTypeDefinition {
                    schema: None,
                    id: "00000000-0000-0000-0000-000000000001".to_string(),
                    version: 1,
                    key: "precedes".to_string(),
                    namespace: "com.semanticops.srs".to_string(),
                    label: "Precedes".to_string(),
                    description: "Source precedes target".to_string(),
                    category: RelationTypeCategory::Association,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    canonical_direction: None,
                    inverse_type: None,
                    irreflexive: None,
                    require_same_type: None,
                    status: None,
                    updated_at: None,
                    meta: None,
                },
            )
            .unwrap();
        store
    }

    #[test]
    fn test_rebuild_precedes_chain_creates_n_minus_1_edges() {
        let store = make_store_for_rebuild_chain(&["id-a", "id-b", "id-c"]);
        let result = rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec!["id-a".into(), "id-b".into(), "id-c".into()],
                clear_ids: vec![],
            },
        )
        .unwrap();
        assert_eq!(result.created.len(), 2, "3 ids → 2 edges");
        assert_eq!(result.created[0].source_id, "id-a");
        assert_eq!(result.created[0].target_id, "id-b");
        assert_eq!(result.created[1].source_id, "id-b");
        assert_eq!(result.created[1].target_id, "id-c");
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(
            all.iter().filter(|r| r.relation_type == "precedes").count(),
            2
        );
    }

    #[test]
    fn test_rebuild_precedes_chain_clears_existing_precedes() {
        let store = make_store_for_rebuild_chain(&["id-a", "id-b", "id-c"]);
        // Pre-populate with old precedes edges (b→c and c→a — wrong order)
        crate::store::write_relations_standalone_for_test(
            &store,
            &json!({
                "relations": [
                    {
                        "relationId": "eeeeeeee-0000-4000-8000-00000000ff01",
                        "relationType": "precedes",
                        "sourceInstanceId": "id-b",
                        "targetInstanceId": "id-c",
                        "createdAt": "2026-01-01T00:00:00Z"
                    },
                    {
                        "relationId": "eeeeeeee-0000-4000-8000-00000000ff02",
                        "relationType": "precedes",
                        "sourceInstanceId": "id-c",
                        "targetInstanceId": "id-a",
                        "createdAt": "2026-01-01T00:00:00Z"
                    }
                ]
            }),
        );
        let result = rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec!["id-a".into(), "id-b".into(), "id-c".into()],
                clear_ids: vec!["id-a".into(), "id-b".into(), "id-c".into()],
            },
        )
        .unwrap();
        assert_eq!(result.created.len(), 2);
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        let precedes: Vec<_> = all
            .iter()
            .filter(|r| r.relation_type == "precedes")
            .collect();
        assert_eq!(precedes.len(), 2, "old edges replaced by new edges");
        assert!(
            !precedes
                .iter()
                .any(|r| r.relation_id == "eeeeeeee-0000-4000-8000-00000000ff01"),
            "old-1 must be removed"
        );
        assert!(
            !precedes
                .iter()
                .any(|r| r.relation_id == "eeeeeeee-0000-4000-8000-00000000ff02"),
            "old-2 must be removed"
        );
        assert_eq!(result.created[0].source_id, "id-a");
        assert_eq!(result.created[0].target_id, "id-b");
        assert_eq!(result.created[1].source_id, "id-b");
        assert_eq!(result.created[1].target_id, "id-c");
    }

    #[test]
    fn test_rebuild_precedes_chain_empty_instance_ids() {
        let store = make_store_for_rebuild_chain(&["id-a"]);
        let result = rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec![],
                clear_ids: vec![],
            },
        )
        .unwrap();
        assert!(result.created.is_empty(), "empty input → no edges created");
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_rebuild_precedes_chain_single_instance_id() {
        let store = make_store_for_rebuild_chain(&["id-a"]);
        let result = rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec!["id-a".into()],
                clear_ids: vec![],
            },
        )
        .unwrap();
        assert!(result.created.is_empty(), "single id → no edges created");
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_rebuild_precedes_chain_does_not_clear_non_precedes() {
        let store = make_store_for_rebuild_chain(&["id-a", "id-b"]);
        // Pre-populate with a non-precedes edge involving id-a (should survive)
        // and a precedes edge (should be cleared)
        crate::store::write_relations_standalone_for_test(
            &store,
            &json!({
                "relations": [
                    {
                        "relationId": "eeeeeeee-0000-4000-8000-00000000fe01",
                        "relationType": "contains",
                        "sourceInstanceId": "id-a",
                        "targetInstanceId": "id-b",
                        "createdAt": "2026-01-01T00:00:00Z"
                    },
                    {
                        "relationId": "eeeeeeee-0000-4000-8000-00000000fe02",
                        "relationType": "precedes",
                        "sourceInstanceId": "id-a",
                        "targetInstanceId": "id-b",
                        "createdAt": "2026-01-01T00:00:00Z"
                    }
                ]
            }),
        );
        let result = rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec!["id-a".into(), "id-b".into()],
                clear_ids: vec!["id-a".into(), "id-b".into()],
            },
        )
        .unwrap();
        assert_eq!(result.created.len(), 1);
        let all = list_relations(&store, ListRelationsFilter::default()).unwrap();
        assert_eq!(all.len(), 2, "non-precedes edge must survive");
        assert!(
            all.iter()
                .any(|r| r.relation_id == "eeeeeeee-0000-4000-8000-00000000fe01"),
            "contains edge must be preserved"
        );
        assert!(
            !all.iter()
                .any(|r| r.relation_id == "eeeeeeee-0000-4000-8000-00000000fe02"),
            "old precedes edge must be removed"
        );
    }

    #[test]
    fn test_rebuild_precedes_chain_roundtrip_json_store() {
        let srsj = serde_json::json!({
            "srsj": "2",
            "manifest": { "dataModelRevision": 2 },
            "data": {
                "package/package.json": {
                    "id": "00000000-0000-0000-0000-000000000099",
                    "namespace": "com.test",
                    "name": "test-package",
                    "version": "1",
                    "relationTypes": ["relation-types/precedes.json"]
                },
                "package/relation-types/precedes.json": {
                    "id": "00000000-0000-0000-0000-000000000001",
                    "version": 1,
                    "namespace": "com.semanticops.srs",
                    "key": "precedes",
                    "label": "Precedes",
                    "description": "Source precedes target",
                    "category": "association",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                // RFC-038: catalog-backed E2 endpoint resolution needs a real
                // (schema-valid) instance body at each declared path — the old
                // manifest.instance_index-only fixture no longer suffices.
                "records/id-a.json": {"instanceId": "id-a", "sections": []},
                "records/id-b.json": {"instanceId": "id-b", "sections": []},
                "records/id-c.json": {"instanceId": "id-c", "sections": []}
            }
        })
        .to_string();
        let store = crate::srsj::open_srsj(&srsj).unwrap();
        rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec!["id-a".into(), "id-b".into(), "id-c".into()],
                clear_ids: vec![],
            },
        )
        .unwrap();
        let exported = crate::srsj::to_srsj_string(&store).unwrap();
        let store2 = crate::srsj::open_srsj(&exported).unwrap();
        let all = list_relations(&store2, ListRelationsFilter::default()).unwrap();
        let precedes: Vec<_> = all
            .iter()
            .filter(|r| r.relation_type == "precedes")
            .collect();
        assert_eq!(
            precedes.len(),
            2,
            "exactly 2 precedes edges after roundtrip"
        );
        // Enumeration order carries no meaning (RFC-038); assert the edge set.
        assert!(precedes
            .iter()
            .any(|r| r.source_id == "id-a" && r.target_id == "id-b"));
        assert!(precedes
            .iter()
            .any(|r| r.source_id == "id-b" && r.target_id == "id-c"));
    }
}
