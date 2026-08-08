use crate::error::RepositoryError;
use crate::record_store;
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::Value;

const BASE_NAMESPACE: &str = "com.semanticops.base";
const BASE_REPO_SETTINGS_TYPE_NAME: &str = "repo_settings";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_mime_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_per_file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_doc_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_total_bytes: Option<u64>,
}

pub struct ReadAttachmentPolicyResult {
    pub policy: AttachmentPolicy,
    pub policy_record_present: bool,
}

/// Read the current attachment policy from the optional `com.semanticops.base/repo_settings`
/// record. Returns built-in defaults (all limits `None`) when the base package is not adopted,
/// when no `repo_settings` record exists, or when the package cannot be loaded.
///
/// Never returns `Err` due to a missing or absent policy — the absence of a policy is not an error.
///
/// TODO(#638): expose via `srs-bindings` WASM binding.
pub fn read_attachment_policy(
    store: &dyn RepositoryStore,
) -> Result<ReadAttachmentPolicyResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    // Scan instance_index for tier-2 records matching com.semanticops.base/repo_settings.
    let mut policy_records = Vec::new();
    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        let Ok(record) = record_store::load_record(store, entry.path()) else {
            continue;
        };
        if record.type_namespace == BASE_NAMESPACE
            && record.type_name == BASE_REPO_SETTINGS_TYPE_NAME
        {
            policy_records.push(record);
        }
    }

    if policy_records.is_empty() {
        return Ok(ReadAttachmentPolicyResult {
            policy: AttachmentPolicy::default(),
            policy_record_present: false,
        });
    }

    // If multiple records exist, use the first (validation.rs reports the duplicate error).
    let policy_record = &policy_records[0];

    let pkg = match store.load_package() {
        Ok(p) => p,
        Err(_) => {
            return Ok(ReadAttachmentPolicyResult {
                policy: AttachmentPolicy::default(),
                policy_record_present: true,
            });
        }
    };

    let get_u64 = |field_name: &str| -> Option<u64> {
        let field = pkg.find_field(BASE_NAMESPACE, field_name)?;
        let fv = policy_record.find_field_value(&field.id)?;
        fv.value.as_u64()
    };

    let max_per_file_bytes = get_u64("max_per_file_bytes");
    let max_doc_bytes = get_u64("max_doc_bytes");
    let max_total_bytes = get_u64("max_total_bytes");

    let allowed_mime_types: Option<Vec<String>> = 'mime: {
        let field = match pkg.find_field(BASE_NAMESPACE, "allowed_mime_types") {
            Some(f) => f,
            None => break 'mime None,
        };
        let fv = match policy_record.find_field_value(&field.id) {
            Some(f) => f,
            None => break 'mime None,
        };
        match &fv.value {
            Value::Array(arr) => Some(
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
            ),
            Value::String(s) => {
                let trimmed = s.trim_start();
                if trimmed.starts_with('[') {
                    // JSON-array string — return None on parse failure (no diagnostic here).
                    serde_json::from_str::<Vec<String>>(trimmed).ok()
                } else {
                    Some(vec![s.clone()])
                }
            }
            _ => None,
        }
    };

    Ok(ReadAttachmentPolicyResult {
        policy: AttachmentPolicy {
            allowed_mime_types,
            max_per_file_bytes,
            max_doc_bytes,
            max_total_bytes,
        },
        policy_record_present: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::{FailPoint, MemoryStore};
    use serde_json::json;
    use srs_core::types::field::Field;
    use srs_core::types::record_type::RecordType;
    use std::path::PathBuf;

    // Synthetic UUIDs for test fixtures — same pattern as validation.rs tests.
    const FIELD_ALLOWED_MIME: &str = "bb000001-0000-4000-b000-000000000001";
    const FIELD_MAX_PER_FILE: &str = "bb000002-0000-4000-b000-000000000002";
    const FIELD_MAX_DOC: &str = "bb000003-0000-4000-b000-000000000003";
    const FIELD_MAX_TOTAL: &str = "bb000004-0000-4000-b000-000000000004";
    const TYPE_ID: &str = "bb000010-0000-4000-b000-000000000010";
    const RECORD_ID: &str = "bb000020-0000-4000-b000-000000000020";
    const RECORD_ID_2: &str = "bb000021-0000-4000-b000-000000000021";

    fn make_base_package() -> Package {
        let allowed_mime_field: Field = serde_json::from_value(json!({
            "id": FIELD_ALLOWED_MIME, "namespace": "com.semanticops.base",
            "name": "allowed_mime_types", "version": 1, "description": "allowed MIME types",
            "aiGuidance": {}, "fieldType": {"datatype": "string", "format": "plain"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_per_file_field: Field = serde_json::from_value(json!({
            "id": FIELD_MAX_PER_FILE, "namespace": "com.semanticops.base",
            "name": "max_per_file_bytes", "version": 1, "description": "max per-file bytes",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_doc_field: Field = serde_json::from_value(json!({
            "id": FIELD_MAX_DOC, "namespace": "com.semanticops.base",
            "name": "max_doc_bytes", "version": 1, "description": "max doc bytes",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_total_field: Field = serde_json::from_value(json!({
            "id": FIELD_MAX_TOTAL, "namespace": "com.semanticops.base",
            "name": "max_total_bytes", "version": 1, "description": "max total bytes",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let repo_settings_type: RecordType = serde_json::from_value(json!({
            "id": TYPE_ID, "namespace": "com.semanticops.base", "name": "repo_settings",
            "version": 1, "description": "repo attachment policy",
            "fields": [
                {"fieldId": FIELD_ALLOWED_MIME, "order": 1, "required": false},
                {"fieldId": FIELD_MAX_PER_FILE, "order": 2, "required": false},
                {"fieldId": FIELD_MAX_DOC, "order": 3, "required": false},
                {"fieldId": FIELD_MAX_TOTAL, "order": 4, "required": false}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        Package {
            id: "bb000000-0000-4000-b000-000000000000".to_string(),
            namespace: "com.semanticops.base".to_string(),
            name: "base".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                allowed_mime_field,
                max_per_file_field,
                max_doc_field,
                max_total_field,
            ],
            record_types: vec![repo_settings_type],
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
        }
    }

    fn policy_record_json(field_values: serde_json::Value) -> serde_json::Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": RECORD_ID,
            "typeId": TYPE_ID,
            "typeVersion": 1,
            "typeNamespace": "com.semanticops.base",
            "typeName": "repo_settings",
            "fieldValues": field_values,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn manifest_with_policy_entry() -> Manifest {
        let manifest_val = json!({
            "srsVersion": "2.0",
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Test",
            "container": {"containerId": "00000000-0000-4000-8000-000000000099", "title": "Test"},
            "instanceIndex": [
                {"instanceId": RECORD_ID, "tier": 2, "path": "records/policy.json"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        serde_json::from_value(manifest_val).unwrap()
    }

    #[test]
    fn read_policy_no_base_package_returns_defaults() {
        // Empty store — no tier-2 records, no package with base namespace.
        let store = MemoryStore::empty();
        let result = read_attachment_policy(&store).expect("should not error");
        assert!(!result.policy_record_present);
        assert_eq!(result.policy, AttachmentPolicy::default());
    }

    #[test]
    fn read_policy_no_repo_settings_record_returns_defaults() {
        // Store has a tier-2 record but of a different type.
        let other_record = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": "aa000001-0000-4000-a000-000000000001",
            "typeId": "aa000010-0000-4000-a000-000000000010",
            "typeVersion": 1,
            "typeNamespace": "com.example.other",
            "typeName": "other_type",
            "fieldValues": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest_val = json!({
            "srsVersion": "2.0",
            "repositoryId": "00000000-0000-4000-8000-000000000001",
            "title": "Test",
            "container": {"containerId": "00000000-0000-4000-8000-000000000001", "title": "Test"},
            "instanceIndex": [
                {"instanceId": "aa000001-0000-4000-a000-000000000001", "tier": 2, "path": "records/other.json"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest: Manifest = serde_json::from_value(manifest_val).unwrap();
        let store = MemoryStore::new(manifest, make_base_package())
            .with_data("records/other.json", other_record);
        let result = read_attachment_policy(&store).expect("should not error");
        assert!(!result.policy_record_present);
        assert_eq!(result.policy, AttachmentPolicy::default());
    }

    #[test]
    fn read_policy_record_present_extracts_limits() {
        let field_values = json!([
            {"fieldId": FIELD_MAX_PER_FILE, "value": 1048576},
            {"fieldId": FIELD_MAX_DOC, "value": 5242880},
            {"fieldId": FIELD_MAX_TOTAL, "value": 104857600},
            {"fieldId": FIELD_ALLOWED_MIME, "value": ["application/pdf", "text/plain"]}
        ]);
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(field_values));

        let result = read_attachment_policy(&store).expect("should not error");
        assert!(result.policy_record_present);
        assert_eq!(result.policy.max_per_file_bytes, Some(1048576));
        assert_eq!(result.policy.max_doc_bytes, Some(5242880));
        assert_eq!(result.policy.max_total_bytes, Some(104857600));
        assert_eq!(
            result.policy.allowed_mime_types,
            Some(vec![
                "application/pdf".to_string(),
                "text/plain".to_string()
            ])
        );
    }

    #[test]
    fn read_policy_allowed_mime_types_array() {
        let field_values = json!([
            {"fieldId": FIELD_ALLOWED_MIME, "value": ["image/png", "image/jpeg"]}
        ]);
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(field_values));

        let result = read_attachment_policy(&store).expect("should not error");
        assert_eq!(
            result.policy.allowed_mime_types,
            Some(vec!["image/png".to_string(), "image/jpeg".to_string()])
        );
    }

    #[test]
    fn read_policy_allowed_mime_types_json_string() {
        // allowed_mime_types stored as a JSON-encoded array string.
        let field_values = json!([
            {"fieldId": FIELD_ALLOWED_MIME, "value": "[\"application/pdf\", \"text/plain\"]"}
        ]);
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(field_values));

        let result = read_attachment_policy(&store).expect("should not error");
        assert_eq!(
            result.policy.allowed_mime_types,
            Some(vec![
                "application/pdf".to_string(),
                "text/plain".to_string()
            ])
        );
    }

    #[test]
    fn read_policy_allowed_mime_types_malformed_json_string() {
        // Malformed JSON array — service returns None silently (no diagnostic).
        // This diverges from validation.rs which pushes a Warning diagnostic.
        let field_values = json!([
            {"fieldId": FIELD_ALLOWED_MIME, "value": "[not valid json"}
        ]);
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(field_values));

        let result = read_attachment_policy(&store).expect("should not error");
        assert!(result.policy_record_present);
        assert!(
            result.policy.allowed_mime_types.is_none(),
            "malformed JSON array must yield None, not an error"
        );
    }

    #[test]
    fn read_policy_allowed_mime_types_single_string() {
        let field_values = json!([
            {"fieldId": FIELD_ALLOWED_MIME, "value": "text/plain"}
        ]);
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(field_values));

        let result = read_attachment_policy(&store).expect("should not error");
        assert_eq!(
            result.policy.allowed_mime_types,
            Some(vec!["text/plain".to_string()])
        );
    }

    #[test]
    fn read_policy_record_present_empty_field_values() {
        // Policy record exists but has no field values set — all limits return None.
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(json!([])));
        let result = read_attachment_policy(&store).expect("should not error");
        assert!(result.policy_record_present);
        assert_eq!(result.policy, AttachmentPolicy::default());
    }

    #[test]
    fn read_policy_record_present_package_load_error() {
        // Policy record is in the index but load_package() fails (e.g. corrupt package.json).
        // The service should return defaults with policy_record_present: true.
        let store = MemoryStore::new(manifest_with_policy_entry(), make_base_package())
            .with_data("records/policy.json", policy_record_json(json!([])))
            .with_fail_at(FailPoint::LoadPackage);
        let result = read_attachment_policy(&store).expect("should not error");
        assert!(result.policy_record_present);
        assert_eq!(result.policy, AttachmentPolicy::default());
    }

    #[test]
    fn read_policy_multiple_records_returns_first() {
        // Two repo_settings records in the manifest — service uses the first.
        let record_1 = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": RECORD_ID,
            "typeId": TYPE_ID, "typeVersion": 1,
            "typeNamespace": "com.semanticops.base", "typeName": "repo_settings",
            "fieldValues": [{"fieldId": FIELD_MAX_PER_FILE, "value": 1000}],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let record_2 = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": RECORD_ID_2,
            "typeId": TYPE_ID, "typeVersion": 1,
            "typeNamespace": "com.semanticops.base", "typeName": "repo_settings",
            "fieldValues": [{"fieldId": FIELD_MAX_PER_FILE, "value": 9999}],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest_val = json!({
            "srsVersion": "2.0",
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Test",
            "container": {"containerId": "00000000-0000-4000-8000-000000000099", "title": "Test"},
            "instanceIndex": [
                {"instanceId": RECORD_ID, "tier": 2, "path": "records/policy.json"},
                {"instanceId": RECORD_ID_2, "tier": 2, "path": "records/policy2.json"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest: Manifest = serde_json::from_value(manifest_val).unwrap();
        let store = MemoryStore::new(manifest, make_base_package())
            .with_data("records/policy.json", record_1)
            .with_data("records/policy2.json", record_2);

        let result = read_attachment_policy(&store).expect("should not error");
        assert!(result.policy_record_present);
        // First record has max_per_file_bytes = 1000, second has 9999.
        assert_eq!(result.policy.max_per_file_bytes, Some(1000));
    }

    #[test]
    fn read_policy_filestore_roundtrip() {
        use crate::store::FileStore;

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".srs")).unwrap();
        std::fs::create_dir_all(repo_root.join("records")).unwrap();
        std::fs::create_dir_all(repo_root.join("package/fields")).unwrap();
        std::fs::create_dir_all(repo_root.join("package/types")).unwrap();

        // Write individual field files (FieldJson format — camelCase).
        let write_json = |rel: &str, val: serde_json::Value| {
            std::fs::write(
                repo_root.join(rel),
                serde_json::to_string_pretty(&val).unwrap(),
            )
            .unwrap()
        };
        write_json(
            "package/fields/allowed_mime_types.json",
            json!({"id": FIELD_ALLOWED_MIME, "namespace": "com.semanticops.base",
                "name": "allowed_mime_types", "version": 1, "fieldType": {"datatype": "string", "format": "plain"},
                "description": "allowed MIME types", "aiGuidance": {},
                "createdAt": "2026-01-01T00:00:00Z"}),
        );
        write_json(
            "package/fields/max_per_file_bytes.json",
            json!({"id": FIELD_MAX_PER_FILE, "namespace": "com.semanticops.base",
                "name": "max_per_file_bytes", "version": 1, "fieldType": {"datatype": "number"},
                "description": "max per-file bytes", "aiGuidance": {},
                "createdAt": "2026-01-01T00:00:00Z"}),
        );
        write_json(
            "package/fields/max_doc_bytes.json",
            json!({"id": FIELD_MAX_DOC, "namespace": "com.semanticops.base",
                "name": "max_doc_bytes", "version": 1, "fieldType": {"datatype": "number"},
                "description": "max doc bytes", "aiGuidance": {},
                "createdAt": "2026-01-01T00:00:00Z"}),
        );
        write_json(
            "package/fields/max_total_bytes.json",
            json!({"id": FIELD_MAX_TOTAL, "namespace": "com.semanticops.base",
                "name": "max_total_bytes", "version": 1, "fieldType": {"datatype": "number"},
                "description": "max total bytes", "aiGuidance": {},
                "createdAt": "2026-01-01T00:00:00Z"}),
        );

        // Write type file.
        write_json(
            "package/types/repo_settings.json",
            json!({"id": TYPE_ID, "namespace": "com.semanticops.base",
            "name": "repo_settings", "version": 1,
            "description": "repo attachment policy",
            "fields": [
                {"fieldId": FIELD_ALLOWED_MIME, "order": 1, "required": false},
                {"fieldId": FIELD_MAX_PER_FILE, "order": 2, "required": false},
                {"fieldId": FIELD_MAX_DOC, "order": 3, "required": false},
                {"fieldId": FIELD_MAX_TOTAL, "order": 4, "required": false}
            ]}),
        );

        // Write package.json referencing the field and type files.
        write_json(
            "package/package.json",
            json!({
                "id": "bb000000-0000-4000-b000-000000000000",
                "namespace": "com.semanticops.base",
                "name": "base", "version": "1.0.0",
                "fields": ["fields/allowed_mime_types.json", "fields/max_per_file_bytes.json",
                           "fields/max_doc_bytes.json", "fields/max_total_bytes.json"],
                "types": ["types/repo_settings.json"]
            }),
        );

        // Write the policy record.
        write_json(
            "records/policy.json",
            json!({"$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": RECORD_ID, "typeId": TYPE_ID, "typeVersion": 1,
                "typeNamespace": "com.semanticops.base", "typeName": "repo_settings",
                "fieldValues": [
                    {"fieldId": FIELD_MAX_PER_FILE, "value": 2097152},
                    {"fieldId": FIELD_MAX_DOC, "value": 10485760}
                ],
                "createdAt": "2026-01-01T00:00:00Z"}),
        );

        // Write manifest.json.
        write_json(
            "manifest.json",
            json!({"$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
                "srsVersion": "2.0",
                "repositoryId": "00000000-0000-4000-8000-000000000099",
                "title": "Roundtrip Test",
                "container": {"containerId": "00000000-0000-4000-8000-000000000099", "title": "Roundtrip Test"},
                "instanceIndex": [
                    {"instanceId": RECORD_ID, "tier": 2, "path": "records/policy.json"}
                ],
                "createdAt": "2026-01-01T00:00:00Z"}),
        );

        let store = FileStore::new(repo_root);
        let result = read_attachment_policy(&store).expect("should not error");

        assert!(
            result.policy_record_present,
            "policy record should be present"
        );
        assert_eq!(result.policy.max_per_file_bytes, Some(2097152));
        assert_eq!(result.policy.max_doc_bytes, Some(10485760));
        assert!(result.policy.max_total_bytes.is_none());
        assert!(result.policy.allowed_mime_types.is_none());
    }
}
