// Semantic identifiers for com.semanticops.core/purpose — namespace and name are stable
// string constants; UUIDs and version are looked up from the embedded core bundle at
// runtime so they can never drift from core_package::core_package() (#434).
pub(crate) const PURPOSE_TYPE_NAMESPACE: &str = "com.semanticops.core";
pub(crate) const PURPOSE_TYPE_NAME: &str = "purpose";

use crate::core_package::core_package;
use srs_core::types::record::{FieldValue, Record};
use std::collections::HashMap;

/// Returns the `com.semanticops.core/purpose` type ID from the embedded core bundle.
pub(crate) fn purpose_type_id() -> &'static str {
    core_package()
        .record_types
        .iter()
        .find(|rt| rt.namespace == PURPOSE_TYPE_NAMESPACE && rt.name == PURPOSE_TYPE_NAME)
        .map(|rt| rt.id.as_str())
        .expect("embedded core package must contain the purpose type")
}

/// Returns the `com.semanticops.core/purpose` type version from the embedded core bundle.
pub(crate) fn purpose_type_version() -> u32 {
    core_package()
        .record_types
        .iter()
        .find(|rt| rt.namespace == PURPOSE_TYPE_NAMESPACE && rt.name == PURPOSE_TYPE_NAME)
        .map(|rt| rt.version)
        .expect("embedded core package must contain the purpose type")
}

/// Returns the field ID of `com.semanticops.core/statement` from the embedded core bundle.
pub(crate) fn statement_field_id() -> &'static str {
    core_package()
        .fields
        .iter()
        .find(|f| f.namespace == PURPOSE_TYPE_NAMESPACE && f.name == "statement")
        .map(|f| f.id.as_str())
        .expect("embedded core package must contain the statement field")
}

/// Returns the field ID of `com.semanticops.core/title` from the embedded core bundle.
pub(crate) fn title_field_id() -> &'static str {
    core_package()
        .fields
        .iter()
        .find(|f| f.namespace == PURPOSE_TYPE_NAMESPACE && f.name == "title")
        .map(|f| f.id.as_str())
        .expect("embedded core package must contain the title field")
}

/// Build an in-memory `com.semanticops.core/purpose` Record.
/// Does not perform any I/O — the caller writes and batches as appropriate.
pub(crate) fn build_purpose_record(
    instance_id: &str,
    statement: &str,
    title: Option<&str>,
    now: &str,
) -> Record {
    let mut field_values = vec![FieldValue {
        field_id: statement_field_id().to_string(),
        value: serde_json::Value::String(statement.to_string()),
        entries: None,
        source: None,
        edited_at: None,
    }];
    if let Some(t) = title {
        field_values.push(FieldValue {
            field_id: title_field_id().to_string(),
            value: serde_json::Value::String(t.to_string()),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    Record {
        instance_id: instance_id.to_string(),
        type_id: purpose_type_id().to_string(),
        type_version: purpose_type_version(),
        type_namespace: PURPOSE_TYPE_NAMESPACE.to_string(),
        type_name: PURPOSE_TYPE_NAME.to_string(),
        field_values,
        group_values: None,
        lifecycle_state: None,
        tags: None,
        created_at: Some(now.to_string()),
        updated_at: Some(now.to_string()),
        extra: HashMap::new(),
    }
}
