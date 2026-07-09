// Hardcoded UUIDs for com.semanticops.core purpose, statement, and title.
// These match the embedded core-bundle (core_package.rs / assets/core-bundle.srsj, #423).
// They will be retired in #434 (WASM binding plan) once callers switch to
// core_package::core_package() lookups. Until then, keep them here — STATEMENT_FIELD_ID and
// TITLE_FIELD_ID are shared by both repository_lifecycle and migrate_identity_service so the
// two paths can't diverge again as they once did (#441).
//
// Canonical values from srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj
pub(crate) const PURPOSE_TYPE_ID: &str = "3c000001-0000-4000-a000-000000000001";
pub(crate) const PURPOSE_TYPE_VERSION: u32 = 1;
pub(crate) const PURPOSE_TYPE_NAMESPACE: &str = "com.semanticops.core";
pub(crate) const PURPOSE_TYPE_NAME: &str = "purpose";
pub(crate) const STATEMENT_FIELD_ID: &str = "3b000001-0000-4000-a000-000000000001";
pub(crate) const TITLE_FIELD_ID: &str = "3b000002-0000-4000-a000-000000000002";

use srs_core::types::record::{FieldValue, Record};

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
use std::collections::HashMap;

/// Build an in-memory `com.semanticops.core/purpose` Record.
/// Does not perform any I/O — the caller writes and batches as appropriate.
pub(crate) fn build_purpose_record(
    instance_id: &str,
    statement: &str,
    title: Option<&str>,
    now: &str,
) -> Record {
    let mut field_values = vec![FieldValue {
        field_id: STATEMENT_FIELD_ID.to_string(),
        value: serde_json::Value::String(statement.to_string()),
        entries: None,
        source: None,
        edited_at: None,
    }];
    if let Some(t) = title {
        field_values.push(FieldValue {
            field_id: TITLE_FIELD_ID.to_string(),
            value: serde_json::Value::String(t.to_string()),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    Record {
        instance_id: instance_id.to_string(),
        type_id: PURPOSE_TYPE_ID.to_string(),
        type_version: PURPOSE_TYPE_VERSION,
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
