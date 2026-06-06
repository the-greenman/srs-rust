use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub instance_id: String,
    pub type_id: String,
    pub type_version: u32,
    pub type_namespace: String,
    pub type_name: String,
    pub field_values: Vec<FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_values: Option<Vec<FieldGroupValue>>,
    /// ext:lifecycle — current lifecycle state. Must name a state in the associated
    /// Type's lifecycle.states[] when the Type declares a lifecycle (Invariant 6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// Topic membership labels. When a Vocabulary is declared, each value MUST resolve to a
    /// Term key or alias (tier-graduated: Records enforce resolution; Notes do not).
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
pub struct FieldValueEntry {
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldValue {
    pub field_id: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<FieldValueEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldGroupEntry {
    pub field_values: Vec<FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldGroupValue {
    pub group_id: String,
    pub entries: Vec<FieldGroupEntry>,
}

impl Record {
    /// Find a field value by field_id
    pub fn find_field_value(&self, field_id: &str) -> Option<&FieldValue> {
        self.field_values.iter().find(|fv| fv.field_id == field_id)
    }

    pub fn find_group_value(&self, group_id: &str) -> Option<&FieldGroupValue> {
        self.group_values
            .as_ref()?
            .iter()
            .find(|gv| gv.group_id == group_id)
    }

    /// Get a string value from a field
    pub fn get_field_value_str(&self, field_id: &str) -> Option<&str> {
        self.find_field_value(field_id)
            .and_then(|fv| fv.value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_record() -> Record {
        Record {
            instance_id: "00000000-0000-4000-8000-000000000001".to_string(),
            type_id: "00000000-0000-4000-8000-000000000002".to_string(),
            type_version: 1,
            type_namespace: "test.ns".to_string(),
            type_name: "test-type".to_string(),
            field_values: vec![],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn record_roundtrips_json() {
        let record = Record {
            field_values: vec![
                FieldValue {
                    field_id: "field-1".to_string(),
                    value: json!("value1"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "field-2".to_string(),
                    value: json!(42),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            ..minimal_record()
        };

        let json_str = serde_json::to_string(&record).unwrap();
        let parsed: Record = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.instance_id, record.instance_id);
        assert_eq!(parsed.type_namespace, "test.ns");
        assert_eq!(parsed.type_name, "test-type");
        assert_eq!(parsed.field_values.len(), 2);
    }

    #[test]
    fn record_extra_fields_survive_roundtrip() {
        let json_str = r#"{
            "instanceId": "00000000-0000-4000-8000-000000000001",
            "typeId": "00000000-0000-4000-8000-000000000002",
            "typeVersion": 1,
            "typeNamespace": "test.ns",
            "typeName": "test-type",
            "fieldValues": [],
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json"
        }"#;

        let record: Record = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            record.extra.get("$schema"),
            Some(&json!("https://srs.semanticops.com/schema/2.0/record.json"))
        );

        let serialized = serde_json::to_string(&record).unwrap();
        assert!(serialized.contains("$schema"));
    }

    #[test]
    fn find_field_value_works() {
        let record = Record {
            field_values: vec![FieldValue {
                field_id: "field-a".to_string(),
                value: json!("value-a"),
                entries: None,
                source: None,
                edited_at: None,
            }],
            ..minimal_record()
        };

        assert!(record.find_field_value("field-a").is_some());
        assert!(record.find_field_value("unknown").is_none());
        assert_eq!(record.get_field_value_str("field-a"), Some("value-a"));
        assert_eq!(record.get_field_value_str("unknown"), None);
    }

    #[test]
    fn multiselect_field_value_is_array() {
        let fv = FieldValue {
            field_id: "roles".to_string(),
            value: json!(["foundation", "navigation"]),
            entries: None,
            source: None,
            edited_at: None,
        };

        let arr = fv.value.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], json!("foundation"));
        assert_eq!(arr[1], json!("navigation"));
    }

    #[test]
    fn lifecycle_state_roundtrips_and_not_in_extra() {
        let record = Record {
            lifecycle_state: Some("active".to_string()),
            ..minimal_record()
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["lifecycleState"], json!("active"));

        let parsed: Record = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.lifecycle_state.as_deref(), Some("active"));
        assert!(!parsed.extra.contains_key("lifecycleState"));
    }

    #[test]
    fn lifecycle_state_absent_omits_key() {
        let record = minimal_record();
        let value = serde_json::to_value(&record).unwrap();
        assert!(value.get("lifecycleState").is_none());
    }

    #[test]
    fn minimal_record_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let record = minimal_record();
        let mut value = serde_json::to_value(&record).unwrap();
        value["$schema"] = json!("https://srs.semanticops.com/schema/2.0/record.json");
        reg.validate_by_id(srs_schema::RECORD_SCHEMA_ID, &value)
            .expect("minimal Record must pass record.json schema");
    }

    #[test]
    fn field_value_entry_roundtrips_json() {
        let entry = FieldValueEntry {
            value: json!("hello"),
            source: Some("human".to_string()),
            edited_at: None,
        };
        let value = serde_json::to_value(&entry).unwrap();
        let parsed: FieldValueEntry = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.source.as_deref(), Some("human"));
        assert_eq!(parsed.value, json!("hello"));
    }

    #[test]
    fn field_value_entries_roundtrips() {
        let field_value = FieldValue {
            field_id: "f1".to_string(),
            value: json!("primary"),
            entries: Some(vec![FieldValueEntry {
                value: json!("v1"),
                source: Some("human".to_string()),
                edited_at: Some("2026-01-01T00:00:00Z".to_string()),
            }]),
            source: None,
            edited_at: None,
        };
        let value = serde_json::to_value(&field_value).unwrap();
        let parsed: FieldValue = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.entries.as_ref().map(std::vec::Vec::len), Some(1));
    }

    #[test]
    fn field_value_without_entries_omits_entries_key() {
        let field_value = FieldValue {
            field_id: "f1".to_string(),
            value: json!("primary"),
            entries: None,
            source: None,
            edited_at: None,
        };
        let value = serde_json::to_value(&field_value).unwrap();
        assert!(value.get("entries").is_none());
    }

    #[test]
    fn field_group_value_roundtrips_json() {
        let value = FieldGroupValue {
            group_id: "g1".to_string(),
            entries: vec![
                FieldGroupEntry {
                    field_values: vec![FieldValue {
                        field_id: "f1".to_string(),
                        value: json!("a"),
                        entries: None,
                        source: None,
                        edited_at: None,
                    }],
                    entry_id: Some("e1".to_string()),
                },
                FieldGroupEntry {
                    field_values: vec![FieldValue {
                        field_id: "f2".to_string(),
                        value: json!("b"),
                        entries: None,
                        source: None,
                        edited_at: None,
                    }],
                    entry_id: Some("e2".to_string()),
                },
            ],
        };
        let json_value = serde_json::to_value(&value).unwrap();
        let parsed: FieldGroupValue = serde_json::from_value(json_value).unwrap();
        assert_eq!(parsed.entries.len(), 2);
    }

    #[test]
    fn record_with_group_values_roundtrips() {
        let record = Record {
            group_values: Some(vec![FieldGroupValue {
                group_id: "g1".to_string(),
                entries: vec![FieldGroupEntry {
                    field_values: vec![],
                    entry_id: None,
                }],
            }]),
            ..minimal_record()
        };
        let value = serde_json::to_value(&record).unwrap();
        let parsed: Record = serde_json::from_value(value).unwrap();
        assert!(parsed.group_values.is_some());
    }

    #[test]
    fn record_without_group_values_omits_key() {
        let record = minimal_record();
        let value = serde_json::to_value(&record).unwrap();
        assert!(value.get("groupValues").is_none());
    }

    #[test]
    fn find_group_value_returns_correct_group() {
        let record = Record {
            group_values: Some(vec![
                FieldGroupValue {
                    group_id: "g1".to_string(),
                    entries: vec![],
                },
                FieldGroupValue {
                    group_id: "g2".to_string(),
                    entries: vec![],
                },
            ]),
            ..minimal_record()
        };
        assert_eq!(
            record.find_group_value("g2").map(|g| g.group_id.as_str()),
            Some("g2")
        );
    }

    #[test]
    fn find_group_value_returns_none_for_unknown() {
        let record = Record {
            group_values: Some(vec![FieldGroupValue {
                group_id: "g1".to_string(),
                entries: vec![],
            }]),
            ..minimal_record()
        };
        assert!(record.find_group_value("missing").is_none());
    }
}
