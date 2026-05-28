use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordType {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub fields: Vec<FieldAssignment>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldAssignment {
    pub field_id: String,
    pub order: u32,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
}

impl FieldAssignment {
    pub fn is_required(&self) -> bool {
        self.required
    }
}

impl RecordType {
    /// Find a field assignment by field_id
    pub fn find_field_assignment(&self, field_id: &str) -> Option<&FieldAssignment> {
        self.fields.iter().find(|f| f.field_id == field_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_type_roundtrips_json() {
        let record_type = RecordType {
            id: "00000000-0000-4000-8000-000000000020".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "A test type".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "00000000-0000-4000-8000-000000000010".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Field One".to_string()),
                },
                FieldAssignment {
                    field_id: "00000000-0000-4000-8000-000000000011".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                },
            ],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&record_type).unwrap();
        let parsed: RecordType = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, record_type.id);
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(
            parsed.fields[0].field_id,
            "00000000-0000-4000-8000-000000000010"
        );
        assert!(!parsed.fields[1].required);
    }

    #[test]
    fn field_assignment_required_roundtrips() {
        let fa_true = FieldAssignment {
            field_id: "00000000-0000-4000-8000-000000000010".to_string(),
            order: 0,
            required: true,
            display_label: None,
        };
        let fa_false = FieldAssignment {
            field_id: "00000000-0000-4000-8000-000000000011".to_string(),
            order: 1,
            required: false,
            display_label: None,
        };

        assert!(fa_true.is_required());
        assert!(!fa_false.is_required());
    }

    #[test]
    fn find_field_assignment_works() {
        let rt = RecordType {
            id: "00000000-0000-4000-8000-000000000020".to_string(),
            namespace: "ns".to_string(),
            name: "name".to_string(),
            version: 1,
            description: "a type".to_string(),
            fields: vec![FieldAssignment {
                field_id: "00000000-0000-4000-8000-000000000010".to_string(),
                order: 0,
                required: true,
                display_label: None,
            }],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        assert!(rt
            .find_field_assignment("00000000-0000-4000-8000-000000000010")
            .is_some());
        assert!(rt.find_field_assignment("unknown").is_none());
    }

    #[test]
    fn minimal_record_type_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let rt = RecordType {
            id: "00000000-0000-4000-8000-000000000020".to_string(),
            namespace: "test".to_string(),
            name: "decision".to_string(),
            version: 1,
            description: "A decision record type".to_string(),
            fields: vec![FieldAssignment {
                field_id: "00000000-0000-4000-8000-000000000010".to_string(),
                order: 0,
                required: true,
                display_label: None,
            }],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let mut value = serde_json::to_value(&rt).unwrap();
        value["$schema"] = serde_json::json!("https://srs.semanticops.com/schema/2.0/type.json");
        reg.validate_by_id(srs_schema::TYPE_SCHEMA_ID, &value)
            .expect("minimal RecordType must pass type.json schema");
    }
}
