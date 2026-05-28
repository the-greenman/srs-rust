use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub value_type: ValueType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
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
            id: "test-id".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-field".to_string(),
            version: 1,
            value_type: ValueType::Select,
            description: Some("A test field".to_string()),
            ai_guidance: None,
            allowed_values: Some(vec!["a".to_string(), "b".to_string()]),
            default_value: Some(json!("a")),
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&field).unwrap();
        let parsed: Field = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, field.id);
        assert_eq!(parsed.value_type, ValueType::Select);
        assert_eq!(parsed.allowed_values, Some(vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn field_extra_fields_survive_roundtrip() {
        let json_str = r#"{
            "id": "test-id",
            "namespace": "test.ns",
            "name": "test-field",
            "version": 1,
            "valueType": "string",
            "unknownFutureField": "preserved",
            "anotherExtra": 42
        }"#;

        let field: Field = serde_json::from_str(json_str).unwrap();
        assert_eq!(field.extra.get("unknownFutureField"), Some(&json!("preserved")));
        assert_eq!(field.extra.get("anotherExtra"), Some(&json!(42)));

        let serialized = serde_json::to_string(&field).unwrap();
        assert!(serialized.contains("unknownFutureField"));
        assert!(serialized.contains("anotherExtra"));
    }

    #[test]
    fn value_type_serializes_to_lowercase() {
        assert_eq!(serde_json::to_string(&ValueType::String).unwrap(), "\"string\"");
        assert_eq!(serde_json::to_string(&ValueType::Multiselect).unwrap(), "\"multiselect\"");
    }
}
