use crate::error::CoreError;
use crate::types::record::Record;
use crate::types::record_type::{RecordType, TypeLifecycle};
use std::collections::HashSet;

/// Validates a record against its record type definition.
///
/// Checks:
/// - All FieldAssignments where `required` is `true` have a matching FieldValue
/// - All FieldValues reference a `field_id` that exists in the RecordType's `fields`
pub fn validate_record(record: &Record, record_type: &RecordType) -> Result<(), CoreError> {
    let valid_field_ids: HashSet<&str> = record_type
        .fields
        .iter()
        .map(|fa| fa.field_id.as_str())
        .collect();

    for field_value in &record.field_values {
        if !valid_field_ids.contains(field_value.field_id.as_str()) {
            return Err(CoreError::UnknownField {
                field_id: field_value.field_id.clone(),
            });
        }
    }

    let present_field_ids: HashSet<&str> = record
        .field_values
        .iter()
        .map(|fv| fv.field_id.as_str())
        .collect();

    for field_assignment in &record_type.fields {
        if field_assignment.is_required()
            && !present_field_ids.contains(field_assignment.field_id.as_str())
        {
            return Err(CoreError::MissingRequiredField {
                field_id: field_assignment.field_id.clone(),
            });
        }
    }

    for field_assignment in &record_type.fields {
        let Some(field_value) = record.find_field_value(&field_assignment.field_id) else {
            continue;
        };

        if !field_assignment.repeatable && field_value.entries.is_some() {
            return Err(CoreError::EntriesOnNonRepeatableField {
                field_id: field_assignment.field_id.clone(),
            });
        }

        if field_assignment.repeatable {
            let count = field_value.entries.as_ref().map(|e| e.len()).unwrap_or(0);
            if let Some(min) = field_assignment.min_items {
                if count < min as usize {
                    return Err(CoreError::TooFewEntries {
                        field_id: field_assignment.field_id.clone(),
                        count,
                        min,
                    });
                }
            }
            if let Some(max) = field_assignment.max_items {
                if count > max as usize {
                    return Err(CoreError::TooManyEntries {
                        field_id: field_assignment.field_id.clone(),
                        count,
                        max,
                    });
                }
            }
        }
    }

    if let Some(field_groups) = &record_type.field_groups {
        for group in field_groups {
            let group_value = record.find_group_value(&group.group_id);
            if group.required && group_value.is_none() {
                return Err(CoreError::MissingRequiredFieldGroup {
                    group_id: group.group_id.clone(),
                });
            }
            if let Some(group_value) = group_value {
                let count = group_value.entries.len();
                if let Some(min) = group.min_items {
                    if count < min as usize {
                        return Err(CoreError::TooFewGroupEntries {
                            group_id: group.group_id.clone(),
                            count,
                            min,
                        });
                    }
                }
                if let Some(max) = group.max_items {
                    if count > max as usize {
                        return Err(CoreError::TooManyGroupEntries {
                            group_id: group.group_id.clone(),
                            count,
                            max,
                        });
                    }
                }
            }
        }
    }

    // Invariant 6 (ext:lifecycle): Record.lifecycleState must name a state in the
    // associated Type's lifecycle.states[] when the Type declares a lifecycle.
    if let (Some(state), Some(lc)) = (&record.lifecycle_state, &record_type.lifecycle) {
        let valid = lc.states.iter().any(|s| &s.name == state);
        if !valid {
            return Err(CoreError::InvalidLifecycleState {
                state: state.clone(),
            });
        }
    }

    Ok(())
}

/// Validate a Type's lifecycle definition (Invariants 4 and 5, ext:lifecycle).
///
/// - Invariant 4: `initialState` must name a state with `isInitial: true`.
/// - Invariant 5: All `from`/`to` in `transitions[]` must name valid states.
pub fn validate_type_lifecycle(lifecycle: &TypeLifecycle) -> Result<(), CoreError> {
    let state_names: HashSet<&str> = lifecycle.states.iter().map(|s| s.name.as_str()).collect();

    // Invariant 4
    let initial_state = lifecycle
        .states
        .iter()
        .find(|s| s.name == lifecycle.initial_state);
    match initial_state {
        None => {
            return Err(CoreError::InvalidLifecycleInitialState {
                state: lifecycle.initial_state.clone(),
            })
        }
        Some(s) if s.is_initial != Some(true) => {
            return Err(CoreError::InvalidLifecycleInitialState {
                state: lifecycle.initial_state.clone(),
            })
        }
        _ => {}
    }

    // Invariant 5
    for transition in &lifecycle.transitions {
        if !state_names.contains(transition.from.as_str()) {
            return Err(CoreError::InvalidLifecycleTransitionState {
                state: transition.from.clone(),
                transition_name: transition.name.clone(),
            });
        }
        if !state_names.contains(transition.to.as_str()) {
            return Err(CoreError::InvalidLifecycleTransitionState {
                state: transition.to.clone(),
                transition_name: transition.name.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::record::{
        FieldGroupEntry, FieldGroupValue, FieldValue, FieldValueEntry, Record,
    };
    use crate::types::record_type::{FieldAssignment, FieldGroup, RecordType};
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_record_type() -> RecordType {
        RecordType {
            id: "type-1".to_string(),
            namespace: "test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test type".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "required-field".to_string(),
                    order: 0,
                    required: true,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "optional-field".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "explicit-required".to_string(),
                    order: 2,
                    required: true,
                    display_label: None,
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
            lifecycle: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn create_record_with_fields(field_values: Vec<FieldValue>) -> Record {
        Record {
            instance_id: "inst-1".to_string(),
            type_id: "type-1".to_string(),
            type_version: 1,
            type_namespace: "test".to_string(),
            type_name: "test-type".to_string(),
            field_values,
            group_values: None,
            lifecycle_state: None,
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
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ]);

        assert!(validate_record(&record, &record_type).is_ok());
    }

    #[test]
    fn validate_record_missing_required_field() {
        let record_type = create_test_record_type();
        let record = create_record_with_fields(vec![FieldValue {
            field_id: "explicit-required".to_string(),
            value: json!("value2"),
            entries: None,
            source: None,
            edited_at: None,
        }]);

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
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
                entries: None,
                source: None,
                edited_at: None,
            },
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
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "explicit-required".to_string(),
                value: json!("value2"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "unknown-field".to_string(),
                value: json!("value3"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ]);

        let result = validate_record(&record, &record_type);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::UnknownField { field_id } if field_id == "unknown-field"
        ));
    }

    #[test]
    fn validate_repeatable_field_entry_count_ok() {
        let mut record_type = create_test_record_type();
        record_type.fields = vec![FieldAssignment {
            field_id: "repeatable".to_string(),
            order: 0,
            required: true,
            display_label: None,
            repeatable: true,
            min_items: Some(1),
            max_items: Some(3),
        }];
        let record = create_record_with_fields(vec![FieldValue {
            field_id: "repeatable".to_string(),
            value: json!("ignored"),
            entries: Some(vec![
                FieldValueEntry {
                    value: json!("a"),
                    source: None,
                    edited_at: None,
                },
                FieldValueEntry {
                    value: json!("b"),
                    source: None,
                    edited_at: None,
                },
            ]),
            source: None,
            edited_at: None,
        }]);
        assert!(validate_record(&record, &record_type).is_ok());
    }

    #[test]
    fn validate_repeatable_field_too_few_entries() {
        let mut record_type = create_test_record_type();
        record_type.fields = vec![FieldAssignment {
            field_id: "repeatable".to_string(),
            order: 0,
            required: true,
            display_label: None,
            repeatable: true,
            min_items: Some(2),
            max_items: None,
        }];
        let record = create_record_with_fields(vec![FieldValue {
            field_id: "repeatable".to_string(),
            value: json!("ignored"),
            entries: Some(vec![FieldValueEntry {
                value: json!("a"),
                source: None,
                edited_at: None,
            }]),
            source: None,
            edited_at: None,
        }]);
        assert!(matches!(
            validate_record(&record, &record_type),
            Err(CoreError::TooFewEntries {
                count: 1,
                min: 2,
                ..
            })
        ));
    }

    #[test]
    fn validate_repeatable_field_too_many_entries() {
        let mut record_type = create_test_record_type();
        record_type.fields = vec![FieldAssignment {
            field_id: "repeatable".to_string(),
            order: 0,
            required: true,
            display_label: None,
            repeatable: true,
            min_items: None,
            max_items: Some(2),
        }];
        let record = create_record_with_fields(vec![FieldValue {
            field_id: "repeatable".to_string(),
            value: json!("ignored"),
            entries: Some(vec![
                FieldValueEntry {
                    value: json!("a"),
                    source: None,
                    edited_at: None,
                },
                FieldValueEntry {
                    value: json!("b"),
                    source: None,
                    edited_at: None,
                },
                FieldValueEntry {
                    value: json!("c"),
                    source: None,
                    edited_at: None,
                },
            ]),
            source: None,
            edited_at: None,
        }]);
        assert!(matches!(
            validate_record(&record, &record_type),
            Err(CoreError::TooManyEntries {
                count: 3,
                max: 2,
                ..
            })
        ));
    }

    #[test]
    fn validate_entries_on_non_repeatable_field_fails() {
        let mut record_type = create_test_record_type();
        record_type.fields = vec![FieldAssignment {
            field_id: "single".to_string(),
            order: 0,
            required: true,
            display_label: None,
            repeatable: false,
            min_items: None,
            max_items: None,
        }];
        let record = create_record_with_fields(vec![FieldValue {
            field_id: "single".to_string(),
            value: json!("value"),
            entries: Some(vec![FieldValueEntry {
                value: json!("a"),
                source: None,
                edited_at: None,
            }]),
            source: None,
            edited_at: None,
        }]);
        assert!(matches!(
            validate_record(&record, &record_type),
            Err(CoreError::EntriesOnNonRepeatableField { field_id }) if field_id == "single"
        ));
    }

    #[test]
    fn validate_repeatable_no_min_max_any_count_ok() {
        let mut record_type = create_test_record_type();
        record_type.fields = vec![FieldAssignment {
            field_id: "repeatable".to_string(),
            order: 0,
            required: true,
            display_label: None,
            repeatable: true,
            min_items: None,
            max_items: None,
        }];
        let record = create_record_with_fields(vec![FieldValue {
            field_id: "repeatable".to_string(),
            value: json!("ignored"),
            entries: Some(vec![]),
            source: None,
            edited_at: None,
        }]);
        assert!(validate_record(&record, &record_type).is_ok());
    }

    fn create_field_group_rt(
        required: bool,
        min_items: Option<u32>,
        max_items: Option<u32>,
    ) -> RecordType {
        RecordType {
            id: "type-1".to_string(),
            namespace: "test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test type".to_string(),
            fields: vec![],
            field_groups: Some(vec![FieldGroup {
                group_id: "group-1".to_string(),
                order: 0,
                fields: vec![],
                label: None,
                description: None,
                required,
                repeatable: true,
                min_items,
                max_items,
            }]),
            lifecycle: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn create_group_record(entries_len: usize) -> Record {
        let entries = (0..entries_len)
            .map(|_| FieldGroupEntry {
                field_values: vec![],
                entry_id: None,
            })
            .collect();
        let mut record = create_record_with_fields(vec![]);
        record.group_values = Some(vec![FieldGroupValue {
            group_id: "group-1".to_string(),
            entries,
        }]);
        record
    }

    #[test]
    fn validate_required_field_group_present_ok() {
        let rt = create_field_group_rt(true, Some(1), None);
        let record = create_group_record(1);
        assert!(validate_record(&record, &rt).is_ok());
    }

    #[test]
    fn validate_required_field_group_missing_fails() {
        let rt = create_field_group_rt(true, None, None);
        let mut record = create_record_with_fields(vec![]);
        record.group_values = None;
        assert!(matches!(
            validate_record(&record, &rt),
            Err(CoreError::MissingRequiredFieldGroup { group_id }) if group_id == "group-1"
        ));
    }

    #[test]
    fn validate_optional_field_group_absent_ok() {
        let rt = create_field_group_rt(false, None, None);
        let mut record = create_record_with_fields(vec![]);
        record.group_values = None;
        assert!(validate_record(&record, &rt).is_ok());
    }

    #[test]
    fn validate_field_group_entry_count_too_few() {
        let rt = create_field_group_rt(false, Some(2), None);
        let record = create_group_record(1);
        assert!(matches!(
            validate_record(&record, &rt),
            Err(CoreError::TooFewGroupEntries {
                count: 1,
                min: 2,
                ..
            })
        ));
    }

    #[test]
    fn validate_field_group_entry_count_too_many() {
        let rt = create_field_group_rt(false, None, Some(1));
        let record = create_group_record(2);
        assert!(matches!(
            validate_record(&record, &rt),
            Err(CoreError::TooManyGroupEntries {
                count: 2,
                max: 1,
                ..
            })
        ));
    }

    #[test]
    fn validate_field_group_no_min_max_any_count_ok() {
        let rt = create_field_group_rt(false, None, None);
        let record = create_group_record(3);
        assert!(validate_record(&record, &rt).is_ok());
    }
}
