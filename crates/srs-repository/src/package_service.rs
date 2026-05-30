use crate::error::RepositoryError;
use crate::manifest_service::list_package_refs;
use crate::store::RepositoryStore;
use serde::de::Error as SerdeDeError;
use serde::{Deserialize, Serialize};
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;

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
    /// Boundary path of the package that owns this field.
    /// `None` = primary package (`package/`); `Some(path)` = sub-package path.
    pub source_package: Option<String>,
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
    /// Boundary path of the package that owns this type.
    /// `None` = primary package (`package/`); `Some(path)` = sub-package path.
    pub source_package: Option<String>,
}

/// Metadata for a package boundary (primary or sub-package).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageBoundaryInfo {
    /// `None` for the primary package; `Some(path)` for sub-packages.
    pub boundary_path: Option<String>,
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub field_count: usize,
    pub type_count: usize,
}

/// Result for get_field_by_id
#[derive(Debug, Clone)]
pub enum GetFieldResult {
    Found(Box<Field>),
    NotFound,
}

/// Result for get_type_by_id
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum GetTypeResult {
    Found(RecordType),
    NotFound,
}

/// Result for create_field
#[derive(Debug, Clone)]
pub struct CreateFieldResult {
    pub field: Field,
}

/// Result for update_field
#[derive(Debug, Clone)]
pub struct UpdateFieldResult {
    pub field: Field,
}

/// Result for delete_field
#[derive(Debug, Clone)]
pub struct DeleteFieldResult {
    pub id: String,
}

/// Result for create_type
#[derive(Debug, Clone)]
pub struct CreateTypeResult {
    pub record_type: RecordType,
}

/// Result for update_type
#[derive(Debug, Clone)]
pub struct UpdateTypeResult {
    pub record_type: RecordType,
}

/// Result for delete_type
#[derive(Debug, Clone)]
pub struct DeleteTypeResult {
    pub id: String,
}

/// List all fields across all package boundaries with source provenance.
pub fn list_fields(store: &dyn RepositoryStore) -> Result<Vec<FieldSummary>, RepositoryError> {
    list_fields_internal(store, None)
}

/// List fields filtered by namespace.
pub fn list_fields_by_namespace(
    store: &dyn RepositoryStore,
    namespace: &str,
) -> Result<Vec<FieldSummary>, RepositoryError> {
    let all = list_fields(store)?;
    Ok(all
        .into_iter()
        .filter(|f| f.namespace == namespace)
        .collect())
}

/// List fields belonging to a specific package boundary.
/// Pass `None` for the primary package; `Some(path)` for a sub-package.
pub fn list_fields_by_package(
    store: &dyn RepositoryStore,
    boundary_path: Option<&str>,
) -> Result<Vec<FieldSummary>, RepositoryError> {
    list_fields_internal(store, Some(boundary_path))
}

fn list_fields_internal(
    store: &dyn RepositoryStore,
    filter: Option<Option<&str>>,
) -> Result<Vec<FieldSummary>, RepositoryError> {
    // Build a map from field ID → boundary_path by walking each package boundary
    let mut provenance: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    // Primary package
    if let Ok(pkg_json) = store.load_package_json() {
        if let Some(fields) = pkg_json["fields"].as_array() {
            for entry in fields {
                if let Some(path) = entry.as_str() {
                    let full = format!("package/{path}");
                    if let Ok(val) = store.load_instance_json(&full) {
                        if let Some(id) = val["id"].as_str() {
                            provenance.insert(id.to_string(), None);
                        }
                    }
                }
            }
        }
    }

    // Sub-packages
    for pkg_ref in list_package_refs(store)? {
        let pkg_json_path = format!("{}/package.json", pkg_ref.path);
        if let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) {
            if let Some(fields) = pkg_json["fields"].as_array() {
                for entry in fields {
                    if let Some(rel) = entry.as_str() {
                        let full = format!("{}/{rel}", pkg_ref.path);
                        if let Ok(val) = store.load_instance_json(&full) {
                            if let Some(id) = val["id"].as_str() {
                                provenance
                                    .entry(id.to_string())
                                    .or_insert_with(|| Some(pkg_ref.path.clone()));
                            }
                        }
                    }
                }
            }
        }
    }

    let package = store.load_package()?;
    let summaries = package
        .fields
        .iter()
        .filter(|f| match filter {
            None => true,
            Some(boundary) => provenance.get(&f.id).map(|p| p.as_deref()) == Some(boundary),
        })
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
            source_package: provenance.get(&f.id).cloned().flatten(),
        })
        .collect();

    Ok(summaries)
}

/// Get a field by its ID
pub fn get_field_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetFieldResult, RepositoryError> {
    let package = store.load_package()?;

    match package.resolve_field(id) {
        Some(field) => Ok(GetFieldResult::Found(Box::new(field.clone()))),
        None => Ok(GetFieldResult::NotFound),
    }
}

/// List all types across all package boundaries with source provenance.
pub fn list_types(store: &dyn RepositoryStore) -> Result<Vec<TypeSummary>, RepositoryError> {
    list_types_internal(store, None)
}

/// List types filtered by namespace.
pub fn list_types_by_namespace(
    store: &dyn RepositoryStore,
    namespace: &str,
) -> Result<Vec<TypeSummary>, RepositoryError> {
    let all = list_types(store)?;
    Ok(all
        .into_iter()
        .filter(|t| t.namespace == namespace)
        .collect())
}

/// List types belonging to a specific package boundary.
/// Pass `None` for the primary package; `Some(path)` for a sub-package.
pub fn list_types_by_package(
    store: &dyn RepositoryStore,
    boundary_path: Option<&str>,
) -> Result<Vec<TypeSummary>, RepositoryError> {
    list_types_internal(store, Some(boundary_path))
}

fn list_types_internal(
    store: &dyn RepositoryStore,
    filter: Option<Option<&str>>,
) -> Result<Vec<TypeSummary>, RepositoryError> {
    let mut provenance: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    if let Ok(pkg_json) = store.load_package_json() {
        if let Some(types) = pkg_json["types"].as_array() {
            for entry in types {
                if let Some(path) = entry.as_str() {
                    let full = format!("package/{path}");
                    if let Ok(val) = store.load_instance_json(&full) {
                        if let Some(id) = val["id"].as_str() {
                            provenance.insert(id.to_string(), None);
                        }
                    }
                }
            }
        }
    }

    for pkg_ref in list_package_refs(store)? {
        let pkg_json_path = format!("{}/package.json", pkg_ref.path);
        if let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) {
            if let Some(types) = pkg_json["types"].as_array() {
                for entry in types {
                    if let Some(rel) = entry.as_str() {
                        let full = format!("{}/{rel}", pkg_ref.path);
                        if let Ok(val) = store.load_instance_json(&full) {
                            if let Some(id) = val["id"].as_str() {
                                provenance
                                    .entry(id.to_string())
                                    .or_insert_with(|| Some(pkg_ref.path.clone()));
                            }
                        }
                    }
                }
            }
        }
    }

    let package = store.load_package()?;
    let summaries = package
        .record_types
        .iter()
        .filter(|t| match filter {
            None => true,
            Some(boundary) => provenance.get(&t.id).map(|p| p.as_deref()) == Some(boundary),
        })
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
            source_package: provenance.get(&t.id).cloned().flatten(),
        })
        .collect();

    Ok(summaries)
}

/// Get a type by its ID and version
pub fn get_type_by_id(
    store: &dyn RepositoryStore,
    id: &str,
    version: u32,
) -> Result<GetTypeResult, RepositoryError> {
    let package = store.load_package()?;

    match package.resolve_type(id, version) {
        Some(record_type) => Ok(GetTypeResult::Found(record_type.clone())),
        None => Ok(GetTypeResult::NotFound),
    }
}

/// Get a type by its ID using the latest available version.
pub fn get_type_by_id_latest(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetTypeResult, RepositoryError> {
    let package = store.load_package()?;

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
    store: &dyn RepositoryStore,
    namespace: &str,
    name: &str,
) -> Result<GetTypeResult, RepositoryError> {
    let package = store.load_package()?;

    match package.resolve_type_by_name(namespace, name) {
        Some(record_type) => Ok(GetTypeResult::Found(record_type.clone())),
        None => Ok(GetTypeResult::NotFound),
    }
}

/// Create a new field definition.
/// Writes the field JSON file and updates package.json.
pub fn create_field(
    store: &dyn RepositoryStore,
    field: Field,
) -> Result<CreateFieldResult, RepositoryError> {
    let mut package_json = store.load_package_json()?;

    let filename = format!("fields/{}-{}.json", slugify(&field.name), &field.id[..8]);

    store.ensure_fields_dir()?;

    let created_at = if field.created_at.trim().is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        field.created_at.clone()
    };

    let field_with_timestamp = Field {
        created_at: created_at.clone(),
        ..field
    };

    store.save_field(&filename, &field_with_timestamp)?;

    let fields_array =
        package_json["fields"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: std::path::PathBuf::from("package/package.json"),
                source: serde_json::Error::custom("fields is not an array"),
            })?;

    if !fields_array.iter().any(|f| f.as_str() == Some(&filename)) {
        fields_array.push(serde_json::json!(filename));
    }

    store.save_package_json(&package_json)?;

    Ok(CreateFieldResult {
        field: field_with_timestamp,
    })
}

/// Update an existing field definition.
/// Re-writes the field JSON file.
pub fn update_field(
    store: &dyn RepositoryStore,
    field: Field,
) -> Result<UpdateFieldResult, RepositoryError> {
    let relative_path =
        find_field_path(store, &field.id)?.ok_or_else(|| RepositoryError::FieldNotFound {
            field_id: field.id.clone(),
        })?;
    store.update_field_file(&relative_path, &field)?;
    Ok(UpdateFieldResult { field })
}

/// Delete a field definition.
/// Removes the field JSON file and updates package.json.
pub fn delete_field(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<DeleteFieldResult, RepositoryError> {
    let relative_path =
        find_field_path(store, id)?.ok_or_else(|| RepositoryError::FieldNotFound {
            field_id: id.to_string(),
        })?;

    let mut package_json = store.load_package_json()?;
    let fields_array =
        package_json["fields"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: std::path::PathBuf::from("package/package.json"),
                source: serde_json::Error::custom("fields is not an array"),
            })?;
    fields_array.retain(|f| f.as_str() != Some(&relative_path));

    store.delete_field_file(&relative_path)?;
    store.save_package_json(&package_json)?;
    Ok(DeleteFieldResult { id: id.to_string() })
}

/// Find the package.json-relative path for a field by scanning each listed file for a matching id.
fn find_field_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<String>, RepositoryError> {
    let package_json = store.load_package_json()?;
    let fields = match package_json["fields"].as_array() {
        Some(a) => a.clone(),
        None => return Ok(None),
    };
    for entry in &fields {
        if let Some(rel) = entry.as_str() {
            let full = format!("package/{rel}");
            if let Ok(val) = store.load_instance_json(&full) {
                if val["id"].as_str() == Some(id) {
                    return Ok(Some(rel.to_string()));
                }
            }
        }
    }
    Ok(None)
}

/// Convert a name to a filesystem-friendly slug
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// Create a new type definition.
/// Writes the type JSON file and updates package.json.
pub fn create_type(
    store: &dyn RepositoryStore,
    record_type: RecordType,
) -> Result<CreateTypeResult, RepositoryError> {
    let mut package_json = store.load_package_json()?;

    store.ensure_types_dir()?;

    let filename = format!(
        "types/{}-{}.json",
        slugify(&record_type.name),
        &record_type.id[..8]
    );

    store.save_type(&filename, &record_type)?;

    let types_array =
        package_json["types"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: std::path::PathBuf::from("package/package.json"),
                source: serde_json::Error::custom("types is not an array"),
            })?;

    if !types_array.iter().any(|t| t.as_str() == Some(&filename)) {
        types_array.push(serde_json::json!(filename.clone()));
    }

    store.save_package_json(&package_json)?;

    Ok(CreateTypeResult { record_type })
}

/// Update an existing type definition.
/// Re-writes the type JSON file.
pub fn update_type(
    store: &dyn RepositoryStore,
    record_type: RecordType,
) -> Result<UpdateTypeResult, RepositoryError> {
    let relative_path =
        find_type_path(store, &record_type.id)?.ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: record_type.id.clone(),
            version: record_type.version,
        })?;
    store.update_type_file(&relative_path, &record_type)?;
    Ok(UpdateTypeResult { record_type })
}

/// Delete a type definition.
/// Removes the type JSON file and updates package.json.
pub fn delete_type(
    store: &dyn RepositoryStore,
    id: &str,
    version: u32,
) -> Result<DeleteTypeResult, RepositoryError> {
    let relative_path =
        find_type_path(store, id)?.ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: id.to_string(),
            version,
        })?;

    let mut package_json = store.load_package_json()?;
    let types_array =
        package_json["types"]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: std::path::PathBuf::from("package/package.json"),
                source: serde_json::Error::custom("types is not an array"),
            })?;
    types_array.retain(|t| t.as_str() != Some(&relative_path));

    store.delete_type_file(&relative_path)?;
    store.save_package_json(&package_json)?;
    Ok(DeleteTypeResult { id: id.to_string() })
}

/// Find the package.json-relative path for a type by scanning each listed file for a matching id.
fn find_type_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<String>, RepositoryError> {
    let package_json = store.load_package_json()?;
    let types = match package_json["types"].as_array() {
        Some(a) => a.clone(),
        None => return Ok(None),
    };
    for entry in &types {
        if let Some(rel) = entry.as_str() {
            let full = format!("package/{rel}");
            if let Ok(val) = store.load_instance_json(&full) {
                if val["id"].as_str() == Some(id) {
                    return Ok(Some(rel.to_string()));
                }
            }
        }
    }
    Ok(None)
}

/// List all package boundaries (primary + declared sub-packages).
pub fn list_packages(
    store: &dyn RepositoryStore,
) -> Result<Vec<PackageBoundaryInfo>, RepositoryError> {
    let mut result = Vec::new();

    // Primary package
    if let Ok(pkg_json) = store.load_package_json() {
        result.push(package_boundary_info_from_json(&pkg_json, None));
    }

    // Sub-packages
    for pkg_ref in list_package_refs(store)? {
        let pkg_json_path = format!("{}/package.json", pkg_ref.path);
        if let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) {
            result.push(package_boundary_info_from_json(
                &pkg_json,
                Some(pkg_ref.path),
            ));
        }
    }

    Ok(result)
}

fn package_boundary_info_from_json(
    pkg_json: &serde_json::Value,
    boundary_path: Option<String>,
) -> PackageBoundaryInfo {
    PackageBoundaryInfo {
        boundary_path,
        id: pkg_json["id"].as_str().unwrap_or("").to_string(),
        namespace: pkg_json["namespace"].as_str().unwrap_or("").to_string(),
        name: pkg_json["name"].as_str().unwrap_or("").to_string(),
        version: pkg_json["version"].as_str().unwrap_or("").to_string(),
        field_count: pkg_json["fields"].as_array().map(|a| a.len()).unwrap_or(0),
        type_count: pkg_json["types"].as_array().map(|a| a.len()).unwrap_or(0),
    }
}

/// Input for creating a new package boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePackageInput {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    /// Optional sub-package path relative to repo root (e.g. `package/my-ext`).
    /// If `None`, this replaces the primary package — use only for repo init.
    /// If `Some`, a new sub-package boundary is created and registered in the manifest.
    pub boundary_path: Option<String>,
}

/// Result for create_package
#[derive(Debug, Clone)]
pub struct CreatePackageResult {
    pub boundary_path: Option<String>,
    pub id: String,
}

/// Create a new sub-package boundary, write its `package.json`, and register it
/// in the manifest `packageRefs`. The primary package is managed by
/// `repository_lifecycle::create_repository` — this function is for sub-packages only.
pub fn create_package(
    store: &dyn RepositoryStore,
    input: CreatePackageInput,
) -> Result<CreatePackageResult, RepositoryError> {
    let boundary = input.boundary_path.as_ref().ok_or_else(|| {
        RepositoryError::InvalidRepositoryInitialization {
            message: "boundary_path is required for create_package (use create_repository for the primary package)".to_string(),
        }
    })?;

    // Validate the path won't collide with primary package
    if boundary == "package" {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "boundary_path 'package' is reserved for the primary package".to_string(),
        });
    }

    // Validate fields
    for (field, value) in [
        ("id", input.id.trim()),
        ("namespace", input.namespace.trim()),
        ("name", input.name.trim()),
        ("version", input.version.trim()),
        ("boundary_path", boundary.trim()),
    ] {
        if value.is_empty() {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: format!("{field} must not be empty"),
            });
        }
    }

    // Write package.json for the new boundary
    let pkg_json_path = format!("{boundary}/package.json");
    let package_json = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
        "id": input.id,
        "namespace": input.namespace,
        "name": input.name,
        "version": input.version,
        "title": input.name,
        "description": "",
        "status": "active",
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "fields": [],
        "types": [],
        "relationTypes": [],
        "views": [],
        "documentViews": []
    });
    store.ensure_instance_dir(boundary)?;
    store.save_instance_json(&pkg_json_path, &package_json)?;

    // Register in manifest packageRefs (reuse validate_package_ref_path for FileStore safety)
    store.validate_package_ref_path(boundary)?;
    let mut manifest = store.load_manifest()?;
    let mut refs: Vec<serde_json::Value> = manifest
        .extra
        .get("packageRefs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let already_registered = refs
        .iter()
        .any(|r| r.get("path").and_then(|p| p.as_str()) == Some(boundary));
    if !already_registered {
        refs.push(serde_json::json!({"mode": "local", "path": boundary}));
        refs.sort_by(|a, b| {
            a.get("path")
                .and_then(|p| p.as_str())
                .cmp(&b.get("path").and_then(|p| p.as_str()))
        });
        manifest
            .extra
            .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
        store.save_manifest(&manifest)?;
    }

    Ok(CreatePackageResult {
        boundary_path: Some(boundary.clone()),
        id: input.id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use srs_core::types::field::ValueType;
    use std::collections::HashMap;

    fn make_field(id: &str, name: &str) -> Field {
        Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "A test field".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn make_type(id: &str, name: &str) -> RecordType {
        RecordType {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "A test type".to_string(),
            fields: vec![],
            field_groups: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn list_fields_returns_fields() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));
        let fields = list_fields(&store).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "test-field");
        assert_eq!(fields[0].namespace, "com.test");
    }

    #[test]
    fn list_fields_by_namespace_filters() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        let fields = list_fields_by_namespace(&store, "com.test").unwrap();
        assert_eq!(fields.len(), 1);

        let empty = list_fields_by_namespace(&store, "other").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn get_field_by_id_finds_field() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        let result = get_field_by_id(&store, "00000000-0000-0000-0000-000000000001").unwrap();
        match result {
            GetFieldResult::Found(field) => assert_eq!(field.name, "test-field"),
            GetFieldResult::NotFound => panic!("Should have found field"),
        }
    }

    #[test]
    fn get_field_by_id_not_found() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        let result = get_field_by_id(&store, "00000000-0000-0000-0000-000000000999").unwrap();
        match result {
            GetFieldResult::Found(_) => panic!("Should not have found field"),
            GetFieldResult::NotFound => (),
        }
    }

    #[test]
    fn list_types_returns_types() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));
        let types = list_types(&store).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "test-type");
        assert_eq!(types[0].field_count, 0);
    }

    #[test]
    fn get_type_by_id_finds_type() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));

        let result = get_type_by_id(&store, "00000000-0000-0000-0000-000000000002", 1).unwrap();
        match result {
            GetTypeResult::Found(record_type) => assert_eq!(record_type.name, "test-type"),
            GetTypeResult::NotFound => panic!("Should have found type"),
        }
    }

    #[test]
    fn get_type_by_name_finds_type() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));

        let result = get_type_by_name(&store, "com.test", "test-type").unwrap();
        match result {
            GetTypeResult::Found(record_type) => assert_eq!(record_type.name, "test-type"),
            GetTypeResult::NotFound => panic!("Should have found type"),
        }
    }

    #[test]
    fn field_create_updates_package_json() {
        let store = MemoryStore::default();
        let field = make_field("00000000-0000-0000-0000-000000000010", "new-field");

        let result = create_field(&store, field).unwrap();
        assert_eq!(result.field.id, "00000000-0000-0000-0000-000000000010");

        // package.json should now list the field
        let pkg = store.load_package_json().unwrap();
        let fields = pkg["fields"].as_array().unwrap();
        assert!(fields
            .iter()
            .any(|f| f.as_str().unwrap().contains("new-field")));
    }

    #[test]
    fn field_delete_removes_from_package_json() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        delete_field(&store, "00000000-0000-0000-0000-000000000001").unwrap();

        let pkg = store.load_package_json().unwrap();
        let fields = pkg["fields"].as_array().unwrap();
        assert!(!fields
            .iter()
            .any(|f| f.as_str().unwrap().contains("00000000")));
    }

    #[test]
    fn type_create_updates_package_json() {
        let store = MemoryStore::default();
        let rt = make_type("00000000-0000-0000-0000-000000000020", "new-type");

        let result = create_type(&store, rt).unwrap();
        assert_eq!(
            result.record_type.id,
            "00000000-0000-0000-0000-000000000020"
        );

        let pkg = store.load_package_json().unwrap();
        let types = pkg["types"].as_array().unwrap();
        assert!(types
            .iter()
            .any(|t| t.as_str().unwrap().contains("new-type")));
    }

    #[test]
    fn type_delete_removes_from_package_json() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));

        delete_type(&store, "00000000-0000-0000-0000-000000000002", 1).unwrap();

        let pkg = store.load_package_json().unwrap();
        let types = pkg["types"].as_array().unwrap();
        assert!(!types
            .iter()
            .any(|t| t.as_str().unwrap().contains("00000000")));
    }
}
