use crate::error::CoreError;
use crate::types::record::Record;
use crate::types::record_type::RecordType;
use std::collections::HashSet;

/// Validates a record against its record type definition.
/// 
/// Checks:
/// - All FieldAssignments where `required` is `true` or `None` have a matching FieldValue
/// - All FieldValues reference a `field_id` that exists in the RecordType's `fields`
pub fn validate_record(record: &Record, record_type: &RecordType) -> Result<(), CoreError> {
    // Build a set of valid field IDs from the record type
    let valid_field_ids: HashSet<&str> = record_type
        .fields
        .iter()
        .map(|fa| fa.field_id.as_str())
        .collect();

    // Check for unknown fields in the record
    for field_value in &record.field_values {
        if !valid_field_ids.contains(field_value.field_id.as_str()) {
            return Err(CoreError::UnknownField {
                field_id: field_value.field_id.clone(),
            });
        }
    }

    // Build a set of field IDs present in the record
    let present_field_ids: HashSet<&str> = record
        .field_values
        .iter()
        .map(|fv| fv.field_id.as_str())
        .collect();

    // Check for missing required fields
    for field_assignment in &record_type.fields {
        if field_assignment.is_required()
            && !present_field_ids.contains(field_assignment.field_id.as_str())
        {
            return Err(CoreError::MissingRequiredField {
                field_id: field_assignment.field_id.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::record::{FieldValue, Record};
    use crate::types::record_type::{RecordType, FieldAssignment};
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_record_type() -> RecordType {
        RecordType {
            id: "type-1".to_string(),
            namespace: "test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            fields: vec![
                FieldAssignment {
                    field_id: "required-field".to_string(),
                    order: 0,
                    required: None, // Defaults to true
                    display_label: None,
                },
                FieldAssignment {
                    field_id: "optional-field".to_string(),
                    order: 1,
                    required: Some(false),
                    display_label: None,
                },
                FieldAssignment {
                    field_id: "explicit-required".to_string(),
                    order: 2,
                    required: Some(true),
                    display_label: None,
                },
            ],
            description: None,
            extra: HashMap::new(),
        }
    }

    fn create_record_with_fields(field_values: Vec<FieldValue>) -> Record {
        Record {
            instance_id: "inst-1".to_string(),
            type_id: "type-1".to_string(),
            type_version: 1,
            type_namespace: None,
            type_name: None,
            field_values,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn validate_record_passes_with_all_required_fields() {
        let record_type = create_test_record_type();
        let record = create_record_with_fields(vec![
            FieldValue {
                field_id: "required-field".to_string(),
                value: json!("value1"),
            },
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
            },
        ]);

        assert!(validate_record(&record, &record_type).is_ok());
    }

    #[test]
    fn validate_record_missing_required_field() {
        let record_type = create_test_record_type();
        let record = create_record_with_fields(vec![
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
            },
        ]);

        let result = validate_record(&record, &record_type);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::MissingRequiredField { field_id } if field_id == "required-field"
        ));
    }

    #[test]
    fn validate_record_optional_field_absent_is_ok() {
        let record_type = create_test_record_type();
        let record = create_record_with_fields(vec![
            FieldValue {
                field_id: "required-field".to_string(),
                value: json!("value1"),
            },
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
            },
            // optional-field is absent - this should be fine
        ]);

        assert!(validate_record(&record, &record_type).is_ok());
    }

    #[test]
    fn validate_record_unknown_field() {
        let record_type = create_test_record_type();
        let record = create_record_with_fields(vec![
            FieldValue {
                field_id: "required-field".to_string(),
                value: json!("value1"),
            },
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
            },
            FieldValue {
                field_id: "unknown-field".to_string(),
                value: json!("value3"),
            },
        ]);

        let result = validate_record(&record, &record_type);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::UnknownField { field_id } if field_id == "unknown-field"
        ));
    }
}
