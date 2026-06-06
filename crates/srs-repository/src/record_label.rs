use crate::error::RepositoryError;
use crate::package_service;
use crate::store::RepositoryStore;
use srs_core::types::record::Record;
use std::collections::HashMap;

/// Build a `field_id → field_name` index from the repository package.
/// Load once per batch operation; pass to `record_display_label`.
pub(crate) fn build_field_name_index(
    store: &dyn RepositoryStore,
) -> Result<HashMap<String, String>, RepositoryError> {
    let fields = package_service::list_fields(store)?;
    Ok(fields.into_iter().map(|f| (f.id, f.name)).collect())
}

/// Extract the best display label for a record using a pre-built field name index.
///
/// Priority: field named "title" > "name" > "label" > `type_name` fallback.
pub(crate) fn record_display_label(
    record: &Record,
    field_name_index: &HashMap<String, String>,
) -> String {
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

    #[test]
    fn display_label_finds_title_field() {
        let record = make_record_with_field("f-title", "My Title");
        let index = make_index(&[("f-title", "title")]);
        assert_eq!(record_display_label(&record, &index), "My Title");
    }

    #[test]
    fn display_label_finds_name_field() {
        let record = make_record_with_field("f-name", "My Name");
        let index = make_index(&[("f-name", "name")]);
        assert_eq!(record_display_label(&record, &index), "My Name");
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
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        };
        let index = make_index(&[("f-title", "title"), ("f-name", "name")]);
        assert_eq!(record_display_label(&record, &index), "A Title");
    }

    #[test]
    fn display_label_fallback_to_type_name() {
        let record = make_record_with_field("f-other", "something");
        let index = make_index(&[("f-other", "description")]);
        assert_eq!(record_display_label(&record, &index), "my-type");
    }
}
