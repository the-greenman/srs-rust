use serde::{Deserialize, Serialize};

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::{
    container_service, package_service, protocol_run_service, record_store, relation_service,
};
use relation_service::ListRelationsFilter;
use srs_core::arrangement::OutlineEntry;
use srs_core::types::relation_type_definition::RelationTypeCategory;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldContextQuery {
    pub record_id: String,
    pub field_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordContextQuery {
    pub record_id: String,
    /// When set, the record must be a member of this container and the result carries its
    /// arrangement `entry` and `subtree` (#1134).
    #[serde(default)]
    pub container_id: Option<String>,
    /// Drop edges whose relation type's `RelationTypeDefinition.category` is listed (#1188),
    /// e.g. `composition` + `sequence` removes contains/precedes without naming them. Empty =
    /// no filtering (default). Edges whose type has no installed definition are kept.
    #[serde(default)]
    pub exclude_relation_categories: Vec<RelationTypeCategory>,
}

/// Which end of the edge the context record sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeDirection {
    /// The context record is the relation's source.
    Out,
    /// The context record is the relation's target.
    In,
}

impl EdgeDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeDirection::Out => "out",
            EdgeDirection::In => "in",
        }
    }
}

impl std::str::FromStr for EdgeDirection {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "out" => Ok(EdgeDirection::Out),
            "in" => Ok(EdgeDirection::In),
            other => Err(format!("invalid direction '{other}' (expected out|in)")),
        }
    }
}

/// A relation neighbour, loaded by tier (`LoadedInstance` is not `Serialize`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContextInstance {
    Record(srs_core::types::record::Record),
    Note(srs_core::types::note::Note),
}

/// One edge touching the context record: the relation, which way it points, and the
/// instance at the other end inline (`None` when it does not resolve).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRelation {
    pub direction: EdgeDirection,
    #[serde(flatten)]
    pub relation: crate::relation_service::RelationSummary,
    pub neighbour: Option<ContextInstance>,
    /// The relation's own provenance (#1246): when it was asserted and by whom, so an
    /// agent can find the attachments it made. Absent on legacy/unstamped relations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_by: Option<srs_core::types::actor::Actor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldContextResult {
    pub record_id: String,
    pub field_id: String,
    pub field_name: Option<String>,
    pub field_namespace: Option<String>,
    /// None when field not in package, or when field.ai_guidance.purpose is empty
    pub ai_guidance: Option<serde_json::Value>,
    pub current_value: Option<serde_json::Value>,
    /// Always empty; placeholder for tagged-chunk storage (#582)
    pub tagged_chunks: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordContextResult {
    pub record_id: String,
    /// type_id/type_name/type_namespace are String (not Option) — always present on a
    /// found Tier-2 Record
    pub type_id: String,
    pub type_name: String,
    pub type_namespace: String,
    pub display_label: String,
    pub field_values: srs_core::types::record::FieldValues,
    /// Every relation touching the record, both directions, sorted by relationType, then
    /// neighbour `createdAt`, then relationId (a comment thread reads chronologically).
    pub relations: Vec<ContextRelation>,
    /// Set with `RecordContextQuery::container_id`: this record's arrangement entry.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub container_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub entry: Option<OutlineEntry>,
    /// Descendants of `entry` in that container (outline entries only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subtree: Option<Vec<OutlineEntry>>,
    /// Always empty; placeholder for tagged-chunk storage (#582)
    pub tagged_chunks: Vec<serde_json::Value>,
    /// Protocol runs targeting this record
    pub protocol_run_history: Vec<serde_json::Value>,
}

/// Assemble field context: current value and aiGuidance from package.
pub fn get_field_context(
    store: &dyn RepositoryStore,
    query: FieldContextQuery,
) -> Result<FieldContextResult, RepositoryError> {
    let record = record_store::get_record_by_id(store, &query.record_id)?.ok_or_else(|| {
        RepositoryError::NotFound {
            path: std::path::PathBuf::from(&query.record_id),
        }
    })?;

    // Query addresses the field by id; the RFC-039 carrier keys by name —
    // recover the name through the package (Type-mediated resolution).
    let field_name = store
        .load_package()?
        .resolve_field(&query.field_id)
        .map(|f| f.name.clone());
    let current_value = field_name
        .as_deref()
        .and_then(|name| record.value(name))
        .cloned()
        .filter(|v| !v.is_null());

    let (field_name, field_namespace, ai_guidance) =
        match package_service::get_field_by_id(store, &query.field_id)? {
            package_service::GetFieldResult::Found(field) => {
                let guidance = field
                    .ai_guidance
                    .as_ref()
                    .filter(|g| !g.purpose.is_empty())
                    .and_then(|g| serde_json::to_value(g).ok());
                (
                    Some(field.name.clone()),
                    Some(field.namespace.clone()),
                    guidance,
                )
            }
            package_service::GetFieldResult::NotFound => (None, None, None),
        };

    Ok(FieldContextResult {
        record_id: query.record_id,
        field_id: query.field_id,
        field_name,
        field_namespace,
        ai_guidance,
        current_value,
        tagged_chunks: vec![],
    })
}

/// Assemble record context: all field values, every relation touching the record (both
/// directions, neighbour inline) and, given a container, the record's arrangement subtree.
pub fn get_record_context(
    store: &dyn RepositoryStore,
    query: RecordContextQuery,
) -> Result<RecordContextResult, RepositoryError> {
    let summary =
        record_store::get_record_summary_by_id(store, &query.record_id)?.ok_or_else(|| {
            RepositoryError::NotFound {
                path: std::path::PathBuf::from(&query.record_id),
            }
        })?;

    // Only load the package when filtering: the default path stays as it was.
    let package = if query.exclude_relation_categories.is_empty() {
        None
    } else {
        Some(store.load_package()?)
    };
    // ponytail: two full relation scans and a neighbour load per edge; add a per-id cache /
    // single pass if hub records measure slow. A self-relation appears once as out, once as in.
    let mut relations = Vec::new();
    for (direction, filter) in [
        (
            EdgeDirection::Out,
            ListRelationsFilter {
                source: Some(query.record_id.clone()),
                ..Default::default()
            },
        ),
        (
            EdgeDirection::In,
            ListRelationsFilter {
                target: Some(query.record_id.clone()),
                ..Default::default()
            },
        ),
    ] {
        for relation in relation_service::list_relations(store, filter)? {
            if let Some(def) = package
                .as_ref()
                .and_then(|p| p.resolve_relation_type(&relation.relation_type))
            {
                if query.exclude_relation_categories.contains(&def.category) {
                    continue;
                }
            }
            let other = match direction {
                EdgeDirection::Out => &relation.target_id,
                EdgeDirection::In => &relation.source_id,
            };
            let neighbour = record_store::get_instance_by_id(store, other)?;
            let created = neighbour
                .as_ref()
                .and_then(|n| n.created_at())
                .map(str::to_string);
            let neighbour = neighbour.map(|n| match n {
                record_store::LoadedInstance::Record(r) => ContextInstance::Record(r),
                record_store::LoadedInstance::Note(n) => ContextInstance::Note(n),
            });
            let own = store.load_relation(&relation.relation_id).ok();
            relations.push((
                created,
                ContextRelation {
                    direction,
                    created_at: own.as_ref().and_then(|r| r.created_at.clone()),
                    created_by: own.and_then(|r| r.created_by),
                    relation,
                    neighbour,
                },
            ));
        }
    }
    relations.sort_by(|(ca, a), (cb, b)| {
        // createdAt: None sorts last (Option's own order puts it first).
        let key = |c: &Option<String>| (c.is_none(), c.clone());
        (&a.relation.relation_type, key(ca), &a.relation.relation_id).cmp(&(
            &b.relation.relation_type,
            key(cb),
            &b.relation.relation_id,
        ))
    });
    let relations = relations.into_iter().map(|(_, r)| r).collect();

    let (entry, subtree) = match query.container_id.as_deref() {
        None => (None, None),
        Some(cid) => {
            let outline = container_service::get_outline(store, cid)?;
            let i = outline
                .entries
                .iter()
                .position(|e| e.instance_id == query.record_id)
                .ok_or_else(|| RepositoryError::InvalidInput {
                    message: format!("{} is not a member of container {cid}", query.record_id),
                })?;
            let entry = outline.entries[i].clone();
            let subtree = outline.entries[i + 1..entry.run_end].to_vec();
            (Some(entry), Some(subtree))
        }
    };

    let protocol_run_history = protocol_run_service::list_runs_for_record(store, &query.record_id)
        .unwrap_or_else(|_| vec![])
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
        .collect();

    Ok(RecordContextResult {
        record_id: query.record_id,
        type_id: summary.record.type_id.clone(),
        type_name: summary.record.type_name.clone(),
        type_namespace: summary.record.type_namespace.clone(),
        display_label: summary.display_label.clone(),
        field_values: summary.record.field_values.clone(),
        relations,
        container_id: query.container_id,
        entry,
        subtree,
        tagged_chunks: vec![],
        protocol_run_history,
    })
}

/// Which edges of `instance_id` to list (srs-rust#1229). `direction: None` = both.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighboursQuery {
    pub instance_id: String,
    #[serde(default)]
    pub relation_type: Option<String>,
    #[serde(default)]
    pub direction: Option<EdgeDirection>,
}

/// Result shaping for [`list_neighbours`], mirroring `discovery_service::FindPage`:
/// `limit: None` means every edge; any default cap is the adapter's choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NeighboursPage {
    pub limit: Option<usize>,
    pub offset: usize,
}

/// The instance at the other end of an edge: identity and a display label only, never the
/// record. `label`/`type*` are omitted when the neighbour does not resolve (or fails to load)
/// and `type*` for a Tier-0 note.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighbourSummary {
    pub instance_id: String,
    /// `srs://<repo>/record/<id>`, the same URI a `find` hit carries (#1227).
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighbourEdge {
    pub direction: EdgeDirection,
    pub relation_id: String,
    pub relation_type: String,
    pub neighbour: NeighbourSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighboursResult {
    pub instance_id: String,
    /// Every matching edge, before paging.
    pub total: usize,
    pub neighbours: Vec<NeighbourEdge>,
}

/// Bounded read of an instance's relation neighbours. Edges are filtered, sorted by
/// `(relationType, createdAt none-last, relationId)` and paged first; only the returned page
/// loads its neighbours (unlike [`get_record_context`], which inlines all of them).
pub fn list_neighbours(
    store: &dyn RepositoryStore,
    query: NeighboursQuery,
    page: NeighboursPage,
) -> Result<NeighboursResult, RepositoryError> {
    let id = &query.instance_id;
    if record_store::get_instance_by_id(store, id)?.is_none() {
        return Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from(id),
        });
    }
    let wants = |d| query.direction.is_none_or(|q| q == d);
    let mut edges = Vec::new();
    for r in relation_service::load_relations(store)? {
        if query
            .relation_type
            .as_ref()
            .is_some_and(|t| *t != r.relation_type)
        {
            continue;
        }
        // A self-relation is both an out and an in edge, as in `get_record_context`.
        if r.source_instance_id == *id && wants(EdgeDirection::Out) {
            edges.push((EdgeDirection::Out, r.target_instance_id.clone(), r.clone()));
        }
        if r.target_instance_id == *id && wants(EdgeDirection::In) {
            edges.push((EdgeDirection::In, r.source_instance_id.clone(), r));
        }
    }
    // createdAt: None sorts last (Option's own order puts it first).
    edges.sort_by(|(_, _, a), (_, _, b)| {
        (
            &a.relation_type,
            a.created_at.is_none(),
            &a.created_at,
            &a.relation_id,
        )
            .cmp(&(
                &b.relation_type,
                b.created_at.is_none(),
                &b.created_at,
                &b.relation_id,
            ))
    });
    let total = edges.len();
    // Label indexes only matter for Tier-2 neighbours; tolerate a package that will not load.
    let indexes = crate::record_label::build_label_indexes(store).ok();
    let manifest = store.load_manifest()?;
    let repo_id = crate::resource_uri::repository_id(&manifest).unwrap_or_default();
    let neighbours = edges
        .into_iter()
        .skip(page.offset)
        .take(page.limit.unwrap_or(usize::MAX))
        .map(|(direction, other, r)| {
            let (label, type_namespace, type_name) =
                match record_store::get_instance_by_id(store, &other) {
                    Ok(Some(record_store::LoadedInstance::Record(rec))) => (
                        indexes.as_ref().map(|(fni, ifi)| {
                            crate::record_label::record_display_label(&rec, ifi, fni)
                        }),
                        Some(rec.type_namespace),
                        Some(rec.type_name),
                    ),
                    Ok(Some(record_store::LoadedInstance::Note(n))) => {
                        (Some(n.title.unwrap_or(n.instance_id)), None, None)
                    }
                    Ok(None) | Err(_) => (None, None, None),
                };
            NeighbourEdge {
                direction,
                relation_id: r.relation_id,
                relation_type: r.relation_type,
                neighbour: NeighbourSummary {
                    uri: crate::resource_uri::record_uri(repo_id, &other),
                    instance_id: other,
                    label,
                    type_namespace,
                    type_name,
                },
            }
        })
        .collect();
    Ok(NeighboursResult {
        instance_id: query.instance_id,
        total,
        neighbours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::record_store;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record::FieldValues;
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use std::path::PathBuf;

    fn make_store() -> MemoryStore {
        make_store_with(vec![])
    }

    fn make_store_with(
        relation_type_definitions: Vec<
            srs_core::types::relation_type_definition::RelationTypeDefinition,
        >,
    ) -> MemoryStore {
        let name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            // Intentionally empty (not the crate-wide "Test guidance" default) — this
            // MemoryStore-typed-Package fixture never round-trips through JSON/catalog
            // validation, and `field_context_ai_guidance_null` specifically exercises
            // the empty-guidance → `ai_guidance: None` behavior.
            ai_guidance: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let test_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-name-001".to_string(),
                order: 0,
                required: true,
                display_label: Some("Name".to_string()),
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field],
            record_types: vec![test_type],
            relation_type_definitions,
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    fn make_field_values(name: &str, value: serde_json::Value) -> FieldValues {
        let mut fv = FieldValues::new();
        fv.insert(name, value);
        fv
    }

    #[test]
    fn field_context_current_value() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Alice"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id.clone(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.record_id, rec.instance_id);
        assert_eq!(result.field_id, "field-name-001");
        assert_eq!(result.current_value, Some(json!("Alice")));
    }

    #[test]
    fn field_context_ai_guidance_from_package() {
        // Build a store where field-name-001 has non-null ai_guidance
        let mut name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let test_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-name-001".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        name_field.ai_guidance = Some(AiGuidance {
            purpose: "Write the full legal name".to_string(),
            ..Default::default()
        });
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field],
            record_types: vec![test_type],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package);

        let fv = make_field_values("test-name", json!("Charlie"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id,
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            result.ai_guidance,
            Some(json!({"purpose": "Write the full legal name"}))
        );
        assert_eq!(result.field_name, Some("test-name".to_string()));
        assert_eq!(result.field_namespace, Some("com.test".to_string()));
    }

    #[test]
    fn field_context_ai_guidance_null() {
        // make_store() has ai_guidance: null on field-name-001
        let store = make_store();
        let fv = make_field_values("test-name", json!("Dana"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id,
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert!(result.ai_guidance.is_none());
    }

    #[test]
    fn field_context_not_found() {
        let store = make_store();
        let err = get_field_context(
            &store,
            FieldContextQuery {
                record_id: "nonexistent-record-id".to_string(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }

    #[test]
    fn record_context_field_values() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Eve"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: rec.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
            },
        )
        .unwrap();

        assert_eq!(result.record_id, rec.instance_id);
        assert_eq!(result.type_id, "type-test-001");
        assert_eq!(result.type_name, "test-type");
        assert_eq!(result.type_namespace, "com.test");
        assert_eq!(result.field_values.len(), 1);
        assert_eq!(result.field_values.get("test-name"), Some(&json!("Eve")));
        assert!(result.tagged_chunks.is_empty());
        assert!(result.protocol_run_history.is_empty());
    }

    #[test]
    fn record_context_relations() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };
        let store = make_store();
        let depends_on_def = RelationTypeDefinition {
            schema: None,
            id: "rtd-depends-on".to_string(),
            version: 1,
            key: "depends-on".to_string(),
            namespace: "com.test".to_string(),
            label: "depends-on".to_string(),
            description: "Dependency relation".to_string(),
            category: RelationTypeCategory::Dependency,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };
        let defs = vec![depends_on_def];
        let fv1 = make_field_values("test-name", json!("Source"));
        let fv2 = make_field_values("test-name", json!("Target"));
        let src = record_store::create_record(&store, "type-test-001", 1, fv1, None, None).unwrap();
        let tgt = record_store::create_record(&store, "type-test-001", 1, fv2, None, None).unwrap();
        let unrelated = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("Unrelated")),
            None,
            None,
        )
        .unwrap();

        create_relation(
            &store,
            Relation {
                created_by: None,
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: src.instance_id.clone(),
                target_instance_id: tgt.instance_id.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            },
            &defs,
        )
        .unwrap();
        // Create a relation FROM unrelated to something — must not appear in src's context
        create_relation(
            &store,
            Relation {
                created_by: None,
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: unrelated.instance_id.clone(),
                target_instance_id: src.instance_id.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            },
            &defs,
        )
        .unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: src.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
            },
        )
        .unwrap();

        // Both directions are returned; the unrelated record's edge TO src is the inbound one.
        assert_eq!(result.relations.len(), 2);
        let out = result
            .relations
            .iter()
            .find(|r| r.direction == EdgeDirection::Out)
            .unwrap();
        assert_eq!(out.relation.source_id, src.instance_id);
        assert_eq!(out.relation.target_id, tgt.instance_id);
        assert!(
            matches!(&out.neighbour, Some(ContextInstance::Record(r)) if r.instance_id == tgt.instance_id)
        );
        let inn = result
            .relations
            .iter()
            .find(|r| r.direction == EdgeDirection::In)
            .unwrap();
        assert_eq!(inn.relation.source_id, unrelated.instance_id);
        assert!(
            matches!(&inn.neighbour, Some(ContextInstance::Record(r)) if r.instance_id == unrelated.instance_id)
        );
        assert!(result.entry.is_none() && result.subtree.is_none());
    }

    #[test]
    fn record_context_excludes_relation_categories() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };
        let def = |key: &str, category| RelationTypeDefinition {
            schema: None,
            id: format!("rtd-{key}"),
            version: 1,
            key: key.to_string(),
            namespace: "com.test".to_string(),
            label: key.to_string(),
            description: key.to_string(),
            category,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };
        let defs = vec![
            def("a-seq", RelationTypeCategory::Sequence),
            def("b-assoc", RelationTypeCategory::Association),
        ];
        let store = make_store_with(defs.clone());
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let (a, b, c) = (mk("A"), mk("B"), mk("C"));
        for (ty, tgt) in [("a-seq", &b), ("b-assoc", &c)] {
            create_relation(
                &store,
                Relation {
                    created_by: None,
                    relation_id: String::new(),
                    relation_type: ty.to_string(),
                    source_instance_id: a.clone(),
                    target_instance_id: tgt.clone(),
                    created_at: None,
                    notes: None,
                    source_refs: None,
                    meta: None,
                },
                &defs,
            )
            .unwrap();
        }
        let ctx = |ex: Vec<RelationTypeCategory>| {
            get_record_context(
                &store,
                RecordContextQuery {
                    record_id: a.clone(),
                    container_id: None,
                    exclude_relation_categories: ex,
                },
            )
            .unwrap()
            .relations
        };
        assert_eq!(ctx(vec![]).len(), 2);
        let kept = ctx(vec![RelationTypeCategory::Sequence]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].relation.relation_type, "b-assoc");
    }

    #[test]
    fn record_context_note_neighbour_dangling_and_order() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        let store = make_store();
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
        };
        let defs = vec![
            srs_core::types::relation_type_definition::RelationTypeDefinition {
                schema: None,
                id: "rtd-depends-on".to_string(),
                version: 1,
                key: "depends-on".to_string(),
                namespace: "com.test".to_string(),
                label: "depends-on".to_string(),
                description: "d".to_string(),
                category:
                    srs_core::types::relation_type_definition::RelationTypeCategory::Dependency,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                canonical_direction: None,
                inverse_type: None,
                irreflexive: None,
                require_same_type: None,
                status: None,
                updated_at: None,
                meta: None,
            },
        ];
        let para = mk("para");
        let rel = |src: &str, tgt: &str| Relation {
            created_by: None,
            relation_id: String::new(),
            relation_type: "depends-on".to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        };
        // Two inbound edges created second-then-first: the thread must come back by neighbour
        // createdAt (creation order), not by call order or relationId.
        let first = mk("first");
        let second = mk("second");
        for n in [&second, &first] {
            create_relation(&store, rel(&n.instance_id, &para.instance_id), &defs).unwrap();
        }
        let r = get_record_context(
            &store,
            RecordContextQuery {
                record_id: para.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
            },
        )
        .unwrap();
        let ids: Vec<_> = r
            .relations
            .iter()
            .map(|e| e.relation.source_id.clone())
            .collect();
        assert_eq!(ids.len(), 2);
        let created = |id: &str| {
            r.relations
                .iter()
                .find(|e| e.relation.source_id == id)
                .and_then(|e| match &e.neighbour {
                    Some(ContextInstance::Record(rec)) => rec.created_at.clone(),
                    _ => None,
                })
        };
        assert!(created(&ids[0]) <= created(&ids[1]));
        assert!(r.relations.iter().all(|e| e.direction == EdgeDirection::In));
    }

    #[test]
    fn record_context_subtree_slice() {
        use srs_core::types::container::{Container, ContainerEntry};
        let store = make_store();
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let (a, b, c, d) = (mk("a"), mk("b"), mk("c"), mk("d"));
        let entry = |id: &String, depth| ContainerEntry {
            instance_id: id.clone(),
            depth,
        };
        // a(0) > b(1) > c(2); d(0)
        let container = Container {
            container_id: String::new(),
            title: "doc".into(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            member_instance_ids: Some(vec![
                entry(&a, None),
                entry(&b, Some(1)),
                entry(&c, Some(2)),
                entry(&d, None),
            ]),
            child_container_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: Default::default(),
        };
        let cid = container_service::create_container(&store, container)
            .unwrap()
            .container_id;
        let ctx = |id: &String| {
            get_record_context(
                &store,
                RecordContextQuery {
                    record_id: id.clone(),
                    container_id: Some(cid.clone()),
                    exclude_relation_categories: vec![],
                },
            )
            .unwrap()
        };
        let ra = ctx(&a);
        let ids = |r: &RecordContextResult| -> Vec<String> {
            r.subtree
                .as_ref()
                .unwrap()
                .iter()
                .map(|e| e.instance_id.clone())
                .collect()
        };
        assert_eq!(ids(&ra), vec![b.clone(), c.clone()]);
        assert_eq!(ra.entry.as_ref().unwrap().instance_id, a);
        assert_eq!(ids(&ctx(&b)), vec![c.clone()]);
        assert!(ids(&ctx(&c)).is_empty());
        assert!(ids(&ctx(&d)).is_empty());
        // A real container that does not contain the record: InvalidInput, not not-found.
        let outsider = mk("outsider");
        let err = get_record_context(
            &store,
            RecordContextQuery {
                record_id: outsider,
                container_id: Some(cid.clone()),
                exclude_relation_categories: vec![],
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn record_context_non_member_container_errors() {
        let store = make_store();
        let rec = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("x")),
            None,
            None,
        )
        .unwrap();
        let err = get_record_context(
            &store,
            RecordContextQuery {
                record_id: rec.instance_id,
                container_id: Some("no-such-container".into()),
                exclude_relation_categories: vec![],
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn record_context_includes_run_history() {
        use crate::protocol_run_service::{create_run, CreateRunInput};
        let store = make_store();

        let fv = make_field_values("test-name", json!("run-history-test"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        // Create a run targeting this record.
        create_run(
            &store,
            CreateRunInput {
                protocol_id: "proto-ctx".to_string(),
                protocol_version: 1,
                container_id: "c-ctx-run".to_string(),
                target_record_id: Some(rec.instance_id.clone()),
                initial_stage_id: None,
            },
        )
        .unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: rec.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
            },
        )
        .unwrap();

        assert_eq!(result.protocol_run_history.len(), 1);
        let entry = &result.protocol_run_history[0];
        assert_eq!(entry["protocolId"], "proto-ctx");
        assert_eq!(entry["status"], "Active");
    }
    /// A hub with `n_in` inbound `depends-on` edges and one outbound `refines`.
    fn neighbours_fixture(n_in: usize) -> (crate::store::memory::MemoryStore, String, String) {
        use srs_core::types::relation::Relation;
        let store = make_store();
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let hub = mk("hub");
        let out_target = mk("target");
        let rel = |t: &str, s: &str, d: &str| Relation {
            created_by: None,
            relation_id: String::new(),
            relation_type: t.to_string(),
            source_instance_id: s.to_string(),
            target_instance_id: d.to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        };
        let mut all = vec![rel("refines", &hub, &out_target)];
        for i in 0..n_in {
            all.push(rel("depends-on", &mk(&format!("n{i}")), &hub));
        }
        for r in all {
            // No relation-type definitions needed: write the standalone object directly.
            let mut r = r;
            r.relation_id = uuid::Uuid::new_v4().to_string();
            store.save_relation(&r).unwrap();
        }
        (store, hub, out_target)
    }

    fn nq(id: &str) -> NeighboursQuery {
        NeighboursQuery {
            instance_id: id.to_string(),
            relation_type: None,
            direction: None,
        }
    }

    #[test]
    fn neighbours_pages_with_total() {
        let (store, hub, _) = neighbours_fixture(5);
        let all = list_neighbours(&store, nq(&hub), NeighboursPage::default()).unwrap();
        assert_eq!((all.total, all.neighbours.len()), (6, 6));
        let page = list_neighbours(
            &store,
            nq(&hub),
            NeighboursPage {
                limit: Some(3),
                offset: 2,
            },
        )
        .unwrap();
        assert_eq!((page.total, page.neighbours.len()), (6, 3));
        let ids = |r: &NeighboursResult| {
            r.neighbours
                .iter()
                .map(|e| e.relation_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&page),
            ids(&all)[2..5].to_vec(),
            "page is a slice of the sorted whole"
        );
        let past = list_neighbours(
            &store,
            nq(&hub),
            NeighboursPage {
                limit: Some(3),
                offset: 99,
            },
        )
        .unwrap();
        assert_eq!((past.total, past.neighbours.len()), (6, 0));
    }

    #[test]
    fn neighbours_filters_direction_and_type() {
        let (store, hub, target) = neighbours_fixture(2);
        let out = list_neighbours(
            &store,
            NeighboursQuery {
                direction: Some(EdgeDirection::Out),
                ..nq(&hub)
            },
            NeighboursPage::default(),
        )
        .unwrap();
        assert_eq!(out.total, 1);
        assert_eq!(out.neighbours[0].neighbour.instance_id, target);
        assert!(out.neighbours[0]
            .neighbour
            .uri
            .ends_with(&format!("/record/{target}")));
        assert!(out.neighbours[0].neighbour.label.is_some());
        assert_eq!(
            out.neighbours[0].neighbour.type_name.as_deref(),
            Some("test-type")
        );
        let deps = list_neighbours(
            &store,
            NeighboursQuery {
                relation_type: Some("depends-on".into()),
                direction: Some(EdgeDirection::In),
                ..nq(&hub)
            },
            NeighboursPage::default(),
        )
        .unwrap();
        assert_eq!(deps.total, 2);
        assert!(deps
            .neighbours
            .iter()
            .all(|e| e.direction == EdgeDirection::In));
    }

    #[test]
    fn neighbours_missing_subject_not_found() {
        let store = make_store();
        let err = list_neighbours(&store, nq("nope"), NeighboursPage::default()).unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }
    #[test]
    fn neighbours_order_is_type_then_created_at_none_last_then_id() {
        use srs_core::types::relation::Relation;
        let (store, hub, _) = neighbours_fixture(0);
        let other = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("o")),
            None,
            None,
        )
        .unwrap()
        .instance_id;
        let rel = |id: &str, t: &str, at: Option<&str>| Relation {
            created_by: None,
            relation_id: id.to_string(),
            relation_type: t.to_string(),
            source_instance_id: other.clone(),
            target_instance_id: hub.clone(),
            created_at: at.map(str::to_string),
            notes: None,
            source_refs: None,
            meta: None,
        };
        for r in [
            rel("00000000-0000-4000-8000-000000000003", "depends-on", None),
            rel(
                "00000000-0000-4000-8000-000000000002",
                "depends-on",
                Some("2026-02-01T00:00:00Z"),
            ),
            rel(
                "00000000-0000-4000-8000-000000000001",
                "depends-on",
                Some("2026-01-01T00:00:00Z"),
            ),
        ] {
            store.save_relation(&r).unwrap();
        }
        let ids: Vec<_> = list_neighbours(&store, nq(&hub), NeighboursPage::default())
            .unwrap()
            .neighbours
            .into_iter()
            .map(|e| (e.relation_type, e.relation_id))
            .collect();
        let pos = |id: &str| ids.iter().position(|(_, r)| r == id).unwrap();
        assert!(
            pos("00000000-0000-4000-8000-000000000001")
                < pos("00000000-0000-4000-8000-000000000002")
                && pos("00000000-0000-4000-8000-000000000002")
                    < pos("00000000-0000-4000-8000-000000000003"),
            "{ids:?}"
        );
        assert_eq!(
            ids[0].0, "depends-on",
            "depends-on sorts before refines: {ids:?}"
        );
    }

    #[test]
    fn neighbours_note_subject_and_note_neighbour() {
        use srs_core::types::relation::Relation;
        let (store, hub, _) = neighbours_fixture(0);
        let note = crate::services::create_note(
            &store,
            serde_json::from_value(json!({ "title": "A note", "sections": [] })).unwrap(),
        )
        .unwrap()
        .note
        .instance_id;
        store
            .save_relation(&Relation {
                created_by: None,
                relation_id: "00000000-0000-4000-8000-0000000000aa".into(),
                relation_type: "depends-on".into(),
                source_instance_id: note.clone(),
                target_instance_id: hub.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            })
            .unwrap();
        let from_note = list_neighbours(&store, nq(&note), NeighboursPage::default()).unwrap();
        assert_eq!(from_note.total, 1);
        assert_eq!(from_note.neighbours[0].neighbour.instance_id, hub);
        let from_hub = list_neighbours(&store, nq(&hub), NeighboursPage::default()).unwrap();
        let n = from_hub
            .neighbours
            .iter()
            .find(|e| e.neighbour.instance_id == note)
            .unwrap();
        assert_eq!(n.neighbour.label.as_deref(), Some("A note"));
        assert!(n.neighbour.type_name.is_none());
    }
}
