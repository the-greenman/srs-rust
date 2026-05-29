use crate::error::RepositoryError;
use crate::package::load_package;
use serde::de::Error as SerdeDeError;
use serde::{Deserialize, Serialize};
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;
use std::path::Path;

/// Summary for field list operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub value_type: String,
    pub description: Option<String>,
}

/// Summary for type list operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: Option<String>,
    pub field_count: usize,
}

/// Result for get_field_by_id
#[derive(Debug, Clone)]
pub enum GetFieldResult {
    Found(Box<Field>),
    NotFound,
}

/// Result for get_type_by_id
#[derive(Debug, Clone)]
pub enum GetTypeResult {
    Found(RecordType),
    NotFound,
}

/// Result for create_field
#[derive(Debug, Clone)]
pub struct CreateFieldResult {
    pub field: Field,
    pub path: String,
}

/// Result for update_field
#[derive(Debug, Clone)]
pub struct UpdateFieldResult {
    pub field: Field,
    pub path: String,
}

/// Result for delete_field
#[derive(Debug, Clone)]
pub struct DeleteFieldResult {
    pub id: String,
    pub path: String,
}

/// Result for create_type
#[derive(Debug, Clone)]
pub struct CreateTypeResult {
    pub record_type: RecordType,
    pub path: String,
}

/// Result for update_type
#[derive(Debug, Clone)]
pub struct UpdateTypeResult {
    pub record_type: RecordType,
    pub path: String,
}

/// Result for delete_type
#[derive(Debug, Clone)]
pub struct DeleteTypeResult {
    pub id: String,
    pub path: String,
}

/// List all fields in the package
pub fn list_fields(repo_root: &Path) -> Result<Vec<FieldSummary>, RepositoryError> {
    let package = load_package(repo_root)?;

    let summaries = package
        .fields
        .iter()
        .map(|f| FieldSummary {
            id: f.id.clone(),
            namespace: f.namespace.clone(),
            name: f.name.clone(),
            version: f.version,
            value_type: format!("{:?}", f.value_type).to_lowercase(),
            description: if f.description.is_empty() {
                None
            } else {
                Some(f.description.clone())
            },
        })
        .collect();

    Ok(summaries)
}

/// List fields filtered by namespace
pub fn list_fields_by_namespace(
    repo_root: &Path,
    namespace: &str,
) -> Result<Vec<FieldSummary>, RepositoryError> {
    let all = list_fields(repo_root)?;
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|f| f.namespace == namespace)
        .collect();
    Ok(filtered)
}

/// Get a field by its ID
pub fn get_field_by_id(repo_root: &Path, id: &str) -> Result<GetFieldResult, RepositoryError> {
    let package = load_package(repo_root)?;

    match package.resolve_field(id) {
        Some(field) => Ok(GetFieldResult::Found(Box::new(field.clone()))),
        None => Ok(GetFieldResult::NotFound),
    }
}

/// List all types in the package
pub fn list_types(repo_root: &Path) -> Result<Vec<TypeSummary>, RepositoryError> {
    let package = load_package(repo_root)?;

    let summaries = package
        .record_types
        .iter()
        .map(|t| TypeSummary {
            id: t.id.clone(),
            namespace: t.namespace.clone(),
            name: t.name.clone(),
            version: t.version,
            description: if t.description.is_empty() {
                None
            } else {
                Some(t.description.clone())
            },
            field_count: t.fields.len(),
        })
        .collect();

    Ok(summaries)
}

/// List types filtered by namespace
pub fn list_types_by_namespace(
    repo_root: &Path,
    namespace: &str,
) -> Result<Vec<TypeSummary>, RepositoryError> {
    let all = list_types(repo_root)?;
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|t| t.namespace == namespace)
        .collect();
    Ok(filtered)
}

/// Get a type by its ID and version
pub fn get_type_by_id(
    repo_root: &Path,
    id: &str,
    version: u32,
) -> Result<GetTypeResult, RepositoryError> {
    let package = load_package(repo_root)?;

    match package.resolve_type(id, version) {
        Some(record_type) => Ok(GetTypeResult::Found(record_type.clone())),
        None => Ok(GetTypeResult::NotFound),
    }
}

/// Get a type by its ID using the latest available version.
pub fn get_type_by_id_latest(repo_root: &Path, id: &str) -> Result<GetTypeResult, RepositoryError> {
    let package = load_package(repo_root)?;

    let latest = package
        .record_types
        .iter()
        .filter(|rt| rt.id == id)
        .max_by_key(|rt| rt.version);

    match latest {
        Some(record_type) => Ok(GetTypeResult::Found(record_type.clone())),
        None => Ok(GetTypeResult::NotFound),
    }
}

/// Get a type by its namespace and name (latest version)
pub fn get_type_by_name(
    repo_root: &Path,
    namespace: &str,
    name: &str,
) -> Result<GetTypeResult, RepositoryError> {
    let package = load_package(repo_root)?;

    match package.resolve_type_by_name(namespace, name) {
        Some(record_type) => Ok(GetTypeResult::Found(record_type.clone())),
        None => Ok(GetTypeResult::NotFound),
    }
}

/// Create a new field definition
/// Writes the field JSON file and updates package.json
pub fn create_field(repo_root: &Path, field: Field) -> Result<CreateFieldResult, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read package.json
    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;
    let mut package_json: serde_json::Value =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path.clone(),
            source: e,
        })?;

    // Generate filename from field name
    let filename = format!("fields/{}-{}.json", slugify(&field.name), &field.id[..8]);
    let field_path = package_dir.join(&filename);

    // Ensure fields directory exists
    std::fs::create_dir_all(package_dir.join("fields")).map_err(|e| RepositoryError::Io {
        path: package_dir.join("fields"),
        source: e,
    })?;

    let created_at = if field.created_at.trim().is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        field.created_at.clone()
    };

    // Serialize field to JSON
    let field_json = serde_json::json!({
        "id": field.id,
        "namespace": field.namespace,
        "name": field.name,
        "version": field.version,
        "valueType": format!("{:?}", field.value_type).to_lowercase(),
        "description": field.description,
        "aiGuidance": field.ai_guidance,
        "allowedValues": field.allowed_values,
        "defaultValue": field.default_value,
        "createdAt": created_at,
    });

    // Write field file
    std::fs::write(
        &field_path,
        serde_json::to_string_pretty(&field_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: field_path.clone(),
        source: e,
    })?;

    // Update package.json fields array
    let fields_array =
        package_json["fields"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: package_json_path.clone(),
                source: serde_json::Error::custom("fields is not an array"),
            })?;

    let relative_path = format!("fields/{}-{}.json", slugify(&field.name), &field.id[..8]);
    if !fields_array
        .iter()
        .any(|f| f.as_str() == Some(&relative_path))
    {
        fields_array.push(serde_json::json!(relative_path));
    }

    // Write updated package.json
    std::fs::write(
        &package_json_path,
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: package_json_path.clone(),
        source: e,
    })?;

    Ok(CreateFieldResult {
        field: Field { created_at, ..field },
        path: relative_path,
    })
}

/// Update an existing field definition
/// Re-writes the field JSON file
pub fn update_field(repo_root: &Path, field: Field) -> Result<UpdateFieldResult, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read package.json to find the field path
    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;
    let package_json: serde_json::Value =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path.clone(),
            source: e,
        })?;

    // Find the field in the fields array
    let fields_array =
        package_json["fields"]
            .as_array()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: package_json_path.clone(),
                source: serde_json::Error::custom("fields is not an array"),
            })?;

    let field_entry = fields_array
        .iter()
        .find(|f| f.as_str().is_some_and(|path| path.contains(&field.id[..8])))
        .ok_or_else(|| RepositoryError::NotFound {
            path: package_dir.join("fields"),
        })?;

    let relative_path = field_entry.as_str().unwrap();
    let field_path = package_dir.join(relative_path);

    // Serialize updated field to JSON
    let field_json = serde_json::json!({
        "id": field.id,
        "namespace": field.namespace,
        "name": field.name,
        "version": field.version,
        "valueType": format!("{:?}", field.value_type).to_lowercase(),
        "description": field.description,
        "aiGuidance": field.ai_guidance,
        "allowedValues": field.allowed_values,
        "defaultValue": field.default_value,
        "createdAt": field.created_at,
    });

    // Write field file
    std::fs::write(
        &field_path,
        serde_json::to_string_pretty(&field_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: field_path.clone(),
        source: e,
    })?;

    Ok(UpdateFieldResult {
        field,
        path: relative_path.to_string(),
    })
}

/// Delete a field definition
/// Removes the field JSON file and updates package.json
pub fn delete_field(repo_root: &Path, id: &str) -> Result<DeleteFieldResult, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read package.json
    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;
    let mut package_json: serde_json::Value =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path.clone(),
            source: e,
        })?;

    // Find and remove the field from the fields array
    let fields_array =
        package_json["fields"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: package_json_path.clone(),
                source: serde_json::Error::custom("fields is not an array"),
            })?;

    let pos = fields_array
        .iter()
        .position(|f| f.as_str().is_some_and(|path| path.contains(&id[..8])))
        .ok_or_else(|| RepositoryError::NotFound {
            path: package_dir.join("fields"),
        })?;

    let relative_path = fields_array[pos].as_str().unwrap().to_string();
    fields_array.remove(pos);

    // Remove the field file
    let field_path = package_dir.join(&relative_path);
    if field_path.exists() {
        std::fs::remove_file(&field_path).map_err(|e| RepositoryError::Io {
            path: field_path.clone(),
            source: e,
        })?;
    }

    // Write updated package.json
    std::fs::write(
        &package_json_path,
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: package_json_path.clone(),
        source: e,
    })?;

    Ok(DeleteFieldResult {
        id: id.to_string(),
        path: relative_path,
    })
}

/// Convert a name to a filesystem-friendly slug
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// Create a new type definition
/// Writes the type JSON file and updates package.json
pub fn create_type(
    repo_root: &Path,
    record_type: RecordType,
) -> Result<CreateTypeResult, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read package.json
    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;
    let mut package_json: serde_json::Value =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path.clone(),
            source: e,
        })?;

    // Ensure types directory exists
    std::fs::create_dir_all(package_dir.join("types")).map_err(|e| RepositoryError::Io {
        path: package_dir.join("types"),
        source: e,
    })?;

    // Generate filename from type name and ID fragment
    let filename = format!(
        "types/{}-{}.json",
        slugify(&record_type.name),
        &record_type.id[..8]
    );
    let type_path = package_dir.join(&filename);

    // Build fields array
    let fields_json: Vec<serde_json::Value> = record_type
        .fields
        .iter()
        .map(|f| {
            let mut obj = serde_json::json!({
                "fieldId": f.field_id,
                "order": f.order,
            });
            obj["required"] = serde_json::json!(f.required);
            if let Some(ref label) = f.display_label {
                obj["displayLabel"] = serde_json::json!(label);
            }
            obj
        })
        .collect();

    // Serialize type to JSON
    let type_json = serde_json::json!({
        "id": record_type.id,
        "namespace": record_type.namespace,
        "name": record_type.name,
        "version": record_type.version,
        "description": record_type.description,
        "fields": fields_json,
        "createdAt": record_type.created_at,
    });

    // Write type file
    std::fs::write(
        &type_path,
        serde_json::to_string_pretty(&type_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: type_path.clone(),
        source: e,
    })?;

    // Update package.json types array
    let types_array =
        package_json["types"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: package_json_path.clone(),
                source: serde_json::Error::custom("types is not an array"),
            })?;

    if !types_array.iter().any(|t| t.as_str() == Some(&filename)) {
        types_array.push(serde_json::json!(filename.clone()));
    }

    // Write updated package.json
    std::fs::write(
        &package_json_path,
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: package_json_path.clone(),
        source: e,
    })?;

    Ok(CreateTypeResult {
        record_type,
        path: filename,
    })
}

/// Update an existing type definition
/// Re-writes the type JSON file
pub fn update_type(
    repo_root: &Path,
    record_type: RecordType,
) -> Result<UpdateTypeResult, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read package.json to find the type path
    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;
    let package_json: serde_json::Value =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path.clone(),
            source: e,
        })?;

    // Find the type in the types array
    let types_array =
        package_json["types"]
            .as_array()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: package_json_path.clone(),
                source: serde_json::Error::custom("types is not an array"),
            })?;

    let type_entry = types_array
        .iter()
        .find(|t| {
            t.as_str().is_some_and(|path| {
                path.contains(&record_type.id[..8])
                    || (path.contains(&slugify(&record_type.name))
                        && path.contains(&format!("v{}", record_type.version)))
            })
        })
        .ok_or_else(|| RepositoryError::NotFound {
            path: package_dir.join("types"),
        })?;

    let relative_path = type_entry.as_str().unwrap();
    let type_path = package_dir.join(relative_path);

    // Build fields array
    let fields_json: Vec<serde_json::Value> = record_type
        .fields
        .iter()
        .map(|f| {
            let mut obj = serde_json::json!({
                "fieldId": f.field_id,
                "order": f.order,
            });
            obj["required"] = serde_json::json!(f.required);
            if let Some(ref label) = f.display_label {
                obj["displayLabel"] = serde_json::json!(label);
            }
            obj
        })
        .collect();

    // Serialize updated type to JSON
    let type_json = serde_json::json!({
        "id": record_type.id,
        "namespace": record_type.namespace,
        "name": record_type.name,
        "version": record_type.version,
        "description": record_type.description,
        "fields": fields_json,
        "createdAt": record_type.created_at,
    });

    // Write type file
    std::fs::write(
        &type_path,
        serde_json::to_string_pretty(&type_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: type_path.clone(),
        source: e,
    })?;

    Ok(UpdateTypeResult {
        record_type,
        path: relative_path.to_string(),
    })
}

/// Delete a type definition
/// Removes the type JSON file and updates package.json
pub fn delete_type(
    repo_root: &Path,
    id: &str,
    _version: u32,
) -> Result<DeleteTypeResult, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read package.json
    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;
    let mut package_json: serde_json::Value =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path.clone(),
            source: e,
        })?;

    // Find and remove the type from the types array
    let types_array =
        package_json["types"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: package_json_path.clone(),
                source: serde_json::Error::custom("types is not an array"),
            })?;

    let pos = types_array
        .iter()
        .position(|t| t.as_str().is_some_and(|path| path.contains(&id[..8])))
        .ok_or_else(|| RepositoryError::NotFound {
            path: package_dir.join("types"),
        })?;

    let relative_path = types_array[pos].as_str().unwrap().to_string();
    types_array.remove(pos);

    // Remove the type file
    let type_path = package_dir.join(&relative_path);
    if type_path.exists() {
        std::fs::remove_file(&type_path).map_err(|e| RepositoryError::Io {
            path: type_path.clone(),
            source: e,
        })?;
    }

    // Write updated package.json
    std::fs::write(
        &package_json_path,
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .map_err(|e| RepositoryError::Io {
        path: package_json_path.clone(),
        source: e,
    })?;

    Ok(DeleteTypeResult {
        id: id.to_string(),
        path: relative_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_test_package_structure(temp: &TempDir) {
        let package_dir = temp.path().join("package");
        std::fs::create_dir_all(&package_dir).unwrap();
        std::fs::create_dir_all(package_dir.join("fields")).unwrap();
        std::fs::create_dir_all(package_dir.join("types")).unwrap();

        // Create package.json
        let package_json = json!({
            "id": "test-package",
            "namespace": "com.test",
            "name": "test",
            "version": "1.0.0",
            "fields": ["fields/test-field.json"],
            "types": ["types/test-type.json"]
        });
        std::fs::write(
            package_dir.join("package.json"),
            serde_json::to_string_pretty(&package_json).unwrap(),
        )
        .unwrap();

        // Create a test field
        let field_json = json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "namespace": "com.test",
            "name": "test-field",
            "version": 1,
            "valueType": "string",
            "description": "A test field",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::write(
            package_dir.join("fields/test-field.json"),
            serde_json::to_string_pretty(&field_json).unwrap(),
        )
        .unwrap();

        // Create a test type
        let type_json = json!({
            "id": "00000000-0000-0000-0000-000000000002",
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "A test type",
            "fields": [
                {
                    "fieldId": "00000000-0000-0000-0000-000000000001",
                    "order": 1,
                    "required": true
                }
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::write(
            package_dir.join("types/test-type.json"),
            serde_json::to_string_pretty(&type_json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_fields_returns_fields() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let fields = list_fields(temp.path()).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "test-field");
        assert_eq!(fields[0].namespace, "com.test");
    }

    #[test]
    fn list_fields_by_namespace_filters() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let fields = list_fields_by_namespace(temp.path(), "com.test").unwrap();
        assert_eq!(fields.len(), 1);

        let empty = list_fields_by_namespace(temp.path(), "other").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn get_field_by_id_finds_field() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let result = get_field_by_id(temp.path(), "00000000-0000-0000-0000-000000000001").unwrap();
        match result {
            GetFieldResult::Found(field) => {
                assert_eq!(field.name, "test-field");
            }
            GetFieldResult::NotFound => panic!("Should have found field"),
        }
    }

    #[test]
    fn get_field_by_id_not_found() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let result = get_field_by_id(temp.path(), "00000000-0000-0000-0000-000000000999").unwrap();
        match result {
            GetFieldResult::Found(_) => panic!("Should not have found field"),
            GetFieldResult::NotFound => (), // Expected
        }
    }

    #[test]
    fn list_types_returns_types() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let types = list_types(temp.path()).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "test-type");
        assert_eq!(types[0].field_count, 1);
    }

    #[test]
    fn get_type_by_id_finds_type() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let result =
            get_type_by_id(temp.path(), "00000000-0000-0000-0000-000000000002", 1).unwrap();
        match result {
            GetTypeResult::Found(record_type) => {
                assert_eq!(record_type.name, "test-type");
            }
            GetTypeResult::NotFound => panic!("Should have found type"),
        }
    }

    #[test]
    fn get_type_by_name_finds_type() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        let result = get_type_by_name(temp.path(), "com.test", "test-type").unwrap();
        match result {
            GetTypeResult::Found(record_type) => {
                assert_eq!(record_type.name, "test-type");
            }
            GetTypeResult::NotFound => panic!("Should have found type"),
        }
    }

    // Acceptance Criteria Tests for Phase 2

    #[test]
    fn field_create_update_delete_updates_package_manifest() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        // CREATE: Add a new field
        let new_field = Field {
            id: "00000000-0000-0000-0000-000000000010".to_string(),
            namespace: "com.test".to_string(),
            name: "new-field".to_string(),
            version: 1,
            value_type: srs_core::types::field::ValueType::String,
            description: "A new test field".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            default_value: None,
            created_at: "2026-01-02T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        };

        let create_result = create_field(temp.path(), new_field.clone()).unwrap();
        assert_eq!(
            create_result.field.id,
            "00000000-0000-0000-0000-000000000010"
        );

        // Verify file was created
        let field_path = temp.path().join("package").join(&create_result.path);
        assert!(field_path.exists());

        // Verify package.json was updated
        let package_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
        )
        .unwrap();
        let fields = package_json["fields"].as_array().unwrap();
        assert!(fields
            .iter()
            .any(|f| f.as_str().unwrap().contains("new-field")));

        // UPDATE: Modify the field
        let mut updated_field = new_field.clone();
        updated_field.description = "Updated description".to_string();

        let update_result = update_field(temp.path(), updated_field).unwrap();
        assert_eq!(update_result.field.description, "Updated description");

        // Verify file was updated
        let file_content = std::fs::read_to_string(&field_path).unwrap();
        let file_field: serde_json::Value = serde_json::from_str(&file_content).unwrap();
        assert_eq!(file_field["description"], "Updated description");

        // DELETE: Remove the field
        let delete_result =
            delete_field(temp.path(), "00000000-0000-0000-0000-000000000010").unwrap();
        assert_eq!(delete_result.id, "00000000-0000-0000-0000-000000000010");

        // Verify file was removed
        assert!(!field_path.exists());

        // Verify package.json was updated
        let package_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
        )
        .unwrap();
        let fields = package_json["fields"].as_array().unwrap();
        assert!(!fields
            .iter()
            .any(|f| f.as_str().unwrap().contains("new-field")));
    }

    #[test]
    fn type_create_update_delete_updates_package_manifest() {
        let temp = TempDir::new().unwrap();
        create_test_package_structure(&temp);

        // CREATE: Add a new type
        let new_type = RecordType {
            id: "00000000-0000-0000-0000-000000000020".to_string(),
            namespace: "com.test".to_string(),
            name: "new-type".to_string(),
            version: 1,
            description: "A new test type".to_string(),
            fields: vec![],
            created_at: "2026-01-02T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        };

        let create_result = create_type(temp.path(), new_type.clone()).unwrap();
        assert_eq!(
            create_result.record_type.id,
            "00000000-0000-0000-0000-000000000020"
        );

        // Verify file was created
        let type_path = temp.path().join("package").join(&create_result.path);
        assert!(type_path.exists());

        // Verify package.json was updated
        let package_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
        )
        .unwrap();
        let types = package_json["types"].as_array().unwrap();
        assert!(types
            .iter()
            .any(|t| t.as_str().unwrap().contains("new-type")));

        // UPDATE: Modify the type
        let mut updated_type = new_type.clone();
        updated_type.description = "Updated type description".to_string();

        let update_result = update_type(temp.path(), updated_type).unwrap();
        assert_eq!(
            update_result.record_type.description,
            "Updated type description"
        );

        // Verify file was updated
        let file_content = std::fs::read_to_string(&type_path).unwrap();
        let file_type: serde_json::Value = serde_json::from_str(&file_content).unwrap();
        assert_eq!(file_type["description"], "Updated type description");

        // DELETE: Remove the type
        let delete_result =
            delete_type(temp.path(), "00000000-0000-0000-0000-000000000020", 1).unwrap();
        assert_eq!(delete_result.id, "00000000-0000-0000-0000-000000000020");

        // Verify file was removed
        assert!(!type_path.exists());

        // Verify package.json was updated
        let package_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
        )
        .unwrap();
        let types = package_json["types"].as_array().unwrap();
        assert!(!types
            .iter()
            .any(|t| t.as_str().unwrap().contains("new-type")));
    }
}
