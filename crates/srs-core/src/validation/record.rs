use crate::error::CoreError;
use crate::types::record::Record;
use crate::types::record_type::{RecordType, TypeLifecycle};
use crate::validation::value_shape::{validate_field_values_map, EffectiveField, RangeResolver};
use std::collections::HashSet;

/// Validates a record against its record type definition (RFC-039 carrier).
///
/// `effective_fields` is the pre-computed resolved field set — assignments
/// joined to their Field definitions (name + fieldType) — for inheriting
/// types the merged base + own list. The caller builds it from the package
/// (srs-core has no I/O). `resolver` resolves inline-composite range Types
/// for [R3]'s recursive descent.
///
/// Fail-fast wrapper over the accumulating validator: returns the first
/// diagnostic in check order.
pub fn validate_record(
    record: &Record,
    record_type: &RecordType,
    effective_fields: &[EffectiveField],
    resolver: &dyn RangeResolver,
) -> Result<(), CoreError> {
    match validate_record_all(record, record_type, effective_fields, resolver)
        .into_iter()
        .next()
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Validates a record, collecting **all** diagnostics. Check order:
/// [R1] unknown keys → [R5] required-present → [R3]/[R16] value shapes
/// (recursive) → [R6] fieldMeta keys → tags → lifecycle. An empty vec means
/// the record is valid.
pub fn validate_record_all(
    record: &Record,
    record_type: &RecordType,
    effective_fields: &[EffectiveField],
    resolver: &dyn RangeResolver,
) -> Vec<CoreError> {
    let mut diagnostics = Vec::new();

    // [R1] + [R5] + [R3]/[R16], shared with the inline-composite recursion.
    validate_field_values_map(
        "",
        &record.field_values.0,
        effective_fields,
        resolver,
        &mut diagnostics,
    );

    // [R6]: fieldMeta keys are a subset of fieldValues keys.
    if let Some(meta) = &record.field_meta {
        let value_keys: HashSet<&str> = record.field_values.keys().map(String::as_str).collect();
        for key in meta.keys() {
            if !value_keys.contains(key.as_str()) {
                diagnostics.push(CoreError::FieldMetaUnknownKey { key: key.clone() });
            }
        }
    }

    // Invariant: Record.tags values must be non-empty strings. Vocabulary resolution
    // (Term key/alias) is enforced in srs-repository (requires package access).
    if let Some(tags) = &record.tags {
        for tag in tags {
            if tag.is_empty() {
                diagnostics.push(CoreError::InvalidTagValue { tag: tag.clone() });
            }
        }
    }

    // Invariant 6 (ext:lifecycle): Record.lifecycleState must name a state in the
    // associated Type's lifecycle.states[] when the Type declares a lifecycle.
    if let (Some(state), Some(lc)) = (&record.lifecycle_state, &record_type.lifecycle) {
        let valid = lc.states.iter().any(|s| &s.key == state);
        if !valid {
            diagnostics.push(CoreError::InvalidLifecycleState {
                state: state.clone(),
            });
        }
    }

    diagnostics
}

/// Validate a Type's lifecycle definition (Invariants 4 and 5, ext:lifecycle).
///
/// - Invariant 4: `initialState` must name a state with `isInitial: true`.
/// - Invariant 5: All `from`/`to` in `transitions[]` must name valid states.
pub fn validate_type_lifecycle(lifecycle: &TypeLifecycle) -> Result<(), CoreError> {
    let state_names: HashSet<&str> = lifecycle.states.iter().map(|s| s.key.as_str()).collect();

    // Invariant 4
    let initial_state = lifecycle
        .states
        .iter()
        .find(|s| s.key == lifecycle.initial_state);
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
    use crate::types::field_type::FieldType;
    use crate::types::record::{FieldMeta, FieldValues, Record};
    use crate::types::record_type::{FieldAssignment, RecordType};
    use indexmap::IndexMap;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn no_resolver() -> impl Fn(&str, u32) -> Option<Vec<EffectiveField>> {
        |_: &str, _: u32| None
    }

    fn ef(name: &str, required: bool) -> EffectiveField {
        EffectiveField {
            field_id: format!("id-{name}"),
            name: name.to_string(),
            required,
            order: 0,
            field_type: Some(FieldType::string()),
        }
    }

    fn create_test_record_type() -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-1".to_string(),
            namespace: "test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test type".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "id-required_field".to_string(),
                    order: 0,
                    required: true,
                    display_label: None,
                    description: None,
                },
                FieldAssignment {
                    field_id: "id-optional_field".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn effective() -> Vec<EffectiveField> {
        vec![ef("required_field", true), ef("optional_field", false)]
    }

    fn record_with(values: serde_json::Value) -> Record {
        let serde_json::Value::Object(map) = values else {
            panic!("test values must be an object")
        };
        Record {
            instance_id: "inst-1".to_string(),
            type_id: "type-1".to_string(),
            type_version: 1,
            type_namespace: "test".to_string(),
            type_name: "test-type".to_string(),
            field_values: FieldValues(map),
            field_meta: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn validate_record_passes_with_all_required_fields() {
        let rt = create_test_record_type();
        let record = record_with(json!({"required_field": "v1", "optional_field": "v2"}));
        assert!(validate_record(&record, &rt, &effective(), &no_resolver()).is_ok());
    }

    #[test]
    fn validate_record_optional_field_absent_is_ok() {
        let rt = create_test_record_type();
        let record = record_with(json!({"required_field": "v1"}));
        assert!(validate_record(&record, &rt, &effective(), &no_resolver()).is_ok());
    }

    #[test]
    fn validate_record_missing_required_field() {
        let rt = create_test_record_type();
        let record = record_with(json!({"optional_field": "v2"}));
        let result = validate_record(&record, &rt, &effective(), &no_resolver());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::MissingRequiredField { key } if key == "required_field"
        ));
    }

    #[test]
    fn validate_record_unknown_key() {
        let rt = create_test_record_type();
        let record = record_with(json!({"required_field": "v1", "mystery": "v3"}));
        let result = validate_record(&record, &rt, &effective(), &no_resolver());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::UnknownFieldKey { key } if key == "mystery"
        ));
    }

    #[test]
    fn validate_record_all_collects_multiple() {
        // Record both omits a required field AND carries an unknown key —
        // validate_record_all must report both, not stop at the first.
        let rt = create_test_record_type();
        let record = record_with(json!({"mystery": "v"}));
        let diags = validate_record_all(&record, &rt, &effective(), &no_resolver());
        assert!(diags.len() >= 2, "{diags:?}");
        assert!(diags
            .iter()
            .any(|e| matches!(e, CoreError::UnknownFieldKey { key } if key == "mystery")));
        assert!(diags.iter().any(
            |e| matches!(e, CoreError::MissingRequiredField { key } if key == "required_field")
        ));
        // [R1] unknown keys are reported before missing-required, and the
        // fail-fast wrapper surfaces that same first diagnostic.
        assert!(matches!(diags[0], CoreError::UnknownFieldKey { .. }));
        assert!(matches!(
            validate_record(&record, &rt, &effective(), &no_resolver()),
            Err(CoreError::UnknownFieldKey { .. })
        ));
    }

    #[test]
    fn validate_record_all_empty_when_valid() {
        let rt = create_test_record_type();
        let record = record_with(json!({"required_field": "v1"}));
        assert!(validate_record_all(&record, &rt, &effective(), &no_resolver()).is_empty());
    }

    #[test]
    fn null_value_rejected_key_absence_is_unset() {
        // [R5]: null is rejected; the same field simply absent is fine when
        // not required.
        let rt = create_test_record_type();
        let record = record_with(json!({"required_field": "v", "optional_field": null}));
        let result = validate_record(&record, &rt, &effective(), &no_resolver());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::NullFieldValue { key } if key == "optional_field"
        ));
    }

    #[test]
    fn required_empty_string_satisfies_r5() {
        // [R5a]: structural presence is key presence — "" is a present value
        // and validates; its rendering absence is RFC-001 Step 2's concern,
        // not validation's.
        let rt = create_test_record_type();
        let record = record_with(json!({"required_field": ""}));
        assert!(validate_record(&record, &rt, &effective(), &no_resolver()).is_ok());
    }

    #[test]
    fn field_meta_key_not_in_field_values_rejected() {
        let rt = create_test_record_type();
        let mut record = record_with(json!({"required_field": "v"}));
        let mut meta = IndexMap::new();
        meta.insert(
            "phantom".to_string(),
            FieldMeta {
                source: Some("human".to_string()),
                ..Default::default()
            },
        );
        record.field_meta = Some(meta);
        let result = validate_record(&record, &rt, &effective(), &no_resolver());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::FieldMetaUnknownKey { key } if key == "phantom"
        ));
    }

    #[test]
    fn field_meta_matching_key_ok() {
        let rt = create_test_record_type();
        let mut record = record_with(json!({"required_field": "v"}));
        let mut meta = IndexMap::new();
        meta.insert("required_field".to_string(), FieldMeta::default());
        record.field_meta = Some(meta);
        assert!(validate_record(&record, &rt, &effective(), &no_resolver()).is_ok());
    }
}
