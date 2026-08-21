use crate::error::CoreError;
use crate::types::field::{Datatype, Field, FieldType};
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

/// What a cross-field rule needs to know about one assigned field.
///
/// Rules address fields by `fieldId` (a Type-level declaration, unchanged by
/// RFC-039), but the record carrier keys values by `Field.name` — so the map
/// carries the name to bridge the two. `effective-single` is cardinality-only
/// since the srs#242 Phase-B train (Change-I condition 4).
#[derive(Debug, Clone)]
pub struct CrossFieldFieldType {
    /// The field's declared `fieldType`, in full.
    pub field_type: FieldType,
    /// `Field.name` — the record's carrier key for this field.
    pub name: String,
}

/// Build the field-type map [`validate_cross_field_rules`] expects, pairing each
/// Field's `fieldType` with its `FieldAssignment.repeatable` in `record_type`.
///
/// Every caller derives the map this way; sharing one builder keeps the
/// `effective-single` union from being restated (and drifting) per call site.
pub fn cross_field_type_map(
    fields: &[Field],
    _record_type: &RecordType,
) -> HashMap<String, CrossFieldFieldType> {
    fields
        .iter()
        .map(|f| {
            (
                f.id.clone(),
                CrossFieldFieldType {
                    field_type: f.field_type.clone(),
                    name: f.name.clone(),
                },
            )
        })
        .collect()
}

/// Evaluate all cross-field rules from `ext:cross-field-validation` against a record.
///
/// `field_types` maps field IDs to the `fieldType` their package declares plus the
/// `FieldAssignment.repeatable` of their assignment in the Type under validation —
/// build it with [`cross_field_type_map`].
/// Returns one `CoreError` per violated rule. Returns empty vec if all rules pass.
pub fn validate_cross_field_rules(
    record: &Record,
    rules: &[CrossFieldRule],
    field_types: &HashMap<String, CrossFieldFieldType>,
) -> Vec<CoreError> {
    let mut errors = Vec::new();
    for rule in rules {
        match rule.rule_type {
            CrossFieldRuleKind::ConditionalRequired => {
                evaluate_conditional_required(record, rule, field_types, &mut errors);
            }
            CrossFieldRuleKind::FieldOrdering => {
                evaluate_field_ordering(record, rule, field_types, &mut errors);
            }
            CrossFieldRuleKind::MutualExclusion => {
                evaluate_mutual_exclusion(record, rule, field_types, &mut errors);
            }
        }
    }
    errors
}

/// I-92/94/95/96 (ext:cross-field-validation) all say a misconfigured rule "MUST be
/// reported as a Type-level validation error". [`validate_cross_field_rules`] above is
/// record-driven — every call site enforces it while writing or validating a Record —
/// so a Type carrying a misconfigured rule but owning zero Records is never flagged.
/// That gap is narrower than the invariants' text.
///
/// Rather than duplicating the three `evaluate_*` bodies, this runs them against a
/// field-value-less phantom Record of the Type. Every value-comparison guard in
/// `evaluate_conditional_required` / `evaluate_field_ordering` / `evaluate_mutual_exclusion`
/// short-circuits (`return`, or `populated_count` staying `0`) the moment a field value is
/// absent, so only the structural/eligibility [`CoreError::CrossFieldRuleMisconfigured`]
/// diagnostics can surface from a phantom record — never a spurious
/// `CrossFieldConditionalRequired` / `CrossFieldOrdering` / `CrossFieldMutualExclusion`
/// violation, which is why the result is filtered down to that one variant rather than
/// returned as-is.
pub fn validate_cross_field_rules_for_type(
    record_type: &RecordType,
    fields: &[Field],
) -> Vec<CoreError> {
    let Some(rules) = record_type
        .validation_rules
        .as_ref()
        .filter(|r| !r.is_empty())
    else {
        return Vec::new();
    };
    let field_types = cross_field_type_map(fields, record_type);
    let phantom = Record {
        instance_id: String::new(),
        type_id: record_type.id.clone(),
        type_version: record_type.version,
        type_namespace: record_type.namespace.clone(),
        type_name: record_type.name.clone(),
        field_values: crate::types::record::FieldValues::new(),
        field_meta: None,
        lifecycle_state: None,
        tags: None,
        created_at: None,
        updated_at: None,
        extra: std::collections::BTreeMap::new(),
    };
    validate_cross_field_rules(&phantom, rules, &field_types)
        .into_iter()
        .filter(|e| matches!(e, CoreError::CrossFieldRuleMisconfigured { .. }))
        .collect()
}

/// Returns the non-empty string value for the field with `field_id` in the
/// record, or None. Rules address fields by id; the carrier keys by name —
/// the map bridges. An id absent from the map resolves to None (absent from
/// the package; referential integrity is checked elsewhere).
fn field_value_str<'a>(
    record: &'a Record,
    field_types: &HashMap<String, CrossFieldFieldType>,
    field_id: &str,
) -> Option<&'a str> {
    let name = &field_types.get(field_id)?.name;
    record.value_str(name).filter(|s| !s.is_empty())
}

fn evaluate_conditional_required(
    record: &Record,
    rule: &CrossFieldRule,
    field_types: &HashMap<String, CrossFieldFieldType>,
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

    // I-94 / RFC-019 [R6]: the predicate field must be effective-single with a
    // datatype the single declared `predicateValue` can be compared against.
    // As in `evaluate_field_ordering`, a field id absent from the package is
    // skipped silently — referential integrity is checked elsewhere.
    if let Some(predicate_type) = field_types.get(predicate_field_id) {
        if !predicate_type.field_type.is_conditional_required_eligible() {
            errors.push(CoreError::CrossFieldRuleMisconfigured {
                reason: format!(
                    "conditional-required: predicate field '{}' must be a single-valued string, date or date-time field",
                    predicate_field_id
                ),
            });
            return;
        }
    }

    if field_value_str(record, field_types, predicate_field_id) == Some(predicate_value)
        && field_value_str(record, field_types, target_field_id).is_none()
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
    field_types: &HashMap<String, CrossFieldFieldType>,
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
    let Some(target_vtype) = field_types
        .get(target_field_id)
        .map(|t| t.field_type.datatype)
    else {
        return;
    };
    let Some(predicate_vtype) = field_types
        .get(predicate_field_id)
        .map(|t| t.field_type.datatype)
    else {
        return;
    };
    let (target_vtype, predicate_vtype) = (&target_vtype, &predicate_vtype);

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
    let Some(target_val) = field_value_str(record, field_types, target_field_id) else {
        return;
    };
    let Some(predicate_val) = field_value_str(record, field_types, predicate_field_id) else {
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

fn evaluate_mutual_exclusion(
    record: &Record,
    rule: &CrossFieldRule,
    field_types: &HashMap<String, CrossFieldFieldType>,
    errors: &mut Vec<CoreError>,
) {
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
        .filter(|id| field_value_str(record, field_types, id).is_some())
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
    use crate::types::record_type::{
        CrossFieldRule, CrossFieldRuleEffect, CrossFieldRuleKind, RecordType, TypeLifecycle,
    };
    use std::collections::HashMap;

    fn make_rt(lifecycle: bool, lifecycle_ref: bool) -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
            id: "rt-1".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![],
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
        }
    }

    fn make_record(field_values: Vec<(&str, serde_json::Value)>) -> Record {
        // Test field ids double as Field.names: the cross_field_type_map in
        // each test carries name == id, so the carrier keys by the same string.
        let mut fv = crate::types::record::FieldValues::new();
        for (id, val) in field_values {
            fv.insert(id, val);
        }
        Record {
            instance_id: "rec-1".to_string(),
            type_id: "rt-1".to_string(),
            type_name: "test-type".to_string(),
            type_namespace: "com.test".to_string(),
            type_version: 1,
            created_at: None,
            updated_at: None,
            field_meta: None,
            lifecycle_state: None,
            tags: None,
            field_values: fv,
            extra: std::collections::BTreeMap::new(),
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

    /// A field-type map of plain single-valued fields, keyed by field id.
    fn ftm(entries: &[(&str, Datatype)]) -> HashMap<String, CrossFieldFieldType> {
        entries
            .iter()
            .map(|(id, dt)| {
                (
                    id.to_string(),
                    CrossFieldFieldType {
                        field_type: FieldType::new(*dt),
                        name: (*id).to_string(),
                    },
                )
            })
            .collect()
    }

    /// The same, with one entry's `FieldType` given explicitly.
    fn ftm_one(id: &str, field_type: FieldType) -> HashMap<String, CrossFieldFieldType> {
        HashMap::from([(
            id.to_string(),
            CrossFieldFieldType {
                field_type,
                name: id.to_string(),
            },
        )])
    }

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
        let ft = ftm(&[
            ("f-predicate", Datatype::String),
            ("f-target", Datatype::String),
        ]);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
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

    // I-94 / RFC-019 [R6] — the predicate field's eligibility. Constructed
    // fixtures throughout: `predicateFieldId` has zero first-party use sites, so
    // the corpus cannot exercise any of this.

    #[test]
    fn i94_r6_conditional_required_rejects_list_predicate_field() {
        let record = make_record(vec![("f-predicate", serde_json::json!("yes"))]);
        let rule = cond_required_rule("f-predicate", "yes", "f-target");
        let ft = ftm_one("f-predicate", FieldType::string().into_list());
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert!(
            matches!(&errs[..], [CoreError::CrossFieldRuleMisconfigured { reason }]
                if reason.contains("must be a single-valued string, date or date-time")),
            "a list predicate field must be rejected, got: {errs:?}"
        );
    }

    #[test]
    fn cross_field_rules_resolve_field_id_to_carrier_name() {
        // Rules address fields by `fieldId`; the RFC-039 carrier keys values
        // by `Field.name`. The map bridges — a record keyed by the *name*
        // must satisfy a rule declared over the *id*.
        let mut fv = crate::types::record::FieldValues::new();
        fv.insert("predicate_name", serde_json::json!("yes"));
        let mut record = make_record(vec![]);
        record.field_values = fv;
        let rule = cond_required_rule("f-predicate", "yes", "f-target");
        let ft = HashMap::from([
            (
                "f-predicate".to_string(),
                CrossFieldFieldType {
                    field_type: FieldType::string(),
                    name: "predicate_name".to_string(),
                },
            ),
            (
                "f-target".to_string(),
                CrossFieldFieldType {
                    field_type: FieldType::string(),
                    name: "target_name".to_string(),
                },
            ),
        ]);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
        assert!(
            matches!(&errs[..], [CoreError::CrossFieldConditionalRequired { .. }]),
            "the id-declared rule must fire against the name-keyed record, got: {errs:?}"
        );
    }

    #[test]
    fn i94_r6_conditional_required_rejects_non_comparable_datatypes() {
        for rejected in [Datatype::Boolean, Datatype::Number, Datatype::Integer] {
            let record = make_record(vec![("f-predicate", serde_json::json!("yes"))]);
            let rule = cond_required_rule("f-predicate", "yes", "f-target");
            let ft = ftm(&[("f-predicate", rejected)]);
            let errs = validate_cross_field_rules(&record, &[rule], &ft);
            assert!(
                matches!(&errs[..], [CoreError::CrossFieldRuleMisconfigured { .. }]),
                "{rejected:?} must be rejected as a predicate field, got: {errs:?}"
            );
        }
    }

    #[test]
    fn i94_r6_conditional_required_admits_eligible_predicate_field() {
        // Eligibility must not swallow the rule it guards: with an eligible
        // predicate field the conditional-required violation still fires.
        for admitted in [Datatype::String, Datatype::Date, Datatype::DateTime] {
            let record = make_record(vec![("f-predicate", serde_json::json!("yes"))]);
            let rule = cond_required_rule("f-predicate", "yes", "f-target");
            let ft = ftm(&[("f-predicate", admitted)]);
            let errs = validate_cross_field_rules(&record, &[rule], &ft);
            assert!(
                matches!(&errs[..], [CoreError::CrossFieldConditionalRequired { .. }]),
                "{admitted:?} must be admitted and the rule still enforced, got: {errs:?}"
            );
        }
    }

    /// The glue every cross-field call site depends on: `cross_field_type_map`
    /// must carry `Field.name` across, since the carrier keys by name and the
    /// rules key by id. The other I-94 tests hand-build `CrossFieldFieldType`,
    /// so without this a bug that dropped the name bridge would go unnoticed.
    #[test]
    fn cross_field_type_map_carries_field_names() {
        let fields: Vec<Field> = [("f-plain", "plain_name"), ("f-two", "two_name")]
            .iter()
            .map(|(id, name)| {
                serde_json::from_value(serde_json::json!({
                    "id": id, "namespace": "com.test", "name": name, "version": 1,
                    "description": "d", "fieldType": { "datatype": "string" },
                    "createdAt": "2026-01-01T00:00:00Z",
                }))
                .expect("field fixture should deserialize")
            })
            .collect();
        let rt: RecordType = serde_json::from_value(serde_json::json!({
            "id": "t-1", "namespace": "com.test", "name": "t", "version": 1,
            "description": "d",
            "fields": [
                { "fieldId": "f-plain", "order": 0, "required": false },
                { "fieldId": "f-two", "order": 1, "required": false },
            ],
            "createdAt": "2026-01-01T00:00:00Z",
        }))
        .expect("type fixture should deserialize");

        let map = cross_field_type_map(&fields, &rt);
        assert_eq!(map["f-plain"].name, "plain_name");
        assert_eq!(map["f-two"].name, "two_name");
    }

    /// I-92/94/95/96: a Type carrying a misconfigured rule must be flagged even
    /// with **zero Records** — the gap `validate_cross_field_rules_for_type` exists
    /// to close, since every other call site only runs against an actual Record.
    /// No record is constructed anywhere in this test.
    #[test]
    fn i94_type_level_flags_ineligible_predicate_field_with_no_records() {
        // Ineligible via list cardinality — the sole mechanism post Change-I.
        let fields: Vec<Field> = ["f-repeat", "f-target"]
            .iter()
            .map(|id| {
                let cardinality = if *id == "f-repeat" { "list" } else { "single" };
                serde_json::from_value(serde_json::json!({
                    "id": id, "namespace": "com.test", "name": id, "version": 1,
                    "description": "d",
                    "fieldType": { "datatype": "string", "cardinality": cardinality },
                    "createdAt": "2026-01-01T00:00:00Z",
                }))
                .expect("field fixture should deserialize")
            })
            .collect();
        let rt: RecordType = serde_json::from_value(serde_json::json!({
            "id": "t-1", "namespace": "com.test", "name": "t", "version": 1,
            "description": "d",
            "fields": [
                { "fieldId": "f-repeat", "order": 0, "required": false },
                { "fieldId": "f-target", "order": 1, "required": false },
            ],
            "validationRules": [{
                "type": "conditional-required",
                "predicateFieldId": "f-repeat",
                "predicateValue": "yes",
                "targetFieldId": "f-target",
            }],
            "createdAt": "2026-01-01T00:00:00Z",
        }))
        .expect("type fixture should deserialize");

        let errs = validate_cross_field_rules_for_type(&rt, &fields);
        assert!(
            matches!(
                &errs[..],
                [CoreError::CrossFieldRuleMisconfigured { reason }]
                    if reason.contains("predicate field")
            ),
            "expected a Type-level CrossFieldRuleMisconfigured with no Record involved, got: {errs:?}"
        );
    }

    #[test]
    fn i94_type_level_silent_for_well_configured_rule_with_no_records() {
        // Same shape, eligible predicate field — the Type-level pass must not fire
        // on a well-formed rule just because there are no Records to check it against.
        let fields: Vec<Field> = ["f-predicate", "f-target"]
            .iter()
            .map(|id| {
                serde_json::from_value(serde_json::json!({
                    "id": id, "namespace": "com.test", "name": id, "version": 1,
                    "description": "d", "fieldType": { "datatype": "string" },
                    "createdAt": "2026-01-01T00:00:00Z",
                }))
                .expect("field fixture should deserialize")
            })
            .collect();
        let rt: RecordType = serde_json::from_value(serde_json::json!({
            "id": "t-1", "namespace": "com.test", "name": "t", "version": 1,
            "description": "d",
            "fields": [
                { "fieldId": "f-predicate", "order": 0, "required": false },
                { "fieldId": "f-target", "order": 1, "required": false },
            ],
            "validationRules": [{
                "type": "conditional-required",
                "predicateFieldId": "f-predicate",
                "predicateValue": "yes",
                "targetFieldId": "f-target",
            }],
            "createdAt": "2026-01-01T00:00:00Z",
        }))
        .expect("type fixture should deserialize");

        assert!(validate_cross_field_rules_for_type(&rt, &fields).is_empty());
    }

    #[test]
    fn field_ordering_must_precede_passes() {
        let record = make_record(vec![
            ("f-start", serde_json::json!("10")),
            ("f-end", serde_json::json!("20")),
        ]);
        // f-start must precede f-end: start < end → pass
        let rule = ordering_rule("f-end", "f-start", CrossFieldRuleEffect::MustPrecede);
        let ft = ftm(&[("f-start", Datatype::Number), ("f-end", Datatype::Number)]);
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
        let ft = ftm(&[("f-start", Datatype::Number), ("f-end", Datatype::Number)]);
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
        let ft = ftm(&[
            ("f-end-date", Datatype::Date),
            ("f-start-date", Datatype::Date),
        ]);
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
        let ft = ftm(&[
            ("f-end-date", Datatype::Date),
            ("f-start-date", Datatype::Date),
        ]);
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
        let ft = ftm(&[("f-text", Datatype::String), ("f-end", Datatype::String)]);
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
        let ft = ftm(&[("f-start", Datatype::Number), ("f-end", Datatype::Number)]);
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
        let ft = ftm(&[]);
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
        let ft = ftm(&[("f-a", Datatype::String), ("f-b", Datatype::String)]);
        let errs = validate_cross_field_rules(&record, &[rule], &ft);
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
