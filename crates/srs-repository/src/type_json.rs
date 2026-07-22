//! Shared JSON intermediate for Type definition files (mirror of
//! [`crate::field_json`]).
//!
//! Both loaders (`store.rs` FileStore and `json_store.rs` JsonStore) parse
//! type definition files through [`TypeJson`] and convert with
//! [`TypeJson::into_record_type`]. The `#[serde(flatten)]` tail preserves
//! non-modelled keys (`$schema`, `aiGuidance`, …) into [`RecordType::extra`]
//! so they survive load → edit → save (previously they were silently dropped
//! — the archive/type fidelity bug fixed under #684).

use srs_core::types::record_type::{
    CrossFieldRule, FieldAssignment, FieldAssignmentOverride, FieldGroup, RecordType, TypeLifecycle,
};
use std::collections::HashMap;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TypeJson {
    id: String,
    namespace: String,
    name: String,
    version: u32,
    description: Option<String>,
    fields: Vec<FieldAssignmentJson>,
    #[serde(default)]
    field_groups: Option<Vec<FieldGroupJson>>,
    #[serde(default)]
    extends_type_id: Option<String>,
    #[serde(default)]
    extends_type_version: Option<u32>,
    #[serde(default)]
    field_order: Option<Vec<String>>,
    #[serde(default)]
    field_assignment_overrides: Option<Vec<FieldAssignmentOverrideJson>>,
    #[serde(default)]
    identity_field_id: Option<String>,
    #[serde(default)]
    lifecycle: Option<TypeLifecycle>,
    #[serde(default)]
    lifecycle_ref: Option<String>,
    #[serde(default)]
    validation_rules: Option<Vec<CrossFieldRule>>,
    created_at: Option<String>,
    /// Non-modelled keys (`$schema`, `aiGuidance`, …) — preserved, not dropped.
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldAssignmentJson {
    field_id: String,
    order: u32,
    required: Option<bool>,
    display_label: Option<String>,
    #[serde(default)]
    repeatable: bool,
    min_items: Option<u32>,
    max_items: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldGroupJson {
    group_id: String,
    order: u32,
    fields: Vec<FieldAssignmentJson>,
    label: Option<String>,
    description: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    repeatable: bool,
    min_items: Option<u32>,
    max_items: Option<u32>,
    composite_renderer: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldAssignmentOverrideJson {
    field_id: String,
    display_label: Option<String>,
    display_hint: Option<String>,
    required: Option<bool>,
}

fn into_assignment(fa: FieldAssignmentJson) -> FieldAssignment {
    FieldAssignment {
        field_id: fa.field_id,
        order: fa.order,
        required: fa.required.unwrap_or(true),
        display_label: fa.display_label,
        repeatable: fa.repeatable,
        min_items: fa.min_items,
        max_items: fa.max_items,
    }
}

impl TypeJson {
    pub(crate) fn into_record_type(self) -> RecordType {
        let fields: Vec<FieldAssignment> = self.fields.into_iter().map(into_assignment).collect();
        let field_groups = self.field_groups.map(|groups| {
            groups
                .into_iter()
                .map(|g| FieldGroup {
                    group_id: g.group_id,
                    order: g.order,
                    fields: g.fields.into_iter().map(into_assignment).collect(),
                    label: g.label,
                    description: g.description,
                    required: g.required,
                    repeatable: g.repeatable,
                    min_items: g.min_items,
                    max_items: g.max_items,
                    composite_renderer: g.composite_renderer,
                })
                .collect()
        });
        let field_assignment_overrides = self.field_assignment_overrides.map(|overrides| {
            overrides
                .into_iter()
                .map(|o| FieldAssignmentOverride {
                    field_id: o.field_id,
                    display_label: o.display_label,
                    display_hint: o.display_hint,
                    required: o.required,
                })
                .collect()
        });
        RecordType {
            id: self.id,
            namespace: self.namespace,
            name: self.name,
            version: self.version,
            description: self.description.unwrap_or_default(),
            fields,
            field_groups,
            extends_type_id: self.extends_type_id,
            extends_type_version: self.extends_type_version,
            field_order: self.field_order,
            field_assignment_overrides,
            identity_field_id: self.identity_field_id,
            lifecycle: self.lifecycle,
            lifecycle_ref: self.lifecycle_ref,
            validation_rules: self.validation_rules,
            created_at: self.created_at.unwrap_or_default(),
            extra: self.extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_keys_survive_into_record_type() {
        let tj: TypeJson = serde_json::from_str(
            r#"{
                "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                "id": "t1",
                "namespace": "com.test",
                "name": "thing",
                "version": 1,
                "aiGuidance": "guidance text",
                "fields": [{"fieldId": "f1", "order": 1}]
            }"#,
        )
        .unwrap();
        let rt = tj.into_record_type();
        assert_eq!(
            rt.extra.get("$schema").and_then(|v| v.as_str()),
            Some("https://srs.semanticops.com/schema/2.0/type.json")
        );
        assert_eq!(
            rt.extra.get("aiGuidance").and_then(|v| v.as_str()),
            Some("guidance text")
        );
        // Round-trip: serialization re-emits the preserved keys.
        let val = serde_json::to_value(&rt).unwrap();
        assert_eq!(val["aiGuidance"], "guidance text");
    }
}
