use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub container_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_instance_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_instance_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerIndexEntry {
    pub container_id: String,
    pub title: String,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_roundtrips_all_fields() {
        let mut extra = HashMap::new();
        extra.insert("xCustom".to_string(), serde_json::json!("value"));
        let container = Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Sprint 1".to_string(),
            namespace: Some("team".to_string()),
            name: Some("sprint-1".to_string()),
            description: Some("desc".to_string()),
            container_type: Some("project".to_string()),
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            member_instance_ids: Some(vec!["22222222-2222-4222-8222-222222222222".to_string()]),
            tags: Some(vec!["alpha".to_string()]),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: Some("2026-01-02T00:00:00Z".to_string()),
            meta: Some(serde_json::json!({"k":"v"})),
            extra,
        };

        let json = serde_json::to_string(&container).unwrap();
        let parsed: Container = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, container);
    }

    #[test]
    fn container_minimal_roundtrips() {
        let container = Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Minimal".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        };

        let value = serde_json::to_value(&container).unwrap();
        assert!(value.get("namespace").is_none());
        assert!(value.get("memberInstanceIds").is_none());

        let parsed: Container = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, container);
    }

    #[test]
    fn container_extra_fields_survive() {
        let value = serde_json::json!({
            "containerId": "550e8400-e29b-41d4-a716-446655440000",
            "title": "Extra",
            "xOne": 1,
            "xTwo": {"a": true}
        });

        let parsed: Container = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.extra.get("xOne"), Some(&serde_json::json!(1)));
        assert_eq!(
            parsed.extra.get("xTwo"),
            Some(&serde_json::json!({"a": true}))
        );
    }

    #[test]
    fn container_index_entry_roundtrips() {
        let entry = ContainerIndexEntry {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Sprint 1".to_string(),
            path: "containers/sprint-1-550e8400.json".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ContainerIndexEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }
}
