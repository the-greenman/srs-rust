use crate::types::field::AiGuidance;
use serde::{Deserialize, Serialize};

/// A reference to a Field within a Type, per ext:protocol FieldRef definition.
///
/// `{ fieldId: UUID, typeId?: UUID }` — `typeId` scopes the field to a specific Type
/// when the same fieldId appears in multiple Types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldRef {
    pub field_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
}

/// Protocol stage definition for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolStage {
    pub stage_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub order: i32,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_criteria: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributes_to: Option<Vec<FieldRef>>,
    /// Extraction guidance for an AI assistant working this stage. The closed,
    /// structured [`AiGuidance`] object used everywhere else in the model —
    /// no plain-string alternative (rfc-decision-a270e98a, srs#379: structured
    /// over serialised; `Field.aiGuidance` is core/typed/always with no string
    /// escape hatch, and `ProtocolStage` gets none either).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<AiGuidance>,
    /// srs#487/#868: a bare-UUID LINEAGE reference (rfc-decision-c8704763) — `typeVersion`
    /// dropped, the pre-RFC-009 version-optional `TypeRef` object form retired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<String>,
}

/// Protocol definition.
///
/// Stored as a Package definition (`Package.protocols[]`, file under
/// `package/protocols/`), exactly parallel to [`crate::types::blueprint::Blueprint`].
/// Per the spec (subsection 05-1-5-1, Invariant 037) Protocols are definitions, not
/// instance Records.
///
/// Identity/structural property names are UNPREFIXED (`id`, `namespace`, `name`,
/// `version`, `description`, `targetType`, `stages`, `tags`, `createdAt`), matching
/// every other package-declared definition entity — Type, Field, Vocabulary,
/// Lifecycle, RelationTypeDefinition, Theme, Blueprint all reuse the shared
/// identity shape unprefixed. The interim `protocol`-prefixed shape (#297/#378)
/// is retired (rfc-decision-a270e98a, srs#379; srs-rust#930).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Protocol {
    /// The `$schema` pointer the file may carry — declared by the schema itself,
    /// preserved so a loaded-then-written definition keeps it.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A LINEAGE reference (bare Type UUID, or `""`; rfc-decision-c8704763) —
    /// value shape unchanged by the rfc-decision-a270e98a rename, property name only.
    pub target_type: String,
    pub stages: Vec<ProtocolStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: String,
}

/// Protocol validation diagnostic
#[derive(Debug, Clone)]
pub struct ProtocolDiagnostic {
    pub message: String,
    pub severity: ProtocolDiagnosticSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolDiagnosticSeverity {
    Error,
    Warning,
}

/// Status of a protocol run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProtocolRunStatus {
    Active,
    Completed,
    Abandoned,
}

/// Status of an individual stage within a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StageStatus {
    Pending,
    Active,
    Completed,
    Skipped,
}

/// Per-stage execution state captured on each advance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageState {
    pub stage_id: String,
    pub status: StageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// An instance of a Protocol execution within a repository.
///
/// Created on `protocol run create`; advanced via `protocol run advance`.
/// `attention_state` is updated on every stage advance, satisfying the spec
/// mandate (subsection 08-5-2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRun {
    pub run_id: String,
    pub protocol_id: String,
    pub protocol_version: i32,
    pub container_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_record_id: Option<String>,
    pub status: ProtocolRunStatus,
    pub attention_state: crate::types::address::AttentionState,
    pub stage_states: Vec<StageState>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Top-level wrapper for `runs/runs.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRunsCollection {
    pub runs: Vec<ProtocolRun>,
}

/// Protocol validation result
#[derive(Debug, Clone)]
pub struct ProtocolValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<ProtocolDiagnostic>,
}

impl ProtocolValidationResult {
    pub fn ok() -> Self {
        Self {
            valid: true,
            diagnostics: vec![],
        }
    }

    pub fn with_error(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            diagnostics: vec![ProtocolDiagnostic {
                message: message.into(),
                severity: ProtocolDiagnosticSeverity::Error,
            }],
        }
    }

    pub fn with_errors(messages: Vec<String>) -> Self {
        Self {
            valid: false,
            diagnostics: messages
                .into_iter()
                .map(|m| ProtocolDiagnostic {
                    message: m,
                    severity: ProtocolDiagnosticSeverity::Error,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::AttentionState;

    fn make_run(run_id: &str, stage_id: Option<&str>) -> ProtocolRun {
        ProtocolRun {
            run_id: run_id.to_string(),
            protocol_id: "proto-1".to_string(),
            protocol_version: 1,
            container_id: "c-1".to_string(),
            target_record_id: None,
            status: ProtocolRunStatus::Active,
            attention_state: AttentionState {
                container_id: "c-1".to_string(),
                record_id: None,
                field_id: None,
                protocol_run_id: Some(run_id.to_string()),
                stage_id: stage_id.map(|s| s.to_string()),
            },
            stage_states: stage_id
                .map(|sid| {
                    vec![StageState {
                        stage_id: sid.to_string(),
                        status: StageStatus::Active,
                        started_at: Some("2026-01-01T00:00:00Z".to_string()),
                        completed_at: None,
                    }]
                })
                .unwrap_or_default(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            completed_at: None,
        }
    }

    #[test]
    fn protocol_run_roundtrip() {
        let run = ProtocolRun {
            run_id: "run-1".to_string(),
            protocol_id: "proto-1".to_string(),
            protocol_version: 2,
            container_id: "c-1".to_string(),
            target_record_id: Some("rec-1".to_string()),
            status: ProtocolRunStatus::Active,
            attention_state: AttentionState {
                container_id: "c-1".to_string(),
                record_id: Some("rec-1".to_string()),
                field_id: None,
                protocol_run_id: Some("run-1".to_string()),
                stage_id: Some("stage-a".to_string()),
            },
            stage_states: vec![StageState {
                stage_id: "stage-a".to_string(),
                status: StageStatus::Active,
                started_at: Some("2026-01-01T00:00:00Z".to_string()),
                completed_at: None,
            }],
            started_at: "2026-01-01T00:00:00Z".to_string(),
            completed_at: None,
        };
        let v = serde_json::to_value(&run).unwrap();
        let parsed: ProtocolRun = serde_json::from_value(v).unwrap();
        assert_eq!(parsed, run);
        assert_eq!(parsed.target_record_id.as_deref(), Some("rec-1"));
        assert_eq!(parsed.attention_state.stage_id.as_deref(), Some("stage-a"));
    }

    #[test]
    fn protocol_run_roundtrip_minimal() {
        let run = make_run("run-min", None);
        let v = serde_json::to_value(&run).unwrap();
        assert!(
            v.get("targetRecordId").is_none(),
            "absent optional must not appear"
        );
        assert!(
            v.get("completedAt").is_none(),
            "absent optional must not appear"
        );
        let parsed: ProtocolRun = serde_json::from_value(v).unwrap();
        assert_eq!(parsed, run);
    }

    #[test]
    fn stage_state_status_serializes_pascal_case() {
        let v = serde_json::to_value(StageStatus::Completed).unwrap();
        assert_eq!(v, serde_json::json!("Completed"));
        let v2 = serde_json::to_value(ProtocolRunStatus::Abandoned).unwrap();
        assert_eq!(v2, serde_json::json!("Abandoned"));
    }

    #[test]
    fn protocol_runs_collection_roundtrip() {
        let col = ProtocolRunsCollection {
            runs: vec![make_run("run-a", Some("stage-1")), make_run("run-b", None)],
        };
        let v = serde_json::to_value(&col).unwrap();
        assert_eq!(v["runs"].as_array().unwrap().len(), 2);
        let parsed: ProtocolRunsCollection = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.runs.len(), 2);
        assert_eq!(parsed.runs[0].run_id, "run-a");
    }

    #[test]
    fn test_protocol_stage_output_type_roundtrip() {
        let json = serde_json::json!({
            "stageId": "s1",
            "name": "Draft",
            "order": 1,
            "outputType": "abc-123"
        });
        let stage: ProtocolStage = serde_json::from_value(json).unwrap();
        assert_eq!(stage.output_type, Some("abc-123".to_string()));
        let reserialized = serde_json::to_value(&stage).unwrap();
        assert_eq!(reserialized["outputType"], "abc-123");
    }

    #[test]
    fn test_protocol_stage_output_type_absent() {
        let stage = ProtocolStage {
            stage_id: "s1".to_string(),
            name: "Draft".to_string(),
            purpose: None,
            order: 1,
            depends_on: vec![],
            question: None,
            completion_criteria: None,
            contributes_to: None,
            ai_guidance: None,
            output_type: None,
        };
        let json = serde_json::to_string(&stage).unwrap();
        assert!(
            !json.contains("outputType"),
            "absent output_type must not appear in JSON: {json}"
        );
    }

    #[test]
    fn test_field_ref_deserialization() {
        let with_type_id: FieldRef =
            serde_json::from_value(serde_json::json!({"fieldId": "abc", "typeId": "xyz"})).unwrap();
        assert_eq!(
            with_type_id,
            FieldRef {
                field_id: "abc".to_string(),
                type_id: Some("xyz".to_string()),
            }
        );

        let without_type_id: FieldRef =
            serde_json::from_value(serde_json::json!({"fieldId": "abc"})).unwrap();
        assert_eq!(
            without_type_id,
            FieldRef {
                field_id: "abc".to_string(),
                type_id: None,
            }
        );
    }
}
