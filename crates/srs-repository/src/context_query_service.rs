use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::{package_service, protocol_run_service, record_store, relation_service};
use relation_service::ListRelationsFilter;

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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionTraceQuery {
    pub record_id: String,
    pub field_id: String,
    pub revision_id: String,
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
    pub revisions: Vec<srs_core::types::revision::Revision>,
    /// Always empty; placeholder for tagged-chunk storage (#252)
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
    pub relations: Vec<crate::relation_service::RelationSummary>,
    /// Always empty; placeholder for tagged-chunk storage (#252)
    pub tagged_chunks: Vec<serde_json::Value>,
    /// Always empty; placeholder for protocol run history (#252)
    pub protocol_run_history: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionTraceResult {
    pub record_id: String,
    pub field_id: String,
    pub revision: srs_core::types::revision::Revision,
    pub prior_chain: Vec<srs_core::types::revision::Revision>,
}

/// Assemble field context: current value, revision history, and aiGuidance from package.
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

    let revisions = record_store::list_record_revisions(
        store,
        &query.record_id,
        Some(&query.field_id),
        None,
        None,
    )?;

    let (field_name, field_namespace, ai_guidance) =
        match package_service::get_field_by_id(store, &query.field_id)? {
            package_service::GetFieldResult::Found(field) => {
                let guidance = if field.ai_guidance.purpose.is_empty() {
                    None
                } else {
                    serde_json::to_value(&field.ai_guidance).ok()
                };
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
        revisions,
        tagged_chunks: vec![],
    })
}

/// Assemble record context: all field values and source-filtered relations.
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

    // Intentionally source-only: outbound relations give the context of what this record
    // depends on / contains / supersedes. Inbound edges are part of the stage-context
    // pattern deferred to #252.
    let relations = relation_service::list_relations(
        store,
        ListRelationsFilter {
            source: Some(query.record_id.clone()),
            ..Default::default()
        },
    )?;

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
        tagged_chunks: vec![],
        protocol_run_history,
    })
}

/// Trace a revision: return the target revision and its prior chain (oldest-first).
///
/// Builds the prior chain by following `prior_revision_id` links. A HashSet guards
/// against cycles so a malformed sidecar cannot loop forever.
pub fn get_revision_trace(
    store: &dyn RepositoryStore,
    query: RevisionTraceQuery,
) -> Result<RevisionTraceResult, RepositoryError> {
    let all_revisions = record_store::list_record_revisions(
        store,
        &query.record_id,
        Some(&query.field_id),
        None,
        None,
    )?;

    let target = all_revisions
        .iter()
        .find(|r| r.revision_id == query.revision_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(&query.revision_id),
        })?;

    let index: HashMap<&str, &srs_core::types::revision::Revision> = all_revisions
        .iter()
        .map(|r| (r.revision_id.as_str(), r))
        .collect();

    let mut chain = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    // Seed with target so a lasso-shaped chain (ancestor → target) is detected immediately.
    seen.insert(target.revision_id.clone());
    let mut current_prior = target.prior_revision_id.clone();
    while let Some(ref prior_id) = current_prior {
        if !seen.insert(prior_id.clone()) {
            break;
        }
        match index.get(prior_id.as_str()) {
            Some(rev) => {
                chain.push((*rev).clone());
                current_prior = rev.prior_revision_id.clone();
            }
            None => break,
        }
    }
    chain.reverse(); // oldest-first

    Ok(RevisionTraceResult {
        record_id: query.record_id,
        field_id: query.field_id,
        revision: target,
        prior_chain: chain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use crate::{record_store, revision_service};
    use serde_json::json;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record::FieldValues;
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_core::types::revision::{Revision, RevisionAgent};
    use std::path::PathBuf;

    fn make_store() -> MemoryStore {
        let name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            ai_guidance: AiGuidance::default(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let test_type = RecordType {
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
            extra: std::collections::BTreeMap::new(),
        };
        let manifest = Manifest {
            instance_index: vec![],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            source_document_index: None,
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
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
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

    fn make_revision(id: &str, record_id: &str, field_id: &str, prior: Option<&str>) -> Revision {
        Revision {
            revision_id: id.to_string(),
            record_id: record_id.to_string(),
            field_id: field_id.to_string(),
            value: json!("v"),
            prior_revision_id: prior.map(|s| s.to_string()),
            agent: RevisionAgent::Human,
            provenance: None,
            source_refs: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn field_context_no_revisions() {
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
        assert!(result.revisions.is_empty());
        assert_eq!(result.current_value, Some(json!("Alice")));
    }

    #[test]
    fn field_context_filters_by_field_id() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Bob"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let path = store
            .load_manifest()
            .unwrap()
            .instance_index
            .iter()
            .find(|e| e.instance_id() == rec.instance_id)
            .unwrap()
            .path()
            .to_string();

        revision_service::append(
            &store,
            &path,
            make_revision("rev-a1", &rec.instance_id, "field-name-001", None),
        )
        .unwrap();
        revision_service::append(
            &store,
            &path,
            make_revision("rev-b1", &rec.instance_id, "field-other-001", None),
        )
        .unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id.clone(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.revisions.len(), 1);
        assert_eq!(result.revisions[0].revision_id, "rev-a1");
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
            ai_guidance: AiGuidance::default(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let test_type = RecordType {
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
            extra: std::collections::BTreeMap::new(),
        };
        name_field.ai_guidance = AiGuidance {
            purpose: "Write the full legal name".to_string(),
            ..Default::default()
        };
        let manifest = Manifest {
            instance_index: vec![],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            source_document_index: None,
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
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
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
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: None,
            updated_at: None,
            properties: None,
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
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: src.instance_id.clone(),
                target_instance_id: tgt.instance_id.clone(),
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
            },
            &defs,
        )
        .unwrap();
        // Create a relation FROM unrelated to something — must not appear in src's context
        create_relation(
            &store,
            Relation {
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: unrelated.instance_id.clone(),
                target_instance_id: src.instance_id.clone(),
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
            },
            &defs,
        )
        .unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: src.instance_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(result.relations.len(), 1);
        assert_eq!(result.relations[0].source_id, src.instance_id);
        assert_eq!(result.relations[0].target_id, tgt.instance_id);
    }

    #[test]
    fn revision_trace_prior_chain() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Frank"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let path = store
            .load_manifest()
            .unwrap()
            .instance_index
            .iter()
            .find(|e| e.instance_id() == rec.instance_id)
            .unwrap()
            .path()
            .to_string();

        revision_service::append(
            &store,
            &path,
            make_revision("rev-1", &rec.instance_id, "field-name-001", None),
        )
        .unwrap();
        revision_service::append(
            &store,
            &path,
            make_revision("rev-2", &rec.instance_id, "field-name-001", Some("rev-1")),
        )
        .unwrap();
        revision_service::append(
            &store,
            &path,
            make_revision("rev-3", &rec.instance_id, "field-name-001", Some("rev-2")),
        )
        .unwrap();

        let result = get_revision_trace(
            &store,
            RevisionTraceQuery {
                record_id: rec.instance_id.clone(),
                field_id: "field-name-001".to_string(),
                revision_id: "rev-3".to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.revision.revision_id, "rev-3");
        assert_eq!(result.prior_chain.len(), 2);
        assert_eq!(result.prior_chain[0].revision_id, "rev-1");
        assert_eq!(result.prior_chain[1].revision_id, "rev-2");
    }

    #[test]
    fn revision_trace_not_found() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Grace"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let err = get_revision_trace(
            &store,
            RevisionTraceQuery {
                record_id: rec.instance_id,
                field_id: "field-name-001".to_string(),
                revision_id: "rev-nonexistent".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }

    #[test]
    fn field_context_cross_store_roundtrip() {
        use crate::json_store::JsonStore;

        // 1. Build MemoryStore, create record, append revision
        let store = make_store();
        let fv = make_field_values("test-name", json!("Heidi"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == rec.instance_id)
            .unwrap();
        let record_path = entry.path().to_string();

        revision_service::append(
            &store,
            &record_path,
            make_revision("rev-x1", &rec.instance_id, "field-name-001", None),
        )
        .unwrap();

        // 2. Build SRSJ from MemoryStore data
        let record_json = store.load_instance_json(&record_path).unwrap();
        let sidecar_path = revision_service::sidecar_path_for(&record_path);
        let sidecar_json = store.load_instance_json(&sidecar_path).unwrap();

        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "instanceIndex": [{
                    "instanceId": rec.instance_id,
                    "tier": 2,
                    "path": record_path
                }],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                record_path: record_json,
                sidecar_path: sidecar_json,
                // RFC-039: the carrier keys by Field.name, so field-id queries
                // need the Field definition to bridge id → name.
                "package/fields/test-name.json": {
                    "id": "field-name-001",
                    "namespace": "com.test",
                    "name": "test-name",
                    "version": 1,
                    "description": "Name field",
                    "fieldType": {"datatype": "string"},
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/package.json": {
                    "id": "test-pkg",
                    "namespace": "com.test",
                    "name": "test-package",
                    "version": "1.0.0",
                    "fields": ["fields/test-name.json"],
                    "types": [],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": [],
                    "blueprints": [],
                    "protocols": [],
                    "vocabularies": [],
                    "lifecycles": []
                }
            }
        })
        .to_string();

        // 3. Load into JsonStore, call get_field_context on both, assert results match
        let json_store = JsonStore::from_srsj(&srsj).unwrap();

        let mem_result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id.clone(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();
        let json_result = get_field_context(
            &json_store,
            FieldContextQuery {
                record_id: rec.instance_id.clone(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert_eq!(mem_result.revisions.len(), json_result.revisions.len());
        assert_eq!(
            mem_result.revisions[0].revision_id,
            json_result.revisions[0].revision_id
        );
        assert_eq!(mem_result.current_value, json_result.current_value);
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
            },
        )
        .unwrap();

        assert_eq!(result.protocol_run_history.len(), 1);
        let entry = &result.protocol_run_history[0];
        assert_eq!(entry["protocolId"], "proto-ctx");
        assert_eq!(entry["status"], "Active");
    }
}
