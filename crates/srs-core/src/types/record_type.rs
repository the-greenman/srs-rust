use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordType {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub fields: Vec<FieldAssignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldAssignment {
    pub field_id: String,
    pub order: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
}

impl FieldAssignment {
    /// Returns true if the field is required.
    /// None means true (default), explicit Some(false) means optional.
    pub fn is_required(&self) -> bool {
        self.required.unwrap_or(true)
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
            id: "type-id".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-type".to_string(),
            version: 1,
            fields: vec![
                FieldAssignment {
                    field_id: "field-1".to_string(),
                    order: 0,
                    required: None,
                    display_label: Some("Field One".to_string()),
                },
                FieldAssignment {
                    field_id: "field-2".to_string(),
                    order: 1,
                    required: Some(false),
                    display_label: None,
                },
            ],
            description: Some("A test type".to_string()),
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&record_type).unwrap();
        let parsed: RecordType = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, record_type.id);
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(parsed.fields[0].field_id, "field-1");
        assert_eq!(parsed.fields[1].required, Some(false));
    }

    #[test]
    fn field_assignment_required_defaults_to_true() {
        let fa_none = FieldAssignment {
            field_id: "f1".to_string(),
            order: 0,
            required: None,
            display_label: None,
        };
        let fa_true = FieldAssignment {
            field_id: "f2".to_string(),
            order: 1,
            required: Some(true),
            display_label: None,
        };
        let fa_false = FieldAssignment {
            field_id: "f3".to_string(),
            order: 2,
            required: Some(false),
            display_label: None,
        };

        assert!(fa_none.is_required());
        assert!(fa_true.is_required());
        assert!(!fa_false.is_required());
    }

    #[test]
    fn find_field_assignment_works() {
        let rt = RecordType {
            id: "type-id".to_string(),
            namespace: "ns".to_string(),
            name: "name".to_string(),
            version: 1,
            fields: vec![
                FieldAssignment {
                    field_id: "field-1".to_string(),
                    order: 0,
                    required: None,
                    display_label: None,
                },
            ],
            description: None,
            extra: HashMap::new(),
        };

        assert!(rt.find_field_assignment("field-1").is_some());
        assert!(rt.find_field_assignment("unknown").is_none());
    }
}
