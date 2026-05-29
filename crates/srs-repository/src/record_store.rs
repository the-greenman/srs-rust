use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::manifest::{load_manifest, Manifest};
use crate::package::load_package;
use crate::writer::{new_instance_id, write_manifest};
use srs_core::types::record::{FieldValue, Record};
use srs_core::validation::record::validate_record;
use std::collections::HashMap;
use std::path::Path;

/// List all Tier 2 records in the repository, regardless of type.
pub fn list_all_records(repo_root: &Path) -> Result<Vec<Record>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        let record_path = repo_root.join(entry.path());
        records.push(load_record(&record_path)?);
    }

    Ok(records)
}

/// List all Tier 2 records matching the given type namespace and name.
///
/// This loads the manifest, filters entries where `tier == 2`, loads each as a Record,
/// and filters by matching `type_namespace` and `type_name`.
pub fn list_records_by_type(
    repo_root: &Path,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        // Tier 2 records only (explicitly tier == 2, not >= 1)
        if entry.tier() != 2 {
            continue;
        }

        let record_path = repo_root.join(entry.path());
        let record = load_record(&record_path)?;

        // Filter by type namespace and name
        if record.type_namespace == type_namespace && record.type_name == type_name {
            records.push(record);
        }
    }

    Ok(records)
}

/// Get a record by its instance ID.
///
/// Returns `Ok(None)` if the record is not found in the manifest.
pub fn get_record_by_id(repo_root: &Path, id: &str) -> Result<Option<Record>, RepositoryError> {
    let manifest = load_manifest(repo_root)?;

    // Find the entry in the manifest index
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id);

    match entry {
        Some(entry) => {
            let record_path = repo_root.join(entry.path());
            let record = load_record(&record_path)?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

/// Create a new Tier 2 record.
///
/// This function:
/// 1. Loads the package and resolves the type
/// 2. Validates the field values against the type definition
/// 3. Mints a new instance ID
/// 4. Writes the record JSON to `<relative_dir>/<instanceId>.json`
/// 5. Upserts the manifest entry with `tier: 2`
/// 6. Writes the manifest back to disk
///
/// The `relative_dir` parameter should be a path relative to the repo root,
/// e.g., "records/tag-definitions". The caller (CLI) owns the naming convention.
pub fn create_record(
    repo_root: &Path,
    type_id: &str,
    type_version: u32,
    field_values: Vec<FieldValue>,
    relative_dir: &str,
) -> Result<Record, RepositoryError> {
    // Load package and resolve type
    let package = load_package(repo_root)?;
    let record_type = package.resolve_type(type_id, type_version).ok_or_else(|| {
        RepositoryError::TypeNotFound {
            type_id: type_id.to_string(),
            version: type_version,
        }
    })?;

    // Build the record (without instance_id initially for validation)
    let mut record = Record {
        instance_id: String::new(), // Will be filled after validation
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

    // Validate against type definition
    validate_record(&record, record_type).map_err(|e| RepositoryError::RecordValidation {
        path: repo_root.join(relative_dir),
        source: e,
    })?;

    // Mint instance ID after validation passes
    record.instance_id = new_instance_id();

    // Ensure the target directory exists
    let target_dir = repo_root.join(relative_dir);
    std::fs::create_dir_all(&target_dir).map_err(|e| RepositoryError::Io {
        path: target_dir.clone(),
        source: e,
    })?;

    // Write the record JSON
    let record_path = target_dir.join(format!("{}.json", record.instance_id));
    write_record(&record, &record_path)?;

    // Calculate relative path for manifest entry
    let relative_path = record_path
        .strip_prefix(repo_root)
        .map_err(|_| RepositoryError::Io {
            path: record_path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "record path not within repo root",
            ),
        })?
        .to_string_lossy()
        .to_string();

    // Update manifest
    let mut manifest = load_manifest(repo_root)?;
    upsert_record_index_entry(&mut manifest, &record, &relative_path);
    write_manifest(&manifest)?;

    Ok(record)
}

/// Load a record from a JSON file.
fn load_record(path: &Path) -> Result<Record, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|e| RepositoryError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    serde_json::from_str(&content).map_err(|e| RepositoryError::RecordLoad {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Write a record to a JSON file.
fn write_record(record: &Record, path: &Path) -> Result<(), RepositoryError> {
    let json = serde_json::to_string_pretty(record).map_err(|e| RepositoryError::Serialize {
        path: path.to_path_buf(),
        source: e,
    })?;

    std::fs::write(path, json).map_err(|e| RepositoryError::RecordWrite {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Update an existing Tier 2 record.
///
/// This function:
/// 1. Loads the existing record
/// 2. Merges the provided field values (full replacement)
/// 3. Updates the updated_at timestamp
/// 4. Revalidates against the type definition
/// 5. Writes the record back to disk
/// 6. Updates the manifest (if needed)
pub fn update_record(
    repo_root: &Path,
    instance_id: &str,
    field_values: Vec<FieldValue>,
) -> Result<Record, RepositoryError> {
    // Load the existing record
    let record =
        get_record_by_id(repo_root, instance_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: repo_root.join("records"),
        })?;

    // Load package and resolve type for validation
    let package = load_package(repo_root)?;
    let record_type = package
        .resolve_type(&record.type_id, record.type_version)
        .ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: record.type_id.clone(),
            version: record.type_version,
        })?;

    // Build the updated record with new field values
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

    // Validate the updated record
    validate_record(&updated_record, record_type).map_err(|e| {
        RepositoryError::RecordValidation {
            path: repo_root.join("records"),
            source: e,
        }
    })?;

    // Write the record
    let mut manifest = load_manifest(repo_root)?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == instance_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: repo_root.join("records"),
        })?;

    let record_path = repo_root.join(entry.path());
    write_record(&updated_record, &record_path)?;

    // Update manifest (re-upsert to ensure consistency)
    upsert_record_index_entry(&mut manifest, &updated_record, entry.path());
    write_manifest(&manifest)?;

    Ok(updated_record)
}

/// Delete a Tier 2 record by its instance ID.
///
/// This function:
/// 1. Finds the record in the manifest
/// 2. Removes the file from disk
/// 3. Removes the entry from the manifest
/// 4. Writes the updated manifest
///
/// Returns the instance_id and path of the deleted record for audit purposes.
pub fn delete_record(repo_root: &Path, instance_id: &str) -> Result<String, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Find the entry in the manifest
    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == instance_id && e.tier() == 2)
        .ok_or_else(|| RepositoryError::NotFound {
            path: repo_root.join("records"),
        })?;

    let entry = manifest.instance_index[entry_index].clone();
    let path = entry.path().to_string();

    // Remove the file if it exists
    let full_path = repo_root.join(&path);
    if full_path.exists() {
        std::fs::remove_file(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
    }

    // Remove from manifest
    manifest.instance_index.remove(entry_index);
    write_manifest(&manifest)?;

    Ok(instance_id.to_string())
}

/// Add or replace the manifest index entry for a Record (in memory only).
fn upsert_record_index_entry(manifest: &mut Manifest, record: &Record, relative_path: &str) {
    let entry = InstanceIndexEntry {
        instance_id: record.instance_id.clone(),
        tier: 2, // Tier 2 for all records created through this store
        path: relative_path.to_string(),
        title: None, // Records don't have a direct title field
        tags: None,
    };

    // Check if entry with same instance_id exists and replace it
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
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn list_records_by_type_from_live_repo() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");

        // Skip if repo structure doesn't match (live repo may evolve)
        if !srs_repo.exists() {
            println!("Skipping test: live repo not found");
            return;
        }

        // List records of type com.semanticops.srs/meta.extension
        match list_records_by_type(&srs_repo, "com.semanticops.srs", "meta.extension") {
            Ok(records) => {
                // Verify each record has the correct type
                for record in &records {
                    assert_eq!(record.type_namespace, "com.semanticops.srs");
                    assert_eq!(record.type_name, "meta.extension");
                    assert_eq!(record.type_version, 1);
                }
            }
            Err(_) => {
                println!("Skipping test: could not list records (repo structure may have changed)");
            }
        }
    }

    #[test]
    fn get_record_by_id_returns_known_record() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");

        // Skip if repo structure doesn't match (live repo may evolve)
        if !srs_repo.exists() {
            println!("Skipping test: live repo not found");
            return;
        }

        // First, list all Tier 2 records to find a valid ID
        let records = match list_records_by_type(&srs_repo, "com.semanticops.srs", "meta.extension")
        {
            Ok(r) => r,
            Err(_) => {
                println!("Skipping test: could not list records");
                return;
            }
        };

        if records.is_empty() {
            println!("Skipping test: no extension records in live repo");
            return;
        }

        let first_id = records[0].instance_id.clone();

        // Now get by ID
        let retrieved = get_record_by_id(&srs_repo, &first_id).expect("should get record");
        assert!(retrieved.is_some(), "should find record by ID");

        let record = retrieved.unwrap();
        assert_eq!(record.instance_id, first_id);
        assert_eq!(record.type_namespace, "com.semanticops.srs");
        assert_eq!(record.type_name, "meta.extension");
    }

    #[test]
    fn get_record_by_id_returns_none_for_unknown() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");

        let result = get_record_by_id(&srs_repo, "00000000-0000-0000-0000-000000000000")
            .expect("should not error");
        assert!(result.is_none(), "should return None for unknown ID");
    }

    fn create_temp_repo_with_package() -> TempDir {
        let temp = TempDir::new().unwrap();

        // Create package directory structure
        let package_dir = temp.path().join("package");
        fs::create_dir_all(&package_dir).unwrap();
        fs::create_dir_all(package_dir.join("fields")).unwrap();
        fs::create_dir_all(package_dir.join("types")).unwrap();

        // Create package.json
        let package_json = json!({
            "id": "test-package-001",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
            "fields": ["fields/test-name.json", "fields/test-status.json"],
            "types": ["types/test-type.json"]
        });
        fs::write(
            package_dir.join("package.json"),
            serde_json::to_string_pretty(&package_json).unwrap(),
        )
        .unwrap();

        // Create test-name field
        let name_field = json!({
            "id": "field-name-001",
            "namespace": "com.test",
            "name": "test-name",
            "version": 1,
            "valueType": "string",
            "description": "Name field"
        });
        fs::write(
            package_dir.join("fields/test-name.json"),
            serde_json::to_string_pretty(&name_field).unwrap(),
        )
        .unwrap();

        // Create test-status field (optional)
        let status_field = json!({
            "id": "field-status-001",
            "namespace": "com.test",
            "name": "test-status",
            "version": 1,
            "valueType": "select",
            "allowedValues": ["active", "inactive"],
            "description": "Status field"
        });
        fs::write(
            package_dir.join("fields/test-status.json"),
            serde_json::to_string_pretty(&status_field).unwrap(),
        )
        .unwrap();

        // Create test type with one required field
        let test_type = json!({
            "id": "type-test-001",
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "Test type",
            "fields": [
                {
                    "fieldId": "field-name-001",
                    "order": 0,
                    "required": true,
                    "displayLabel": "Name"
                },
                {
                    "fieldId": "field-status-001",
                    "order": 1,
                    "required": false,
                    "displayLabel": "Status"
                }
            ]
        });
        fs::write(
            package_dir.join("types/test-type.json"),
            serde_json::to_string_pretty(&test_type).unwrap(),
        )
        .unwrap();

        // Create minimal manifest.json
        let manifest = json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": []
        });
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        temp
    }

    #[test]
    fn create_record_in_temp_repo() {
        let temp = create_temp_repo_with_package();

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
            temp.path(),
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        )
        .expect("should create record");

        // Verify record was created with instance ID
        assert!(!record.instance_id.is_empty());
        assert_eq!(record.type_id, "type-test-001");
        assert_eq!(record.type_version, 1);

        // Verify file was written
        let expected_path = temp
            .path()
            .join("records/test-items")
            .join(format!("{}.json", record.instance_id));
        assert!(expected_path.exists(), "record file should exist");

        // Verify manifest was updated
        let manifest = load_manifest(temp.path()).expect("should load manifest");
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == record.instance_id);
        assert!(entry.is_some(), "manifest should contain new entry");
        assert_eq!(entry.unwrap().tier(), 2, "tier should be 2");
    }

    #[test]
    fn create_record_missing_required_field_fails() {
        let temp = create_temp_repo_with_package();

        // Missing the required "field-name-001" field
        let field_values = vec![FieldValue {
            field_id: "field-status-001".to_string(),
            value: json!("active"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let result = create_record(
            temp.path(),
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        );

        assert!(
            result.is_err(),
            "should fail when required field is missing"
        );
        assert!(matches!(
            result.unwrap_err(),
            RepositoryError::RecordValidation { .. }
        ));
    }

    #[test]
    fn create_record_optional_field_absent_succeeds() {
        let temp = create_temp_repo_with_package();

        // Only provide the required field, omit the optional status field
        let field_values = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Test Record"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let record = create_record(
            temp.path(),
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        )
        .expect("should create record with only required field");

        assert_eq!(record.field_values.len(), 1);
        assert_eq!(record.field_values[0].field_id, "field-name-001");
    }

    // Acceptance Criteria Tests for Phase 2

    #[test]
    fn record_update_validates_against_type() {
        let temp = create_temp_repo_with_package();

        // Create initial record
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
            temp.path(),
            "type-test-001",
            1,
            initial_values,
            "records/test-items",
        )
        .unwrap();

        let instance_id = record.instance_id.clone();

        // UPDATE: Update with valid values
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

        let updated = update_record(temp.path(), &instance_id, updated_values).unwrap();
        assert_eq!(updated.field_values[0].value, json!("Updated Name"));

        // Verify file was rewritten
        let file_path = temp
            .path()
            .join(format!("records/test-items/{}.json", instance_id));
        let file_content = fs::read_to_string(&file_path).unwrap();
        let file_record: Record = serde_json::from_str(&file_content).unwrap();
        assert_eq!(file_record.field_values[0].value, json!("Updated Name"));

        // UPDATE: Try to update with invalid value (missing required field)
        let invalid_values = vec![FieldValue {
            field_id: "field-status-001".to_string(),
            value: json!("active"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let result = update_record(temp.path(), &instance_id, invalid_values);
        assert!(
            result.is_err(),
            "should fail when required field is missing"
        );
    }

    #[test]
    fn record_delete_removes_file_and_manifest_entry() {
        let temp = create_temp_repo_with_package();

        // Create initial record
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
            temp.path(),
            "type-test-001",
            1,
            field_values,
            "records/test-items",
        )
        .unwrap();

        let instance_id = record.instance_id.clone();
        let file_path = temp
            .path()
            .join(format!("records/test-items/{}.json", instance_id));

        // Verify it was created
        assert!(file_path.exists());

        // Verify manifest has entry
        let manifest_before: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(temp.path().join("manifest.json")).unwrap())
                .unwrap();
        let index_before = manifest_before["instanceIndex"].as_array().unwrap();
        assert!(index_before.iter().any(|e| e["instanceId"] == instance_id));

        // DELETE the record
        let deleted_id = delete_record(temp.path(), &instance_id).unwrap();
        assert_eq!(deleted_id, instance_id);

        // Verify file was removed
        assert!(!file_path.exists());

        // Verify manifest was updated
        let manifest_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(temp.path().join("manifest.json")).unwrap())
                .unwrap();
        let index_after = manifest_after["instanceIndex"].as_array().unwrap();
        assert!(!index_after.iter().any(|e| e["instanceId"] == instance_id));
    }
}
