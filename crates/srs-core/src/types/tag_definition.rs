use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Semantic definition of a tag, including its roles and metadata.
///
/// TagDefinition is a core SRS type (peer to `Note`), not a user-defined Tier 2 Record.
/// It gives meaning to raw string tags used on Notes — definitions are additive enrichment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDefinition {
    #[serde(default)]
    pub instance_id: String,
    /// The raw tag string this definition describes. Must be non-empty.
    /// Matches the string used in Note.tags / NoteSection.tags.
    pub tag_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Semantic roles this tag plays. Well-known values: "foundation", "navigation", "lifecycle".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>, // "draft" | "active" | "deprecated" | "obsolete"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl TagDefinition {
    /// Check if this tag definition has the specified role.
    pub fn has_role(&self, role: &str) -> bool {
        self.roles
            .as_ref()
            .map(|rs| rs.iter().any(|r| r == role))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tag_definition_roundtrips_json() {
        let td = TagDefinition {
            instance_id: "test-id-123".to_string(),
            tag_key: "foundation".to_string(),
            label: Some("Foundation".to_string()),
            description: Some("Core signal tags for AI context selection.".to_string()),
            roles: Some(vec!["foundation".to_string(), "navigation".to_string()]),
            aliases: Some(vec!["core".to_string(), "primary".to_string()]),
            status: Some("active".to_string()),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: Some("2026-05-28T12:00:00Z".to_string()),
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&td).unwrap();
        let parsed: TagDefinition = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.instance_id, td.instance_id);
        assert_eq!(parsed.tag_key, td.tag_key);
        assert_eq!(parsed.label, td.label);
        assert_eq!(parsed.description, td.description);
        assert_eq!(parsed.roles, td.roles);
        assert_eq!(parsed.aliases, td.aliases);
        assert_eq!(parsed.status, td.status);
        assert_eq!(parsed.created_at, td.created_at);
        assert_eq!(parsed.updated_at, td.updated_at);
    }

    #[test]
    fn tag_definition_minimal_roundtrips() {
        let td = TagDefinition {
            instance_id: "minimal-id".to_string(),
            tag_key: "minimal".to_string(),
            label: None,
            description: None,
            roles: None,
            aliases: None,
            status: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&td).unwrap();
        assert!(!json_str.contains("label"));
        assert!(!json_str.contains("description"));

        let parsed: TagDefinition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.tag_key, "minimal");
        assert_eq!(parsed.label, None);
        assert_eq!(parsed.roles, None);
    }

    #[test]
    fn tag_definition_extra_fields_survive() {
        let json_str = r#"{
            "instanceId": "test-id",
            "tagKey": "test-tag",
            "unknownFutureField": "preserved",
            "anotherExtra": 42
        }"#;

        let td: TagDefinition = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            td.extra.get("unknownFutureField"),
            Some(&json!("preserved"))
        );
        assert_eq!(td.extra.get("anotherExtra"), Some(&json!(42)));

        let serialized = serde_json::to_string(&td).unwrap();
        assert!(serialized.contains("unknownFutureField"));
        assert!(serialized.contains("anotherExtra"));
    }

    #[test]
    fn has_role_returns_true_when_present() {
        let td = TagDefinition {
            instance_id: "id".to_string(),
            tag_key: "foundation".to_string(),
            label: None,
            description: None,
            roles: Some(vec!["foundation".to_string(), "navigation".to_string()]),
            aliases: None,
            status: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };

        assert!(td.has_role("foundation"));
        assert!(td.has_role("navigation"));
    }

    #[test]
    fn has_role_returns_false_when_absent() {
        let td = TagDefinition {
            instance_id: "id".to_string(),
            tag_key: "foundation".to_string(),
            label: None,
            description: None,
            roles: Some(vec!["foundation".to_string(), "navigation".to_string()]),
            aliases: None,
            status: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };

        assert!(!td.has_role("lifecycle"));
        assert!(!td.has_role("unknown"));
    }

    #[test]
    fn has_role_returns_false_when_no_roles() {
        let td = TagDefinition {
            instance_id: "id".to_string(),
            tag_key: "plain-tag".to_string(),
            label: None,
            description: None,
            roles: None,
            aliases: None,
            status: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };

        assert!(!td.has_role("foundation"));
        assert!(!td.has_role("anything"));
    }
}
