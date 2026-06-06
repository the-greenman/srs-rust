//! # Package Service
//!
//! Public API for field, type, and package definition operations. This module is the
//! sole entry point for all package-level logic. CLI handlers and future API handlers
//! must call these functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let filter = FieldListFilter { namespace: ns, package: pkg };
//! let result = package_service::list_fields_filtered(store, filter)?;
//! output::ok("field list", result)
//! ```

use crate::error::RepositoryError;
use crate::package_types::{DefinitionKind, PackageBoundary, PackageSelector};
use crate::relation_service;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use serde::{Deserialize, Serialize};
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_schema::{SchemaRegistry, FIELD_SCHEMA_ID};

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
    // Build a map from field ID → boundary selector by walking each boundary
    let mut provenance: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    let boundaries = store.list_package_boundaries()?;
    for boundary in &boundaries {
        let prefix = match &boundary.selector {
            None => "package".to_string(),
            Some(p) => p.clone(),
        };
        for rel_path in &boundary.field_paths {
            let full = format!("{prefix}/{rel_path}");
            if let Ok(val) = store.load_instance_json(&full) {
                if let Some(id) = val["id"].as_str() {
                    provenance
                        .entry(id.to_string())
                        .or_insert_with(|| boundary.selector.clone());
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

    let boundaries = store.list_package_boundaries()?;
    for boundary in &boundaries {
        let prefix = match &boundary.selector {
            None => "package".to_string(),
            Some(p) => p.clone(),
        };
        for rel_path in &boundary.type_paths {
            let full = format!("{prefix}/{rel_path}");
            if let Ok(val) = store.load_instance_json(&full) {
                if let Some(id) = val["id"].as_str() {
                    provenance
                        .entry(id.to_string())
                        .or_insert_with(|| boundary.selector.clone());
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

/// Filter options for listing fields
#[derive(Debug, Clone, Default)]
pub struct FieldListFilter {
    /// If Some, only return fields with this namespace.
    pub namespace: Option<String>,
    /// If Some, only return fields from this package boundary path
    /// (`None` string = primary package; `Some(path)` = sub-package).
    pub package: Option<Option<String>>,
}

/// Filter options for listing types
#[derive(Debug, Clone, Default)]
pub struct TypeListFilter {
    /// If Some, only return types with this namespace.
    pub namespace: Option<String>,
    /// If Some, only return types from this package boundary path.
    pub package: Option<Option<String>>,
}

/// Unified field listing function combining namespace and package filtering.
pub fn list_fields_filtered(
    store: &dyn RepositoryStore,
    filter: FieldListFilter,
) -> Result<Vec<FieldSummary>, RepositoryError> {
    let all = if let Some(boundary) = filter.package {
        list_fields_internal(store, Some(boundary.as_deref()))?
    } else {
        list_fields_internal(store, None)?
    };

    Ok(if let Some(ref ns) = filter.namespace {
        all.into_iter().filter(|f| &f.namespace == ns).collect()
    } else {
        all
    })
}

/// Unified type listing function combining namespace and package filtering.
pub fn list_types_filtered(
    store: &dyn RepositoryStore,
    filter: TypeListFilter,
) -> Result<Vec<TypeSummary>, RepositoryError> {
    let all = if let Some(boundary) = filter.package {
        list_types_internal(store, Some(boundary.as_deref()))?
    } else {
        list_types_internal(store, None)?
    };

    Ok(if let Some(ref ns) = filter.namespace {
        all.into_iter().filter(|t| &t.namespace == ns).collect()
    } else {
        all
    })
}

/// List relation type definitions with optional status filter.
///
/// If `status` is None, all definitions are returned. If Some, only definitions
/// whose serialized status string matches are returned.
pub fn list_relation_types_filtered(
    store: &dyn RepositoryStore,
    status: Option<String>,
) -> Result<Vec<RelationTypeDefinition>, RepositoryError> {
    let package = store.load_package()?;
    let defs = package.relation_type_definitions;

    Ok(if let Some(ref status_filter) = status {
        defs.into_iter()
            .filter(|rtd| {
                let serialized = rtd
                    .status
                    .as_ref()
                    .and_then(|s| serde_json::to_value(s).ok())
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                &serialized == status_filter
            })
            .collect()
    } else {
        defs
    })
}

/// Create a field with normalized defaults (description, aiGuidance, createdAt),
/// deserializing from a raw JSON value.
///
/// Moves normalization logic from CLI handlers into the service layer.
pub fn create_field_normalized(
    store: &dyn RepositoryStore,
    mut raw: serde_json::Value,
    package_selector: PackageSelector,
) -> Result<CreateFieldResult, RepositoryError> {
    // Normalize optional fields first so that schema validation sees a complete
    // document. The schema requires description/aiGuidance/createdAt; we supply
    // sensible defaults for callers that omit them.
    if raw["id"].as_str().is_none_or(|s| s.is_empty()) {
        raw["id"] = serde_json::json!(new_instance_id());
    }
    if raw.get("description").is_none() || raw["description"].is_null() {
        raw["description"] = serde_json::json!("");
    }
    // aiGuidance requires a "purpose" property; default to empty string when absent.
    match raw.get("aiGuidance") {
        None | Some(serde_json::Value::Null) => {
            raw["aiGuidance"] = serde_json::json!({ "purpose": "" });
        }
        Some(serde_json::Value::Object(_)) if raw["aiGuidance"].get("purpose").is_none() => {
            raw["aiGuidance"]["purpose"] = serde_json::json!("");
        }
        _ => {}
    }
    if raw.get("createdAt").is_none() || raw["createdAt"].is_null() {
        raw["createdAt"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
    }

    // Validate after normalization so defaults satisfy the schema.
    SchemaRegistry::global()
        .validate_by_id(FIELD_SCHEMA_ID, &raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from("<stdin>"),
            message: e.to_string(),
        })?;

    let field: Field = serde_json::from_value(raw).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from("fields"),
        source: e,
    })?;

    create_field_in_package(store, field, package_selector)
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

/// Result for build_type_schema
#[derive(Debug, Clone)]
pub struct TypeSchemaResult {
    pub type_id: String,
    pub type_version: u32,
    pub schema: serde_json::Value,
}

/// Build a draft-07 JSON Schema describing the `fieldValues` of a single Record
/// for the given type. Each property maps the field `name` to a JSON Schema
/// derived from its `valueType`, with `required`, `title`, `default`,
/// `x-srs-order`, and `x-srs-ai-guidance` annotations.
pub fn build_type_schema(
    store: &dyn RepositoryStore,
    type_id: &str,
    type_version: Option<u32>,
) -> Result<Option<TypeSchemaResult>, RepositoryError> {
    let package = store.load_package()?;

    let record_type = match type_version {
        Some(v) => package.resolve_type(type_id, v),
        None => package
            .record_types
            .iter()
            .filter(|rt| rt.id == type_id)
            .max_by_key(|rt| rt.version),
    };

    let record_type = match record_type {
        Some(rt) => rt,
        None => return Ok(None),
    };

    let mut properties = serde_json::Map::new();
    let mut required_fields: Vec<serde_json::Value> = Vec::new();

    let mut assignments = record_type.fields.clone();
    assignments.sort_by_key(|fa| fa.order);

    for assignment in &assignments {
        let field = match package.resolve_field(&assignment.field_id) {
            Some(f) => f,
            None => continue,
        };

        let mut prop = field_schema_object(field);

        let desc_title = if field.description.is_empty() {
            None
        } else {
            Some(field.description.as_str())
        };
        let title = assignment
            .display_label
            .as_deref()
            .or(desc_title)
            .unwrap_or(&field.name);
        prop.insert(
            "title".to_string(),
            serde_json::Value::String(title.to_string()),
        );
        prop.insert(
            "x-srs-order".to_string(),
            serde_json::Value::Number(assignment.order.into()),
        );

        if let Some(default) = &field.default_value {
            prop.insert("default".to_string(), default.clone());
        }

        if let serde_json::Value::Object(ref ai) = field.ai_guidance {
            if !ai.is_empty() {
                prop.insert("x-srs-ai-guidance".to_string(), field.ai_guidance.clone());
            }
        }

        if assignment.required {
            required_fields.push(serde_json::Value::String(field.name.clone()));
        }

        properties.insert(field.name.clone(), serde_json::Value::Object(prop));
    }

    let schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": properties,
        "required": required_fields,
        "additionalProperties": false,
    });

    Ok(Some(TypeSchemaResult {
        type_id: record_type.id.clone(),
        type_version: record_type.version,
        schema,
    }))
}

fn field_schema_object(field: &Field) -> serde_json::Map<String, serde_json::Value> {
    use srs_core::types::field::ValueType;
    let mut obj = serde_json::Map::new();
    match field.value_type {
        ValueType::String => {
            obj.insert("type".to_string(), serde_json::json!("string"));
        }
        ValueType::Text => {
            obj.insert("type".to_string(), serde_json::json!("string"));
            obj.insert("x-srs-widget".to_string(), serde_json::json!("textarea"));
        }
        ValueType::Number => {
            obj.insert("type".to_string(), serde_json::json!("number"));
        }
        ValueType::Boolean => {
            obj.insert("type".to_string(), serde_json::json!("boolean"));
        }
        ValueType::Date => {
            obj.insert("type".to_string(), serde_json::json!("string"));
            obj.insert("format".to_string(), serde_json::json!("date"));
        }
        ValueType::Url => {
            obj.insert("type".to_string(), serde_json::json!("string"));
            obj.insert("format".to_string(), serde_json::json!("uri"));
        }
        ValueType::Select => {
            let enum_values: Vec<serde_json::Value> = field
                .allowed_values
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|v| serde_json::Value::String(v.clone()))
                .collect();
            obj.insert("enum".to_string(), serde_json::Value::Array(enum_values));
        }
        ValueType::Multiselect => {
            let enum_values: Vec<serde_json::Value> = field
                .allowed_values
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|v| serde_json::Value::String(v.clone()))
                .collect();
            obj.insert("type".to_string(), serde_json::json!("array"));
            obj.insert(
                "items".to_string(),
                serde_json::json!({ "enum": enum_values }),
            );
        }
    }
    obj
}

/// Create a new field definition in the primary package.
/// Writes the field JSON file and updates the boundary index.
pub fn create_field(
    store: &dyn RepositoryStore,
    field: Field,
) -> Result<CreateFieldResult, RepositoryError> {
    create_field_in_package(store, field, None)
}

/// Create a new field definition in a specific package boundary.
/// Pass `selector = None` for the primary package; `Some(path)` for a sub-package.
pub fn create_field_in_package(
    store: &dyn RepositoryStore,
    field: Field,
    selector: PackageSelector,
) -> Result<CreateFieldResult, RepositoryError> {
    // Validate the boundary exists before touching the filesystem.
    store.load_package_boundary(&selector)?;

    let boundary_path = selector.as_deref().unwrap_or("package");
    let rel_filename = format!("fields/{}-{}.json", slugify(&field.name), &field.id[..8]);
    let full_path = format!("{boundary_path}/{rel_filename}");

    store.ensure_fields_dir(&format!("{boundary_path}/fields"))?;

    let created_at = if field.created_at.trim().is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        field.created_at.clone()
    };

    let field_with_timestamp = Field {
        created_at: created_at.clone(),
        ..field
    };

    store.save_field(&full_path, &field_with_timestamp)?;
    store.add_definition_to_boundary(&selector, DefinitionKind::Field, &rel_filename)?;

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
    let (relative_path, _owner) =
        find_field_path(store, &field.id)?.ok_or_else(|| RepositoryError::FieldNotFound {
            field_id: field.id.clone(),
        })?;
    store.update_field_file(&relative_path, &field)?;
    Ok(UpdateFieldResult { field })
}

/// Delete a field definition.
/// Removes the field JSON file and updates the boundary index.
pub fn delete_field(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<DeleteFieldResult, RepositoryError> {
    let (full_path, owner) =
        find_field_path(store, id)?.ok_or_else(|| RepositoryError::FieldNotFound {
            field_id: id.to_string(),
        })?;
    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();

    store.delete_field_file(&full_path)?;
    store.remove_definition_from_boundary(&owner, DefinitionKind::Field, &rel_path)?;
    Ok(DeleteFieldResult { id: id.to_string() })
}

/// Find the repo-root-relative path and owner boundary for a field by its ID.
fn find_field_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::Field) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    // Walk the boundary's field_paths to find the matching path
    let boundary = store.load_package_boundary(&owner)?;
    let prefix = owner.as_deref().unwrap_or("package");
    for rel_path in &boundary.field_paths {
        let full = format!("{prefix}/{rel_path}");
        if let Ok(val) = store.load_instance_json(&full) {
            if val["id"].as_str() == Some(id) {
                return Ok(Some((full, owner)));
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

/// Create a new type definition in the primary package.
/// Writes the type JSON file and updates the boundary index.
pub fn create_type(
    store: &dyn RepositoryStore,
    record_type: RecordType,
) -> Result<CreateTypeResult, RepositoryError> {
    create_type_in_package(store, record_type, None)
}

/// Create a new type definition in a specific package boundary.
/// Pass `selector = None` for the primary package; `Some(path)` for a sub-package.
pub fn create_type_in_package(
    store: &dyn RepositoryStore,
    mut record_type: RecordType,
    selector: PackageSelector,
) -> Result<CreateTypeResult, RepositoryError> {
    // Validate the boundary exists before touching the filesystem.
    store.load_package_boundary(&selector)?;

    if record_type.id.trim().is_empty() {
        record_type.id = new_instance_id();
    }
    let boundary_path = selector.as_deref().unwrap_or("package");
    let rel_filename = format!(
        "types/{}-{}.json",
        slugify(&record_type.name),
        &record_type.id[..8]
    );
    let full_path = format!("{boundary_path}/{rel_filename}");

    store.ensure_types_dir(&format!("{boundary_path}/types"))?;

    store.save_type(&full_path, &record_type)?;
    store.add_definition_to_boundary(&selector, DefinitionKind::Type, &rel_filename)?;

    Ok(CreateTypeResult { record_type })
}

/// Update an existing type definition.
/// Re-writes the type JSON file.
pub fn update_type(
    store: &dyn RepositoryStore,
    record_type: RecordType,
) -> Result<UpdateTypeResult, RepositoryError> {
    let (relative_path, _owner) =
        find_type_path(store, &record_type.id)?.ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: record_type.id.clone(),
            version: record_type.version,
        })?;
    store.update_type_file(&relative_path, &record_type)?;
    Ok(UpdateTypeResult { record_type })
}

/// Delete a type definition.
/// Removes the type JSON file and updates the boundary index.
pub fn delete_type(
    store: &dyn RepositoryStore,
    id: &str,
    version: u32,
) -> Result<DeleteTypeResult, RepositoryError> {
    let (full_path, owner) =
        find_type_path(store, id)?.ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: id.to_string(),
            version,
        })?;
    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();

    store.delete_type_file(&full_path)?;
    store.remove_definition_from_boundary(&owner, DefinitionKind::Type, &rel_path)?;
    Ok(DeleteTypeResult { id: id.to_string() })
}

/// Find the repo-root-relative path and owner for a type by its ID.
fn find_type_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::Type) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let boundary = store.load_package_boundary(&owner)?;
    let prefix = owner.as_deref().unwrap_or("package");
    for rel_path in &boundary.type_paths {
        let full = format!("{prefix}/{rel_path}");
        if let Ok(val) = store.load_instance_json(&full) {
            if val["id"].as_str() == Some(id) {
                return Ok(Some((full, owner)));
            }
        }
    }
    Ok(None)
}

// ── Relation type result types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRelationTypeResult {
    pub relation_type_definition: RelationTypeDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRelationTypeResult {
    pub relation_type_definition: RelationTypeDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRelationTypeResult {
    pub id: String,
}

// ── Relation type CRUD ───────────────────────────────────────────────────────

/// Create a new relation type definition in the primary package.
/// Writes the definition JSON file and updates the boundary index.
/// Auto-generates `id` if empty.
pub fn create_relation_type(
    store: &dyn RepositoryStore,
    mut def: RelationTypeDefinition,
) -> Result<CreateRelationTypeResult, RepositoryError> {
    if def.id.trim().is_empty() {
        def.id = new_instance_id();
    }
    if def.created_at.trim().is_empty() {
        def.created_at = chrono::Utc::now().to_rfc3339();
    }

    let slug = slugify(&def.key);
    let id_prefix = &def.id[..8.min(def.id.len())];
    let rel_filename = format!("relation-types/{slug}-{id_prefix}.json");
    let full_path = format!("package/{rel_filename}");

    store.ensure_relation_types_dir("package/relation-types")?;
    store.save_relation_type_definition(&full_path, &def)?;
    store.add_definition_to_boundary(&None, DefinitionKind::RelationType, &rel_filename)?;

    Ok(CreateRelationTypeResult {
        relation_type_definition: def,
    })
}

/// Update an existing relation type definition.
/// Re-writes the definition JSON file in place.
pub fn update_relation_type(
    store: &dyn RepositoryStore,
    def: RelationTypeDefinition,
) -> Result<UpdateRelationTypeResult, RepositoryError> {
    let (relative_path, _owner) = find_relation_type_path(store, &def.id)?
        .ok_or_else(|| RepositoryError::DefinitionNotFound { id: def.id.clone() })?;
    store.save_relation_type_definition(&relative_path, &def)?;
    Ok(UpdateRelationTypeResult {
        relation_type_definition: def,
    })
}

/// Delete a relation type definition.
/// Removes the definition JSON file and updates the boundary index.
/// Returns the IDs of any Relations whose `relationType` matches `type_name`.
fn find_relations_of_type(
    store: &dyn RepositoryStore,
    type_name: &str,
) -> Result<Vec<String>, RepositoryError> {
    let refs: Vec<String> = relation_service::load_relations(store)?
        .into_iter()
        .filter(|r| r.relation_type == type_name)
        .map(|r| r.relation_id)
        .collect();
    Ok(refs)
}

/// Delete a RelationTypeDefinition by ID.
/// Returns `CannotDeleteInUse` if any Relations use this type.
pub fn delete_relation_type(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<DeleteRelationTypeResult, RepositoryError> {
    let (full_path, owner) = find_relation_type_path(store, id)?
        .ok_or_else(|| RepositoryError::DefinitionNotFound { id: id.to_string() })?;

    // Resolve the type's name so we can match against relation_type strings
    let type_name = {
        if let Ok(val) = store.load_instance_json(&full_path) {
            val["relationType"].as_str().unwrap_or("").to_string()
        } else {
            String::new()
        }
    };

    if !type_name.is_empty() {
        let refs = find_relations_of_type(store, &type_name)?;
        if !refs.is_empty() {
            return Err(RepositoryError::CannotDeleteInUse {
                entity_type: "relation-type".to_string(),
                id: id.to_string(),
                used_by: refs,
            });
        }
    }

    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();

    store.delete_relation_type_file(&full_path)?;
    store.remove_definition_from_boundary(&owner, DefinitionKind::RelationType, &rel_path)?;
    Ok(DeleteRelationTypeResult { id: id.to_string() })
}

/// Find the repo-root-relative path and owner for a relation type definition by its ID.
fn find_relation_type_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::RelationType) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let pkg_json = store.load_package_json()?;
    if let Some(paths) = pkg_json.get("relationTypes").and_then(|v| v.as_array()) {
        for entry in paths {
            if let Some(rel) = entry.as_str() {
                let full = format!("package/{rel}");
                if let Ok(val) = store.load_instance_json(&full) {
                    if val["id"].as_str() == Some(id) {
                        return Ok(Some((full, owner)));
                    }
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
    let boundaries = store.list_package_boundaries()?;
    Ok(boundaries
        .into_iter()
        .map(|b| PackageBoundaryInfo {
            boundary_path: b.selector,
            id: b.id,
            namespace: b.namespace,
            name: b.name,
            version: b.version,
            field_count: b.field_paths.len(),
            type_count: b.type_paths.len(),
        })
        .collect())
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
    let boundary_path = input.boundary_path.as_ref().ok_or_else(|| {
        RepositoryError::InvalidRepositoryInitialization {
            message: "boundary_path is required for create_package (use create_repository for the primary package)".to_string(),
        }
    })?;

    // Validate the path won't collide with primary package
    if boundary_path == "package" {
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
        ("boundary_path", boundary_path.trim()),
    ] {
        if value.is_empty() {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: format!("{field} must not be empty"),
            });
        }
    }

    let selector: PackageSelector = Some(boundary_path.clone());

    // Check for duplicate: reject if id or path already registered
    if let Ok(existing) = store.load_package_boundary(&selector) {
        if !existing.id.is_empty() {
            return Err(RepositoryError::PackageAlreadyRegistered { id: existing.id });
        }
    }
    // Also check by id across all boundaries
    let all = store.list_package_boundaries()?;
    if all.iter().any(|b| b.id == input.id) {
        return Err(RepositoryError::PackageAlreadyRegistered {
            id: input.id.clone(),
        });
    }

    // Ensure directory exists and write package.json via boundary methods
    store.ensure_instance_dir(boundary_path)?;
    let boundary = PackageBoundary {
        selector: selector.clone(),
        id: input.id.clone(),
        namespace: input.namespace.clone(),
        name: input.name.clone(),
        version: input.version.clone(),
        field_paths: vec![],
        type_paths: vec![],
    };
    store.save_package_boundary_metadata(&boundary)?;
    store.register_package_boundary(&selector)?;

    Ok(CreatePackageResult {
        boundary_path: Some(boundary_path.clone()),
        id: input.id,
    })
}

/// Input for importing a local pre-existing package directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPackageLocalInput {
    /// Path relative to repository root of a directory containing a `package.json`.
    pub source_path: String,
}

/// Result for import_package_local
#[derive(Debug, Clone)]
pub struct ImportPackageLocalResult {
    pub selector: PackageSelector,
    pub id: String,
    pub namespace: String,
    pub name: String,
}

/// Import a pre-existing local package directory as a new boundary.
///
/// `source_path` is a pre-existing directory (relative to repo root) that already
/// contains a `package.json`. The service reads it via `load_instance_json` — no
/// file copying, no `std::fs`.
pub fn import_package_local(
    store: &dyn RepositoryStore,
    input: ImportPackageLocalInput,
) -> Result<ImportPackageLocalResult, RepositoryError> {
    let source_path = input.source_path.trim().to_string();
    if source_path.is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "source_path must not be empty".to_string(),
        });
    }

    // Read the existing package.json from source_path
    let pkg_json_key = format!("{source_path}/package.json");
    let pkg_json = store.load_instance_json(&pkg_json_key).map_err(|_| {
        RepositoryError::PackageRefMissing {
            path: source_path.clone(),
        }
    })?;

    let id = pkg_json["id"].as_str().unwrap_or("").to_string();
    let namespace = pkg_json["namespace"].as_str().unwrap_or("").to_string();
    let name = pkg_json["name"].as_str().unwrap_or("").to_string();
    let version = pkg_json["version"].as_str().unwrap_or("").to_string();

    if id.is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "source package.json missing required 'id' field".to_string(),
        });
    }

    // Reject duplicate id
    let all = store.list_package_boundaries()?;
    if all.iter().any(|b| b.id == id) {
        return Err(RepositoryError::PackageAlreadyRegistered { id });
    }

    let selector: PackageSelector = Some(source_path.clone());

    // Register the boundary using metadata from the existing package.json
    let boundary = PackageBoundary {
        selector: selector.clone(),
        id: id.clone(),
        namespace: namespace.clone(),
        name: name.clone(),
        version,
        field_paths: pkg_json["fields"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        type_paths: pkg_json["types"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    };
    store.save_package_boundary_metadata(&boundary)?;
    store.register_package_boundary(&selector)?;

    Ok(ImportPackageLocalResult {
        selector,
        id,
        namespace,
        name,
    })
}

/// Input for updating package boundary metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePackageMetadataInput {
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
}

/// Result for update_package_metadata
#[derive(Debug, Clone)]
pub struct UpdatePackageMetadataResult {
    pub boundary: PackageBoundary,
}

/// Update package boundary metadata (namespace/name/version only).
/// Never touches field_paths or type_paths.
pub fn update_package_metadata(
    store: &dyn RepositoryStore,
    selector: PackageSelector,
    input: UpdatePackageMetadataInput,
) -> Result<UpdatePackageMetadataResult, RepositoryError> {
    let mut boundary = store.load_package_boundary(&selector)?;
    if let Some(ns) = input.namespace {
        boundary.namespace = ns;
    }
    if let Some(name) = input.name {
        boundary.name = name;
    }
    if let Some(version) = input.version {
        boundary.version = version;
    }
    store.save_package_boundary_metadata(&boundary)?;
    Ok(UpdatePackageMetadataResult { boundary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_types::DefinitionKind;
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
            vocabulary_ref: None,
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
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
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

    // --- Phase 3: package lifecycle tests ---

    #[test]
    fn create_package_auto_registers_boundary() {
        let store = MemoryStore::default();
        let input = CreatePackageInput {
            id: "ext-pkg-001".to_string(),
            namespace: "com.ext".to_string(),
            name: "extension".to_string(),
            version: "1.0.0".to_string(),
            boundary_path: Some("pkg/ext".to_string()),
        };
        create_package(&store, input).unwrap();

        let boundaries = store.list_package_boundaries().unwrap();
        let has_ext = boundaries
            .iter()
            .any(|b| b.selector == Some("pkg/ext".to_string()));
        assert!(
            has_ext,
            "boundary should be registered after create_package"
        );
    }

    #[test]
    fn create_package_rejects_primary_path() {
        let store = MemoryStore::default();
        let input = CreatePackageInput {
            id: "bad-pkg".to_string(),
            namespace: "com.ext".to_string(),
            name: "bad".to_string(),
            version: "1.0.0".to_string(),
            boundary_path: Some("package".to_string()),
        };
        let result = create_package(&store, input);
        assert!(
            matches!(
                result,
                Err(RepositoryError::InvalidRepositoryInitialization { .. })
            ),
            "expected error for reserved path 'package'"
        );
    }

    #[test]
    fn create_package_rejects_duplicate_id() {
        let store = MemoryStore::default();
        let input = CreatePackageInput {
            id: "dup-pkg".to_string(),
            namespace: "com.ext".to_string(),
            name: "ext".to_string(),
            version: "1.0.0".to_string(),
            boundary_path: Some("pkg/ext".to_string()),
        };
        create_package(&store, input.clone()).unwrap();
        let second = create_package(
            &store,
            CreatePackageInput {
                boundary_path: Some("pkg/ext2".to_string()),
                ..input
            },
        );
        assert!(
            matches!(
                second,
                Err(RepositoryError::PackageAlreadyRegistered { .. })
            ),
            "second create with same id should fail"
        );
    }

    #[test]
    fn import_package_local_registers_logical_package() {
        let store = MemoryStore::default();
        // Pre-seed a package.json at a sub-package path
        let pkg_json = serde_json::json!({
            "id": "import-pkg-001",
            "namespace": "com.imported",
            "name": "imported",
            "version": "2.0.0",
            "fields": [],
            "types": []
        });
        store
            .save_instance_json("external/mypkg/package.json", &pkg_json)
            .unwrap();

        let input = ImportPackageLocalInput {
            source_path: "external/mypkg".to_string(),
        };
        let result = import_package_local(&store, input).unwrap();
        assert_eq!(result.id, "import-pkg-001");
        assert_eq!(result.namespace, "com.imported");
        assert_eq!(result.selector, Some("external/mypkg".to_string()));

        // list_packages should now include it
        let packages = list_packages(&store).unwrap();
        assert!(
            packages.iter().any(|p| p.id == "import-pkg-001"),
            "imported package should appear in list_packages"
        );
    }

    #[test]
    fn import_package_local_rejects_duplicate() {
        let store = MemoryStore::default();
        let pkg_json = serde_json::json!({
            "id": "dup-import",
            "namespace": "com.test",
            "name": "dup",
            "version": "1.0.0",
            "fields": [],
            "types": []
        });
        store
            .save_instance_json("ext/dup/package.json", &pkg_json)
            .unwrap();

        import_package_local(
            &store,
            ImportPackageLocalInput {
                source_path: "ext/dup".to_string(),
            },
        )
        .unwrap();

        // Seed a second path with same id
        store
            .save_instance_json("ext/dup2/package.json", &pkg_json)
            .unwrap();
        let second = import_package_local(
            &store,
            ImportPackageLocalInput {
                source_path: "ext/dup2".to_string(),
            },
        );
        assert!(
            matches!(
                second,
                Err(RepositoryError::PackageAlreadyRegistered { .. })
            ),
            "duplicate import should return PackageAlreadyRegistered"
        );
    }

    #[test]
    fn import_package_local_rejects_missing_source() {
        let store = MemoryStore::default();
        let result = import_package_local(
            &store,
            ImportPackageLocalInput {
                source_path: "nonexistent/path".to_string(),
            },
        );
        assert!(
            matches!(result, Err(RepositoryError::PackageRefMissing { .. })),
            "missing source should return PackageRefMissing"
        );
    }

    #[test]
    fn update_package_metadata_does_not_rewrite_definitions() {
        use crate::store::RepositoryStore;

        let store = MemoryStore::default();
        // Add a field path to the primary boundary
        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/keep-me.json")
            .unwrap();

        update_package_metadata(
            &store,
            None,
            UpdatePackageMetadataInput {
                name: Some("updated-name".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let boundary = store.load_package_boundary(&None).unwrap();
        assert_eq!(boundary.name, "updated-name");
        assert!(
            boundary
                .field_paths
                .contains(&"fields/keep-me.json".to_string()),
            "field_paths should be preserved after metadata update"
        );
    }

    #[test]
    fn update_package_metadata_changes_name() {
        let store = MemoryStore::default();
        update_package_metadata(
            &store,
            None,
            UpdatePackageMetadataInput {
                name: Some("new-name".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let packages = list_packages(&store).unwrap();
        let primary = packages.iter().find(|p| p.boundary_path.is_none()).unwrap();
        assert_eq!(primary.name, "new-name");
    }

    // --- Phase 4: package-aware definition tests ---

    #[test]
    fn create_field_in_sub_package() {
        use crate::store::RepositoryStore;

        let store = MemoryStore::default();
        let selector = Some("pkg/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let field = make_field("00000000-0000-0000-0000-abc000000001", "ext-field");
        create_field_in_package(&store, field, selector.clone()).unwrap();

        let boundary = store.load_package_boundary(&selector).unwrap();
        assert!(
            boundary.field_paths.iter().any(|p| p.contains("ext-field")),
            "field path should appear in sub-package boundary"
        );

        // Primary boundary should NOT have it
        let primary = store.load_package_boundary(&None).unwrap();
        assert!(
            !primary.field_paths.iter().any(|p| p.contains("ext-field")),
            "field should not appear in primary boundary"
        );
    }

    #[test]
    fn delete_field_removes_from_boundary_index() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        delete_field(&store, "00000000-0000-0000-0000-000000000001").unwrap();

        let boundary = store.load_package_boundary(&None).unwrap();
        assert!(
            !boundary.field_paths.iter().any(|p| p.contains("00000000")),
            "field path should be removed from boundary after delete"
        );
    }

    #[test]
    fn update_field_resolves_owner_package() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        let mut updated_field = make_field("00000000-0000-0000-0000-000000000001", "test-field");
        updated_field.description = "updated description".to_string();

        let result = update_field(&store, updated_field).unwrap();
        assert_eq!(result.field.description, "updated description");
    }

    #[test]
    fn delete_type_resolves_owner_package() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));

        delete_type(&store, "00000000-0000-0000-0000-000000000002", 1).unwrap();

        let boundary = store.load_package_boundary(&None).unwrap();
        assert!(
            !boundary.type_paths.iter().any(|p| p.contains("00000000")),
            "type path should be removed from boundary after delete"
        );
    }

    #[test]
    fn list_fields_by_package_filters_correctly() {
        use crate::store::RepositoryStore;

        let store = MemoryStore::default();
        let selector = Some("pkg/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        // Create field in sub-package
        create_field_in_package(
            &store,
            make_field("00000000-0000-0000-0000-aaa000000001", "ext-only-field"),
            selector.clone(),
        )
        .unwrap();

        // Seed sub-package data so load_package() can see it (need package.json with this field)
        // list_fields_by_package uses list_fields_internal which uses load_package() —
        // since MemoryStore's Package object won't have the sub-package field loaded,
        // only the provenance filtering is tested here.

        // Create a field in primary
        create_field(
            &store,
            make_field("00000000-0000-0000-0000-bbb000000002", "primary-field"),
        )
        .unwrap();

        // list_fields_by_package for sub-package should return only sub-package fields
        // (but load_package() won't see the sub-field since MemoryStore's Package is static)
        // The key behaviour: primary filter should not include sub-package fields
        let primary_fields = list_fields_by_package(&store, None).unwrap();
        assert!(
            primary_fields.iter().all(|f| f.source_package.is_none()),
            "fields from primary package filter should have no source_package"
        );
    }

    #[test]
    fn list_fields_includes_source_package() {
        let store = MemoryStore::with_field(make_field(
            "00000000-0000-0000-0000-000000000001",
            "test-field",
        ));

        let fields = list_fields(&store).unwrap();
        // Primary package fields have source_package = None
        assert!(
            fields.iter().any(|f| f.source_package.is_none()),
            "primary package fields should have source_package = None"
        );
    }

    #[test]
    fn list_types_by_package_filters_correctly() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));

        let primary_types = list_types_by_package(&store, None).unwrap();
        assert!(primary_types.iter().all(|t| t.source_package.is_none()));
    }

    #[test]
    fn list_types_includes_source_package() {
        let store = MemoryStore::with_type(make_type(
            "00000000-0000-0000-0000-000000000002",
            "test-type",
        ));

        let types = list_types(&store).unwrap();
        assert!(types.iter().any(|t| t.source_package.is_none()));
    }

    #[test]
    fn create_package_rejects_duplicate_path() {
        // Two packages with different IDs but the same boundary_path must fail.
        let store = MemoryStore::default();
        let first = CreatePackageInput {
            id: "pkg-id-001".to_string(),
            namespace: "com.ext".to_string(),
            name: "ext".to_string(),
            version: "1.0.0".to_string(),
            boundary_path: Some("pkg/ext".to_string()),
        };
        create_package(&store, first).unwrap();
        // Same path, different id — the boundary already exists with a non-empty id,
        // so the duplicate guard (load_package_boundary → non-empty id) must fire.
        let second = create_package(
            &store,
            CreatePackageInput {
                id: "pkg-id-002".to_string(),
                namespace: "com.ext".to_string(),
                name: "ext2".to_string(),
                version: "1.0.0".to_string(),
                boundary_path: Some("pkg/ext".to_string()),
            },
        );
        assert!(
            matches!(
                second,
                Err(RepositoryError::PackageAlreadyRegistered { .. })
            ),
            "second create with same path but different id should fail"
        );
    }

    #[test]
    fn save_package_boundary_metadata_preserves_field_paths() {
        // Regression for Bug 1: save_package_boundary_metadata must not wipe field_paths/type_paths.
        use crate::store::RepositoryStore;
        let store = MemoryStore::default();

        // Add a field to the primary boundary so field_paths is non-empty.
        create_field(
            &store,
            make_field("00000000-0000-0000-0000-preserve0001", "preserve-me"),
        )
        .unwrap();

        // Now call update_package_metadata, which internally calls save_package_boundary_metadata.
        update_package_metadata(
            &store,
            None,
            UpdatePackageMetadataInput {
                name: Some("mutated-name".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let boundary = store.load_package_boundary(&None).unwrap();
        assert_eq!(boundary.name, "mutated-name", "name should be updated");
        assert!(
            !boundary.field_paths.is_empty(),
            "field_paths must be preserved after save_package_boundary_metadata"
        );
    }

    #[test]
    fn relation_type_delete_blocked_when_instances_exist() {
        use crate::relation_service::load_relations;
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        let store = MemoryStore::default();

        let def = RelationTypeDefinition {
            schema: None,
            id: "rt-001".to_string(),
            version: 1,
            key: "test-link".to_string(),
            namespace: "com.test".to_string(),
            label: "Test Link".to_string(),
            description: "A test relation type".to_string(),
            category: RelationTypeCategory::Dependency,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            status: None,
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            updated_at: None,
            properties: None,
        };
        create_relation_type(&store, def).unwrap();

        // Write a relation of this type directly, bypassing instance-existence validation
        let rel_json = serde_json::json!({
            "relations": [{
                "relationId": "rel-test-rt-001",
                "relationType": "test-link",
                "sourceInstanceId": "source-001",
                "targetInstanceId": "target-001"
            }]
        });
        store
            .save_relations_json("relations/relations-collection.json", &rel_json)
            .unwrap();

        let result = delete_relation_type(&store, "rt-001");
        match result {
            Err(RepositoryError::CannotDeleteInUse {
                entity_type,
                id,
                used_by,
            }) => {
                assert_eq!(entity_type, "relation-type");
                assert_eq!(id, "rt-001");
                assert!(used_by.contains(&"rel-test-rt-001".to_string()));
            }
            other => panic!("expected CannotDeleteInUse, got {:?}", other),
        }

        // Relations unchanged
        let remaining = load_relations(&store).unwrap();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn relation_type_delete_succeeds_when_no_instances_exist() {
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        let store = MemoryStore::default();

        let def = RelationTypeDefinition {
            schema: None,
            id: "rt-002".to_string(),
            version: 1,
            key: "unused-link".to_string(),
            namespace: "com.test".to_string(),
            label: "Unused Link".to_string(),
            description: "A relation type with no instances".to_string(),
            category: RelationTypeCategory::Dependency,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            status: None,
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            updated_at: None,
            properties: None,
        };
        create_relation_type(&store, def).unwrap();

        delete_relation_type(&store, "rt-002").unwrap();
    }

    #[test]
    fn list_packages_returns_primary_and_sub_packages() {
        let store = MemoryStore::default();
        let input = CreatePackageInput {
            id: "sub-pkg-001".to_string(),
            namespace: "com.sub".to_string(),
            name: "sub".to_string(),
            version: "1.0.0".to_string(),
            boundary_path: Some("pkg/sub".to_string()),
        };
        create_package(&store, input).unwrap();

        let packages = list_packages(&store).unwrap();
        assert_eq!(packages.len(), 2, "should have primary + 1 sub-package");
        assert!(
            packages.iter().any(|p| p.boundary_path.is_none()),
            "primary package should be present"
        );
        assert!(
            packages
                .iter()
                .any(|p| p.boundary_path == Some("pkg/sub".to_string())),
            "sub-package should be present"
        );
    }

    use crate::manifest::Manifest;
    use crate::package::Package;

    fn make_field_with_type(id: &str, name: &str, vt: ValueType) -> Field {
        Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            value_type: vt,
            description: String::new(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn make_assignment(
        field_id: &str,
        order: u32,
        required: bool,
    ) -> srs_core::types::record_type::FieldAssignment {
        srs_core::types::record_type::FieldAssignment {
            field_id: field_id.to_string(),
            order,
            required,
            display_label: None,
            repeatable: false,
            min_items: None,
            max_items: None,
        }
    }

    fn make_type_with_fields(
        type_id: &str,
        name: &str,
        assignments: Vec<srs_core::types::record_type::FieldAssignment>,
    ) -> RecordType {
        RecordType {
            id: type_id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            fields: assignments,
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn schema_store(fields: Vec<Field>, types: Vec<RecordType>) -> MemoryStore {
        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: types,
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    #[test]
    fn type_schema_not_found_returns_none() {
        let store = schema_store(vec![], vec![]);
        let result =
            build_type_schema(&store, "00000000-0000-0000-0000-000000000999", None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn type_schema_all_value_types() {
        use serde_json::json;

        let mut sel = make_field_with_type(
            "00000000-0000-0000-0000-000000000007",
            "sel_field",
            ValueType::Select,
        );
        sel.allowed_values = Some(vec!["a".to_string(), "b".to_string()]);
        let mut multi = make_field_with_type(
            "00000000-0000-0000-0000-000000000008",
            "multi_field",
            ValueType::Multiselect,
        );
        multi.allowed_values = Some(vec!["x".to_string(), "y".to_string()]);

        let fields = vec![
            make_field_with_type(
                "00000000-0000-0000-0000-000000000001",
                "str_field",
                ValueType::String,
            ),
            make_field_with_type(
                "00000000-0000-0000-0000-000000000002",
                "text_field",
                ValueType::Text,
            ),
            make_field_with_type(
                "00000000-0000-0000-0000-000000000003",
                "num_field",
                ValueType::Number,
            ),
            make_field_with_type(
                "00000000-0000-0000-0000-000000000004",
                "bool_field",
                ValueType::Boolean,
            ),
            make_field_with_type(
                "00000000-0000-0000-0000-000000000005",
                "date_field",
                ValueType::Date,
            ),
            make_field_with_type(
                "00000000-0000-0000-0000-000000000006",
                "url_field",
                ValueType::Url,
            ),
            sel,
            multi,
        ];

        let type_id = "00000000-0000-0000-0000-000000000010";
        let assignments = vec![
            make_assignment("00000000-0000-0000-0000-000000000001", 0, true),
            make_assignment("00000000-0000-0000-0000-000000000002", 1, false),
            make_assignment("00000000-0000-0000-0000-000000000003", 2, false),
            make_assignment("00000000-0000-0000-0000-000000000004", 3, false),
            make_assignment("00000000-0000-0000-0000-000000000005", 4, false),
            make_assignment("00000000-0000-0000-0000-000000000006", 5, false),
            make_assignment("00000000-0000-0000-0000-000000000007", 6, false),
            make_assignment("00000000-0000-0000-0000-000000000008", 7, false),
        ];
        let record_type = make_type_with_fields(type_id, "all-types", assignments);
        let store = schema_store(fields, vec![record_type]);

        let result = build_type_schema(&store, type_id, None).unwrap().unwrap();
        let props = result.schema["properties"].as_object().unwrap();

        assert_eq!(props["str_field"]["type"], json!("string"));
        assert!(props["str_field"].get("x-srs-widget").is_none());

        assert_eq!(props["text_field"]["type"], json!("string"));
        assert_eq!(props["text_field"]["x-srs-widget"], json!("textarea"));

        assert_eq!(props["num_field"]["type"], json!("number"));
        assert_eq!(props["bool_field"]["type"], json!("boolean"));

        assert_eq!(props["date_field"]["type"], json!("string"));
        assert_eq!(props["date_field"]["format"], json!("date"));

        assert_eq!(props["url_field"]["type"], json!("string"));
        assert_eq!(props["url_field"]["format"], json!("uri"));

        assert_eq!(props["sel_field"]["enum"], json!(["a", "b"]));
        assert!(props["sel_field"].get("type").is_none());

        assert_eq!(props["multi_field"]["type"], json!("array"));
        assert_eq!(props["multi_field"]["items"]["enum"], json!(["x", "y"]));

        let required = result.schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], json!("str_field"));
    }

    #[test]
    fn type_schema_preserves_order() {
        let fields: Vec<Field> = (0u32..3)
            .map(|i| {
                make_field_with_type(
                    &format!("00000000-0000-0000-0000-{:012}", i + 1),
                    &format!("field_{i}"),
                    ValueType::String,
                )
            })
            .collect();

        let type_id = "00000000-0000-0000-0000-000000000010";
        let assignments = vec![
            make_assignment("00000000-0000-0000-0000-000000000003", 2, false),
            make_assignment("00000000-0000-0000-0000-000000000001", 0, false),
            make_assignment("00000000-0000-0000-0000-000000000002", 1, false),
        ];
        let record_type = make_type_with_fields(type_id, "ordered", assignments);
        let store = schema_store(fields, vec![record_type]);

        let result = build_type_schema(&store, type_id, None).unwrap().unwrap();
        let props = result.schema["properties"].as_object().unwrap();

        assert_eq!(props["field_0"]["x-srs-order"], serde_json::json!(0));
        assert_eq!(props["field_1"]["x-srs-order"], serde_json::json!(1));
        assert_eq!(props["field_2"]["x-srs-order"], serde_json::json!(2));
    }

    #[test]
    fn type_schema_explicit_version() {
        let field = make_field_with_type(
            "00000000-0000-0000-0000-000000000001",
            "f1",
            ValueType::String,
        );
        let type_id = "00000000-0000-0000-0000-000000000010";
        let record_type = make_type_with_fields(
            type_id,
            "versioned",
            vec![make_assignment(
                "00000000-0000-0000-0000-000000000001",
                0,
                false,
            )],
        );
        let store = schema_store(vec![field], vec![record_type]);

        // explicit version 1 resolves
        let result = build_type_schema(&store, type_id, Some(1))
            .unwrap()
            .unwrap();
        assert_eq!(result.type_version, 1);
        assert_eq!(result.schema["properties"].as_object().unwrap().len(), 1);

        // explicit version 99 not found
        let missing = build_type_schema(&store, type_id, Some(99)).unwrap();
        assert!(missing.is_none());
    }
}
