use super::term::VocabularyEntryStatus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single state in a lifecycle state machine (substrate specialization).
/// `key` is the machine-readable identifier (was `name` in pre-RFC-006 data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleState {
    /// Stable UUID identity. Optional for inline lifecycle blocks; required for standalone Lifecycle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// The machine-readable state key (unified substrate field).
    /// Deserializes from either `"key"` or the legacy `"name"` field.
    #[serde(alias = "name")]
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_initial: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_final: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<VocabularyEntryStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

impl LifecycleState {
    pub fn effective_status(&self) -> &VocabularyEntryStatus {
        self.status
            .as_ref()
            .unwrap_or(&VocabularyEntryStatus::Active)
    }
}

/// A directed transition between lifecycle state keys.
/// `name` is the display label (NOT renamed to `key` — it is not a substrate entry key).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleTransition {
    /// Stable UUID identity for this edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Human-readable display label (e.g. "promote", "approve").
    pub name: String,
    /// Source state key.
    pub from: String,
    /// Target state key.
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

/// A standalone, installable, referenceable lifecycle container.
/// Types may reference this via `lifecycleRef` instead of declaring an inline lifecycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lifecycle {
    pub id: String,
    pub version: u32,
    pub namespace: String,
    pub name: String,
    pub states: Vec<LifecycleState>,
    pub transitions: Vec<LifecycleTransition>,
    pub initial_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends_lifecycle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends_lifecycle_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_state_accepts_key_field() {
        let json = r#"{"key":"draft","isInitial":true}"#;
        let s: LifecycleState = serde_json::from_str(json).unwrap();
        assert_eq!(s.key, "draft");
        assert_eq!(s.is_initial, Some(true));
    }

    #[test]
    fn lifecycle_state_accepts_name_alias() {
        let json = r#"{"name":"draft","isInitial":true}"#;
        let s: LifecycleState = serde_json::from_str(json).unwrap();
        assert_eq!(s.key, "draft");
    }

    #[test]
    fn lifecycle_state_serializes_as_key() {
        let s = LifecycleState {
            id: None,
            version: None,
            namespace: None,
            key: "active".to_string(),
            label: None,
            description: None,
            aliases: None,
            is_initial: None,
            is_final: None,
            status: None,
            properties: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"key\""));
        assert!(!json.contains("\"name\""));
    }

    #[test]
    fn lifecycle_transition_name_unchanged() {
        let t = LifecycleTransition {
            id: Some("t-id".to_string()),
            name: "promote".to_string(),
            from: "draft".to_string(),
            to: "active".to_string(),
            description: None,
            properties: None,
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"name\""));
        assert!(!json.contains("\"key\""));
        let parsed: LifecycleTransition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "promote");
        assert_eq!(parsed.id, Some("t-id".to_string()));
    }

    #[test]
    fn lifecycle_roundtrips_json() {
        let lc = Lifecycle {
            id: "lc-id".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "record-status".to_string(),
            states: vec![
                LifecycleState {
                    id: Some("s1".to_string()),
                    version: None,
                    namespace: None,
                    key: "draft".to_string(),
                    label: Some("Draft".to_string()),
                    description: None,
                    aliases: None,
                    is_initial: Some(true),
                    is_final: None,
                    status: None,
                    properties: None,
                },
                LifecycleState {
                    id: Some("s2".to_string()),
                    version: None,
                    namespace: None,
                    key: "active".to_string(),
                    label: None,
                    description: None,
                    aliases: None,
                    is_initial: None,
                    is_final: Some(true),
                    status: None,
                    properties: None,
                },
            ],
            transitions: vec![LifecycleTransition {
                id: Some("t1".to_string()),
                name: "publish".to_string(),
                from: "draft".to_string(),
                to: "active".to_string(),
                description: None,
                properties: None,
            }],
            initial_state: "draft".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&lc).unwrap();
        let parsed: Lifecycle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.states.len(), 2);
        assert_eq!(parsed.transitions[0].name, "publish");
        assert_eq!(parsed.initial_state, "draft");
    }

    #[test]
    fn lifecycle_state_substrate_fields_roundtrip() {
        let json = r#"{
            "key": "draft",
            "id": "s-uuid",
            "version": 1,
            "namespace": "com.test",
            "aliases": ["new"],
            "status": "active",
            "properties": {"color": "blue"}
        }"#;
        let s: LifecycleState = serde_json::from_str(json).unwrap();
        assert_eq!(s.key, "draft");
        assert_eq!(s.id.as_deref(), Some("s-uuid"));
        assert_eq!(s.aliases, Some(vec!["new".to_string()]));
        assert_eq!(s.status, Some(VocabularyEntryStatus::Active));
        assert!(s.properties.is_some());
    }
}
