// Semantic names for com.semanticops.core/purpose — used to identify an existing record
// as already-migrated in migrate_identity_service.
pub(crate) const PURPOSE_TYPE_NAMESPACE: &str = "com.semanticops.core";
pub(crate) const PURPOSE_TYPE_NAME: &str = "purpose";

// Drift-guard constants — used only in tests to verify the embedded core-bundle has not drifted.
// build_purpose_record (below) looks these up from core_package::core_package() at runtime
// instead of reading the constants directly. Canonical values from
// srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj
#[cfg(test)]
pub(crate) const PURPOSE_TYPE_ID: &str = "3c000001-0000-4000-a000-000000000001";
#[cfg(test)]
pub(crate) const PURPOSE_TYPE_VERSION: u32 = 1;
#[cfg(test)]
pub(crate) const STATEMENT_FIELD_ID: &str = "3b000001-0000-4000-a000-000000000001";
#[cfg(test)]
pub(crate) const TITLE_FIELD_ID: &str = "3b000002-0000-4000-a000-000000000002";

use srs_core::types::record::{FieldValue, Record};

use std::collections::HashMap;

/// Return the type_id, type_version, and field_values needed to create a
/// `com.semanticops.core/purpose` record via `create_record` / `create_record_at_dir`.
///
/// Use this over `build_purpose_record` when routing through `create_record` so that
/// CFR validation runs at write time (ADR-002, #481).
pub(crate) fn purpose_record_spec(
    statement: &str,
    title: Option<&str>,
) -> (String, u32, Vec<FieldValue>) {
    let cp = crate::core_package::core_package();
    let purpose_type = cp
        .record_types
        .iter()
        .find(|rt| rt.namespace == PURPOSE_TYPE_NAMESPACE && rt.name == PURPOSE_TYPE_NAME)
        .expect("embedded core bundle must contain com.semanticops.core/purpose type");
    let statement_field = cp
        .fields
        .iter()
        .find(|f| f.namespace == "com.semanticops.core" && f.name == "statement")
        .expect("embedded core bundle must contain com.semanticops.core/statement field");

    let mut field_values = vec![FieldValue {
        field_id: statement_field.id.clone(),
        value: serde_json::Value::String(statement.to_string()),
        entries: None,
        source: None,
        edited_at: None,
    }];
    if let Some(t) = title {
        let title_field = cp
            .fields
            .iter()
            .find(|f| f.namespace == "com.semanticops.core" && f.name == "title")
            .expect("embedded core bundle must contain com.semanticops.core/title field");
        field_values.push(FieldValue {
            field_id: title_field.id.clone(),
            value: serde_json::Value::String(t.to_string()),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    (purpose_type.id.clone(), purpose_type.version, field_values)
}

/// Build an in-memory `com.semanticops.core/purpose` Record.
/// Does not perform any I/O — the caller writes and batches as appropriate.
pub(crate) fn build_purpose_record(
    instance_id: &str,
    statement: &str,
    title: Option<&str>,
    now: &str,
) -> Record {
    let cp = crate::core_package::core_package();
    let purpose_type = cp
        .record_types
        .iter()
        .find(|rt| rt.namespace == PURPOSE_TYPE_NAMESPACE && rt.name == PURPOSE_TYPE_NAME)
        .expect("embedded core bundle must contain com.semanticops.core/purpose type");
    let statement_field = cp
        .fields
        .iter()
        .find(|f| f.namespace == "com.semanticops.core" && f.name == "statement")
        .expect("embedded core bundle must contain com.semanticops.core/statement field");

    let mut field_values = vec![FieldValue {
        field_id: statement_field.id.clone(),
        value: serde_json::Value::String(statement.to_string()),
        entries: None,
        source: None,
        edited_at: None,
    }];
    if let Some(t) = title {
        let title_field = cp
            .fields
            .iter()
            .find(|f| f.namespace == "com.semanticops.core" && f.name == "title")
            .expect("embedded core bundle must contain com.semanticops.core/title field");
        field_values.push(FieldValue {
            field_id: title_field.id.clone(),
            value: serde_json::Value::String(t.to_string()),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    Record {
        instance_id: instance_id.to_string(),
        type_id: purpose_type.id.clone(),
        type_version: purpose_type.version,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_purpose_constants_match_embedded_core_package() {
        let cp = crate::core_package::core_package();

        let purpose = cp
            .record_types
            .iter()
            .find(|rt| rt.name == PURPOSE_TYPE_NAME && rt.namespace == PURPOSE_TYPE_NAMESPACE)
            .expect("core package must contain the purpose type");
        assert_eq!(purpose.id, PURPOSE_TYPE_ID);
        assert_eq!(purpose.version, PURPOSE_TYPE_VERSION);

        let statement = cp
            .fields
            .iter()
            .find(|f| f.id == STATEMENT_FIELD_ID)
            .expect("core package must contain the statement field");
        assert_eq!(statement.namespace, "com.semanticops.core");
        assert_eq!(statement.name, "statement");

        let title = cp
            .fields
            .iter()
            .find(|f| f.id == TITLE_FIELD_ID)
            .expect("core package must contain the title field");
        assert_eq!(title.namespace, "com.semanticops.core");
        assert_eq!(title.name, "title");
    }
}
