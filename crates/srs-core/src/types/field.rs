use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub ai_guidance: serde_json::Value,
    pub value_type: ValueType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocabulary_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    String,
    Text,
    Number,
    Boolean,
    Date,
    Url,
    Select,
    Multiselect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn field_roundtrips_json() {
        let field = Field {
            id: "00000000-0000-4000-8000-000000000010".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-field".to_string(),
            version: 1,
            description: "A test field".to_string(),
            ai_guidance: json!({"purpose": "captures test data"}),
            value_type: ValueType::Select,
            allowed_values: Some(vec!["a".to_string(), "b".to_string()]),
            vocabulary_ref: None,
            default_value: Some(json!("a")),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&field).unwrap();
        let parsed: Field = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, field.id);
        assert_eq!(parsed.value_type, ValueType::Select);
        assert_eq!(
            parsed.allowed_values,
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn field_extra_fields_survive_roundtrip() {
        let json_str = r#"{
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test.ns",
            "name": "test-field",
            "version": 1,
            "description": "A field",
            "aiGuidance": {"purpose": "test"},
            "valueType": "string",
            "createdAt": "2026-01-01T00:00:00Z",
            "unknownFutureField": "preserved",
            "anotherExtra": 42
        }"#;

        let field: Field = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            field.extra.get("unknownFutureField"),
            Some(&json!("preserved"))
        );
        assert_eq!(field.extra.get("anotherExtra"), Some(&json!(42)));

        let serialized = serde_json::to_string(&field).unwrap();
        assert!(serialized.contains("unknownFutureField"));
        assert!(serialized.contains("anotherExtra"));
    }

    #[test]
    fn value_type_serializes_to_lowercase() {
        assert_eq!(
            serde_json::to_string(&ValueType::String).unwrap(),
            "\"string\""
        );
        assert_eq!(
            serde_json::to_string(&ValueType::Multiselect).unwrap(),
            "\"multiselect\""
        );
    }

    #[test]
    fn minimal_field_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let field = Field {
            id: "00000000-0000-4000-8000-000000000010".to_string(),
            namespace: "test".to_string(),
            name: "summary".to_string(),
            version: 1,
            description: "A short summary".to_string(),
            ai_guidance: json!({"purpose": "captures the summary"}),
            value_type: ValueType::Text,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let mut value = serde_json::to_value(&field).unwrap();
        value["$schema"] = json!("https://srs.semanticops.com/schema/2.0/field.json");
        reg.validate_by_id(srs_schema::FIELD_SCHEMA_ID, &value)
            .expect("minimal Field must pass field.json schema");
    }
}
