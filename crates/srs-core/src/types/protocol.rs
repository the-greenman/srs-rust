use serde::{Deserialize, Serialize};

/// Protocol stage definition for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStage {
    pub stage_id: String,
    pub name: String,
    pub order: i32,
    #[serde(default)]
    pub depends_on: Vec<String>,
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

/// Stage summary for protocol stages command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStageSummary {
    pub stage_id: String,
    pub name: String,
    pub order: i32,
    pub depends_on: Vec<String>,
}
