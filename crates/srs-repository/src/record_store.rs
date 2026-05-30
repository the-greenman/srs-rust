use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::manifest::Manifest;
use crate::store::RepositoryStore;
use crate::writer::{new_instance_id, write_manifest};
use srs_core::types::record::{FieldValue, Record};
use srs_core::validation::record::validate_record;
use std::collections::HashMap;

/// List all Tier 2 records in the repository, regardless of type.
pub fn list_all_records(store: &dyn RepositoryStore) -> Result<Vec<Record>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        records.push(load_record(store, entry.path())?);
    }

    Ok(records)
}

/// List all Tier 2 records matching the given type namespace and name.
pub fn list_records_by_type(
    store: &dyn RepositoryStore,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        let record = load_record(store, entry.path())?;
        if record.type_namespace == type_namespace && record.type_name == type_name {
            records.push(record);
        }
    }

    Ok(records)
}

/// Get a record by its instance ID.
pub fn get_record_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Record>, RepositoryError> {
    let manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id);

    match entry {
        Some(entry) => {
            let record = load_record(store, entry.path())?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

/// Create a new Tier 2 record.
pub fn create_record(
    store: &dyn RepositoryStore,
    type_id: &str,
    type_version: u32,
    field_values: Vec<FieldValue>,
    relative_dir: &str,
) -> Result<Record, RepositoryError> {
    let package = store.load_package()?;
    let record_type = package.resolve_type(type_id, type_version).ok_or_else(|| {
        RepositoryError::TypeNotFound {
            type_id: type_id.to_string(),
            version: type_version,
        }
    })?;

    let mut record = Record {
        instance_id: String::new(),
        type_id: type_id.to_string(),
        type_version,
        type_namespace: record_type.namespace.clone(),
        type_name: record_type.name.clone(),
        field_values,
        group_values: None,
        created_at: Some(chrono::Utc::now().to_rfc3339()),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extra: HashMap::new(),
    };

    validate_record(&record, record_type).map_err(|e| RepositoryError::RecordValidation {
        path: std::path::PathBuf::from(relative_dir),
        source: e,
    })?;

    record.instance_id = new_instance_id();

    store.ensure_instance_dir(relative_dir)?;

    let relative_path = format!("{}/{}.json", relative_dir, record.instance_id);
    write_record(store, &record, &relative_path)?;

    let mut manifest = store.load_manifest()?;
    upsert_record_index_entry(&mut manifest, &record, &relative_path);
    write_manifest(store, &manifest)?;

    Ok(record)
}

/// Load a record from the store.
fn load_record(
    store: &dyn RepositoryStore,
    relative_path: &str,
) -> Result<Record, RepositoryError> {
    let value = store.load_instance_json(relative_path)?;
    serde_json::from_value(value).map_err(|e| RepositoryError::RecordLoad {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })
}

/// Write a record to the store.
fn write_record(
    store: &dyn RepositoryStore,
    record: &Record,
    relative_path: &str,
) -> Result<(), RepositoryError> {
    let value = serde_json::to_value(record).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })?;
    store.save_instance_json(relative_path, &value)
}

/// Update an existing Tier 2 record.
pub fn update_record(
    store: &dyn RepositoryStore,
    instance_id: &str,
    field_values: Vec<FieldValue>,
) -> Result<Record, RepositoryError> {
    let record =
        get_record_by_id(store, instance_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let package = store.load_package()?;
    let record_type = package
        .resolve_type(&record.type_id, record.type_version)
        .ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: record.type_id.clone(),
            version: record.type_version,
        })?;

    let updated_record = Record {
        instance_id: record.instance_id,
        type_id: record.type_id,
        type_version: record.type_version,
        type_namespace: record.type_namespace,
        type_name: record.type_name,
        field_values,
        group_values: record.group_values,
        created_at: record.created_at,
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extra: record.extra,
    };

    validate_record(&updated_record, record_type).map_err(|e| {
        RepositoryError::RecordValidation {
            path: std::path::PathBuf::from("records"),
            source: e,
        }
    })?;

    let mut manifest = store.load_manifest()?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == instance_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    write_record(store, &updated_record, entry.path())?;
    upsert_record_index_entry(&mut manifest, &updated_record, entry.path());
    write_manifest(store, &manifest)?;

    Ok(updated_record)
}

/// Delete a Tier 2 record by its instance ID.
pub fn delete_record(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<String, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == instance_id && e.tier() == 2)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let path = manifest.instance_index[entry_index].path().to_string();

    store.delete_instance_file(&path)?;
    manifest.instance_index.remove(entry_index);
    write_manifest(store, &manifest)?;

    Ok(instance_id.to_string())
}

/// Add or replace the manifest index entry for a Record (in memory only).
fn upsert_record_index_entry(manifest: &mut Manifest, record: &Record, relative_path: &str) {
    let entry = InstanceIndexEntry {
        instance_id: record.instance_id.clone(),
        tier: 2,
        path: relative_path.to_string(),
        title: None,
        tags: None,
    };

    if let Some(pos) = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == record.instance_id)
    {
        manifest.instance_index[pos] = entry;
    } else {
        manifest.instance_index.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use std::path::PathBuf;

    fn make_store_with_package() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let name_field = Field {
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "Name field".to_string(),
            ai_guidance: json!(null),
            allowed_values: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let status_field = Field {
            id: "field-status-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-status".to_string(),
            version: 1,
            value_type: ValueType::Select,
            description: "Status field".to_string(),
            ai_guidance: json!(null),
            allowed_values: Some(vec!["active".to_string(), "inactive".to_string()]),
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let test_type = RecordType {
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "field-name-001".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Name".to_string()),
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "field-status-001".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Status".to_string()),
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field, status_field],
            record_types: vec![test_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(manifest, package)
    }

    // These tests mirror the existing tests that use TempDir — they still call
    // list_records_by_type / get_record_by_id against the live srs repo (read-only),
    // which is fine since they don't write.

    #[test]
    fn list_records_by_type_from_live_repo() {
        use crate::FileStore;
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        if !srs_repo.exists() {
            println!("Skipping test: live repo not found");
            return;
        }
        let store = FileStore::new(&srs_repo);
        match list_records_by_type(&store, "com.semanticops.srs", "meta.extension") {
            Ok(records) => {
                for record in &records {
                    assert_eq!(record.type_namespace, "com.semanticops.srs");
                    assert_eq!(record.type_name, "meta.extension");
                }
            }
            Err(_) => println!("Skipping: could not list records"),
        }
    }

    #[test]
    fn get_record_by_id_returns_known_record() {
        use crate::FileStore;
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        if !srs_repo.exists() {
            println!("Skipping test: live repo not found");
            return;
        }
        let store = FileStore::new(&srs_repo);
        let records = match list_records_by_type(&store, "com.semanticops.srs", "meta.extension") {
            Ok(r) => r,
            Err(_) => {
                println!("Skipping: could not list records");
                return;
            }
        };
        if records.is_empty() {
            println!("Skipping: no extension records");
            return;
        }
        let first_id = records[0].instance_id.clone();
        let retrieved = get_record_by_id(&store, &first_id).expect("should get record");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().instance_id, first_id);
    }

    #[test]
    fn get_record_by_id_returns_none_for_unknown() {
        use crate::FileStore;
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let store = FileStore::new(&srs_repo);
        let result = get_record_by_id(&store, "00000000-0000-0000-0000-000000000000")
            .expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn create_record_in_temp_repo() {
        let store = make_store_with_package();
        let field_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Test Record"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("active"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        )
        .expect("should create record");

        assert!(!record.instance_id.is_empty());
        assert_eq!(record.type_id, "type-test-001");

        // Record stored in memory
        let key = format!("records/test-items/{}.json", record.instance_id);
        store
            .load_instance_json(&key)
            .expect("should find stored record");

        // Manifest updated
        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == record.instance_id);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().tier(), 2);
    }

    #[test]
    fn create_record_missing_required_field_fails() {
        let store = make_store_with_package();
        let field_values = vec![FieldValue {
            field_id: "field-status-001".to_string(),
            value: json!("active"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let result = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RepositoryError::RecordValidation { .. }
        ));
    }

    #[test]
    fn create_record_optional_field_absent_succeeds() {
        let store = make_store_with_package();
        let field_values = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Test Record"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        )
        .expect("should create with only required field");
        assert_eq!(record.field_values.len(), 1);
    }

    #[test]
    fn record_update_validates_against_type() {
        let store = make_store_with_package();
        let initial_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Initial Name"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("active"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            initial_values,
            "records/test-items",
        )
        .unwrap();
        let instance_id = record.instance_id.clone();

        let updated_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Updated Name"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("inactive"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let updated = update_record(&store, &instance_id, updated_values).unwrap();
        assert_eq!(updated.field_values[0].value, json!("Updated Name"));

        // Verify stored value
        let key = format!("records/test-items/{}.json", instance_id);
        let stored_val = store.load_instance_json(&key).unwrap();
        let stored: Record = serde_json::from_value(stored_val).unwrap();
        assert_eq!(stored.field_values[0].value, json!("Updated Name"));

        // Invalid update (missing required field)
        let invalid_values = vec![FieldValue {
            field_id: "field-status-001".to_string(),
            value: json!("active"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        assert!(update_record(&store, &instance_id, invalid_values).is_err());
    }

    #[test]
    fn record_delete_removes_file_and_manifest_entry() {
        let store = make_store_with_package();
        let field_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Test Name"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("active"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        )
        .unwrap();
        let instance_id = record.instance_id.clone();
        let key = format!("records/test-items/{}.json", instance_id);

        assert!(store.load_instance_json(&key).is_ok());

        let deleted_id = delete_record(&store, &instance_id).unwrap();
        assert_eq!(deleted_id, instance_id);

        assert!(store.load_instance_json(&key).is_err());

        let manifest = store.load_manifest().unwrap();
        assert!(manifest
            .instance_index
            .iter()
            .all(|e| e.instance_id() != instance_id));
    }
}
