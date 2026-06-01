use serde::{Deserialize, Serialize};

/// Reference to a specific Type, used within Blueprint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TypeRef {
    pub type_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_version: Option<u32>,
    // u32 enforces non-negative; validation rejects Some(0).
}

/// Declares an expected Relation between two Record types within a Blueprint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelationSpec {
    pub relation_type: String,
    pub source_type: TypeRef,
    pub target_type: TypeRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Blueprint definition — the definition of a complete document type for extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blueprint {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,

    /// Entry-point Types to extract from source material.
    pub root_types: Vec<TypeRef>,

    /// Expected Relations between extracted Records.
    #[serde(default)]
    pub structure: Vec<RelationSpec>,

    /// TypeIds that must be present for this blueprint to be considered "complete".
    /// Each entry must resolve to a type in the blueprint's full type universe
    /// (root_types ∪ structure[].sourceType ∪ structure[].targetType).
    #[serde(default)]
    pub required_types: Vec<TypeRef>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    pub created_at: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<serde_json::Value>,
}

/// Blueprint validation diagnostic
#[derive(Debug, Clone)]
pub struct BlueprintDiagnostic {
    pub message: String,
    pub severity: BlueprintDiagnosticSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlueprintDiagnosticSeverity {
    Error,
    Warning,
}

/// Blueprint validation result
#[derive(Debug, Clone)]
pub struct BlueprintValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<BlueprintDiagnostic>,
}

impl BlueprintValidationResult {
    pub fn ok() -> Self {
        Self {
            valid: true,
            diagnostics: vec![],
        }
    }

    pub fn with_error(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            diagnostics: vec![BlueprintDiagnostic {
                message: message.into(),
                severity: BlueprintDiagnosticSeverity::Error,
            }],
        }
    }

    pub fn with_errors(messages: Vec<String>) -> Self {
        Self {
            valid: false,
            diagnostics: messages
                .into_iter()
                .map(|m| BlueprintDiagnostic {
                    message: m,
                    severity: BlueprintDiagnosticSeverity::Error,
                })
                .collect(),
        }
    }
}
