use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::record::Record;
use std::collections::HashMap;

/// `field_id → field_name`.
pub(crate) type FieldNameIndex = HashMap<String, String>;
/// RFC-020 — `(type_id, type_version) → identityFieldId`.
pub(crate) type IdentityFieldIndex = HashMap<(String, u32), String>;

/// RFC-020 — derive a `(type_id, type_version) → identityFieldId` index from an
/// already-loaded `Package`.
///
/// Only Types with a resolved effective `identityFieldId` get an entry — absence (for any
/// reason, including a resolution error) means "fall through to the name ladder." A Type whose
/// `effective_identity_field_id` errors (e.g. an inheritance cycle on that Type) is silently
/// skipped here rather than propagated: this index is consumed via `?` by every label-producing
/// call site, so propagating would hard-fail record listing/display repository-wide whenever any
/// single Type has a broken chain. Rule [N+33] validation (`validation.rs`) is the correct,
/// non-silent surface for that failure — this index-builder's job is graceful degradation.
pub(crate) fn identity_field_index_from_package(
    package: &crate::package::Package,
) -> IdentityFieldIndex {
    let mut index = HashMap::new();
    for rt in package.record_types() {
        if let Ok(Some(field_id)) = package.effective_identity_field_id(rt) {
            index.insert((rt.id.clone(), rt.version), field_id);
        }
    }
    index
}

/// Build both the `field_id → field_name` and `(type_id, type_version) → identityFieldId`
/// indexes from a single `Package` load. Every `record_display_label` call site needs both
/// indexes together, so this loads the package exactly once rather than twice
/// (`store.load_package()` has no caching and re-reads/re-parses every package file on
/// `FileStore`). Load once per batch operation; pass both maps to `record_display_label`.
pub(crate) fn build_label_indexes(
    store: &dyn RepositoryStore,
) -> Result<(FieldNameIndex, IdentityFieldIndex), RepositoryError> {
    let package = store.load_package()?;
    let field_name_index = package
        .fields
        .iter()
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();
    let identity_field_index = identity_field_index_from_package(&package);
    Ok((field_name_index, identity_field_index))
}

/// Extract the best display label for a record using pre-built identity and field name indexes.
///
/// Priority (RFC-020 Rule [N+36]): the record's Type's effective `identityFieldId`, when
/// present and its value is a non-empty string > field named "title" > "name" > "label" >
/// `type_name` fallback.
pub(crate) fn record_display_label(
    record: &Record,
    identity_field_index: &IdentityFieldIndex,
    field_name_index: &FieldNameIndex,
) -> String {
    if let Some(field_id) = identity_field_index.get(&(record.type_id.clone(), record.type_version))
    {
        if let Some(fv) = record
            .field_values
            .iter()
            .find(|fv| &fv.field_id == field_id)
        {
            if let Some(s) = fv.value.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    for priority in &["title", "name", "label"] {
        for fv in &record.field_values {
            if field_name_index.get(&fv.field_id).map(|n| n.as_str()) == Some(priority) {
                if let Some(s) = fv.value.as_str() {
                    return s.to_string();
                }
            }
        }
    }
    record.type_name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::record::{FieldValue, Record};
    use std::collections::HashMap;

    fn make_record_with_field(field_id: &str, value: &str) -> Record {
        Record {
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "my-type".to_string(),
            field_values: vec![FieldValue {
                field_id: field_id.to_string(),
                value: serde_json::json!(value),
                entries: None,
                source: None,
                edited_at: None,
            }],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    fn make_index(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect()
    }

    fn empty_identity_index() -> HashMap<(String, u32), String> {
        HashMap::new()
    }

    fn make_identity_index(
        type_id: &str,
        type_version: u32,
        field_id: &str,
    ) -> HashMap<(String, u32), String> {
        let mut m = HashMap::new();
        m.insert((type_id.to_string(), type_version), field_id.to_string());
        m
    }

    #[test]
    fn display_label_finds_title_field() {
        let record = make_record_with_field("f-title", "My Title");
        let index = make_index(&[("f-title", "title")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "My Title"
        );
    }

    #[test]
    fn display_label_finds_name_field() {
        let record = make_record_with_field("f-name", "My Name");
        let index = make_index(&[("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "My Name"
        );
    }

    #[test]
    fn display_label_title_takes_priority_over_name() {
        let record = Record {
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "my-type".to_string(),
            field_values: vec![
                FieldValue {
                    field_id: "f-name".to_string(),
                    value: serde_json::json!("A Name"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "f-title".to_string(),
                    value: serde_json::json!("A Title"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };
        let index = make_index(&[("f-title", "title"), ("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "A Title"
        );
    }

    #[test]
    fn display_label_fallback_to_type_name() {
        let record = make_record_with_field("f-other", "something");
        let index = make_index(&[("f-other", "description")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "my-type"
        );
    }

    #[test]
    fn display_label_identity_field_wins_over_title_field() {
        // Record has both an identityFieldId-mapped field ("f-heading") and a "title"-named
        // field with a different value — the identity field must win (RFC-020 Rule [N+36]).
        let record = Record {
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "my-type".to_string(),
            field_values: vec![
                FieldValue {
                    field_id: "f-title".to_string(),
                    value: serde_json::json!("Ignored Title"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "f-heading".to_string(),
                    value: serde_json::json!("The Real Heading"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };
        let identity_index = make_identity_index("t1", 1, "f-heading");
        let name_index = make_index(&[("f-title", "title")]);
        assert_eq!(
            record_display_label(&record, &identity_index, &name_index),
            "The Real Heading"
        );
    }

    #[test]
    fn display_label_no_identity_field_id_falls_through_to_name_ladder_unchanged() {
        // Regression: a record whose Type has no identityFieldId entry in the index must
        // behave exactly as before this change — pure name-ladder resolution.
        let record = make_record_with_field("f-name", "My Name");
        let name_index = make_index(&[("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &name_index),
            "My Name"
        );
    }

    #[test]
    fn display_label_identity_field_empty_value_falls_through_to_name_ladder() {
        let record = Record {
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "my-type".to_string(),
            field_values: vec![
                FieldValue {
                    field_id: "f-heading".to_string(),
                    value: serde_json::json!(""),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: "f-name".to_string(),
                    value: serde_json::json!("Fallback Name"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };
        let identity_index = make_identity_index("t1", 1, "f-heading");
        let name_index = make_index(&[("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &identity_index, &name_index),
            "Fallback Name"
        );
    }

    #[test]
    fn identity_field_index_from_package_skips_broken_type_without_erroring() {
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use std::path::PathBuf;

        fn fa(field_id: &str) -> FieldAssignment {
            FieldAssignment {
                field_id: field_id.to_string(),
                order: 0,
                required: true,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            }
        }

        fn rt(id: &str, identity_field_id: Option<&str>, extends: Option<&str>) -> RecordType {
            RecordType {
                id: id.to_string(),
                namespace: "com.test".to_string(),
                name: id.to_string(),
                version: 1,
                description: "test".to_string(),
                fields: vec![fa("f1")],
                field_groups: None,
                extends_type_id: extends.map(|s| s.to_string()),
                extends_type_version: extends.map(|_| 1),
                field_order: None,
                field_assignment_overrides: None,
                identity_field_id: identity_field_id.map(|s| s.to_string()),
                lifecycle: None,
                lifecycle_ref: None,
                validation_rules: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                extra: HashMap::new(),
            }
        }

        let valid_type = rt("valid-type", Some("f1"), None);
        // Self-extending — effective_identity_field_id will hit TypeInheritanceCycle.
        let broken_type = rt("broken-type", None, Some("broken-type"));

        let package = crate::package::Package {
            id: "pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![valid_type, broken_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        let index = identity_field_index_from_package(&package);
        assert_eq!(
            index.get(&("valid-type".to_string(), 1)),
            Some(&"f1".to_string()),
            "valid type must have an entry"
        );
        assert!(
            !index.contains_key(&("broken-type".to_string(), 1)),
            "broken type must be skipped, not error the whole build"
        );
    }
}
