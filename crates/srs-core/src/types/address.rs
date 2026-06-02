use serde::{Deserialize, Serialize};

/// A stable, resolvable identifier for any addressable element in the SRS space.
///
/// - `Document`: resolves within a container/record/field/revision hierarchy.
/// - `Process`: stub for ext:protocol integration (deferred).
/// - `Conversation`: stub for ext:protocol integration (deferred).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "space")]
pub enum Address {
    Document(DocumentAddress),
    Process,
    Conversation,
}

/// A hierarchical address within document space.
///
/// The `container_id` is the only required field. Each additional field narrows
/// the address to a specific record, field, or revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn process_address_stub() {
        let addr = Address::Process;
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v["space"], json!("Process"));
        let parsed: Address = serde_json::from_value(v).unwrap();
        assert!(matches!(parsed, Address::Process));
    }
}
