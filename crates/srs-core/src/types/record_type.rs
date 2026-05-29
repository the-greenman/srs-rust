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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_groups: Option<Vec<FieldGroup>>,
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
    #[serde(default)]
    pub repeatable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldGroup {
    pub group_id: String,
    pub order: u32,
    pub fields: Vec<FieldAssignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub repeatable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
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

    pub fn find_field_group(&self, group_id: &str) -> Option<&FieldGroup> {
        self.field_groups
            .as_ref()?
            .iter()
            .find(|g| g.group_id == group_id)
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
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "00000000-0000-4000-8000-000000000011".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
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
            repeatable: false,
            min_items: None,
            max_items: None,
        };
        let fa_false = FieldAssignment {
            field_id: "00000000-0000-4000-8000-000000000011".to_string(),
            order: 1,
            required: false,
            display_label: None,
            repeatable: false,
            min_items: None,
            max_items: None,
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
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
            field_groups: None,
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
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
            field_groups: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let mut value = serde_json::to_value(&rt).unwrap();
        value["$schema"] = serde_json::json!("https://srs.semanticops.com/schema/2.0/type.json");
        reg.validate_by_id(srs_schema::TYPE_SCHEMA_ID, &value)
            .expect("minimal RecordType must pass type.json schema");
    }

    #[test]
    fn field_assignment_repeatable_defaults_to_false() {
        let assignment: FieldAssignment = serde_json::from_value(serde_json::json!({
            "fieldId": "f-1",
            "order": 0,
            "required": false
        }))
        .unwrap();
        assert!(!assignment.repeatable);
    }

    #[test]
    fn field_assignment_repeatable_roundtrips() {
        let assignment = FieldAssignment {
            field_id: "f-1".to_string(),
            order: 0,
            required: false,
            display_label: Some("Field".to_string()),
            repeatable: true,
            min_items: Some(1),
            max_items: Some(3),
        };
        let value = serde_json::to_value(&assignment).unwrap();
        let parsed: FieldAssignment = serde_json::from_value(value).unwrap();
        assert!(parsed.repeatable);
        assert_eq!(parsed.min_items, Some(1));
        assert_eq!(parsed.max_items, Some(3));
    }

    #[test]
    fn field_assignment_repeatable_false_omits_min_max_in_json() {
        let assignment = FieldAssignment {
            field_id: "f-1".to_string(),
            order: 0,
            required: false,
            display_label: None,
            repeatable: false,
            min_items: None,
            max_items: None,
        };
        let value = serde_json::to_value(&assignment).unwrap();
        assert!(value.get("minItems").is_none());
        assert!(value.get("maxItems").is_none());
    }

    #[test]
    fn field_group_roundtrips_json() {
        let group = FieldGroup {
            group_id: "g-1".to_string(),
            order: 0,
            fields: vec![FieldAssignment {
                field_id: "f-1".to_string(),
                order: 0,
                required: true,
                display_label: Some("F1".to_string()),
                repeatable: true,
                min_items: Some(1),
                max_items: Some(2),
            }],
            label: Some("Group".to_string()),
            description: Some("Desc".to_string()),
            required: true,
            repeatable: true,
            min_items: Some(1),
            max_items: Some(3),
        };
        let value = serde_json::to_value(&group).unwrap();
        let parsed: FieldGroup = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.group_id, "g-1");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.min_items, Some(1));
        assert_eq!(parsed.max_items, Some(3));
    }

    #[test]
    fn field_group_optional_fields_absent_when_none() {
        let group = FieldGroup {
            group_id: "g-1".to_string(),
            order: 0,
            fields: vec![],
            label: None,
            description: None,
            required: false,
            repeatable: false,
            min_items: None,
            max_items: None,
        };
        let value = serde_json::to_value(&group).unwrap();
        assert!(value.get("label").is_none());
        assert!(value.get("minItems").is_none());
    }

    #[test]
    fn record_type_with_field_groups_roundtrips() {
        let rt = RecordType {
            id: "id".to_string(),
            namespace: "ns".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
            field_groups: Some(vec![FieldGroup {
                group_id: "g-1".to_string(),
                order: 0,
                fields: vec![],
                label: Some("Group".to_string()),
                description: None,
                required: true,
                repeatable: true,
                min_items: Some(1),
                max_items: None,
            }]),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let value = serde_json::to_value(&rt).unwrap();
        let parsed: RecordType = serde_json::from_value(value).unwrap();
        assert!(parsed.field_groups.is_some());
    }

    #[test]
    fn record_type_without_field_groups_omits_key() {
        let rt = RecordType {
            id: "id".to_string(),
            namespace: "ns".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
            field_groups: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let value = serde_json::to_value(&rt).unwrap();
        assert!(value.get("fieldGroups").is_none());
    }

    #[test]
    fn find_field_group_returns_correct_group() {
        let rt = RecordType {
            id: "id".to_string(),
            namespace: "ns".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
            field_groups: Some(vec![
                FieldGroup {
                    group_id: "g-1".to_string(),
                    order: 0,
                    fields: vec![],
                    label: None,
                    description: None,
                    required: false,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldGroup {
                    group_id: "g-2".to_string(),
                    order: 1,
                    fields: vec![],
                    label: None,
                    description: None,
                    required: false,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ]),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        assert_eq!(
            rt.find_field_group("g-2").map(|g| g.group_id.as_str()),
            Some("g-2")
        );
    }

    #[test]
    fn find_field_group_returns_none_for_unknown() {
        let rt = RecordType {
            id: "id".to_string(),
            namespace: "ns".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
            field_groups: Some(vec![]),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        assert!(rt.find_field_group("missing").is_none());
    }

    #[test]
    fn find_field_group_returns_none_when_no_groups() {
        let rt = RecordType {
            id: "id".to_string(),
            namespace: "ns".to_string(),
            name: "n".to_string(),
            version: 1,
            description: "d".to_string(),
            fields: vec![],
            field_groups: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        assert!(rt.find_field_group("missing").is_none());
    }
}
