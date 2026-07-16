use serde::{Deserialize, Serialize};

/// A stable, resolvable identifier for any addressable element in the SRS space.
///
/// The `space` tag determines which sub-type the address resolves to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "space")]
pub enum Address {
    Document(DocumentAddress),
    Process(ProcessAddress),
    Conversation(ConversationAddress),
}

/// A hierarchical address within document space.
///
/// `container_id` is the only required field; each additional field narrows
/// the address to a specific record, field, or revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAddress {
    pub container_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// An address within process space (ext:protocol run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessAddress {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

/// An address within conversation space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationAddress {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
}

/// The live cursor of an active Protocol run — mutable state recording where
/// focus currently is. Structurally related to `Address` but serves a distinct
/// role: an `Address` is a stable identifier; `AttentionState` is a mutable
/// cursor that changes as the Protocol advances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionState {
    pub container_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn document_address_container_only() {
        let addr = Address::Document(DocumentAddress {
            container_id: "c-1".to_string(),
            record_id: None,
            field_id: None,
            revision_id: None,
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("Document"));
        assert_eq!(v["containerId"], json!("c-1"));
        assert!(v.get("recordId").is_none());
    }

    #[test]
    fn document_address_full() {
        let addr = Address::Document(DocumentAddress {
            container_id: "c-1".to_string(),
            record_id: Some("r-1".to_string()),
            field_id: Some("f-1".to_string()),
            revision_id: Some("rev-1".to_string()),
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["revisionId"], json!("rev-1"));
        let parsed: Address = serde_json::from_value(v).unwrap();
        if let Address::Document(da) = parsed {
            assert_eq!(da.revision_id.as_deref(), Some("rev-1"));
        } else {
            panic!("expected Document address");
        }
    }

    #[test]
    fn process_address_run_only() {
        let addr = Address::Process(ProcessAddress {
            run_id: "run-1".to_string(),
            stage_id: None,
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("Process"));
        assert_eq!(v["runId"], json!("run-1"));
        assert!(v.get("stageId").is_none());
        let parsed: Address = serde_json::from_value(v).unwrap();
        if let Address::Process(pa) = parsed {
            assert_eq!(pa.run_id, "run-1");
            assert!(pa.stage_id.is_none());
        } else {
            panic!("expected Process address");
        }
    }

    #[test]
    fn process_address_with_stage() {
        let addr = Address::Process(ProcessAddress {
            run_id: "run-1".to_string(),
            stage_id: Some("stage-a".to_string()),
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("Process"));
        assert_eq!(v["runId"], json!("run-1"));
        assert_eq!(v["stageId"], json!("stage-a"));
        let parsed: Address = serde_json::from_value(v).unwrap();
        if let Address::Process(pa) = parsed {
            assert_eq!(pa.stage_id.as_deref(), Some("stage-a"));
        } else {
            panic!("expected Process address");
        }
    }

    #[test]
    fn conversation_address_session_only() {
        let addr = Address::Conversation(ConversationAddress {
            session_id: "sess-1".to_string(),
            chunk_id: None,
            annotation_id: None,
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("Conversation"));
        assert_eq!(v["sessionId"], json!("sess-1"));
        assert!(v.get("chunkId").is_none());
        assert!(v.get("annotationId").is_none());
        let parsed: Address = serde_json::from_value(v).unwrap();
        if let Address::Conversation(ca) = parsed {
            assert_eq!(ca.session_id, "sess-1");
        } else {
            panic!("expected Conversation address");
        }
    }

    #[test]
    fn conversation_address_full() {
        let addr = Address::Conversation(ConversationAddress {
            session_id: "sess-1".to_string(),
            chunk_id: Some("chunk-42".to_string()),
            annotation_id: Some("ann-7".to_string()),
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("Conversation"));
        assert_eq!(v["chunkId"], json!("chunk-42"));
        assert_eq!(v["annotationId"], json!("ann-7"));
        let parsed: Address = serde_json::from_value(v).unwrap();
        if let Address::Conversation(ca) = parsed {
            assert_eq!(ca.chunk_id.as_deref(), Some("chunk-42"));
            assert_eq!(ca.annotation_id.as_deref(), Some("ann-7"));
        } else {
            panic!("expected Conversation address");
        }
    }

    #[test]
    fn attention_state_minimal() {
        let state = AttentionState {
            container_id: "c-1".to_string(),
            record_id: None,
            field_id: None,
            protocol_run_id: None,
            stage_id: None,
        };
        let v = serde_json::to_value(&state).unwrap();
        assert_eq!(v["containerId"], json!("c-1"));
        assert!(v.get("recordId").is_none());
        assert!(v.get("fieldId").is_none());
        assert!(v.get("protocolRunId").is_none());
        assert!(v.get("stageId").is_none());
        let parsed: AttentionState = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.container_id, "c-1");
    }

    #[test]
    fn attention_state_full() {
        let state = AttentionState {
            container_id: "c-1".to_string(),
            record_id: Some("r-1".to_string()),
            field_id: Some("f-1".to_string()),
            protocol_run_id: Some("run-1".to_string()),
            stage_id: Some("stage-a".to_string()),
        };
        let v = serde_json::to_value(&state).unwrap();
        assert_eq!(v["containerId"], json!("c-1"));
        assert_eq!(v["recordId"], json!("r-1"));
        assert_eq!(v["fieldId"], json!("f-1"));
        assert_eq!(v["protocolRunId"], json!("run-1"));
        assert_eq!(v["stageId"], json!("stage-a"));
        let parsed: AttentionState = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.protocol_run_id.as_deref(), Some("run-1"));
        assert_eq!(parsed.stage_id.as_deref(), Some("stage-a"));
    }
}
