use crate::error::CoreError;
use crate::types::field::Datatype;
use crate::types::record::Record;
use crate::types::record_type::{
    CrossFieldRule, CrossFieldRuleEffect, CrossFieldRuleKind, RecordType,
};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeDiagnostic {
    pub code: RecordTypeDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecordTypeDiagnosticCode {
    /// V7: Type declares both lifecycle and lifecycleRef — mutually exclusive
    V7BothLifecycleAndRef,
}

/// V7: Validate lifecycle/lifecycleRef mutual exclusivity on a RecordType.
pub fn validate_record_type_v7(rt: &RecordType) -> Vec<RecordTypeDiagnostic> {
    let mut diags = Vec::new();

    if rt.lifecycle.is_some() && rt.lifecycle_ref.is_some() {
        diags.push(RecordTypeDiagnostic {
            code: RecordTypeDiagnosticCode::V7BothLifecycleAndRef,
            message: format!(
                "type '{}' declares both lifecycle and lifecycleRef — declare exactly one",
                rt.name
            ),
        });
    }

    diags
}

/// Evaluate all cross-field rules from `ext:cross-field-validation` against a record.
///
/// `field_types` maps field IDs to the `Datatype` their `fieldType` declares in the package.
/// Returns one `CoreError` per violated rule. Returns empty vec if all rules pass.
pub fn validate_cross_field_rules(
    record: &Record,
    rules: &[CrossFieldRule],
    field_types: &HashMap<String, Datatype>,
) -> Vec<CoreError> {
    let mut errors = Vec::new();
    for rule in rules {
        match rule.rule_type {
            CrossFieldRuleKind::ConditionalRequired => {
                evaluate_conditional_required(record, rule, &mut errors);
            }
            CrossFieldRuleKind::FieldOrdering => {
                evaluate_field_ordering(record, rule, field_types, &mut errors);
            }
            CrossFieldRuleKind::MutualExclusion => {
                evaluate_mutual_exclusion(record, rule, &mut errors);
            }
        }
    }
    errors
}

/// Returns the non-empty string value for the field with `field_id` in the record, or None.
fn field_value_str<'a>(record: &'a Record, field_id: &str) -> Option<&'a str> {
    record
        .find_field_value(field_id)
        .and_then(|fv| fv.value.as_str().filter(|s| !s.is_empty()))
}

fn evaluate_conditional_required(
    record: &Record,
    rule: &CrossFieldRule,
    errors: &mut Vec<CoreError>,
) {
    let (Some(predicate_field_id), Some(predicate_value), Some(target_field_id)) = (
        rule.predicate_field_id.as_deref(),
        rule.predicate_value.as_deref(),
        rule.target_field_id.as_deref(),
    ) else {
        errors.push(CoreError::CrossFieldRuleMisconfigured {
            reason: "conditional-required rule requires predicateFieldId, predicateValue, and targetFieldId".to_string(),
        });
        return;
    };

    if field_value_str(record, predicate_field_id) == Some(predicate_value)
        && field_value_str(record, target_field_id).is_none()
    {
        errors.push(CoreError::CrossFieldConditionalRequired {
            predicate_field_id: predicate_field_id.to_string(),
            predicate_value: predicate_value.to_string(),
            target_field_id: target_field_id.to_string(),
        });
    }
}

fn evaluate_field_ordering(
    record: &Record,
    rule: &CrossFieldRule,
    field_types: &HashMap<String, Datatype>,
    errors: &mut Vec<CoreError>,
) {
    let (Some(predicate_field_id), Some(target_field_id), Some(effect)) = (
        rule.predicate_field_id.as_deref(),
        rule.target_field_id.as_deref(),
        rule.effect.as_ref(),
    ) else {
        errors.push(CoreError::CrossFieldRuleMisconfigured {
            reason: "field-ordering rule requires predicateFieldId, targetFieldId, and effect"
                .to_string(),
        });
        return;
    };

    // If field ID is absent from the package, skip silently — referential integrity checked elsewhere.
    let Some(target_vtype) = field_types.get(target_field_id) else {
        return;
    };
    let Some(predicate_vtype) = field_types.get(predicate_field_id) else {
        return;
    };

    // field-ordering applies only to orderable scalars. RFC-032 split the
    // pre-decomposition `date`/`number` pair into four: `integer` and
    // `date-time` are equally comparable and are accepted on the same footing.
    let orderable = |dt: &Datatype| {
        matches!(
            dt,
            Datatype::Date | Datatype::DateTime | Datatype::Number | Datatype::Integer
        )
    };
    if !orderable(target_vtype) || !orderable(predicate_vtype) {
        errors.push(CoreError::CrossFieldRuleMisconfigured {
            reason: "field-ordering applies only to date, date-time, number and integer fields"
                .to_string(),
        });
        return;
    }

    // Skip if either value is absent or empty on the record — nothing to compare.
    let Some(target_val) = field_value_str(record, target_field_id) else {
        return;
    };
    let Some(predicate_val) = field_value_str(record, predicate_field_id) else {
        return;
    };

    let violation = match target_vtype {
        Datatype::Number | Datatype::Integer => {
            let Ok(t) = target_val.parse::<f64>() else {
                errors.push(CoreError::CrossFieldRuleMisconfigured {
                    reason: format!(
                        "field-ordering: could not parse '{}' as a number for field '{}'",
                        target_val, target_field_id
                    ),
                });
                return;
            };
            let Ok(p) = predicate_val.parse::<f64>() else {
                errors.push(CoreError::CrossFieldRuleMisconfigured {
                    reason: format!(
                        "field-ordering: could not parse '{}' as a number for field '{}'",
                        predicate_val, predicate_field_id
                    ),
                });
                return;
            };
            match effect {
                CrossFieldRuleEffect::MustPrecede => t >= p,
                CrossFieldRuleEffect::MustFollow => t <= p,
            }
        }
        Datatype::Date | Datatype::DateTime => {
            // ISO 8601 strings are lexicographically ordered
            match effect {
                CrossFieldRuleEffect::MustPrecede => target_val >= predicate_val,
                CrossFieldRuleEffect::MustFollow => target_val <= predicate_val,
            }
        }
        _ => unreachable!("checked above"),
    };

    if violation {
        let effect_str = match effect {
            CrossFieldRuleEffect::MustPrecede => "must-precede",
            CrossFieldRuleEffect::MustFollow => "must-follow",
        };
        errors.push(CoreError::CrossFieldOrdering {
            target_field_id: target_field_id.to_string(),
            effect: effect_str.to_string(),
            predicate_field_id: predicate_field_id.to_string(),
        });
    }
}

fn evaluate_mutual_exclusion(record: &Record, rule: &CrossFieldRule, errors: &mut Vec<CoreError>) {
    let Some(field_ids) = rule.field_ids.as_ref() else {
        errors.push(CoreError::CrossFieldRuleMisconfigured {
            reason: "mutual-exclusion rule requires fieldIds with at least 2 entries".to_string(),
        });
        return;
    };
    if field_ids.len() < 2 {
        errors.push(CoreError::CrossFieldRuleMisconfigured {
            reason: "mutual-exclusion rule requires fieldIds with at least 2 entries".to_string(),
        });
        return;
    }

    let populated_count = field_ids
        .iter()
        .filter(|id| field_value_str(record, id).is_some())
        .count();

    if populated_count > 1 {
        errors.push(CoreError::CrossFieldMutualExclusion {
            field_ids: field_ids.join(", "),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::record::FieldValue;
    use crate::types::record_type::{
        CrossFieldRule, CrossFieldRuleEffect, CrossFieldRuleKind, RecordType, TypeLifecycle,
    };
    use std::collections::HashMap;

    fn make_rt(lifecycle: bool, lifecycle_ref: bool) -> RecordType {
        RecordType {
            id: "rt-1".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: if lifecycle {
                Some(TypeLifecycle {
                    states: vec![],
                    transitions: vec![],
                    initial_state: "draft".to_string(),
                })
            } else {
                None
            },
            lifecycle_ref: if lifecycle_ref {
                Some("lc-ref-id".to_string())
            } else {
                None
            },
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn make_record(field_values: Vec<(&str, serde_json::Value)>) -> Record {
        Record {
            instance_id: "rec-1".to_string(),
            type_id: "rt-1".to_string(),
            type_name: "test-type".to_string(),
            type_namespace: "com.test".to_string(),
            type_version: 1,
            created_at: None,
            updated_at: None,
            group_values: None,
            lifecycle_state: None,
            tags: None,
            field_values: field_values
                .into_iter()
                .map(|(id, val)| FieldValue {
                    field_id: id.to_string(),
                    value: val,
                    entries: None,
                    source: None,
                    edited_at: None,
                })
                .collect(),
            extra: std::collections::HashMap::new(),
        }
    }

    fn cond_required_rule(
        predicate_field_id: &str,
        predicate_value: &str,
        target_field_id: &str,
    ) -> CrossFieldRule {
        CrossFieldRule {
            rule_type: CrossFieldRuleKind::ConditionalRequired,
            message: None,
            predicate_field_id: Some(predicate_field_id.to_string()),
            predicate_value: Some(predicate_value.to_string()),
            target_field_id: Some(target_field_id.to_string()),
            effect: None,
            field_ids: None,
        }
    }

    fn ordering_rule(
        predicate_field_id: &str,
        target_field_id: &str,
        effect: CrossFieldRuleEffect,
    ) -> CrossFieldRule {
        CrossFieldRule {
            rule_type: CrossFieldRuleKind::FieldOrdering,
            message: None,
            predicate_field_id: Some(predicate_field_id.to_string()),
            predicate_value: None,
            target_field_id: Some(target_field_id.to_string()),
            effect: Some(effect),
            field_ids: None,
        }
    }

    fn mutex_rule(field_ids: Vec<&str>) -> CrossFieldRule {
        CrossFieldRule {
            rule_type: CrossFieldRuleKind::MutualExclusion,
            message: None,
            predicate_field_id: None,
            predicate_value: None,
            target_field_id: None,
            effect: None,
            field_ids: Some(field_ids.into_iter().map(str::to_string).collect()),
        }
    }

    #[test]
    fn record_type_both_lifecycle_and_ref_is_error() {
        let rt = make_rt(true, true);
        let diags = validate_record_type_v7(&rt);
        assert!(diags
            .iter()
            .any(|d| d.code == RecordTypeDiagnosticCode::V7BothLifecycleAndRef));
    }

    #[test]
    fn record_type_only_inline_passes() {
        let rt = make_rt(true, false);
        assert!(validate_record_type_v7(&rt).is_empty());
    }

    #[test]
    fn record_type_only_ref_passes() {
        let rt = make_rt(false, true);
        assert!(validate_record_type_v7(&rt).is_empty());
    }

    #[test]
    fn record_type_neither_passes() {
        let rt = make_rt(false, false);
        assert!(validate_record_type_v7(&rt).is_empty());
    }

    // ── validate_cross_field_rules tests ─────────────────────────────────────

    #[test]
    fn cross_field_no_rules_returns_empty() {
        let record = make_record(vec![]);
        let errs = validate_cross_field_rules(&record, &[], &HashMap::new());
        assert!(errs.is_empty());
    }

    #[test]
    fn conditional_required_not_triggered_predicate_absent() {
        let record = make_record(vec![]);
        let rule = cond_required_rule("f-predicate", "yes", "f-target");
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert!(errs.is_empty());
    }

    #[test]
    fn conditional_required_not_triggered_predicate_different_value() {
        let record = make_record(vec![("f-predicate", serde_json::json!("no"))]);
        let rule = cond_required_rule("f-predicate", "yes", "f-target");
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert!(errs.is_empty());
    }

    #[test]
    fn conditional_required_empty_string_predicate_no_violation() {
        let record = make_record(vec![("f-predicate", serde_json::json!(""))]);
        let rule = cond_required_rule("f-predicate", "", "f-target");
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        // empty string is treated as absent — predicate value "" can never match a non-empty predicate_value
        // but here predicate_value is also "" — field_value_str returns None for "", so predicate is absent → no violation
        assert!(errs.is_empty());
    }

    #[test]
    fn conditional_required_triggered_target_absent() {
        let record = make_record(vec![("f-predicate", serde_json::json!("yes"))]);
        let rule = cond_required_rule("f-predicate", "yes", "f-target");
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0],
            CoreError::CrossFieldConditionalRequired {
                predicate_field_id: "f-predicate".to_string(),
                predicate_value: "yes".to_string(),
                target_field_id: "f-target".to_string(),
            }
        );
    }

    #[test]
    fn conditional_required_triggered_target_present() {
        let record = make_record(vec![
            ("f-predicate", serde_json::json!("yes")),
            ("f-target", serde_json::json!("some value")),
        ]);
        let rule = cond_required_rule("f-predicate", "yes", "f-target");
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert!(errs.is_empty());
    }

    #[test]
    fn field_ordering_must_precede_passes() {
        let record = make_record(vec![
            ("f-start", serde_json::json!("10")),
            ("f-end", serde_json::json!("20")),
        ]);
        // f-start must precede f-end: start < end → pass
        let rule = ordering_rule("f-end", "f-start", CrossFieldRuleEffect::MustPrecede);
        let mut ft = HashMap::new();
        ft.insert("f-start".to_string(), Datatype::Number);
        ft.insert("f-end".to_string(), Datatype::Number);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert!(errs.is_empty());
    }

    #[test]
    fn field_ordering_must_precede_fails() {
        let record = make_record(vec![
            ("f-start", serde_json::json!("30")),
            ("f-end", serde_json::json!("20")),
        ]);
        // f-start must precede f-end: start(30) >= end(20) → violation
        let rule = ordering_rule("f-end", "f-start", CrossFieldRuleEffect::MustPrecede);
        let mut ft = HashMap::new();
        ft.insert("f-start".to_string(), Datatype::Number);
        ft.insert("f-end".to_string(), Datatype::Number);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], CoreError::CrossFieldOrdering { .. }));
    }

    #[test]
    fn field_ordering_must_follow_passes() {
        let record = make_record(vec![
            ("f-end-date", serde_json::json!("2026-06-01")),
            ("f-start-date", serde_json::json!("2026-01-01")),
        ]);
        // f-end-date must follow f-start-date: end > start → pass
        let rule = ordering_rule(
            "f-start-date",
            "f-end-date",
            CrossFieldRuleEffect::MustFollow,
        );
        let mut ft = HashMap::new();
        ft.insert("f-end-date".to_string(), Datatype::Date);
        ft.insert("f-start-date".to_string(), Datatype::Date);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert!(errs.is_empty());
    }

    #[test]
    fn field_ordering_must_follow_fails() {
        let record = make_record(vec![
            ("f-end-date", serde_json::json!("2026-01-01")),
            ("f-start-date", serde_json::json!("2026-06-01")),
        ]);
        // f-end-date must follow f-start-date: end(2026-01-01) <= start(2026-06-01) → violation
        let rule = ordering_rule(
            "f-start-date",
            "f-end-date",
            CrossFieldRuleEffect::MustFollow,
        );
        let mut ft = HashMap::new();
        ft.insert("f-end-date".to_string(), Datatype::Date);
        ft.insert("f-start-date".to_string(), Datatype::Date);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], CoreError::CrossFieldOrdering { .. }));
    }

    #[test]
    fn field_ordering_wrong_value_type_produces_misconfigured() {
        let record = make_record(vec![
            ("f-text", serde_json::json!("hello")),
            ("f-end", serde_json::json!("world")),
        ]);
        let rule = ordering_rule("f-end", "f-text", CrossFieldRuleEffect::MustPrecede);
        let mut ft = HashMap::new();
        ft.insert("f-text".to_string(), Datatype::String);
        ft.insert("f-end".to_string(), Datatype::String);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            CoreError::CrossFieldRuleMisconfigured { reason } if reason.contains("field-ordering applies only to")
        ));
    }

    #[test]
    fn field_ordering_both_field_values_absent_no_violation() {
        let record = make_record(vec![]);
        let rule = ordering_rule("f-end", "f-start", CrossFieldRuleEffect::MustPrecede);
        let mut ft = HashMap::new();
        ft.insert("f-start".to_string(), Datatype::Number);
        ft.insert("f-end".to_string(), Datatype::Number);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert!(errs.is_empty());
    }

    #[test]
    fn field_ordering_nonexistent_field_id_silently_skipped() {
        let record = make_record(vec![
            ("f-start", serde_json::json!("10")),
            ("f-end", serde_json::json!("20")),
        ]);
        // Rule references field IDs not present in field_types map
        let rule = ordering_rule(
            "f-nonexistent-end",
            "f-start",
            CrossFieldRuleEffect::MustPrecede,
        );
        let ft: HashMap<String, Datatype> = HashMap::new();
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert!(
            errs.is_empty(),
            "absent field IDs silently skip, got: {:?}",
            errs
        );
    }

    #[test]
    fn mutual_exclusion_zero_populated_ok() {
        let record = make_record(vec![]);
        let rule = mutex_rule(vec!["f-a", "f-b"]);
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert!(errs.is_empty());
    }

    #[test]
    fn mutual_exclusion_one_populated_ok() {
        let record = make_record(vec![("f-a", serde_json::json!("some value"))]);
        let rule = mutex_rule(vec!["f-a", "f-b"]);
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert!(errs.is_empty());
    }

    #[test]
    fn mutual_exclusion_two_populated_violation() {
        let record = make_record(vec![
            ("f-a", serde_json::json!("value a")),
            ("f-b", serde_json::json!("value b")),
        ]);
        let rule = mutex_rule(vec!["f-a", "f-b"]);
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            CoreError::CrossFieldMutualExclusion { field_ids } if field_ids.contains("f-a")
        ));
    }

    #[test]
    fn mutual_exclusion_insufficient_field_ids_misconfigured() {
        let record = make_record(vec![]);
        let rule = mutex_rule(vec!["f-a"]); // only 1 field — misconfigured
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            errs[0],
            CoreError::CrossFieldRuleMisconfigured { .. }
        ));
    }

    #[test]
    fn conditional_required_misconfigured_missing_predicate() {
        // A conditional-required rule with all required fields absent → misconfigured
        let record = make_record(vec![("f-x", serde_json::json!("some-value"))]);
        let rule = CrossFieldRule {
            rule_type: CrossFieldRuleKind::ConditionalRequired,
            message: None,
            predicate_field_id: None, // missing
            predicate_value: None,    // missing
            target_field_id: None,    // missing
            effect: None,
            field_ids: None,
        };
        let errs = validate_cross_field_rules(&record, &[rule], &HashMap::new());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            CoreError::CrossFieldRuleMisconfigured { reason } if reason.contains("conditional-required")
        ));
    }
}
