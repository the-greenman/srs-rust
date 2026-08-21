use crate::types::field::{Lineage, Provenance};
pub use crate::types::lifecycle::{LifecycleState, LifecycleTransition};
use serde::{Deserialize, Serialize};

/// ext:cross-field-validation — rule kinds for CrossFieldRule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrossFieldRuleKind {
    ConditionalRequired,
    FieldOrdering,
    MutualExclusion,
    /// RFC-040 Change F (srs#477/#486): the if/then/not counterpart to
    /// `ConditionalRequired` — the target field is forbidden when the predicate holds.
    ConditionalForbidden,
}

/// ext:cross-field-validation — ordering direction for field-ordering rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrossFieldRuleEffect {
    MustPrecede,
    MustFollow,
}

/// ext:cross-field-validation — a single cross-field constraint on a Type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrossFieldRule {
    #[serde(rename = "type")]
    pub rule_type: CrossFieldRuleKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<CrossFieldRuleEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordType {
    /// The `$schema` pointer the file may carry — declared by the schema itself,
    /// preserved so a loaded-then-written definition keeps it.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub fields: Vec<FieldAssignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends_type_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends_type_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_order: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_assignment_overrides: Option<Vec<FieldAssignmentOverride>>,
    /// RFC-020 — names one fieldId from this Type's effective field set as the
    /// record's identity/display field. Cascades across the ext:type-inheritance
    /// ancestor chain independently of `field_order`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<TypeLifecycle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_ref: Option<String>,
    /// ext:cross-field-validation — cross-field constraints declared on this type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_rules: Option<Vec<CrossFieldRule>>,
    /// Authoring guidance for the Type as a whole (`type.json` `aiGuidance`).
    /// Carried, not interpreted, beyond `blueprint brief` surfacing it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    /// E4 — declared by `type.json`; carried so a load/write round trip keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_object_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// RFC-040 Change E (srs#477/#486): fork/derivation metadata, same shape as
    /// `Field::lineage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<Lineage>,
    /// RFC-040 Change E (srs#477/#486): publish/import metadata, same shape as
    /// `Field::provenance`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    pub created_at: String,
}

/// ext:type-inheritance — per-field overrides for inherited FieldAssignments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldAssignmentOverride {
    pub field_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// ext:lifecycle — state machine declaration on a Type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeLifecycle {
    pub states: Vec<LifecycleState>,
    pub transitions: Vec<LifecycleTransition>,
    pub initial_state: String,
}

// LifecycleState and LifecycleTransition are now defined in lifecycle.rs and re-exported above.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldAssignment {
    pub field_id: String,
    pub order: u32,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
    /// RFC-040 Change C (srs#477/#486): documentation-only annotation, never a
    /// constraint. On conflict the Field's own semantics/`aiGuidance` win — a
    /// contradicting `description` is a data error, not an override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
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
                    description: None,
                },
                FieldAssignment {
                    field_id: "00000000-0000-4000-8000-000000000011".to_string(),
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
            description: None,
        };
        let fa_false = FieldAssignment {
            field_id: "00000000-0000-4000-8000-000000000011".to_string(),
            order: 1,
            required: false,
            display_label: None,
            description: None,
        };

        assert!(fa_true.is_required());
        assert!(!fa_false.is_required());
    }

    #[test]
    fn find_field_assignment_works() {
        let rt = RecordType {
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
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
                description: None,
            }],
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
        };

        assert!(rt
            .find_field_assignment("00000000-0000-4000-8000-000000000010")
            .is_some());
        assert!(rt.find_field_assignment("unknown").is_none());
    }

    #[test]
    fn cross_field_rule_conditional_required_roundtrip() {
        let rule = CrossFieldRule {
            rule_type: CrossFieldRuleKind::ConditionalRequired,
            message: Some("target is required".to_string()),
            predicate_field_id: Some("field-a".to_string()),
            predicate_value: Some("yes".to_string()),
            target_field_id: Some("field-b".to_string()),
            effect: None,
            field_ids: None,
        };
        let value = serde_json::to_value(&rule).unwrap();
        assert_eq!(value["type"], "conditional-required");
        assert_eq!(value["predicateFieldId"], "field-a");
        assert_eq!(value["predicateValue"], "yes");
        assert_eq!(value["targetFieldId"], "field-b");
        assert!(value.get("effect").is_none());
        assert!(value.get("fieldIds").is_none());
        let parsed: CrossFieldRule = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.rule_type, CrossFieldRuleKind::ConditionalRequired);
        assert_eq!(parsed.predicate_field_id.as_deref(), Some("field-a"));
        assert_eq!(parsed.target_field_id.as_deref(), Some("field-b"));
    }

    #[test]
    fn cross_field_rule_field_ordering_roundtrip() {
        let rule = CrossFieldRule {
            rule_type: CrossFieldRuleKind::FieldOrdering,
            message: None,
            predicate_field_id: Some("date-end".to_string()),
            predicate_value: None,
            target_field_id: Some("date-start".to_string()),
            effect: Some(CrossFieldRuleEffect::MustPrecede),
            field_ids: None,
        };
        let value = serde_json::to_value(&rule).unwrap();
        assert_eq!(value["type"], "field-ordering");
        assert_eq!(value["effect"], "must-precede");
        assert!(value.get("message").is_none());
        let parsed: CrossFieldRule = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.rule_type, CrossFieldRuleKind::FieldOrdering);
        assert_eq!(parsed.effect, Some(CrossFieldRuleEffect::MustPrecede));
    }

    #[test]
    fn cross_field_rule_mutual_exclusion_roundtrip() {
        let rule = CrossFieldRule {
            rule_type: CrossFieldRuleKind::MutualExclusion,
            message: None,
            predicate_field_id: None,
            predicate_value: None,
            target_field_id: None,
            effect: None,
            field_ids: Some(vec!["field-a".to_string(), "field-b".to_string()]),
        };
        let value = serde_json::to_value(&rule).unwrap();
        assert_eq!(value["type"], "mutual-exclusion");
        assert_eq!(value["fieldIds"][0], "field-a");
        assert_eq!(value["fieldIds"][1], "field-b");
        let parsed: CrossFieldRule = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.rule_type, CrossFieldRuleKind::MutualExclusion);
        assert_eq!(parsed.field_ids.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn record_type_with_validation_rules_roundtrip() {
        let rt_json = serde_json::json!({
            "id": "rt-1",
            "namespace": "com.test",
            "name": "my-type",
            "version": 1,
            "description": "a type with rules",
            "fields": [],
            "createdAt": "2026-01-01T00:00:00Z",
            "validationRules": [
                {
                    "type": "conditional-required",
                    "predicateFieldId": "f-a",
                    "predicateValue": "yes",
                    "targetFieldId": "f-b"
                },
                {
                    "type": "mutual-exclusion",
                    "fieldIds": ["f-c", "f-d"]
                }
            ]
        });
        let rt: RecordType = serde_json::from_value(rt_json).unwrap();
        assert!(
            rt.validation_rules.is_some(),
            "validationRules must not fall into extra"
        );
        let rules = rt.validation_rules.as_ref().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].rule_type, CrossFieldRuleKind::ConditionalRequired);
        assert_eq!(rules[1].rule_type, CrossFieldRuleKind::MutualExclusion);
        let serialized = serde_json::to_value(&rt).unwrap();
        assert!(serialized.get("validationRules").is_some());
    }

    #[test]
    fn record_type_no_validation_rules_no_key_in_json() {
        let rt = RecordType {
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
            id: "rt-1".to_string(),
            namespace: "com.test".to_string(),
            name: "my-type".to_string(),
            version: 1,
            description: "no rules".to_string(),
            fields: vec![],
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
        };
        let value = serde_json::to_value(&rt).unwrap();
        assert!(
            value.get("validationRules").is_none(),
            "validationRules must not appear when None"
        );
    }

    #[test]
    fn minimal_record_type_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let rt = RecordType {
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
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
                description: None,
            }],
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
        };
        let mut value = serde_json::to_value(&rt).unwrap();
        value["$schema"] = serde_json::json!("https://srs.semanticops.com/schema/2.0/type.json");
        reg.validate_by_id(srs_schema::TYPE_SCHEMA_ID, &value)
            .expect("minimal RecordType must pass type.json schema");
    }

    #[test]
    fn record_type_with_inheritance_fields_roundtrips() {
        let json = serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000020",
            "namespace": "com.test",
            "name": "specializer",
            "version": 1,
            "description": "A specializing type",
            "fields": [],
            "extendsTypeId": "00000000-0000-4000-8000-000000000021",
            "extendsTypeVersion": 1,
            "fieldOrder": ["00000000-0000-4000-8000-000000000010"],
            "fieldAssignmentOverrides": [
                { "fieldId": "00000000-0000-4000-8000-000000000010", "required": true }
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let rt: RecordType = serde_json::from_value(json).unwrap();
        assert_eq!(
            rt.extends_type_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000021")
        );
        assert_eq!(rt.extends_type_version, Some(1));
        assert_eq!(
            rt.field_order.as_ref().map(|v| v[0].as_str()),
            Some("00000000-0000-4000-8000-000000000010")
        );
        assert!(rt.field_assignment_overrides.is_some());
    }

    #[test]
    fn type_with_inheritance_passes_schema() {
        let reg = srs_schema::SchemaRegistry::global();
        let value = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
            "id": "00000000-0000-4000-8000-000000000020",
            "namespace": "com.test",
            "name": "specializer",
            "version": 1,
            "description": "A specializing type",
            "fields": [],
            "extendsTypeId": "00000000-0000-4000-8000-000000000021",
            "extendsTypeVersion": 1,
            "fieldOrder": ["00000000-0000-4000-8000-000000000010"],
            "fieldAssignmentOverrides": [
                {
                    "fieldId": "00000000-0000-4000-8000-000000000010",
                    "displayLabel": "Override Label",
                    "required": true
                }
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        reg.validate_by_id(srs_schema::TYPE_SCHEMA_ID, &value)
            .expect("type with inheritance fields must pass type.json schema");
    }

    #[test]
    fn type_with_identity_field_id_passes_schema() {
        let reg = srs_schema::SchemaRegistry::global();
        let rt = RecordType {
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
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
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: Some("00000000-0000-4000-8000-000000000010".to_string()),
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let mut value = serde_json::to_value(&rt).unwrap();
        value["$schema"] = serde_json::json!("https://srs.semanticops.com/schema/2.0/type.json");
        assert_eq!(
            value["identityFieldId"], "00000000-0000-4000-8000-000000000010",
            "identity_field_id must serialize as camelCase identityFieldId"
        );
        reg.validate_by_id(srs_schema::TYPE_SCHEMA_ID, &value)
            .expect("RecordType with identityFieldId must pass type.json schema");

        let parsed: RecordType = serde_json::from_value(value).unwrap();
        assert_eq!(
            parsed.identity_field_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000010")
        );
    }
}
