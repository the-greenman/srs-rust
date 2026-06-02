use serde::{Deserialize, Serialize};

/// An addressable, append-only snapshot of a FieldValue at a point in time.
///
/// Revisions form a chain via `prior_revision_id`. Invariant 33: when
/// `prior_revision_id` is present it must reference a Revision for the same
/// `field_id` and `record_id`, and chains must be acyclic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub revision_id: String,
    pub record_id: String,
    pub field_id: String,
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_revision_id: Option<String>,
    pub agent: RevisionAgent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<RevisionProvenance>,
    pub created_at: String,
}

/// Who or what authored this revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RevisionAgent {
    Human,
    Ai,
    Imported {
        #[serde(rename = "importSource", skip_serializing_if = "Option::is_none")]
        import_source: Option<String>,
    },
}

/// Optional contextual metadata attached to a Revision.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RevisionProvenance {
    /// The lifecycle state name this revision was created during a transition to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_transition: Option<String>,
    /// ISO 8601 timestamp of the lifecycle transition, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transitioned_at: Option<String>,
    /// Source system identifier for imported revisions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_source: Option<String>,
}

/// File format for the per-record revision sidecar.
/// Stored at `records/<instanceId>.revisions.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionSidecar {
    pub record_id: String,
    pub revisions: Vec<Revision>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn revision_roundtrips_json() {
        let rev = Revision {
            revision_id: "rev-1".to_string(),
            record_id: "rec-1".to_string(),
            field_id: "field-1".to_string(),
            value: json!("hello"),
            prior_revision_id: None,
            agent: RevisionAgent::Human,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let v = serde_json::to_value(&rev).unwrap();
        assert_eq!(v["revisionId"], json!("rev-1"));
        assert_eq!(v["agent"]["type"], json!("Human"));
        assert!(v.get("priorRevisionId").is_none());
        let parsed: Revision = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.revision_id, "rev-1");
    }

    #[test]
    fn revision_with_lifecycle_provenance() {
        let rev = Revision {
            revision_id: "rev-2".to_string(),
            record_id: "rec-1".to_string(),
            field_id: "field-1".to_string(),
            value: json!("updated"),
            prior_revision_id: Some("rev-1".to_string()),
            agent: RevisionAgent::Ai,
            provenance: Some(RevisionProvenance {
                lifecycle_transition: Some("active".to_string()),
                transitioned_at: Some("2026-06-01T12:00:00Z".to_string()),
                import_source: None,
            }),
            created_at: "2026-06-01T12:00:00Z".to_string(),
        };
        let v = serde_json::to_value(&rev).unwrap();
        assert_eq!(v["priorRevisionId"], json!("rev-1"));
        assert_eq!(v["provenance"]["lifecycleTransition"], json!("active"));
    }

    #[test]
    fn imported_agent_with_source() {
        let rev = Revision {
            revision_id: "rev-3".to_string(),
            record_id: "rec-1".to_string(),
            field_id: "f1".to_string(),
            value: json!(42),
            prior_revision_id: None,
            agent: RevisionAgent::Imported {
                import_source: Some("legacy-system-v1".to_string()),
            },
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let v = serde_json::to_value(&rev).unwrap();
        assert_eq!(v["agent"]["type"], json!("Imported"));
        assert_eq!(v["agent"]["importSource"], json!("legacy-system-v1"));
    }

    #[test]
    fn revision_sidecar_roundtrips() {
        let sidecar = RevisionSidecar {
            record_id: "rec-1".to_string(),
            revisions: vec![],
        };
        let v = serde_json::to_value(&sidecar).unwrap();
        assert_eq!(v["recordId"], json!("rec-1"));
        let parsed: RevisionSidecar = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.record_id, "rec-1");
        assert!(parsed.revisions.is_empty());
    }
}
