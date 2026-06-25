//! `blueprint brief` composition service.
//!
//! Assembles, for one Blueprint, the full layered guidance context in the spec's recommended
//! AI guidance composition order:
//!
//! 1. Blueprint `aiGuidance` + `requiredTypes`
//! 2. For each root Type: Type `aiGuidance`, then each Field in `order` with its guidance
//! 3. `structure[]` RelationSpecs
//! 4. First Protocol whose `targetType` matches a root Type (if any)
//!
//! Also provides `render_brief_markdown` for human/LLM-readable prose output.

use crate::blueprint_service::{get_blueprint_by_id, GetBlueprintResult};
use crate::error::RepositoryError;
use crate::package_service::{
    get_field_by_id, get_type_by_id, get_type_by_id_latest, GetFieldResult, GetTypeResult,
};
use crate::protocol_service::find_protocol_by_target_type;
use crate::store::RepositoryStore;
use srs_core::types::blueprint::TypeRef;
use srs_core::types::protocol::{FieldRef, ProtocolStage};

// ---------------------------------------------------------------------------
// Input / output types
// ---------------------------------------------------------------------------

pub struct BlueprintBriefInput {
    pub blueprint_id: String,
}

#[derive(Debug, Clone)]
pub struct BriefFieldResult {
    pub field_id: String,
    pub name: String,
    pub order: u32,
    pub required: bool,
    pub value_type: String,
    pub ai_guidance: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct BriefTypeResult {
    pub type_id: String,
    pub namespace: String,
    pub name: String,
    pub ai_guidance: Option<serde_json::Value>,
    pub fields: Vec<BriefFieldResult>,
}

#[derive(Debug, Clone)]
pub struct BriefRelationSpecResult {
    pub relation_type: String,
    pub source_type_id: String,
    pub target_type_id: String,
    pub cardinality: Option<String>,
    pub required: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct BriefStageResult {
    pub stage_id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub order: i32,
    pub depends_on: Vec<String>,
    pub question: Option<String>,
    pub completion_criteria: Option<String>,
    pub contributes_to: Option<Vec<FieldRef>>,
    pub ai_guidance: Option<serde_json::Value>,
    pub output_type: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct BriefProtocolResult {
    pub protocol_id: String,
    pub protocol_name: String,
    pub stages: Vec<BriefStageResult>,
}

#[derive(Debug, Clone)]
pub struct BlueprintBriefResult {
    pub blueprint_id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub ai_guidance: Option<serde_json::Value>,
    /// Raw TypeRef values passed through for the payload.
    pub required_types: Vec<serde_json::Value>,
    pub types: Vec<BriefTypeResult>,
    pub structure: Vec<BriefRelationSpecResult>,
    pub protocol: Option<BriefProtocolResult>,
    pub diagnostics: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public service functions
// ---------------------------------------------------------------------------

/// Compose the full layered guidance context for one Blueprint.
///
/// Returns `Err(RepositoryError::BlueprintNotFound)` when the blueprint cannot be resolved.
/// All other failures (unresolvable type refs, missing fields) are non-fatal and reported
/// in `result.diagnostics`.
pub fn blueprint_brief(
    store: &dyn RepositoryStore,
    input: BlueprintBriefInput,
) -> Result<BlueprintBriefResult, RepositoryError> {
    let blueprint = match get_blueprint_by_id(store, &input.blueprint_id)? {
        GetBlueprintResult::Found(bp) => *bp,
        GetBlueprintResult::NotFound => {
            return Err(RepositoryError::BlueprintNotFound {
                blueprint_id: input.blueprint_id,
            })
        }
    };

    let mut diagnostics: Vec<String> = Vec::new();
    let mut types: Vec<BriefTypeResult> = Vec::new();

    for type_ref in &blueprint.root_types {
        if let Some(brief_type) = resolve_brief_type(store, type_ref, &mut diagnostics)? {
            types.push(brief_type);
        }
    }

    let structure = blueprint
        .structure
        .iter()
        .map(|rs| BriefRelationSpecResult {
            relation_type: rs.relation_type.clone(),
            source_type_id: rs.source_type.type_id.clone(),
            target_type_id: rs.target_type.type_id.clone(),
            cardinality: rs.cardinality.clone(),
            required: rs.required,
        })
        .collect();

    let required_types = blueprint
        .required_types
        .iter()
        .filter_map(|tr| serde_json::to_value(tr).ok())
        .collect();

    let protocol = find_protocol_for_roots(store, &blueprint.root_types, &mut diagnostics)?;

    Ok(BlueprintBriefResult {
        blueprint_id: blueprint.id,
        namespace: blueprint.namespace,
        name: blueprint.name,
        version: blueprint.version,
        ai_guidance: blueprint.ai_guidance,
        required_types,
        types,
        structure,
        protocol,
        diagnostics,
    })
}

/// Render a `BlueprintBriefResult` as human/LLM-readable markdown.
pub fn render_brief_markdown(result: &BlueprintBriefResult) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "# Blueprint: {}/{} v{}\n\n",
        result.namespace, result.name, result.version
    ));

    if let Some(guidance) = &result.ai_guidance {
        out.push_str(&format_guidance_prose(guidance));
        out.push('\n');
    }

    if !result.required_types.is_empty() {
        out.push_str("**Required types:**\n");
        for rt in &result.required_types {
            if let Some(id) = rt.get("typeId").and_then(|v| v.as_str()) {
                out.push_str(&format!("- `{id}`\n"));
            }
        }
        out.push('\n');
    }

    for t in &result.types {
        out.push_str(&format!("## Type: {}/{}\n\n", t.namespace, t.name));
        if let Some(guidance) = &t.ai_guidance {
            out.push_str(&format_guidance_prose(guidance));
            out.push('\n');
        }
        if !t.fields.is_empty() {
            out.push_str(
                "| Field | ValueType | Required | Purpose | Extraction | Negative | Examples |\n",
            );
            out.push_str("|---|---|---|---|---|---|---|\n");
            for f in &t.fields {
                let purpose = extract_str_field(&f.ai_guidance, "purpose");
                let extraction = extract_str_field(&f.ai_guidance, "extraction");
                let negative = extract_str_field(&f.ai_guidance, "negativeGuidance");
                let examples = extract_str_field(&f.ai_guidance, "examples");
                let required = if f.required { "yes" } else { "no" };
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} | {} | {} | {} |\n",
                    f.name, f.value_type, required, purpose, extraction, negative, examples
                ));
            }
            out.push('\n');
        }
    }

    if !result.structure.is_empty() {
        out.push_str("## Structure\n\n");
        for rs in &result.structure {
            let card = rs
                .cardinality
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            out.push_str(&format!(
                "- `{}` → `{}` via `{}`{}\n",
                rs.source_type_id, rs.target_type_id, rs.relation_type, card
            ));
        }
        out.push('\n');
    }

    if let Some(proto) = &result.protocol {
        out.push_str(&format!("## Protocol: {}\n\n", proto.protocol_name));
        for stage in &proto.stages {
            out.push_str(&format!("### {}. {}\n\n", stage.order, stage.name));
            // purpose is structural metadata (epistemic label); not rendered — question is the prose entry point
            if let Some(q) = &stage.question {
                out.push_str(&format!("**Question:** {q}\n\n"));
            }
            if let Some(cc) = &stage.completion_criteria {
                out.push_str(&format!("**Done when:** {cc}\n\n"));
            }
            if let Some(ct) = &stage.contributes_to {
                if !ct.is_empty() {
                    let labels: Vec<String> = ct
                        .iter()
                        .map(|r| match &r.type_id {
                            Some(tid) => format!("{}/{}", tid, r.field_id),
                            None => r.field_id.clone(),
                        })
                        .collect();
                    out.push_str(&format!("**Contributes to:** {}\n\n", labels.join(", ")));
                }
            }
            if let Some(dep) = stage.ai_guidance.as_ref() {
                out.push_str(&format_guidance_prose(dep));
                out.push('\n');
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn resolve_brief_type(
    store: &dyn RepositoryStore,
    type_ref: &TypeRef,
    diagnostics: &mut Vec<String>,
) -> Result<Option<BriefTypeResult>, RepositoryError> {
    let record_type = match type_ref.type_version {
        Some(v) => match get_type_by_id(store, &type_ref.type_id, v)? {
            GetTypeResult::Found(rt) => rt,
            GetTypeResult::NotFound => {
                diagnostics.push(format!(
                    "root type {} v{} not found in package",
                    type_ref.type_id, v
                ));
                return Ok(None);
            }
        },
        None => match get_type_by_id_latest(store, &type_ref.type_id)? {
            GetTypeResult::Found(rt) => rt,
            GetTypeResult::NotFound => {
                diagnostics.push(format!(
                    "root type {} not found in package",
                    type_ref.type_id
                ));
                return Ok(None);
            }
        },
    };

    let ai_guidance = record_type.extra.get("aiGuidance").cloned();

    let mut field_assignments = record_type.fields.clone();
    field_assignments.sort_by_key(|fa| fa.order);

    let mut fields = Vec::new();
    for fa in &field_assignments {
        match get_field_by_id(store, &fa.field_id)? {
            GetFieldResult::Found(field) => {
                let field_ai = if field.ai_guidance.is_null() {
                    None
                } else {
                    Some(field.ai_guidance.clone())
                };
                let value_type = serde_json::to_value(&field.value_type)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                fields.push(BriefFieldResult {
                    field_id: field.id.clone(),
                    name: field.name.clone(),
                    order: fa.order,
                    required: fa.required,
                    value_type,
                    ai_guidance: field_ai,
                });
            }
            GetFieldResult::NotFound => {
                diagnostics.push(format!("field {} not found in package", fa.field_id));
            }
        }
    }

    Ok(Some(BriefTypeResult {
        type_id: record_type.id.clone(),
        namespace: record_type.namespace.clone(),
        name: record_type.name.clone(),
        ai_guidance,
        fields,
    }))
}

fn find_protocol_for_roots(
    store: &dyn RepositoryStore,
    root_types: &[TypeRef],
    diagnostics: &mut Vec<String>,
) -> Result<Option<BriefProtocolResult>, RepositoryError> {
    for type_ref in root_types {
        match find_protocol_by_target_type(store, &type_ref.type_id)? {
            Some(proto_raw) => {
                diagnostics.extend(proto_raw.diagnostics);
                let mut stages: Vec<BriefStageResult> = proto_raw
                    .stages
                    .into_iter()
                    .map(BriefStageResult::from)
                    .collect();
                stages.sort_by_key(|s| s.order);
                return Ok(Some(BriefProtocolResult {
                    protocol_id: proto_raw.protocol_id,
                    protocol_name: proto_raw.protocol_name,
                    stages,
                }));
            }
            None => continue,
        }
    }
    Ok(None)
}

impl From<ProtocolStage> for BriefStageResult {
    fn from(stage: ProtocolStage) -> Self {
        Self {
            stage_id: stage.stage_id,
            name: stage.name,
            purpose: stage.purpose,
            order: stage.order,
            depends_on: stage.depends_on,
            question: stage.question,
            completion_criteria: stage.completion_criteria,
            contributes_to: stage.contributes_to,
            ai_guidance: stage.ai_guidance,
            output_type: stage.output_type,
        }
    }
}

fn format_guidance_prose(guidance: &serde_json::Value) -> String {
    if let Some(s) = guidance.as_str() {
        return format!("{s}\n");
    }
    let mut out = String::new();
    if let Some(purpose) = guidance.get("purpose").and_then(|v| v.as_str()) {
        out.push_str(&format!("{purpose}\n"));
    }
    if let Some(extraction) = guidance.get("extraction").and_then(|v| v.as_str()) {
        out.push_str(&format!("\n**Extraction:** {extraction}\n"));
    }
    if let Some(neg) = guidance.get("negativeGuidance").and_then(|v| v.as_str()) {
        out.push_str(&format!("\n**Avoid:** {neg}\n"));
    }
    out
}

fn extract_str_field(guidance: &Option<serde_json::Value>, key: &str) -> String {
    guidance
        .as_ref()
        .and_then(|g| g.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint_service::create_blueprint;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::protocol_service::{import_protocol, ImportProtocolInput};
    use crate::store::memory::MemoryStore;
    use srs_core::types::blueprint::{Blueprint, RelationSpec, TypeRef};
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Build a MemoryStore pre-populated with two fields and one type.
    fn make_package_store(fields: Vec<Field>, record_types: Vec<RecordType>) -> MemoryStore {
        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "test.ns".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types,
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package);
        store.register_package_boundary(&None).unwrap();
        store
    }

    fn make_field(id: &str, name: &str, vt: ValueType) -> Field {
        Field {
            id: id.to_string(),
            namespace: "test.ns".to_string(),
            name: name.to_string(),
            version: 1,
            description: format!("A {name}"),
            ai_guidance: serde_json::json!({ "purpose": format!("captures the {name}") }),
            value_type: vt,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn make_article_type() -> RecordType {
        let mut extra = HashMap::new();
        extra.insert(
            "aiGuidance".to_string(),
            serde_json::json!("Extract a structured article."),
        );
        RecordType {
            id: "type-111".to_string(),
            namespace: "test.ns".to_string(),
            name: "article".to_string(),
            version: 1,
            description: "Article type".to_string(),
            // order=2 for title (field-aaa), order=1 for summary (field-bbb)
            // → sorted result is [summary(1), title(2)]
            fields: vec![
                FieldAssignment {
                    field_id: "field-aaa".to_string(),
                    order: 2,
                    required: true,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "field-bbb".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra,
        }
    }

    fn make_store_with_blueprint_and_type() -> (MemoryStore, String) {
        let store = make_package_store(
            vec![
                make_field("field-aaa", "title", ValueType::String),
                make_field("field-bbb", "summary", ValueType::Text),
            ],
            vec![make_article_type()],
        );
        let blueprint = Blueprint {
            id: String::new(),
            namespace: "test.ns".to_string(),
            name: "test-blueprint".to_string(),
            version: 1,
            description: "A test blueprint".to_string(),
            root_types: vec![TypeRef {
                type_id: "type-111".to_string(),
                type_version: None,
            }],
            structure: vec![],
            required_types: vec![],
            ai_guidance: Some(serde_json::json!("Extract articles from the document.")),
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let created = create_blueprint(&store, blueprint, None).unwrap();
        let bp_id = created.blueprint.id.clone();
        (store, bp_id)
    }

    #[test]
    fn test_brief_blueprint_not_found() {
        let store = MemoryStore::default();
        let result = blueprint_brief(
            &store,
            BlueprintBriefInput {
                blueprint_id: "does-not-exist".to_string(),
            },
        );
        assert!(matches!(
            result,
            Err(RepositoryError::BlueprintNotFound { .. })
        ));
    }

    #[test]
    fn test_brief_basic_composition_fields_sorted_by_order() {
        let (store, bp_id) = make_store_with_blueprint_and_type();
        let result = blueprint_brief(
            &store,
            BlueprintBriefInput {
                blueprint_id: bp_id,
            },
        )
        .unwrap();

        assert_eq!(result.diagnostics, Vec::<String>::new());
        assert_eq!(result.types.len(), 1);
        let t = &result.types[0];
        assert_eq!(t.fields.len(), 2);
        // order=1 (summary) first, order=2 (title) second
        assert_eq!(t.fields[0].order, 1);
        assert_eq!(t.fields[0].name, "summary");
        assert_eq!(t.fields[1].order, 2);
        assert_eq!(t.fields[1].name, "title");
    }

    #[test]
    fn test_brief_unresolvable_type_is_diagnostic() {
        let store = make_package_store(vec![], vec![]);
        let blueprint = Blueprint {
            id: String::new(),
            namespace: "test.ns".to_string(),
            name: "broken-blueprint".to_string(),
            version: 1,
            description: String::new(),
            root_types: vec![TypeRef {
                type_id: "no-such-type".to_string(),
                type_version: None,
            }],
            structure: vec![],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let created = create_blueprint(&store, blueprint, None).unwrap();

        let result = blueprint_brief(
            &store,
            BlueprintBriefInput {
                blueprint_id: created.blueprint.id,
            },
        )
        .unwrap();

        assert!(!result.diagnostics.is_empty());
        assert!(result.diagnostics[0].contains("no-such-type"));
        assert_eq!(result.types.len(), 0);
    }

    #[test]
    fn test_brief_protocol_no_match() {
        let store = MemoryStore::default();
        let proto = find_protocol_by_target_type(&store, "type-999").unwrap();
        assert!(proto.is_none());
    }

    #[test]
    fn test_render_markdown_contains_blueprint_name() {
        let (store, bp_id) = make_store_with_blueprint_and_type();
        let result = blueprint_brief(
            &store,
            BlueprintBriefInput {
                blueprint_id: bp_id,
            },
        )
        .unwrap();
        let md = render_brief_markdown(&result);
        assert!(
            md.contains("test-blueprint"),
            "rendered markdown must contain blueprint name"
        );
        assert!(
            md.contains("title") || md.contains("summary"),
            "must contain at least one field name"
        );
    }

    #[test]
    fn test_deserialize_stage_required_fields() {
        use srs_core::types::protocol::ProtocolStage;
        let v = serde_json::json!({
            "stageId": "s1",
            "name": "Gather context",
            "order": 1,
            "dependsOn": [],
            "question": "What is the main topic?",
            "completionCriteria": "Topic identified.",
            "contributesTo": [{"fieldId": "field-aaa"}],
            "aiGuidance": "Focus on primary subjects."
        });
        let stage: ProtocolStage = serde_json::from_value(v).unwrap();
        let brief = BriefStageResult::from(stage);
        assert_eq!(brief.stage_id, "s1");
        assert_eq!(brief.order, 1);
        assert_eq!(brief.question.as_deref(), Some("What is the main topic?"));
        assert_eq!(
            brief.completion_criteria.as_deref(),
            Some("Topic identified.")
        );
        assert_eq!(
            brief.contributes_to,
            Some(vec![FieldRef {
                field_id: "field-aaa".to_string(),
                type_id: None,
            }])
        );
    }

    #[test]
    fn test_deserialize_stage_missing_required_errors() {
        use srs_core::types::protocol::ProtocolStage;
        let v = serde_json::json!({ "name": "no-id-stage", "order": 1 });
        let err = serde_json::from_value::<ProtocolStage>(v).unwrap_err();
        assert!(
            err.to_string().contains("stageId"),
            "error should mention stageId, got: {err}"
        );
    }

    #[test]
    fn test_render_brief_markdown_contributes_to() {
        let result = BlueprintBriefResult {
            blueprint_id: "bp-1".to_string(),
            namespace: "com.example".to_string(),
            name: "My Blueprint".to_string(),
            version: 1,
            ai_guidance: None,
            required_types: vec![],
            types: vec![],
            structure: vec![],
            protocol: Some(BriefProtocolResult {
                protocol_id: "proto-1".to_string(),
                protocol_name: "Example Protocol".to_string(),
                stages: vec![BriefStageResult {
                    stage_id: "s1".to_string(),
                    name: "Gather".to_string(),
                    purpose: None,
                    order: 1,
                    depends_on: vec![],
                    question: None,
                    completion_criteria: None,
                    contributes_to: Some(vec![
                        FieldRef {
                            field_id: "my-field".to_string(),
                            type_id: None,
                        },
                        FieldRef {
                            field_id: "other-field".to_string(),
                            type_id: Some("type-abc".to_string()),
                        },
                    ]),
                    ai_guidance: None,
                    output_type: None,
                }],
            }),
            diagnostics: vec![],
        };
        let md = render_brief_markdown(&result);
        assert!(
            md.contains("**Contributes to:** my-field, type-abc/other-field"),
            "expected correctly formatted contributes_to in markdown, got:\n{md}"
        );
    }

    /// Build fields and RecordType for the meta.protocol system type (matching
    /// the UUIDs in srs/srs/package/).
    fn make_protocol_fields_and_type() -> (Vec<Field>, RecordType) {
        const FIELDS: &[(&str, &str, ValueType)] = &[
            (
                "6c66d06c-3f95-4d17-8ecf-e1046a6f2ec1",
                "protocol-id",
                ValueType::String,
            ),
            (
                "8d0f55f9-80e3-4dd6-a05c-10c4b6b6cc87",
                "protocol-namespace",
                ValueType::String,
            ),
            (
                "09c5e389-cf6c-4f72-aad6-8cf26bce0b78",
                "protocol-name",
                ValueType::String,
            ),
            (
                "f7d28d9d-f90c-4a01-a3eb-2ff4cad54ff6",
                "protocol-version",
                ValueType::Number,
            ),
            (
                "4939a29b-7f70-481f-bf6b-bf693f8bd67f",
                "protocol-target-type",
                ValueType::String,
            ),
            (
                "0f1232c6-0db5-4383-b91d-64d81195f1c4",
                "protocol-stages",
                ValueType::Text,
            ),
            (
                "b953f716-383a-4218-bebf-96e93c4747a4",
                "protocol-created-at",
                ValueType::Date,
            ),
        ];

        let fields: Vec<Field> = FIELDS
            .iter()
            .map(|(id, name, vt)| Field {
                id: id.to_string(),
                namespace: "com.semanticops.srs".to_string(),
                name: name.to_string(),
                version: 1,
                description: format!("{name} field"),
                ai_guidance: serde_json::Value::Null,
                value_type: *vt,
                allowed_values: None,
                vocabulary_ref: None,
                default_value: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                extra: HashMap::new(),
            })
            .collect();

        let assignments: Vec<FieldAssignment> = FIELDS
            .iter()
            .enumerate()
            .map(|(i, (id, _, _))| FieldAssignment {
                field_id: id.to_string(),
                order: i as u32,
                required: true,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            })
            .collect();

        let proto_type = RecordType {
            id: "48a03f5d-4f27-42f4-b791-999f6c22f8d2".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "meta.protocol".to_string(),
            version: 1,
            description: "Protocol definition type".to_string(),
            fields: assignments,
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        (fields, proto_type)
    }

    #[test]
    fn test_brief_structure_relay() {
        let store = make_package_store(
            vec![make_field("field-aaa", "title", ValueType::String)],
            vec![make_article_type()],
        );
        let blueprint = Blueprint {
            id: String::new(),
            namespace: "test.ns".to_string(),
            name: "relay-blueprint".to_string(),
            version: 1,
            description: String::new(),
            root_types: vec![TypeRef {
                type_id: "type-111".to_string(),
                type_version: None,
            }],
            structure: vec![RelationSpec {
                relation_type: "contains".to_string(),
                source_type: TypeRef {
                    type_id: "type-111".to_string(),
                    type_version: None,
                },
                target_type: TypeRef {
                    type_id: "type-222".to_string(),
                    type_version: None,
                },
                cardinality: Some("1..*".to_string()),
                required: Some(true),
            }],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let created = create_blueprint(&store, blueprint, None).unwrap();
        let result = blueprint_brief(
            &store,
            BlueprintBriefInput {
                blueprint_id: created.blueprint.id,
            },
        )
        .unwrap();

        assert_eq!(result.structure.len(), 1);
        assert_eq!(result.structure[0].relation_type, "contains");
        assert_eq!(result.structure[0].source_type_id, "type-111");
        assert_eq!(result.structure[0].target_type_id, "type-222");
        assert_eq!(result.structure[0].cardinality.as_deref(), Some("1..*"));
        assert_eq!(result.structure[0].required, Some(true));
    }

    #[test]
    fn test_brief_finds_protocol_for_root_type() {
        let (proto_fields, proto_type) = make_protocol_fields_and_type();
        let store = make_package_store(
            [
                vec![make_field("field-aaa", "title", ValueType::String)],
                proto_fields,
            ]
            .concat(),
            vec![make_article_type(), proto_type],
        );
        // Create a blueprint whose root type is "type-111"
        let blueprint = Blueprint {
            id: String::new(),
            namespace: "test.ns".to_string(),
            name: "protocol-blueprint".to_string(),
            version: 1,
            description: String::new(),
            root_types: vec![TypeRef {
                type_id: "type-111".to_string(),
                type_version: None,
            }],
            structure: vec![],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let created = create_blueprint(&store, blueprint, None).unwrap();

        // Import a protocol that targets "type-111"
        import_protocol(
            &store,
            ImportProtocolInput {
                raw: serde_json::json!({
                    "protocolId": "proto-001",
                    "protocolNamespace": "test.ns",
                    "protocolName": "Article Protocol",
                    "protocolVersion": 1,
                    "protocolTargetType": "type-111",
                    "protocolCreatedAt": "2026-01-01T00:00:00Z",
                    "protocolStages": [{
                        "stageId": "s1",
                        "name": "Gather",
                        "order": 1,
                        "dependsOn": [],
                        "question": "What is the topic?"
                    }]
                }),
            },
            None,
        )
        .unwrap();

        let result = blueprint_brief(
            &store,
            BlueprintBriefInput {
                blueprint_id: created.blueprint.id,
            },
        )
        .unwrap();

        assert!(result.protocol.is_some(), "protocol should be found");
        let proto = result.protocol.unwrap();
        assert_eq!(proto.protocol_id, "proto-001");
        assert_eq!(proto.protocol_name, "Article Protocol");
        assert_eq!(proto.stages.len(), 1);
        assert_eq!(proto.stages[0].stage_id, "s1");
        assert_eq!(
            proto.stages[0].question.as_deref(),
            Some("What is the topic?")
        );
    }

    #[test]
    fn brief_stage_from_protocol_stage_maps_purpose() {
        use srs_core::types::protocol::ProtocolStage;
        let v = serde_json::json!({
            "stageId": "s1",
            "name": "Understand",
            "purpose": "builds shared understanding of the problem space",
            "order": 0,
            "dependsOn": []
        });
        let stage: ProtocolStage = serde_json::from_value(v).unwrap();
        let brief = BriefStageResult::from(stage);
        assert_eq!(
            brief.purpose,
            Some("builds shared understanding of the problem space".to_string())
        );
    }

    #[test]
    fn brief_stage_from_protocol_stage_purpose_absent_when_missing() {
        use srs_core::types::protocol::ProtocolStage;
        let v = serde_json::json!({
            "stageId": "s1",
            "name": "Understand",
            "order": 0,
            "dependsOn": []
        });
        let stage: ProtocolStage = serde_json::from_value(v).unwrap();
        let brief = BriefStageResult::from(stage);
        assert_eq!(brief.purpose, None);
    }
}
