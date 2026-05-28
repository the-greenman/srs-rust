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
                },
                FieldValue {
                    field_id: "field-2".to_string(),
                    value: json!(42),
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
        };

        let arr = fv.value.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], json!("foundation"));
        assert_eq!(arr[1], json!("navigation"));
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
}
