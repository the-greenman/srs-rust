use crate::error::RepositoryError;
use srs_core::types::field::{Field, ValueType};
use srs_core::types::record_type::{FieldAssignment, RecordType};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::validation::relation_type_definition::validate_relation_type_definition;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A loaded package containing field definitions and record types.
///
/// The `root` field contains the repository root path (not the package/ subdirectory).
#[derive(Debug, Clone)]
pub struct Package {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub fields: Vec<Field>,
    pub record_types: Vec<RecordType>,
    pub relation_type_definitions: Vec<RelationTypeDefinition>,
    pub root: PathBuf,
}

/// Package metadata as defined in package.json
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageMetadata {
    id: String,
    namespace: String,
    name: String,
    version: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    relation_types: Vec<String>,
}

/// Field JSON format from package/fields/*.json
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldJson {
    id: String,
    namespace: String,
    name: String,
    version: u32,
    value_type: String,
    description: Option<String>,
    ai_guidance: Option<serde_json::Value>,
    allowed_values: Option<Vec<String>>,
    default_value: Option<serde_json::Value>,
    created_at: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// Type JSON format from package/types/*.json
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypeJson {
    id: String,
    namespace: String,
    name: String,
    version: u32,
    description: Option<String>,
    fields: Vec<FieldAssignmentJson>,
    created_at: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// Field assignment format within type JSON
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldAssignmentJson {
    field_id: String,
    order: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_label: Option<String>,
}

impl Package {
    /// Resolve a relation type definition by its UUID id.
    pub fn resolve_relation_type_by_id(&self, id: &str) -> Option<&RelationTypeDefinition> {
        self.relation_type_definitions.iter().find(|rt| rt.id == id)
    }

    /// Resolve a relation type definition by its relationType string.
    pub fn resolve_relation_type(&self, relation_type: &str) -> Option<&RelationTypeDefinition> {
        self.relation_type_definitions
            .iter()
            .find(|rt| rt.relation_type == relation_type)
    }

    /// Get all relation type definitions as a slice.
    pub fn relation_types(&self) -> &[RelationTypeDefinition] {
        &self.relation_type_definitions
    }

    /// Resolve a record type by its ID and version.
    pub fn resolve_type(&self, type_id: &str, version: u32) -> Option<&RecordType> {
        self.record_types
            .iter()
            .find(|rt| rt.id == type_id && rt.version == version)
    }

    /// Resolve a record type by its namespace and name.
    ///
    /// This is the preferred lookup method as it avoids hardcoding UUIDs in tests.
    pub fn resolve_type_by_name(&self, namespace: &str, name: &str) -> Option<&RecordType> {
        self.record_types
            .iter()
            .find(|rt| rt.namespace == namespace && rt.name == name)
    }

    /// Resolve a field by its ID.
    pub fn resolve_field(&self, field_id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == field_id)
    }

    /// Find a field by its name.
    pub fn find_field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Get all fields as a slice.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Get all record types as a slice.
    pub fn record_types(&self) -> &[RecordType] {
        &self.record_types
    }
}

/// Load a package from a repository's `package/` directory.
///
/// The `repo_root` parameter is the path to the repository root (where the package/ directory is located).
pub fn load_package(repo_root: &Path) -> Result<Package, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    let package_content =
        std::fs::read_to_string(&package_json_path).map_err(|e| RepositoryError::Io {
            path: package_json_path.clone(),
            source: e,
        })?;

    let metadata: PackageMetadata =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: package_json_path,
            source: e,
        })?;

    // Load all fields
    let mut fields = Vec::new();
    for field_path in &metadata.fields {
        let full_path = package_dir.join(field_path);
        let field_content =
            std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
                path: full_path.clone(),
                source: e,
            })?;

        let field_json: FieldJson =
            serde_json::from_str(&field_content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path,
                source: e,
            })?;

        fields.push(Field {
            id: field_json.id,
            namespace: field_json.namespace,
            name: field_json.name,
            version: field_json.version,
            value_type: parse_value_type(&field_json.value_type)?,
            description: field_json.description.unwrap_or_default(),
            ai_guidance: field_json.ai_guidance.unwrap_or(serde_json::Value::Null),
            allowed_values: field_json.allowed_values,
            default_value: field_json.default_value,
            created_at: field_json.created_at.unwrap_or_default(),
            extra: HashMap::new(),
        });
    }

    // Load all record types
    let mut record_types = Vec::new();
    for type_path in &metadata.types {
        let full_path = package_dir.join(type_path);
        let type_content =
            std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
                path: full_path.clone(),
                source: e,
            })?;

        let type_json: TypeJson =
            serde_json::from_str(&type_content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path,
                source: e,
            })?;

        let fields: Vec<FieldAssignment> = type_json
            .fields
            .into_iter()
            .map(|fa| FieldAssignment {
                field_id: fa.field_id,
                order: fa.order,
                required: fa.required.unwrap_or(true),
                display_label: fa.display_label,
            })
            .collect();

        record_types.push(RecordType {
            id: type_json.id,
            namespace: type_json.namespace,
            name: type_json.name,
            version: type_json.version,
            description: type_json.description.unwrap_or_default(),
            fields,
            created_at: type_json.created_at.unwrap_or_default(),
            extra: HashMap::new(),
        });
    }

    // Load all relation type definitions, detecting conflicts and coalescing identical defs.
    // Tracks (definition, source_path) to produce good conflict errors.
    let mut rt_by_type: HashMap<String, (RelationTypeDefinition, PathBuf)> = HashMap::new();
    for rt_path in &metadata.relation_types {
        let full_path = package_dir.join(rt_path);
        let rt_content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;

        let def: RelationTypeDefinition =
            serde_json::from_str(&rt_content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;

        validate_relation_type_definition(&def).map_err(|source| {
            RepositoryError::RelationTypeDefinitionValidation {
                path: full_path.clone(),
                source,
            }
        })?;

        if let Some((existing, existing_path)) = rt_by_type.get(&def.relation_type) {
            if existing != &def {
                return Err(RepositoryError::RelationTypeDefinitionConflict {
                    relation_type: def.relation_type.clone(),
                    path_a: existing_path.clone(),
                    path_b: full_path,
                });
            }
            // Coalesce: keep existing, skip duplicate
        } else {
            rt_by_type.insert(def.relation_type.clone(), (def, full_path));
        }
    }
    let relation_type_definitions: Vec<RelationTypeDefinition> =
        rt_by_type.into_values().map(|(def, _)| def).collect();

    Ok(Package {
        id: metadata.id,
        namespace: metadata.namespace,
        name: metadata.name,
        version: metadata.version,
        fields,
        record_types,
        relation_type_definitions,
        root: repo_root.to_path_buf(),
    })
}

fn parse_value_type(s: &str) -> Result<ValueType, RepositoryError> {
    match s {
        "string" => Ok(ValueType::String),
        "text" => Ok(ValueType::Text),
        "number" => Ok(ValueType::Number),
        "boolean" => Ok(ValueType::Boolean),
        "date" => Ok(ValueType::Date),
        "url" => Ok(ValueType::Url),
        "select" => Ok(ValueType::Select),
        "multiselect" => Ok(ValueType::Multiselect),
        _ => Err(RepositoryError::PackageLoad {
            path: PathBuf::from("field.json"),
            source: serde_json::from_str::<()>("").unwrap_err(), // Create a generic parse error
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_package_from_live_repo() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert_eq!(package.namespace, "com.semanticops.srs");
        assert!(
            package.fields.len() > 20,
            "expected >20 fields, got {}",
            package.fields.len()
        );
        assert!(
            package.record_types.len() > 5,
            "expected >5 types, got {}",
            package.record_types.len()
        );
    }

    #[test]
    fn resolve_type_by_name_finds_known_type() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        // Use name-based lookup to avoid hardcoding UUIDs
        let ext_type = package
            .resolve_type_by_name("com.semanticops.srs", "meta.extension")
            .expect("should find meta.extension type");

        assert_eq!(ext_type.name, "meta.extension");
        assert_eq!(ext_type.namespace, "com.semanticops.srs");
        assert_eq!(ext_type.version, 1);
        assert!(!ext_type.fields.is_empty());
    }

    #[test]
    fn find_field_by_name_finds_status() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        let status_field = package
            .find_field_by_name("status")
            .expect("should find status field");

        assert_eq!(status_field.name, "status");
        assert_eq!(status_field.namespace, "com.semanticops.srs");
    }

    #[test]
    fn resolve_type_by_name_returns_none_for_unknown() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert!(package
            .resolve_type_by_name("unknown.namespace", "unknown-type")
            .is_none());
    }

    #[test]
    fn resolve_field_returns_none_for_unknown() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert!(package
            .resolve_field("00000000-0000-0000-0000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_loads_relation_types() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert!(
            package.relation_type_definitions.len() >= 7,
            "expected at least 7 relation types (canonical), got {}",
            package.relation_type_definitions.len()
        );
    }

    #[test]
    fn resolve_canonical_relation_type_precedes() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        let rt = package
            .resolve_relation_type("precedes")
            .expect("should find canonical 'precedes' relation type");

        assert_eq!(rt.namespace, "com.semanticops.srs");
        assert!(rt.is_active());
        assert!(rt.is_irreflexive());
    }

    #[test]
    fn deprecated_relation_types_loaded_with_correct_status() {
        let srs_repo = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        let package = load_package(&srs_repo).expect("should load live srs package");

        let deprecated: Vec<_> = package
            .relation_type_definitions
            .iter()
            .filter(|rt| !rt.is_active())
            .collect();

        assert!(
            !deprecated.is_empty(),
            "expected at least one deprecated relation type"
        );
        for rt in deprecated {
            assert!(
                rt.resolves(),
                "deprecated/tombstone types should still resolve"
            );
        }
    }
}
