use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub instance_id: String,
    pub type_id: String,
    pub type_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    pub field_values: Vec<FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldValue {
    pub field_id: String,
    pub value: serde_json::Value,
}

impl Record {
    /// Find a field value by field_id
    pub fn find_field_value(&self, field_id: &str) -> Option<&FieldValue> {
        self.field_values.iter().find(|fv| fv.field_id == field_id)
    }

    /// Get a string value from a field
    pub fn get_field_value_str(&self, field_id: &str) -> Option<&str> {
        self.find_field_value(field_id)
            .and_then(|fv| fv.value.as_str())
    }

    /// Check if the record has a tag (in the tags list)
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.as_ref()
            .map(|tags| tags.iter().any(|t| t == tag))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_roundtrips_json() {
        let record = Record {
            instance_id: "instance-123".to_string(),
            type_id: "type-abc".to_string(),
            type_version: 1,
            type_namespace: Some("test.ns".to_string()),
            type_name: Some("test-type".to_string()),
            field_values: vec![
                FieldValue {
                    field_id: "field-1".to_string(),
                    value: json!("value1"),
                },
                FieldValue {
                    field_id: "field-2".to_string(),
                    value: json!(42),
                },
            ],
            tags: Some(vec!["tag-a".to_string(), "tag-b".to_string()]),
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            updated_at: None,
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&record).unwrap();
        let parsed: Record = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.instance_id, record.instance_id);
        assert_eq!(parsed.field_values.len(), 2);
        assert_eq!(parsed.tags, Some(vec!["tag-a".to_string(), "tag-b".to_string()]));
    }

    #[test]
    fn record_extra_fields_survive_roundtrip() {
        let json_str = r#"{
            "instanceId": "inst-1",
            "typeId": "type-1",
            "typeVersion": 1,
            "fieldValues": [],
            "$schema": "http://example.com/schema"
        }"#;

        let record: Record = serde_json::from_str(json_str).unwrap();
        assert_eq!(record.extra.get("$schema"), Some(&json!("http://example.com/schema")));

        let serialized = serde_json::to_string(&record).unwrap();
        assert!(serialized.contains("$schema"));
    }

    #[test]
    fn record_null_optional_field_roundtrips() {
        let json_str = r#"{
            "instanceId": "inst-1",
            "typeId": "type-1",
            "typeVersion": 1,
            "fieldValues": [],
            "typeNamespace": null,
            "typeName": "some-name"
        }"#;

        let record: Record = serde_json::from_str(json_str).unwrap();
        // Null values should be preserved in extra or handled gracefully
        assert_eq!(record.type_namespace, None);
        assert_eq!(record.type_name, Some("some-name".to_string()));
    }

    #[test]
    fn find_field_value_works() {
        let record = Record {
            instance_id: "inst-1".to_string(),
            type_id: "type-1".to_string(),
            type_version: 1,
            type_namespace: None,
            type_name: None,
            field_values: vec![
                FieldValue {
                    field_id: "field-a".to_string(),
                    value: json!("value-a"),
                },
            ],
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };

        assert!(record.find_field_value("field-a").is_some());
        assert!(record.find_field_value("unknown").is_none());
        assert_eq!(record.get_field_value_str("field-a"), Some("value-a"));
        assert_eq!(record.get_field_value_str("unknown"), None);
    }

    #[test]
    fn has_tag_works() {
        let record = Record {
            instance_id: "inst-1".to_string(),
            type_id: "type-1".to_string(),
            type_version: 1,
            type_namespace: None,
            type_name: None,
            field_values: vec![],
            tags: Some(vec!["foundation".to_string(), "important".to_string()]),
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };

        assert!(record.has_tag("foundation"));
        assert!(record.has_tag("important"));
        assert!(!record.has_tag("missing"));
    }

    #[test]
    fn multiselect_field_value_is_array() {
        // Multiselect values are stored as arrays of strings
        let fv = FieldValue {
            field_id: "roles".to_string(),
            value: json!(["foundation", "navigation"]),
        };

        let arr = fv.value.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], json!("foundation"));
        assert_eq!(arr[1], json!("navigation"));
    }
}
