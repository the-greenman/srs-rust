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
/// present and its value is a non-empty string > field named "title" > "heading" > "name" >
/// "label" > `type_name` fallback. The name ladder is an implementation-specific heuristic
/// (per Rule [N+36] — the spec leaves the specific field names to the implementation).
pub(crate) fn record_display_label(
    record: &Record,
    identity_field_index: &IdentityFieldIndex,
    field_name_index: &FieldNameIndex,
) -> String {
    if let Some(field_id) = identity_field_index.get(&(record.type_id.clone(), record.type_version))
    {
        // RFC-039: the carrier keys by Field.name — recover the identity
        // field's name through the index built from the package.
        if let Some(name) = field_name_index.get(field_id) {
            if let Some(s) = record.value_str(name) {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    // "heading" is a conventional section-title field name, more specific than the
    // catch-all "name" identifier, so it ranks higher in the heuristic fallback.
    // Under the name-keyed carrier the ladder is a direct key probe ([R2b]).
    for priority in &["title", "heading", "name", "label"] {
        if let Some(s) = record.value_str(priority) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    record.type_name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::record::{FieldValues, Record};
    use std::collections::HashMap;

    fn make_record_with_field(name: &str, value: &str) -> Record {
        Record {
            field_meta: None,
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "my-type".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert(name, serde_json::json!(value));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn make_two_field_record(name1: &str, val1: &str, name2: &str, val2: &str) -> Record {
        Record {
            field_meta: None,
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "my-type".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert(name1, serde_json::json!(val1));
                fv.insert(name2, serde_json::json!(val2));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
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
        let record = make_record_with_field("title", "My Title");
        let index = make_index(&[("f-title", "title")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "My Title"
        );
    }

    #[test]
    fn display_label_finds_name_field() {
        let record = make_record_with_field("name", "My Name");
        let index = make_index(&[("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "My Name"
        );
    }

    #[test]
    fn display_label_finds_heading_field() {
        let record = make_record_with_field("heading", "Section Alpha");
        let index = make_index(&[("f-heading", "heading")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "Section Alpha"
        );
    }

    #[test]
    fn display_label_title_takes_priority_over_heading() {
        let record = make_two_field_record("heading", "A Heading", "title", "A Title");
        let index = make_index(&[("f-heading", "heading"), ("f-title", "title")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "A Title"
        );
    }

    #[test]
    fn display_label_heading_takes_priority_over_name() {
        let record = make_two_field_record("heading", "Heading Value", "name", "Name Value");
        let index = make_index(&[("f-heading", "heading"), ("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "Heading Value"
        );
    }

    #[test]
    fn display_label_name_ladder_skips_empty_string() {
        // Empty heading must fall through to name, not return "".
        let record = make_two_field_record("heading", "", "name", "Fallback Name");
        let index = make_index(&[("f-heading", "heading"), ("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "Fallback Name"
        );
    }

    #[test]
    fn display_label_title_takes_priority_over_name() {
        let record = make_two_field_record("name", "A Name", "title", "A Title");
        let index = make_index(&[("f-title", "title"), ("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &index),
            "A Title"
        );
    }

    #[test]
    fn display_label_fallback_to_type_name() {
        let record = make_record_with_field("description", "something");
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
        let record = make_two_field_record("title", "Ignored Title", "heading", "The Real Heading");
        let identity_index = make_identity_index("t1", 1, "f-heading");
        let name_index = make_index(&[("f-title", "title"), ("f-heading", "heading")]);
        assert_eq!(
            record_display_label(&record, &identity_index, &name_index),
            "The Real Heading"
        );
    }

    #[test]
    fn display_label_no_identity_field_id_falls_through_to_name_ladder_unchanged() {
        // Regression: a record whose Type has no identityFieldId entry in the index must
        // behave exactly as before this change — pure name-ladder resolution.
        let record = make_record_with_field("name", "My Name");
        let name_index = make_index(&[("f-name", "name")]);
        assert_eq!(
            record_display_label(&record, &empty_identity_index(), &name_index),
            "My Name"
        );
    }

    #[test]
    fn display_label_identity_field_empty_value_falls_through_to_name_ladder() {
        let record = make_two_field_record("heading", "", "name", "Fallback Name");
        let identity_index = make_identity_index("t1", 1, "f-heading");
        let name_index = make_index(&[("f-name", "name"), ("f-heading", "heading")]);
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
                description: None,
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
                extends_type_id: extends.map(|s| s.to_string()),
                extends_type_version: extends.map(|_| 1),
                field_order: None,
                field_assignment_overrides: None,
                identity_field_id: identity_field_id.map(|s| s.to_string()),
                lifecycle: None,
                lifecycle_ref: None,
                validation_rules: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                extra: std::collections::BTreeMap::new(),
                lineage: None,
                provenance: None,
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
