// Temporary hardcoded UUIDs pending core-type registry (#423).
// Canonical values from srs/srs/package/core/:
//   fields/statement-3b000001.json, fields/title-3b000002.json, types/purpose-3c000001.json
// Replace with core_package::resolve_type("com.semanticops.core", "purpose") once #423 lands.
// STATEMENT_FIELD_ID/TITLE_FIELD_ID are shared by both repository_lifecycle (repo create
// scaffold) and migrate_identity_service (repo migrate-identity) specifically so the two
// paths can't diverge again as they once did (#441) — do not fork a local copy.
pub(crate) const PURPOSE_TYPE_ID: &str = "3c000001-0000-4000-a000-000000000001";
pub(crate) const PURPOSE_TYPE_VERSION: u32 = 1;
pub(crate) const PURPOSE_TYPE_NAMESPACE: &str = "com.semanticops.core";
pub(crate) const PURPOSE_TYPE_NAME: &str = "purpose";
pub(crate) const STATEMENT_FIELD_ID: &str = "3b000001-0000-4000-a000-000000000001";
pub(crate) const TITLE_FIELD_ID: &str = "3b000002-0000-4000-a000-000000000002";

use srs_core::types::record::{FieldValue, Record};
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
