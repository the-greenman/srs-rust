use serde::{Deserialize, Serialize};

/// A stable identifier for any addressable element in the SRS space (core, RFC-049).
///
/// The `space` tag is one of exactly three spaces ([R0]). Every space may carry
/// `repositoryId`; absent means the repository that holds the address ([R13]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "space", rename_all = "lowercase")]
pub enum Address {
    Document(DocumentAddress),
    Process(ProcessAddress),
    Conversation(ConversationAddress),
}

/// An address within document space: a container, a record, or a field of a
/// record. `containerId` is context, not identity, once `recordId` is given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAddress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

/// An address within process space (ext:protocol run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessAddress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

/// An address within conversation space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationAddress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
}

/// UUIDs compare without regard to case (RFC-049).
fn same_id(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

impl Address {
    pub fn repository_id(&self) -> Option<&str> {
        match self {
            Address::Document(a) => a.repository_id.as_deref(),
            Address::Process(a) => a.repository_id.as_deref(),
            Address::Conversation(a) => a.repository_id.as_deref(),
        }
    }

    /// RFC-049 [R0]: `repositoryId` is a UUID in its 36-character hyphenated text
    /// (any case; `urn:` and braced forms are not); a document address names a
    /// container or a record, and a field only within a record.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(id) = self.repository_id() {
            if id.len() != 36 || uuid::Uuid::parse_str(id).is_err() {
                return Err(format!("repositoryId '{id}' is not a UUID"));
            }
        }
        if let Address::Document(a) = self {
            if a.container_id.is_none() && a.record_id.is_none() {
                return Err("a document address needs containerId or recordId".into());
            }
            if a.field_id.is_some() && a.record_id.is_none() {
                return Err("a document address with fieldId needs recordId".into());
            }
        }
        Ok(())
    }

    /// RFC-049 [R12]: equality of what two addresses name. An absent `repositoryId`
    /// stands for `holder`, the repository that holds the address (or that a request
    /// is made against); with a `recordId`, `containerId` is context and is ignored.
    /// Every id component is a UUID and compares without regard to case; `stageId`
    /// is a protocol stage key, not a UUID, and compares exactly.
    pub fn same_as(&self, other: &Address, holder: &str) -> bool {
        let repo = |a: &Address| Some(a.repository_id().unwrap_or(holder).to_string());
        if !same_id(&repo(self), &repo(other)) {
            return false;
        }
        match (self, other) {
            (Address::Document(a), Address::Document(b)) => {
                (a.record_id.is_some() || same_id(&a.container_id, &b.container_id))
                    && same_id(&a.record_id, &b.record_id)
                    && same_id(&a.field_id, &b.field_id)
            }
            (Address::Process(a), Address::Process(b)) => {
                a.run_id.eq_ignore_ascii_case(&b.run_id) && a.stage_id == b.stage_id
            }
            (Address::Conversation(a), Address::Conversation(b)) => {
                a.session_id.eq_ignore_ascii_case(&b.session_id)
                    && same_id(&a.chunk_id, &b.chunk_id)
                    && same_id(&a.annotation_id, &b.annotation_id)
            }
            _ => false,
        }
    }
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

    const REPO: &str = "00000000-0000-4000-a000-0000000000aa";

    fn doc(repo: Option<&str>, c: Option<&str>, r: Option<&str>, f: Option<&str>) -> Address {
        Address::Document(DocumentAddress {
            repository_id: repo.map(str::to_string),
            container_id: c.map(str::to_string),
            record_id: r.map(str::to_string),
            field_id: f.map(str::to_string),
        })
    }

    #[test]
    fn document_address_container_only() {
        let v = serde_json::to_value(doc(None, Some("c-1"), None, None)).unwrap();
        assert_eq!(v["space"], json!("document"));
        assert_eq!(v["containerId"], json!("c-1"));
        assert!(v.get("recordId").is_none());
        assert!(v.get("repositoryId").is_none());
    }

    #[test]
    fn document_address_full_roundtrips() {
        let addr = doc(Some(REPO), Some("c-1"), Some("r-1"), Some("f-1"));
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["repositoryId"], json!(REPO));
        let parsed: Address = serde_json::from_value(v).unwrap();
        assert_eq!(parsed, addr);
    }

    #[test]
    fn r0_document_address_rules() {
        assert!(doc(None, None, Some("r"), None).validate().is_ok());
        assert!(doc(None, Some("c"), None, None).validate().is_ok());
        assert!(doc(None, None, None, None).validate().is_err());
        assert!(doc(None, Some("c"), None, Some("f")).validate().is_err());
        assert!(doc(Some("not-a-uuid"), Some("c"), None, None)
            .validate()
            .is_err());
        let urn = format!("urn:uuid:{REPO}");
        let braced = format!("{{{REPO}}}");
        assert!(doc(Some(&urn), Some("c"), None, None).validate().is_err());
        assert!(doc(Some(&braced), Some("c"), None, None)
            .validate()
            .is_err());
        assert!(doc(Some(&REPO.to_uppercase()), Some("c"), None, None)
            .validate()
            .is_ok());
    }

    #[test]
    fn r0_only_three_spaces() {
        assert!(serde_json::from_value::<Address>(json!({"space": "other", "x": 1})).is_err());
        assert!(
            serde_json::from_value::<Address>(json!({"space": "Document", "recordId": "r"}))
                .is_err()
        );
    }

    #[test]
    fn r12_container_is_context_once_a_record_is_given() {
        let a = doc(None, Some("c1"), Some("r"), None);
        let b = doc(None, Some("c2"), Some("r"), None);
        assert!(a.same_as(&b, REPO));
        assert!(
            !doc(None, Some("c1"), None, None).same_as(&doc(None, Some("c2"), None, None), REPO)
        );
    }

    #[test]
    fn r12_absent_repository_is_the_holder_and_ids_ignore_case() {
        let held = doc(None, None, Some("r"), None);
        let own = doc(Some(&REPO.to_uppercase()), None, Some("R"), None);
        let other = doc(
            Some("00000000-0000-4000-a000-0000000000bb"),
            None,
            Some("r"),
            None,
        );
        assert!(held.same_as(&own, REPO));
        assert!(!held.same_as(&other, REPO));
    }

    #[test]
    fn process_address_run_only() {
        let addr = Address::Process(ProcessAddress {
            repository_id: None,
            run_id: "run-1".to_string(),
            stage_id: None,
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("process"));
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
            repository_id: None,
            run_id: "run-1".to_string(),
            stage_id: Some("stage-a".to_string()),
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("process"));
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
            repository_id: None,
            session_id: "sess-1".to_string(),
            chunk_id: None,
            annotation_id: None,
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("conversation"));
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
            repository_id: None,
            session_id: "sess-1".to_string(),
            chunk_id: Some("chunk-42".to_string()),
            annotation_id: Some("ann-7".to_string()),
        });
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("conversation"));
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

    #[test]
    fn r12_stage_key_compares_exactly_and_run_id_ignores_case() {
        let p = |run: &str, stage: &str| {
            Address::Process(ProcessAddress {
                repository_id: None,
                run_id: run.into(),
                stage_id: Some(stage.into()),
            })
        };
        assert!(p("abc", "draft").same_as(&p("ABC", "draft"), REPO));
        assert!(!p("abc", "draft").same_as(&p("abc", "Draft"), REPO));
    }
}
