use crate::types::blueprint::TypeRef;
use serde::{Deserialize, Serialize};

/// A reference to a Field within a Type, per ext:protocol FieldRef definition.
///
/// `{ fieldId: UUID, typeId?: UUID }` — `typeId` scopes the field to a specific Type
/// when the same fieldId appears in multiple Types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRef {
    pub field_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
}

/// Protocol stage definition for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<TypeRef>,
}

/// Protocol definition.
///
/// Stored as a Package definition (`Package.protocols[]`, file under
/// `package/protocols/`), exactly parallel to [`crate::types::blueprint::Blueprint`].
/// Per the spec (subsection 05-1-5-1, Invariant 037) Protocols are definitions, not
/// instance Records.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Protocol {
    pub protocol_id: String,
    pub protocol_namespace: String,
    pub protocol_name: String,
    pub protocol_version: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_description: Option<String>,
    pub protocol_target_type: String,
    pub protocol_stages: Vec<ProtocolStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_tags: Option<Vec<String>>,
    pub protocol_created_at: String,
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

    #[test]
    fn test_protocol_stage_output_type_roundtrip() {
        let json = serde_json::json!({
            "stageId": "s1",
            "name": "Draft",
            "order": 1,
            "outputType": {"typeId": "abc-123", "typeVersion": 1}
        });
        let stage: ProtocolStage = serde_json::from_value(json).unwrap();
        assert_eq!(
            stage.output_type,
            Some(TypeRef {
                type_id: "abc-123".to_string(),
                type_version: Some(1),
            })
        );
        let reserialized = serde_json::to_value(&stage).unwrap();
        assert_eq!(reserialized["outputType"]["typeId"], "abc-123");
        assert_eq!(reserialized["outputType"]["typeVersion"], 1);
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
