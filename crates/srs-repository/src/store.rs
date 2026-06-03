use crate::error::RepositoryError;
use crate::manifest::Manifest;
use crate::package::Package;
use crate::package_types::{DefinitionKind, PackageBoundary, PackageSelector};
use crate::repository_lifecycle::{CreateRepositoryResult, InitializeRepositoryInput};
use serde::de::Error as SerdeDeError;
use srs_core::types::field::{Field, ValueType};
use srs_core::types::record_type::{
    FieldAssignment, FieldAssignmentOverride, FieldGroup, RecordType, TypeLifecycle,
};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentView, View};
use srs_core::validation::relation_type_definition::validate_relation_type_definition;
use srs_core::validation::theme::validate_theme;
use srs_core::validation::view::{validate_document_view, validate_view};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// RepositoryStore trait
// ---------------------------------------------------------------------------

/// Abstracts all I/O operations performed by service functions.
///
/// Service functions accept `&dyn RepositoryStore` so the storage backend
/// (filesystem, SQLite, in-memory) can be swapped without touching service logic.
/// All path arguments are *relative* to the repository root so that
/// implementations can resolve them however they choose.
pub trait RepositoryStore {
    // --- Repository lifecycle ---

    fn repository_root(&self) -> PathBuf;
    fn repository_exists(&self) -> Result<bool, RepositoryError>;
    fn initialize_repository(
        &self,
        input: &InitializeRepositoryInput,
    ) -> Result<CreateRepositoryResult, RepositoryError>;

    // --- Manifest ---

    fn load_manifest(&self) -> Result<Manifest, RepositoryError>;
    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError>;

    // --- Package (read) ---

    fn load_package(&self) -> Result<Package, RepositoryError>;

    // --- Package index (package.json raw) ---

    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError>;
    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError>;

    // --- Fields ---

    fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError>;
    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError>;
    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_fields_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Types ---

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError>;
    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError>;
    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Relation type definitions ---

    fn save_relation_type_definition(
        &self,
        relative_path: &str,
        relation_type: &RelationTypeDefinition,
    ) -> Result<(), RepositoryError>;
    fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_relation_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Views (L1) ---

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Document Views (L2) ---

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError>;
    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError>;
    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_document_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Blueprints ---

    fn save_blueprint(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError>;
    fn update_blueprint_file(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError>;
    fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_blueprints_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Instances (Notes, TypedRecords, Records) ---

    fn load_instance_json(&self, relative_path: &str)
        -> Result<serde_json::Value, RepositoryError>;
    fn save_instance_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_instance_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
    /// Returns relative paths of all JSON files directly under `relative_dir`.
    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError>;

    // --- Relations ---

    fn load_relations_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError>;
    fn save_relations_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    fn ensure_relations_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Containers ---

    /// Load a container by its logical `container_id`.
    /// Returns `ContainerNotFound` if no container with that ID is registered.
    fn load_container(
        &self,
        container_id: &str,
    ) -> Result<srs_core::types::container::Container, RepositoryError>;

    /// Persist a container by its logical `container_id`.
    /// Creates it if it does not exist; overwrites if it does.
    fn save_container(
        &self,
        container: &srs_core::types::container::Container,
    ) -> Result<(), RepositoryError>;

    /// Delete a container by its logical `container_id`.
    /// Returns `ContainerNotFound` if no container with that ID is registered.
    fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError>;

    /// List all containers as lightweight summaries `(container_id, title)`.
    /// Order is not guaranteed.
    fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError>;

    // --- Containers (transitional path-based methods — do not use in new service code) ---

    #[deprecated(note = "Use load_container instead")]
    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError>;
    #[deprecated(note = "Use save_container instead")]
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    #[deprecated(note = "Use delete_container instead")]
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    #[deprecated(note = "No-op in logical container model; remove call sites")]
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError>;

    // --- Package boundaries ---

    /// Return metadata for all package boundaries (primary + all sub-packages).
    fn list_package_boundaries(&self) -> Result<Vec<PackageBoundary>, RepositoryError>;

    /// Return metadata for one boundary. Returns `PackageNotFound` if missing.
    fn load_package_boundary(
        &self,
        selector: &PackageSelector,
    ) -> Result<PackageBoundary, RepositoryError>;

    /// Persist id/namespace/name/version for one boundary.
    /// Creates the boundary's `package.json` if it does not exist.
    fn save_package_boundary_metadata(
        &self,
        boundary: &PackageBoundary,
    ) -> Result<(), RepositoryError>;

    /// Register a boundary in the manifest's packageRefs (no-op for primary).
    /// No-op if already registered.
    fn register_package_boundary(&self, selector: &PackageSelector) -> Result<(), RepositoryError>;

    /// Add a definition path to a boundary's index (e.g. `"fields/foo.json"`).
    fn add_definition_to_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError>;

    /// Remove a definition path from a boundary's index.
    fn remove_definition_from_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError>;

    /// Find which boundary owns a field or type by ID.
    ///
    /// **Implementation note:** This is an O(n×m) linear scan in file-backed
    /// and in-memory stores (walks each boundary, loads each definition file
    /// and compares the `id` field). SQL adapters may maintain an index.
    fn resolve_definition_owner(
        &self,
        id: &str,
        kind: DefinitionKind,
    ) -> Result<PackageSelector, RepositoryError>;

    // --- Sub-package path validation ---

    /// List all files under `relative_dir` recursively, returning relative paths.
    /// Returns an empty Vec if the directory does not exist.
    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String>;

    /// Read a text file at `relative_path` and return its contents.
    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError>;

    /// Verify that `relative_path` (relative to repo root) points to a directory
    /// containing a `package.json`.
    ///
    /// Contract:
    ///   - `FileStore`: resolves against `repo_root`, checks the directory and
    ///     `package.json` exist, returns `PackageRefMissing` if not.
    ///   - `MemoryStore`: returns `Ok(())` unconditionally — path existence is
    ///     not meaningful in memory.
    fn validate_package_ref_path(&self, relative_path: &str) -> Result<(), RepositoryError>;
}

// ---------------------------------------------------------------------------
// FileStore — file-backed implementation
// ---------------------------------------------------------------------------

/// File-backed implementation of [`RepositoryStore`].
///
/// All `std::fs` calls in `srs-repository` are funnelled through this type.
/// Service functions must not import `std::fs` directly.
#[derive(Debug, Clone)]
pub struct FileStore {
    repo_root: PathBuf,
}

impl FileStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    fn abs(&self, relative_path: &str) -> PathBuf {
        self.repo_root.join(relative_path)
    }

    fn read_json(&self, path: &std::path::Path) -> Result<serde_json::Value, RepositoryError> {
        let content = std::fs::read_to_string(path).map_err(|e| RepositoryError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        serde_json::from_str(&content).map_err(|e| RepositoryError::Serialize {
            path: path.to_path_buf(),
            source: e,
        })
    }

    fn write_json(
        &self,
        path: &std::path::Path,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        let json = serde_json::to_string_pretty(value).map_err(|e| RepositoryError::Serialize {
            path: path.to_path_buf(),
            source: e,
        })?;
        std::fs::write(path, json).map_err(|e| RepositoryError::Io {
            path: path.to_path_buf(),
            source: e,
        })
    }

    fn ensure_dir(&self, path: &std::path::Path) -> Result<(), RepositoryError> {
        std::fs::create_dir_all(path).map_err(|e| RepositoryError::Io {
            path: path.to_path_buf(),
            source: e,
        })
    }

    fn delete_file(&self, path: &std::path::Path) -> Result<(), RepositoryError> {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| RepositoryError::Io {
                path: path.to_path_buf(),
                source: e,
            })?;
        }
        Ok(())
    }
}

// --- Package loading helpers (private to FileStore) ---

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
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    document_views: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
    #[serde(default)]
    blueprints: Vec<String>,
    #[serde(default)]
    dependency_refs: Vec<crate::package::DependencyRef>,
}

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
    #[serde(default)]
    vocabulary_ref: Option<String>,
    default_value: Option<serde_json::Value>,
    created_at: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypeJson {
    id: String,
    namespace: String,
    name: String,
    version: u32,
    description: Option<String>,
    fields: Vec<FieldAssignmentJson>,
    #[serde(default)]
    field_groups: Option<Vec<FieldGroupJson>>,
    #[serde(default)]
    extends_type_id: Option<String>,
    #[serde(default)]
    extends_type_version: Option<u32>,
    #[serde(default)]
    field_order: Option<Vec<String>>,
    #[serde(default)]
    field_assignment_overrides: Option<Vec<FieldAssignmentOverrideJson>>,
    #[serde(default)]
    lifecycle: Option<TypeLifecycle>,
    #[serde(default)]
    lifecycle_ref: Option<String>,
    created_at: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldAssignmentJson {
    field_id: String,
    order: u32,
    required: Option<bool>,
    display_label: Option<String>,
    #[serde(default)]
    repeatable: bool,
    min_items: Option<u32>,
    max_items: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldGroupJson {
    group_id: String,
    order: u32,
    fields: Vec<FieldAssignmentJson>,
    label: Option<String>,
    description: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    repeatable: bool,
    min_items: Option<u32>,
    max_items: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldAssignmentOverrideJson {
    field_id: String,
    display_label: Option<String>,
    display_hint: Option<String>,
    required: Option<bool>,
}

fn parse_value_type(s: &str, path: &std::path::Path) -> Result<ValueType, RepositoryError> {
    match s {
        "string" => Ok(ValueType::String),
        "text" => Ok(ValueType::Text),
        "number" => Ok(ValueType::Number),
        "boolean" => Ok(ValueType::Boolean),
        "date" => Ok(ValueType::Date),
        "url" => Ok(ValueType::Url),
        "select" => Ok(ValueType::Select),
        "multiselect" => Ok(ValueType::Multiselect),
        _ => Err(RepositoryError::InvalidValueType {
            path: path.to_path_buf(),
            value_type: s.to_string(),
        }),
    }
}

#[allow(clippy::type_complexity)]
fn load_package_from_dir(
    package_dir: &std::path::Path,
    rt_by_type: &mut HashMap<String, (RelationTypeDefinition, PathBuf)>,
) -> Result<
    (
        Vec<Field>,
        Vec<RecordType>,
        Vec<View>,
        Vec<DocumentView>,
        Vec<Theme>,
        Vec<srs_core::types::blueprint::Blueprint>,
    ),
    RepositoryError,
> {
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

    let mut fields = Vec::new();
    for field_path in &metadata.fields {
        let full_path = package_dir.join(field_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let fj: FieldJson =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;
        fields.push(Field {
            id: fj.id,
            namespace: fj.namespace,
            name: fj.name,
            version: fj.version,
            value_type: parse_value_type(&fj.value_type, &full_path)?,
            description: fj.description.unwrap_or_default(),
            ai_guidance: fj.ai_guidance.unwrap_or(serde_json::Value::Null),
            allowed_values: fj.allowed_values,
            vocabulary_ref: fj.vocabulary_ref,
            default_value: fj.default_value,
            created_at: fj.created_at.unwrap_or_default(),
            extra: HashMap::new(),
        });
    }

    let mut record_types = Vec::new();
    for type_path in &metadata.types {
        let full_path = package_dir.join(type_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let tj: TypeJson =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;
        let type_fields: Vec<FieldAssignment> = tj
            .fields
            .into_iter()
            .map(|fa| FieldAssignment {
                field_id: fa.field_id,
                order: fa.order,
                required: fa.required.unwrap_or(true),
                display_label: fa.display_label,
                repeatable: fa.repeatable,
                min_items: fa.min_items,
                max_items: fa.max_items,
            })
            .collect();
        let field_groups = tj.field_groups.map(|groups| {
            groups
                .into_iter()
                .map(|g| FieldGroup {
                    group_id: g.group_id,
                    order: g.order,
                    fields: g
                        .fields
                        .into_iter()
                        .map(|fa| FieldAssignment {
                            field_id: fa.field_id,
                            order: fa.order,
                            required: fa.required.unwrap_or(true),
                            display_label: fa.display_label,
                            repeatable: fa.repeatable,
                            min_items: fa.min_items,
                            max_items: fa.max_items,
                        })
                        .collect(),
                    label: g.label,
                    description: g.description,
                    required: g.required,
                    repeatable: g.repeatable,
                    min_items: g.min_items,
                    max_items: g.max_items,
                })
                .collect()
        });
        let field_assignment_overrides = tj.field_assignment_overrides.map(|overrides| {
            overrides
                .into_iter()
                .map(|o| FieldAssignmentOverride {
                    field_id: o.field_id,
                    display_label: o.display_label,
                    display_hint: o.display_hint,
                    required: o.required,
                })
                .collect()
        });
        record_types.push(RecordType {
            id: tj.id,
            namespace: tj.namespace,
            name: tj.name,
            version: tj.version,
            description: tj.description.unwrap_or_default(),
            fields: type_fields,
            field_groups,
            extends_type_id: tj.extends_type_id,
            extends_type_version: tj.extends_type_version,
            field_order: tj.field_order,
            field_assignment_overrides,
            lifecycle: tj.lifecycle,
            lifecycle_ref: tj.lifecycle_ref,
            created_at: tj.created_at.unwrap_or_default(),
            extra: HashMap::new(),
        });
    }

    for rt_path in &metadata.relation_types {
        let full_path = package_dir.join(rt_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let def: RelationTypeDefinition =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;
        validate_relation_type_definition(&def).map_err(|source| {
            RepositoryError::RelationTypeDefinitionValidation {
                path: full_path.clone(),
                source,
            }
        })?;
        if let Some((existing, existing_path)) = rt_by_type.get(&def.key) {
            if existing != &def {
                return Err(RepositoryError::RelationTypeDefinitionConflict {
                    relation_type: def.key.clone(),
                    path_a: existing_path.clone(),
                    path_b: full_path,
                });
            }
        } else {
            rt_by_type.insert(def.key.clone(), (def, full_path));
        }
    }

    let mut views = Vec::new();
    for view_path in &metadata.views {
        let full_path = package_dir.join(view_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let view: View =
            serde_json::from_str(&content).map_err(|source| RepositoryError::ViewLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_view(&view).map_err(|source| RepositoryError::ViewValidation {
            path: full_path.clone(),
            source,
        })?;
        views.push(view);
    }

    let mut document_views = Vec::new();
    for dv_path in &metadata.document_views {
        let full_path = package_dir.join(dv_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let dv: DocumentView =
            serde_json::from_str(&content).map_err(|source| RepositoryError::DocumentViewLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_document_view(&dv).map_err(|source| RepositoryError::DocumentViewValidation {
            path: full_path.clone(),
            source,
        })?;
        document_views.push(dv);
    }

    let mut themes = Vec::new();
    for theme_path in &metadata.themes {
        let full_path = package_dir.join(theme_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let theme: Theme =
            serde_json::from_str(&content).map_err(|source| RepositoryError::ThemeLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_theme(&theme).map_err(|source| RepositoryError::ThemeValidation {
            path: full_path.clone(),
            source,
        })?;
        themes.push(theme);
    }

    let mut blueprints: Vec<srs_core::types::blueprint::Blueprint> = Vec::new();
    for blueprint_path in &metadata.blueprints {
        let full_path = package_dir.join(blueprint_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let blueprint: srs_core::types::blueprint::Blueprint = serde_json::from_str(&content)
            .map_err(|source| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source,
            })?;
        blueprints.push(blueprint);
    }

    Ok((
        fields,
        record_types,
        views,
        document_views,
        themes,
        blueprints,
    ))
}

impl RepositoryStore for FileStore {
    fn repository_root(&self) -> PathBuf {
        self.repo_root.clone()
    }

    fn repository_exists(&self) -> Result<bool, RepositoryError> {
        let srs_dir = self.repo_root.join(".srs");
        let manifest = self.repo_root.join("manifest.json");
        let package = self.repo_root.join("package/package.json");
        Ok(srs_dir.is_dir() && manifest.is_file() && package.is_file())
    }

    fn initialize_repository(
        &self,
        input: &InitializeRepositoryInput,
    ) -> Result<CreateRepositoryResult, RepositoryError> {
        if self.repository_exists()? {
            return Err(RepositoryError::RepositoryAlreadyExists {
                path: self.repo_root.clone(),
            });
        }

        self.ensure_dir(&self.repo_root.join(".srs"))?;
        self.ensure_dir(&self.repo_root.join("package"))?;

        let mut manifest = serde_json::json!({
            "instanceIndex": [],
            "srsVersion": input.repository.srs_version,
            "repositoryId": input.repository.repository_id,
            "namespace": input.repository.namespace
        });
        if let Some(name) = &input.repository.name {
            manifest["name"] = serde_json::Value::String(name.clone());
        }
        if let Some(desc) = &input.repository.description {
            manifest["description"] = serde_json::Value::String(desc.clone());
        }
        self.write_json(&self.repo_root.join("manifest.json"), &manifest)?;

        let package = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
            "id": input.primary_package.id,
            "namespace": input.primary_package.namespace,
            "name": input.primary_package.name,
            "version": input.primary_package.version,
            "title": input.primary_package.name,
            "description": "",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [],
            "types": [],
            "relationTypes": [],
            "views": [],
            "documentViews": []
        });
        self.write_json(&self.repo_root.join("package/package.json"), &package)?;

        Ok(CreateRepositoryResult {
            repo_root: self.repo_root.clone(),
            repository_id: input.repository.repository_id.clone(),
            package_id: input.primary_package.id.clone(),
        })
    }

    // --- Manifest ---

    fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
        let manifest_path = self.repo_root.join("manifest.json");
        if !manifest_path.exists() {
            return Err(RepositoryError::ManifestMissing {
                path: manifest_path,
            });
        }
        let content = std::fs::read_to_string(&manifest_path).map_err(|e| RepositoryError::Io {
            path: manifest_path.clone(),
            source: e,
        })?;
        let mut manifest: Manifest =
            serde_json::from_str(&content).map_err(|e| RepositoryError::ManifestParse {
                path: manifest_path.clone(),
                source: e,
            })?;
        manifest.root = self.repo_root.clone();
        Ok(manifest)
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError> {
        let manifest_path = self.repo_root.join("manifest.json");
        let value = serde_json::to_value(manifest).map_err(|e| RepositoryError::Serialize {
            path: manifest_path.clone(),
            source: e,
        })?;
        self.write_json(&manifest_path, &value)
    }

    // --- Package ---

    fn load_package(&self) -> Result<Package, RepositoryError> {
        let package_dir = self.repo_root.join("package");
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

        let mut rt_by_type: HashMap<String, (RelationTypeDefinition, PathBuf)> = HashMap::new();
        let (
            mut fields,
            mut record_types,
            mut views,
            mut document_views,
            mut themes,
            mut blueprints,
        ) = load_package_from_dir(&package_dir, &mut rt_by_type)?;

        // Merge sub-packages from manifest packageRefs
        let manifest = self.load_manifest()?;
        if let Some(pkg_refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            let mut field_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut type_sources: HashMap<(String, u32), PathBuf> = HashMap::new();
            let mut view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut doc_view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut theme_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut blueprint_sources: HashMap<String, PathBuf> = HashMap::new();
            for f in &fields {
                field_sources.insert(f.id.clone(), package_dir.clone());
            }
            for rt in &record_types {
                type_sources.insert((rt.id.clone(), rt.version), package_dir.clone());
            }
            for v in &views {
                view_sources.insert(v.id.clone(), package_dir.clone());
            }
            for dv in &document_views {
                doc_view_sources.insert(dv.id.clone(), package_dir.clone());
            }
            for theme in &themes {
                theme_sources.insert(theme.id.clone(), package_dir.clone());
            }
            for bp in &blueprints {
                blueprint_sources.insert(bp.id.clone(), package_dir.clone());
            }

            for pkg_ref in pkg_refs {
                let mode = pkg_ref.get("mode").and_then(|m| m.as_str()).unwrap_or("");
                if mode != "local" {
                    continue;
                }
                let rel_path = match pkg_ref.get("path").and_then(|p| p.as_str()) {
                    Some(p) => p,
                    None => continue,
                };
                let sub_dir = self.repo_root.join(rel_path);
                if !sub_dir.join("package.json").exists() {
                    return Err(RepositoryError::PackageRefMissing {
                        path: rel_path.to_string(),
                    });
                }
                let (sub_fields, sub_types, sub_views, sub_doc_views, sub_themes, sub_blueprints) =
                    load_package_from_dir(&sub_dir, &mut rt_by_type)?;

                for field in sub_fields {
                    if let Some(first_path) = field_sources.get(&field.id) {
                        let existing = fields.iter().find(|f| f.id == field.id).unwrap();
                        if existing.version != field.version
                            || existing.namespace != field.namespace
                            || existing.name != field.name
                        {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "field".to_string(),
                                id: field.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        field_sources.insert(field.id.clone(), sub_dir.clone());
                        fields.push(field);
                    }
                }
                for rt in sub_types {
                    let key = (rt.id.clone(), rt.version);
                    if let Some(first_path) = type_sources.get(&key) {
                        let existing = record_types
                            .iter()
                            .find(|r| r.id == rt.id && r.version == rt.version)
                            .unwrap();
                        if existing.namespace != rt.namespace || existing.name != rt.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "type".to_string(),
                                id: rt.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        type_sources.insert(key, sub_dir.clone());
                        record_types.push(rt);
                    }
                }
                for view in sub_views {
                    if let Some(first_path) = view_sources.get(&view.id) {
                        let existing = views.iter().find(|v| v.id == view.id).unwrap();
                        if existing.name != view.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "view".to_string(),
                                id: view.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        view_sources.insert(view.id.clone(), sub_dir.clone());
                        views.push(view);
                    }
                }
                for dv in sub_doc_views {
                    if let Some(first_path) = doc_view_sources.get(&dv.id) {
                        let existing = document_views.iter().find(|d| d.id == dv.id).unwrap();
                        if existing.name != dv.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "document-view".to_string(),
                                id: dv.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        doc_view_sources.insert(dv.id.clone(), sub_dir.clone());
                        document_views.push(dv);
                    }
                }
                for theme in sub_themes {
                    if let Some(first_path) = theme_sources.get(&theme.id) {
                        let existing = themes.iter().find(|t| t.id == theme.id).unwrap();
                        if existing.namespace != theme.namespace
                            || existing.name != theme.name
                            || existing.version != theme.version
                        {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "theme".to_string(),
                                id: theme.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        theme_sources.insert(theme.id.clone(), sub_dir.clone());
                        themes.push(theme);
                    }
                }
                for bp in sub_blueprints {
                    if let Some(first_path) = blueprint_sources.get(&bp.id) {
                        let existing = blueprints.iter().find(|b| b.id == bp.id).unwrap();
                        if existing.name != bp.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "blueprint".to_string(),
                                id: bp.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        blueprint_sources.insert(bp.id.clone(), sub_dir.clone());
                        blueprints.push(bp);
                    }
                }
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
            views,
            document_views,
            themes,
            blueprints,
            root: self.repo_root.clone(),
            dependency_refs: metadata.dependency_refs.clone(),
            vocabularies: vec![],
            lifecycles: vec![],
        })
    }

    // --- Package JSON ---

    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(&self.repo_root.join("package/package.json"))
    }

    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError> {
        self.write_json(&self.repo_root.join("package/package.json"), value)
    }

    // --- Fields ---

    fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(field).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.abs(relative_path), &value)
    }

    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        self.save_field(relative_path, field)
    }

    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_fields_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    // --- Types ---

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(record_type).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.abs(relative_path), &value)
    }

    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        self.save_type(relative_path, record_type)
    }

    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    fn save_relation_type_definition(
        &self,
        relative_path: &str,
        relation_type: &RelationTypeDefinition,
    ) -> Result<(), RepositoryError> {
        let value =
            serde_json::to_value(relation_type).map_err(|e| RepositoryError::Serialize {
                path: self.abs(relative_path),
                source: e,
            })?;
        self.write_json(&self.abs(relative_path), &value)
    }

    fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_relation_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    // --- Views (L1) ---

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(view).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.abs(relative_path), &value)
    }

    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        self.save_view(relative_path, view)
    }

    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    // --- Document Views (L2) ---

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(view).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.abs(relative_path), &value)
    }

    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        self.save_document_view(relative_path, view)
    }

    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_document_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    // --- Blueprints ---

    fn save_blueprint(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(blueprint).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.abs(relative_path), &value)
    }

    fn update_blueprint_file(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        self.save_blueprint(relative_path, blueprint)
    }

    fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_blueprints_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    // --- Instances ---

    fn load_instance_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(&self.abs(relative_path))
    }

    fn save_instance_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(&self.abs(relative_path), value)
    }

    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_instance_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError> {
        let dir = self.abs(relative_dir);
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| RepositoryError::Io {
            path: dir.clone(),
            source: e,
        })? {
            let entry = entry.map_err(|e| RepositoryError::Io {
                path: dir.clone(),
                source: e,
            })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(rel) = path.strip_prefix(&self.repo_root) {
                    paths.push(rel.to_string_lossy().into_owned());
                }
            }
        }
        Ok(paths)
    }

    // --- Relations ---

    fn load_relations_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(&self.abs(relative_path))
    }

    fn save_relations_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(&self.abs(relative_path), value)
    }

    fn ensure_relations_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.abs(relative_dir))
    }

    // --- Containers ---

    fn load_container(
        &self,
        container_id: &str,
    ) -> Result<srs_core::types::container::Container, RepositoryError> {
        let path = file_store_find_container_path(self, container_id)?;
        let val = self.read_json(&self.abs(&path))?;
        serde_json::from_value(val).map_err(|source| RepositoryError::ManifestParse {
            path: self.abs(&path),
            source,
        })
    }

    fn save_container(
        &self,
        container: &srs_core::types::container::Container,
    ) -> Result<(), RepositoryError> {
        let id = &container.container_id;
        let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from("containers"),
            source,
        })?;
        // Find existing path or derive a new one
        let path = match file_store_find_container_path(self, id) {
            Ok(p) => p,
            Err(RepositoryError::ContainerNotFound { .. }) => {
                // New container — derive path from title slug + id prefix
                let slug = container
                    .title
                    .to_lowercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect::<String>();
                let prefix = &id[..id.len().min(8)];
                let filename = format!("containers/{slug}-{prefix}.json");
                // Register in index
                file_store_upsert_container_index(self, id, &container.title, &filename)?;
                filename
            }
            Err(e) => return Err(e),
        };
        self.ensure_dir(&self.repo_root.join("containers"))?;
        self.write_json(&self.abs(&path), &val)
    }

    fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError> {
        let path = file_store_find_container_path(self, container_id)?;
        // Remove from index
        file_store_remove_container_index(self, container_id)?;
        // Delete file (ignore missing-file errors)
        let _ = self.delete_file(&self.abs(&path));
        Ok(())
    }

    fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
        let index = file_store_load_container_index(self)?;
        Ok(index
            .into_iter()
            .map(|(id, title, _path)| (id, title))
            .collect())
    }

    #[allow(deprecated)]
    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(&self.abs(relative_path))
    }

    #[allow(deprecated)]
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(&self.abs(relative_path), value)
    }

    #[allow(deprecated)]
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    #[allow(deprecated)]
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.repo_root.join("containers"))
    }

    // --- Package boundaries ---

    fn list_package_boundaries(&self) -> Result<Vec<PackageBoundary>, RepositoryError> {
        let mut result = Vec::new();

        // Primary package
        let primary_json = self.read_json(&self.repo_root.join("package/package.json"))?;
        result.push(file_store_boundary_from_json(&primary_json, None));

        // Sub-packages from manifest packageRefs
        let manifest = self.load_manifest()?;
        if let Some(refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            for pkg_ref in refs {
                if pkg_ref.get("mode").and_then(|m| m.as_str()) != Some("local") {
                    continue;
                }
                if let Some(path) = pkg_ref.get("path").and_then(|p| p.as_str()) {
                    let pkg_json_path = self.repo_root.join(path).join("package.json");
                    if let Ok(pkg_json) = self.read_json(&pkg_json_path) {
                        result.push(file_store_boundary_from_json(
                            &pkg_json,
                            Some(path.to_string()),
                        ));
                    }
                }
            }
        }
        Ok(result)
    }

    fn load_package_boundary(
        &self,
        selector: &PackageSelector,
    ) -> Result<PackageBoundary, RepositoryError> {
        let pkg_json_path = match selector {
            None => self.repo_root.join("package/package.json"),
            Some(path) => self.repo_root.join(path).join("package.json"),
        };
        let pkg_json =
            self.read_json(&pkg_json_path)
                .map_err(|_| RepositoryError::PackageNotFound {
                    selector: selector.clone(),
                })?;
        Ok(file_store_boundary_from_json(&pkg_json, selector.clone()))
    }

    fn save_package_boundary_metadata(
        &self,
        boundary: &PackageBoundary,
    ) -> Result<(), RepositoryError> {
        let pkg_json_path = match &boundary.selector {
            None => self.repo_root.join("package/package.json"),
            Some(path) => self.repo_root.join(path).join("package.json"),
        };
        // Load existing or create a skeleton
        let mut pkg_json = if pkg_json_path.exists() {
            self.read_json(&pkg_json_path)?
        } else {
            serde_json::json!({
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            })
        };
        if let Some(obj) = pkg_json.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!(boundary.id));
            obj.insert(
                "namespace".to_string(),
                serde_json::json!(boundary.namespace),
            );
            obj.insert("name".to_string(), serde_json::json!(boundary.name));
            obj.insert("version".to_string(), serde_json::json!(boundary.version));
        }
        self.write_json(&pkg_json_path, &pkg_json)
    }

    fn register_package_boundary(&self, selector: &PackageSelector) -> Result<(), RepositoryError> {
        let path = match selector {
            None => return Ok(()), // primary — no-op
            Some(p) => p,
        };
        let mut manifest = self.load_manifest()?;
        let mut refs: Vec<serde_json::Value> = manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let already = refs
            .iter()
            .any(|r| r.get("path").and_then(|p| p.as_str()) == Some(path));
        if !already {
            refs.push(serde_json::json!({"mode": "local", "path": path}));
            refs.sort_by(|a, b| {
                a.get("path")
                    .and_then(|p| p.as_str())
                    .cmp(&b.get("path").and_then(|p| p.as_str()))
            });
            manifest
                .extra
                .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
            self.save_manifest(&manifest)?;
        }
        Ok(())
    }

    fn add_definition_to_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let pkg_json_path = match selector {
            None => self.repo_root.join("package/package.json"),
            Some(p) => self.repo_root.join(p).join("package.json"),
        };
        let mut pkg_json = self.read_json(&pkg_json_path)?;
        let key = definition_kind_key(kind);
        let arr = pkg_json[key]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: pkg_json_path.clone(),
                source: serde_json::Error::custom(format!("{key} is not an array")),
            })?;
        if !arr.iter().any(|e| e.as_str() == Some(path)) {
            arr.push(serde_json::json!(path));
        }
        self.write_json(&pkg_json_path, &pkg_json)
    }

    fn remove_definition_from_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let pkg_json_path = match selector {
            None => self.repo_root.join("package/package.json"),
            Some(p) => self.repo_root.join(p).join("package.json"),
        };
        let mut pkg_json = self.read_json(&pkg_json_path)?;
        let key = definition_kind_key(kind);
        if let Some(arr) = pkg_json[key].as_array_mut() {
            arr.retain(|e| e.as_str() != Some(path));
        }
        self.write_json(&pkg_json_path, &pkg_json)
    }

    fn resolve_definition_owner(
        &self,
        id: &str,
        kind: DefinitionKind,
    ) -> Result<PackageSelector, RepositoryError> {
        let boundaries = self.list_package_boundaries()?;
        let key = definition_kind_key(kind);
        for boundary in &boundaries {
            let boundary_dir = match &boundary.selector {
                None => self.repo_root.join("package"),
                Some(p) => self.repo_root.join(p),
            };
            let pkg_json_path = boundary_dir.join("package.json");
            if let Ok(pkg_json) = self.read_json(&pkg_json_path) {
                if let Some(paths) = pkg_json[key].as_array() {
                    for entry in paths {
                        if let Some(rel) = entry.as_str() {
                            let full = boundary_dir.join(rel);
                            if let Ok(def_json) = self.read_json(&full) {
                                if def_json["id"].as_str() == Some(id) {
                                    return Ok(boundary.selector.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(RepositoryError::DefinitionNotFound { id: id.to_string() })
    }

    // --- Generic file access ---

    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String> {
        let dir = self.abs(relative_dir);
        let mut result = Vec::new();
        collect_paths_recursive(&self.repo_root, &dir, &mut result);
        result
    }

    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError> {
        let path = self.abs(relative_path);
        std::fs::read_to_string(&path).map_err(|source| RepositoryError::Io { path, source })
    }

    // --- Sub-package path validation ---

    fn validate_package_ref_path(&self, relative_path: &str) -> Result<(), RepositoryError> {
        let repo_root_canonical =
            self.repo_root
                .canonicalize()
                .map_err(|source| RepositoryError::Io {
                    path: self.repo_root.clone(),
                    source,
                })?;

        let candidate = self.repo_root.join(relative_path);
        let candidate_canonical =
            candidate
                .canonicalize()
                .map_err(|_| RepositoryError::PackageRefMissing {
                    path: relative_path.to_string(),
                })?;

        if !candidate_canonical.starts_with(&repo_root_canonical) {
            return Err(RepositoryError::PackageRefOutsideRepo {
                path: relative_path.to_string(),
            });
        }

        if !candidate_canonical.join("package.json").exists() {
            return Err(RepositoryError::PackageRefMissing {
                path: relative_path.to_string(),
            });
        }

        Ok(())
    }
}

/// Build a `PackageBoundary` from a parsed `package.json` value and its selector.
fn file_store_boundary_from_json(
    pkg_json: &serde_json::Value,
    selector: PackageSelector,
) -> PackageBoundary {
    let field_paths = pkg_json["fields"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let type_paths = pkg_json["types"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    PackageBoundary {
        selector,
        id: pkg_json["id"].as_str().unwrap_or("").to_string(),
        namespace: pkg_json["namespace"].as_str().unwrap_or("").to_string(),
        name: pkg_json["name"].as_str().unwrap_or("").to_string(),
        version: pkg_json["version"].as_str().unwrap_or("").to_string(),
        field_paths,
        type_paths,
    }
}

/// Map a `DefinitionKind` to the JSON array key in `package.json`.
pub(crate) fn definition_kind_key(kind: DefinitionKind) -> &'static str {
    match kind {
        DefinitionKind::Field => "fields",
        DefinitionKind::Type => "types",
        DefinitionKind::View => "views",
        DefinitionKind::DocumentView => "documentViews",
        DefinitionKind::RelationType => "relationTypes",
        DefinitionKind::Blueprint => "blueprints",
    }
}

/// Load the container index as `(container_id, title, path)` triples from the manifest.
fn file_store_load_container_index(
    store: &FileStore,
) -> Result<Vec<(String, String, String)>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let entries: Vec<serde_json::Value> = manifest
        .extra
        .get("containerIndex")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    Ok(entries
        .into_iter()
        .filter_map(|e| {
            let id = e["containerId"].as_str()?.to_string();
            let title = e["title"].as_str().unwrap_or("").to_string();
            let path = e["path"].as_str()?.to_string();
            Some((id, title, path))
        })
        .collect())
}

/// Find the file path for a container by its `container_id`.
fn file_store_find_container_path(
    store: &FileStore,
    container_id: &str,
) -> Result<String, RepositoryError> {
    let index = file_store_load_container_index(store)?;
    index
        .into_iter()
        .find(|(id, _, _)| id == container_id)
        .map(|(_, _, path)| path)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })
}

/// Insert or update an entry in the manifest `containerIndex`.
fn file_store_upsert_container_index(
    store: &FileStore,
    container_id: &str,
    title: &str,
    path: &str,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    let mut entries: Vec<serde_json::Value> = manifest
        .extra
        .get("containerIndex")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    entries.retain(|e| e["containerId"].as_str() != Some(container_id));
    entries.push(serde_json::json!({
        "containerId": container_id,
        "title": title,
        "path": path,
    }));
    manifest.extra.insert(
        "containerIndex".to_string(),
        serde_json::to_value(entries).unwrap(),
    );
    store.save_manifest(&manifest)
}

/// Remove an entry from the manifest `containerIndex`.
fn file_store_remove_container_index(
    store: &FileStore,
    container_id: &str,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    let mut entries: Vec<serde_json::Value> = manifest
        .extra
        .get("containerIndex")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    entries.retain(|e| e["containerId"].as_str() != Some(container_id));
    manifest.extra.insert(
        "containerIndex".to_string(),
        serde_json::to_value(entries).unwrap(),
    );
    store.save_manifest(&manifest)
}

/// Recursively collect file paths under `dir`, storing relative paths from `root`.
fn collect_paths_recursive(
    root: &std::path::Path,
    dir: &std::path::Path,
    result: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_paths_recursive(root, &path, result);
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            result.push(relative);
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryStore — in-memory test implementation
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod memory {
    use super::*;
    use std::cell::RefCell;

    /// In-memory implementation of [`RepositoryStore`] for unit tests.
    ///
    /// No filesystem access. All `ensure_*` methods are no-ops.
    pub struct MemoryStore {
        manifest: RefCell<Manifest>,
        package: RefCell<Package>,
        data: RefCell<HashMap<String, serde_json::Value>>,
        repository_initialized: RefCell<bool>,
        /// Package boundary metadata keyed by `PackageSelector`.
        /// Always pre-populated with the primary boundary (`None`).
        boundaries: RefCell<HashMap<Option<String>, crate::package_types::PackageBoundary>>,
    }

    impl MemoryStore {
        pub fn new(manifest: Manifest, package: Package) -> Self {
            let pkg_json = Self::package_to_json(&package);
            let primary_boundary = crate::package_types::PackageBoundary {
                selector: None,
                id: package.id.clone(),
                namespace: package.namespace.clone(),
                name: package.name.clone(),
                version: package.version.clone(),
                field_paths: vec![],
                type_paths: vec![],
            };
            let mut boundaries = HashMap::new();
            boundaries.insert(None, primary_boundary);
            let store = Self {
                manifest: RefCell::new(manifest),
                package: RefCell::new(package),
                data: RefCell::new(HashMap::new()),
                repository_initialized: RefCell::new(true),
                boundaries: RefCell::new(boundaries),
            };
            store
                .data
                .borrow_mut()
                .insert("package/package.json".to_string(), pkg_json);
            store
        }

        /// Minimal empty store — empty manifest, empty package, minimal package.json.
        pub fn empty() -> Self {
            let manifest = Manifest {
                instance_index: vec![],
                extra: HashMap::new(),
                root: PathBuf::from("/memory"),
            };
            let package = Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            };
            Self::new(manifest, package)
        }

        /// Build a store pre-populated with a single field.
        pub fn with_field(field: Field) -> Self {
            let store = Self::empty();
            let filename = format!(
                "fields/{}-{}.json",
                field.name.to_lowercase().replace(' ', "-"),
                &field.id[..8]
            );
            // Update the in-memory package
            store.package.borrow_mut().fields.push(field.clone());
            // Update package.json index (paths are package-relative, no "package/" prefix)
            store
                .data
                .borrow_mut()
                .get_mut("package/package.json")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .get_mut("fields")
                .unwrap()
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(filename.clone()));
            // Store the field data file at repo-root-relative key ("package/fields/...")
            let field_val = serde_json::to_value(&field).unwrap();
            store
                .data
                .borrow_mut()
                .insert(format!("package/{filename}"), field_val);
            // Update primary boundary field_paths for resolve_definition_owner
            store
                .boundaries
                .borrow_mut()
                .get_mut(&None)
                .unwrap()
                .field_paths
                .push(filename);
            store
        }

        /// Build a store pre-populated with a single type.
        pub fn with_type(record_type: RecordType) -> Self {
            let store = Self::empty();
            let filename = format!(
                "types/{}-{}.json",
                record_type.name.to_lowercase().replace(' ', "-"),
                &record_type.id[..8]
            );
            store
                .package
                .borrow_mut()
                .record_types
                .push(record_type.clone());
            store
                .data
                .borrow_mut()
                .get_mut("package/package.json")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .get_mut("types")
                .unwrap()
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(filename.clone()));
            let type_val = serde_json::to_value(&record_type).unwrap();
            store
                .data
                .borrow_mut()
                .insert(format!("package/{filename}"), type_val);
            // Update primary boundary type_paths for resolve_definition_owner
            store
                .boundaries
                .borrow_mut()
                .get_mut(&None)
                .unwrap()
                .type_paths
                .push(filename);
            store
        }

        fn package_to_json(pkg: &Package) -> serde_json::Value {
            serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": pkg.id,
                "namespace": pkg.namespace,
                "name": pkg.name,
                "version": pkg.version,
                "title": pkg.name,
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            })
        }

        /// Pre-populate with a JSON value at the given relative path.
        pub fn with_data(self, path: &str, value: serde_json::Value) -> Self {
            self.data.borrow_mut().insert(path.to_string(), value);
            self
        }

        pub fn uninitialized() -> Self {
            let manifest = Manifest {
                instance_index: vec![],
                extra: HashMap::new(),
                root: PathBuf::from("/memory"),
            };
            let package = Package {
                id: "".to_string(),
                namespace: "".to_string(),
                name: "".to_string(),
                version: "".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            };
            Self {
                manifest: RefCell::new(manifest),
                package: RefCell::new(package),
                data: RefCell::new(HashMap::new()),
                repository_initialized: RefCell::new(false),
                boundaries: RefCell::new(HashMap::new()),
            }
        }

        /// Return a clone of all stored data (for assertions).
        pub fn all_data(&self) -> HashMap<String, serde_json::Value> {
            self.data.borrow().clone()
        }

        /// Sync the `data["<prefix>/package.json"]` JSON entry when a definition
        /// is added or removed, so `load_package_json()` stays consistent.
        fn memory_store_sync_pkg_json(
            &self,
            selector: &PackageSelector,
            kind: crate::package_types::DefinitionKind,
            path: &str,
            add: bool, // true = add, false = remove
        ) -> Result<(), RepositoryError> {
            use crate::store::definition_kind_key;
            let data_key = match selector {
                None => "package/package.json".to_string(),
                Some(p) => format!("{p}/package.json"),
            };
            let array_key = definition_kind_key(kind);
            let mut data = self.data.borrow_mut();
            if let Some(pkg_json) = data.get_mut(&data_key) {
                if let Some(arr) = pkg_json[array_key].as_array_mut() {
                    if add {
                        if !arr.iter().any(|e| e.as_str() == Some(path)) {
                            arr.push(serde_json::json!(path));
                        }
                    } else {
                        arr.retain(|e| e.as_str() != Some(path));
                    }
                }
            }
            Ok(())
        }
    }

    impl Default for MemoryStore {
        fn default() -> Self {
            Self::empty()
        }
    }

    fn not_found(path: &str) -> RepositoryError {
        RepositoryError::Io {
            path: PathBuf::from(path),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found in MemoryStore"),
        }
    }

    impl RepositoryStore for MemoryStore {
        fn repository_root(&self) -> PathBuf {
            PathBuf::from("/memory")
        }

        fn repository_exists(&self) -> Result<bool, RepositoryError> {
            Ok(*self.repository_initialized.borrow())
        }

        fn initialize_repository(
            &self,
            input: &InitializeRepositoryInput,
        ) -> Result<CreateRepositoryResult, RepositoryError> {
            if *self.repository_initialized.borrow() {
                return Err(RepositoryError::RepositoryAlreadyExists {
                    path: PathBuf::from("/memory"),
                });
            }

            let mut manifest_extra = HashMap::new();
            manifest_extra.insert(
                "srsVersion".to_string(),
                serde_json::Value::String(input.repository.srs_version.clone()),
            );
            manifest_extra.insert(
                "repositoryId".to_string(),
                serde_json::Value::String(input.repository.repository_id.clone()),
            );
            manifest_extra.insert(
                "namespace".to_string(),
                serde_json::Value::String(input.repository.namespace.clone()),
            );
            if let Some(name) = &input.repository.name {
                manifest_extra.insert("name".to_string(), serde_json::Value::String(name.clone()));
            }
            if let Some(desc) = &input.repository.description {
                manifest_extra.insert(
                    "description".to_string(),
                    serde_json::Value::String(desc.clone()),
                );
            }

            *self.manifest.borrow_mut() = Manifest {
                instance_index: vec![],
                extra: manifest_extra,
                root: PathBuf::from("/memory"),
            };

            *self.package.borrow_mut() = Package {
                id: input.primary_package.id.clone(),
                namespace: input.primary_package.namespace.clone(),
                name: input.primary_package.name.clone(),
                version: input.primary_package.version.clone(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            };

            let package_json = serde_json::json!({
                "id": input.primary_package.id,
                "namespace": input.primary_package.namespace,
                "name": input.primary_package.name,
                "version": input.primary_package.version,
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            });
            self.data
                .borrow_mut()
                .insert("package/package.json".to_string(), package_json);
            *self.repository_initialized.borrow_mut() = true;

            Ok(CreateRepositoryResult {
                repo_root: PathBuf::from("/memory"),
                repository_id: input.repository.repository_id.clone(),
                package_id: input.primary_package.id.clone(),
            })
        }

        fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
            Ok(self.manifest.borrow().clone())
        }

        fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError> {
            *self.manifest.borrow_mut() = manifest.clone();
            Ok(())
        }

        fn load_package(&self) -> Result<Package, RepositoryError> {
            Ok(self.package.borrow().clone())
        }

        fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get("package/package.json")
                .cloned()
                .ok_or_else(|| not_found("package/package.json"))
        }

        fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert("package/package.json".to_string(), value.clone());
            Ok(())
        }

        fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(field).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_field_file(
            &self,
            relative_path: &str,
            field: &Field,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_field(relative_path, field)
        }

        fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_fields_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_type(
            &self,
            relative_path: &str,
            record_type: &RecordType,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(record_type).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_type_file(
            &self,
            relative_path: &str,
            record_type: &RecordType,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_type(relative_path, record_type)
        }

        fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_relation_type_definition(
            &self,
            relative_path: &str,
            relation_type: &RelationTypeDefinition,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(relation_type).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            let key = format!("package/{relative_path}");
            self.data.borrow_mut().remove(&key);
            Ok(())
        }

        fn ensure_relation_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(view).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_view_file(
            &self,
            relative_path: &str,
            view: &View,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_view(relative_path, view)
        }

        fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_document_view(
            &self,
            relative_path: &str,
            view: &DocumentView,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(view).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_document_view_file(
            &self,
            relative_path: &str,
            view: &DocumentView,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_document_view(relative_path, view)
        }

        fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_document_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_blueprint(
            &self,
            relative_path: &str,
            blueprint: &srs_core::types::blueprint::Blueprint,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(blueprint).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_blueprint_file(
            &self,
            relative_path: &str,
            blueprint: &srs_core::types::blueprint::Blueprint,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_blueprint(relative_path, blueprint)
        }

        fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_blueprints_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn load_instance_json(
            &self,
            relative_path: &str,
        ) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        fn save_instance_json(
            &self,
            relative_path: &str,
            value: &serde_json::Value,
        ) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert(relative_path.to_string(), value.clone());
            Ok(())
        }

        fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_instance_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError> {
            let prefix = if relative_dir.ends_with('/') {
                relative_dir.to_string()
            } else {
                format!("{}/", relative_dir)
            };
            // Direct children only: no additional '/' after the prefix (non-recursive).
            let paths = self
                .data
                .borrow()
                .keys()
                .filter(|k| {
                    k.starts_with(&prefix)
                        && k.ends_with(".json")
                        && !k[prefix.len()..].contains('/')
                })
                .cloned()
                .collect();
            Ok(paths)
        }

        fn load_relations_json(
            &self,
            relative_path: &str,
        ) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        fn save_relations_json(
            &self,
            relative_path: &str,
            value: &serde_json::Value,
        ) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert(relative_path.to_string(), value.clone());
            Ok(())
        }

        fn ensure_relations_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn load_container(
            &self,
            container_id: &str,
        ) -> Result<srs_core::types::container::Container, RepositoryError> {
            let key = format!("containers/{container_id}.json");
            let val = self.data.borrow().get(&key).cloned().ok_or_else(|| {
                RepositoryError::ContainerNotFound {
                    container_id: container_id.to_string(),
                }
            })?;
            serde_json::from_value(val).map_err(|source| RepositoryError::ManifestParse {
                path: std::path::PathBuf::from(&key),
                source,
            })
        }

        fn save_container(
            &self,
            container: &srs_core::types::container::Container,
        ) -> Result<(), RepositoryError> {
            let id = &container.container_id;
            let key = format!("containers/{id}.json");
            let val =
                serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
                    path: std::path::PathBuf::from(&key),
                    source,
                })?;
            self.data.borrow_mut().insert(key, val);
            // Update summary index in manifest
            let mut manifest = self.manifest.borrow_mut();
            let mut entries: Vec<serde_json::Value> = manifest
                .extra
                .get("containerIndex")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            entries.retain(|e| e["containerId"].as_str() != Some(id));
            entries.push(serde_json::json!({ "containerId": id, "title": container.title }));
            manifest
                .extra
                .insert("containerIndex".to_string(), serde_json::json!(entries));
            Ok(())
        }

        fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError> {
            let key = format!("containers/{container_id}.json");
            if self.data.borrow_mut().remove(&key).is_none() {
                return Err(RepositoryError::ContainerNotFound {
                    container_id: container_id.to_string(),
                });
            }
            // Remove from manifest index
            let mut manifest = self.manifest.borrow_mut();
            let mut entries: Vec<serde_json::Value> = manifest
                .extra
                .get("containerIndex")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            entries.retain(|e| e["containerId"].as_str() != Some(container_id));
            manifest
                .extra
                .insert("containerIndex".to_string(), serde_json::json!(entries));
            Ok(())
        }

        fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
            let manifest = self.manifest.borrow();
            let entries: Vec<serde_json::Value> = manifest
                .extra
                .get("containerIndex")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            Ok(entries
                .into_iter()
                .filter_map(|e| {
                    let id = e["containerId"].as_str()?.to_string();
                    let title = e["title"].as_str().unwrap_or("").to_string();
                    Some((id, title))
                })
                .collect())
        }

        #[allow(deprecated)]
        fn load_container_json(
            &self,
            relative_path: &str,
        ) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        #[allow(deprecated)]
        fn save_container_json(
            &self,
            relative_path: &str,
            value: &serde_json::Value,
        ) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert(relative_path.to_string(), value.clone());
            Ok(())
        }

        #[allow(deprecated)]
        fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        #[allow(deprecated)]
        fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn list_files_recursive(&self, relative_dir: &str) -> Vec<String> {
            let prefix = format!("{}/", relative_dir.trim_end_matches('/'));
            self.data
                .borrow()
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect()
        }

        fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .ok_or_else(|| not_found(relative_path))
        }

        fn validate_package_ref_path(&self, _relative_path: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        // --- Package boundaries ---

        fn list_package_boundaries(
            &self,
        ) -> Result<Vec<crate::package_types::PackageBoundary>, RepositoryError> {
            Ok(self.boundaries.borrow().values().cloned().collect())
        }

        fn load_package_boundary(
            &self,
            selector: &PackageSelector,
        ) -> Result<crate::package_types::PackageBoundary, RepositoryError> {
            self.boundaries
                .borrow()
                .get(selector)
                .cloned()
                .ok_or_else(|| RepositoryError::PackageNotFound {
                    selector: selector.clone(),
                })
        }

        fn save_package_boundary_metadata(
            &self,
            boundary: &crate::package_types::PackageBoundary,
        ) -> Result<(), RepositoryError> {
            let mut boundaries = self.boundaries.borrow_mut();
            let entry = boundaries
                .entry(boundary.selector.clone())
                .or_insert_with(|| boundary.clone());
            entry.id = boundary.id.clone();
            entry.namespace = boundary.namespace.clone();
            entry.name = boundary.name.clone();
            entry.version = boundary.version.clone();
            // field_paths and type_paths intentionally not updated — managed by
            // add_definition_to_boundary / remove_definition_from_boundary only.
            Ok(())
        }

        fn register_package_boundary(
            &self,
            selector: &PackageSelector,
        ) -> Result<(), RepositoryError> {
            let path = match selector {
                None => return Ok(()), // primary — no-op
                Some(p) => p.clone(),
            };
            let mut boundaries = self.boundaries.borrow_mut();
            boundaries.entry(selector.clone()).or_insert_with(|| {
                crate::package_types::PackageBoundary {
                    selector: Some(path.clone()),
                    id: String::new(),
                    namespace: String::new(),
                    name: String::new(),
                    version: String::new(),
                    field_paths: vec![],
                    type_paths: vec![],
                }
            });
            drop(boundaries);
            // Seed the sub-package's package.json in data so memory_store_sync_pkg_json
            // can update its arrays and find_view_path can read from it.
            let data_key = format!("{path}/package.json");
            self.data.borrow_mut().entry(data_key).or_insert_with(|| {
                serde_json::json!({
                    "id": "", "namespace": "", "name": "", "version": "",
                    "fields": [], "types": [], "relationTypes": [],
                    "views": [], "documentViews": [], "blueprints": []
                })
            });
            // Also update manifest packageRefs
            let mut manifest = self.manifest.borrow().clone();
            let mut refs: Vec<serde_json::Value> = manifest
                .extra
                .get("packageRefs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let already = refs
                .iter()
                .any(|r| r.get("path").and_then(|p| p.as_str()) == Some(&path));
            if !already {
                refs.push(serde_json::json!({"mode": "local", "path": path}));
                manifest
                    .extra
                    .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
                *self.manifest.borrow_mut() = manifest;
            }
            Ok(())
        }

        fn add_definition_to_boundary(
            &self,
            selector: &PackageSelector,
            kind: crate::package_types::DefinitionKind,
            path: &str,
        ) -> Result<(), RepositoryError> {
            {
                let mut boundaries = self.boundaries.borrow_mut();
                let boundary = boundaries.get_mut(selector).ok_or_else(|| {
                    RepositoryError::PackageNotFound {
                        selector: selector.clone(),
                    }
                })?;
                match kind {
                    crate::package_types::DefinitionKind::Field
                        if !boundary.field_paths.iter().any(|p| p == path) =>
                    {
                        boundary.field_paths.push(path.to_string());
                    }
                    crate::package_types::DefinitionKind::Type
                        if !boundary.type_paths.iter().any(|p| p == path) =>
                    {
                        boundary.type_paths.push(path.to_string());
                    }
                    _ => {} // View/DocumentView/RelationType — no-op in this phase
                }
            }
            // Sync the data["<prefix>/package.json"] so load_package_json stays consistent
            self.memory_store_sync_pkg_json(selector, kind, path, true)
        }

        fn remove_definition_from_boundary(
            &self,
            selector: &PackageSelector,
            kind: crate::package_types::DefinitionKind,
            path: &str,
        ) -> Result<(), RepositoryError> {
            {
                let mut boundaries = self.boundaries.borrow_mut();
                if let Some(boundary) = boundaries.get_mut(selector) {
                    match kind {
                        crate::package_types::DefinitionKind::Field => {
                            boundary.field_paths.retain(|p| p != path);
                        }
                        crate::package_types::DefinitionKind::Type => {
                            boundary.type_paths.retain(|p| p != path);
                        }
                        _ => {}
                    }
                }
            }
            self.memory_store_sync_pkg_json(selector, kind, path, false)
        }

        fn resolve_definition_owner(
            &self,
            id: &str,
            kind: crate::package_types::DefinitionKind,
        ) -> Result<PackageSelector, RepositoryError> {
            use crate::store::definition_kind_key;
            let boundaries = self.boundaries.borrow();
            for (selector, boundary) in boundaries.iter() {
                let prefix = match selector {
                    None => "package".to_string(),
                    Some(p) => p.clone(),
                };
                // For Field/Type use the in-memory boundary paths (fast path).
                // For View/DocumentView/RelationType, read from the boundary's package.json in data.
                match kind {
                    crate::package_types::DefinitionKind::Field => {
                        for rel_path in &boundary.field_paths {
                            let data_key = format!("{prefix}/{rel_path}");
                            if let Some(val) = self.data.borrow().get(&data_key) {
                                if val["id"].as_str() == Some(id) {
                                    return Ok(selector.clone());
                                }
                            }
                        }
                    }
                    crate::package_types::DefinitionKind::Type => {
                        for rel_path in &boundary.type_paths {
                            let data_key = format!("{prefix}/{rel_path}");
                            if let Some(val) = self.data.borrow().get(&data_key) {
                                if val["id"].as_str() == Some(id) {
                                    return Ok(selector.clone());
                                }
                            }
                        }
                    }
                    _ => {
                        // For View, DocumentView, RelationType: scan the boundary's package.json
                        let pkg_key = format!("{prefix}/package.json");
                        let array_key = definition_kind_key(kind);
                        let data = self.data.borrow();
                        if let Some(pkg_json) = data.get(&pkg_key) {
                            if let Some(paths) = pkg_json[array_key].as_array() {
                                for entry in paths {
                                    if let Some(rel) = entry.as_str() {
                                        let def_key = format!("{prefix}/{rel}");
                                        if let Some(val) = data.get(&def_key) {
                                            if val["id"].as_str() == Some(id) {
                                                return Ok(selector.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(RepositoryError::DefinitionNotFound { id: id.to_string() })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::memory::MemoryStore;
    use super::*;
    use tempfile::TempDir;

    fn minimal_manifest(repo_root: &std::path::Path) -> Manifest {
        Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: repo_root.to_path_buf(),
        }
    }

    fn empty_package(repo_root: &std::path::Path) -> Package {
        Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            root: repo_root.to_path_buf(),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn write_minimal_file_repo(temp: &TempDir) {
        let root = temp.path();
        std::fs::create_dir_all(root.join("package")).unwrap();

        let manifest = serde_json::json!({
            "instanceIndex": [],
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo-id",
            "namespace": "com.test"
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let package_json = serde_json::json!({
            "id": "test-pkg",
            "namespace": "com.test",
            "name": "test",
            "version": "1.0.0",
            "fields": [],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&package_json).unwrap(),
        )
        .unwrap();
    }

    // --- FileStore tests ---

    #[test]
    fn file_store_load_manifest_roundtrips() {
        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let manifest = store.load_manifest().unwrap();
        assert!(manifest.instance_index.is_empty());
        assert_eq!(manifest.root, temp.path());
    }

    #[test]
    fn file_store_load_package_returns_package() {
        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let package = store.load_package().unwrap();
        assert_eq!(package.namespace, "com.test");
        assert!(package.fields.is_empty());
    }

    // --- MemoryStore tests ---

    #[test]
    fn memory_store_load_manifest_returns_configured() {
        let root = std::path::PathBuf::from("/fake");
        let manifest = minimal_manifest(&root);
        let store = MemoryStore::new(manifest.clone(), empty_package(&root));
        let loaded = store.load_manifest().unwrap();
        assert_eq!(loaded.instance_index, manifest.instance_index);
    }

    #[test]
    fn memory_store_save_and_load_instance_json() {
        let root = std::path::PathBuf::from("/fake");
        let store = MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let value = serde_json::json!({ "instanceId": "abc-123", "title": "Test" });
        store
            .save_instance_json("notes/test-note.json", &value)
            .unwrap();
        let loaded = store.load_instance_json("notes/test-note.json").unwrap();
        assert_eq!(loaded["instanceId"], "abc-123");
    }

    #[test]
    fn memory_store_delete_instance_removes_key() {
        let root = std::path::PathBuf::from("/fake");
        let store = MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let value = serde_json::json!({ "instanceId": "to-delete" });
        store
            .save_instance_json("notes/to-delete.json", &value)
            .unwrap();
        store.delete_instance_file("notes/to-delete.json").unwrap();
        let result = store.load_instance_json("notes/to-delete.json");
        assert!(result.is_err());
    }

    #[test]
    fn memory_store_list_instance_files_direct_children_only() {
        let root = std::path::PathBuf::from("/fake");
        let store = MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let v = serde_json::json!({});
        store
            .save_instance_json("records/notes/a.json", &v)
            .unwrap();
        store
            .save_instance_json("records/notes/b.json", &v)
            .unwrap();
        // nested — must NOT appear when listing records/notes
        store
            .save_instance_json("records/notes/subdir/c.json", &v)
            .unwrap();
        // sibling directory — must NOT appear
        store
            .save_instance_json("records/other/d.json", &v)
            .unwrap();

        let mut files = store.list_instance_files("records/notes").unwrap();
        files.sort();
        assert_eq!(
            files,
            vec![
                "records/notes/a.json".to_string(),
                "records/notes/b.json".to_string(),
            ]
        );
    }

    // --- Container store tests ---

    fn minimal_container_for_store(id: &str, title: &str) -> srs_core::types::container::Container {
        srs_core::types::container::Container {
            container_id: id.to_string(),
            title: title.to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            root_instance_ids: None,
            member_instance_ids: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn memory_store_container_operations_are_keyed_by_id() {
        let store = MemoryStore::default();
        let container = minimal_container_for_store("c-111", "Sprint 1");

        store.save_container(&container).unwrap();

        // Load back via logical ID
        let loaded = store.load_container("c-111").unwrap();
        assert_eq!(loaded.container_id, "c-111");
        assert_eq!(loaded.title, "Sprint 1");

        // load_instance_json at id-keyed path must succeed (proves storage is id-keyed)
        store
            .load_instance_json("containers/c-111.json")
            .expect("container should be stored at id-keyed path");
    }

    #[test]
    fn memory_store_container_summaries_reflects_saves() {
        let store = MemoryStore::default();
        for i in 1..=3u32 {
            store
                .save_container(&minimal_container_for_store(
                    &format!("cid-{i}"),
                    &format!("Container {i}"),
                ))
                .unwrap();
        }
        let summaries = store.list_container_summaries().unwrap();
        assert_eq!(summaries.len(), 3);
        assert!(summaries.iter().any(|(id, _)| id == "cid-1"));
        assert!(summaries.iter().any(|(id, _)| id == "cid-2"));
        assert!(summaries.iter().any(|(id, _)| id == "cid-3"));
    }

    #[test]
    fn memory_store_delete_container_removes_entry() {
        let store = MemoryStore::default();
        store
            .save_container(&minimal_container_for_store("del-me", "Delete Me"))
            .unwrap();

        store.delete_container("del-me").unwrap();

        let err = store.load_container("del-me").unwrap_err();
        assert!(
            matches!(err, RepositoryError::ContainerNotFound { .. }),
            "should get ContainerNotFound after delete"
        );
        let summaries = store.list_container_summaries().unwrap();
        assert!(
            !summaries.iter().any(|(id, _)| id == "del-me"),
            "summary index should not contain deleted container"
        );
    }

    #[test]
    fn memory_store_delete_container_missing_returns_not_found() {
        let store = MemoryStore::default();
        let err = store.delete_container("nonexistent").unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));
    }

    // --- Package boundary tests ---

    #[test]
    fn memory_store_save_field_uses_package_prefix_key() {
        // This test is load-bearing: it proves MemoryStore stores fields at
        // "package/fields/..." rather than bare "fields/...", which is the key
        // invariant that makes resolve_definition_owner work correctly.
        use crate::package_service::create_field;
        use srs_core::types::field::{Field, ValueType};

        let store = MemoryStore::default();
        let field = Field {
            id: "00000000-0000-0000-0000-aabbccddee01".to_string(),
            namespace: "com.test".to_string(),
            name: "my-field".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: String::new(),
            ai_guidance: serde_json::Value::Null,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        };
        create_field(&store, field).unwrap();

        let data = store.all_data();
        // Must have a key under "package/fields/..."
        let has_package_prefix = data
            .keys()
            .any(|k| k.starts_with("package/fields/") && k.contains("my-field"));
        assert!(
            has_package_prefix,
            "field should be stored at package/fields/... but keys were: {:?}",
            data.keys().collect::<Vec<_>>()
        );
        // Must NOT have a bare "fields/..." key
        let has_bare = data
            .keys()
            .any(|k| k.starts_with("fields/") && k.contains("my-field"));
        assert!(!has_bare, "field should not be stored at bare fields/...");
    }

    #[test]
    fn memory_store_list_package_boundaries_returns_primary() {
        let store = MemoryStore::default();
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 1);
        assert!(
            boundaries[0].selector.is_none(),
            "primary boundary has None selector"
        );
    }

    #[test]
    fn memory_store_register_sub_package_adds_to_boundaries() {
        let store = MemoryStore::default();
        store
            .register_package_boundary(&Some("pkg/ext".to_string()))
            .unwrap();
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 2);
        let has_ext = boundaries
            .iter()
            .any(|b| b.selector == Some("pkg/ext".to_string()));
        assert!(has_ext, "sub-package boundary should be registered");
    }

    #[test]
    fn memory_store_add_definition_to_boundary_updates_paths() {
        use crate::package_types::DefinitionKind;

        let store = MemoryStore::default();
        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/foo.json")
            .unwrap();
        let boundary = store.load_package_boundary(&None).unwrap();
        assert!(
            boundary
                .field_paths
                .contains(&"fields/foo.json".to_string()),
            "field path should appear in primary boundary field_paths"
        );
    }

    #[test]
    fn memory_store_resolve_definition_owner_primary() {
        use crate::package_types::DefinitionKind;
        use srs_core::types::field::{Field, ValueType};

        let store = MemoryStore::default();
        let field_id = "00000000-0000-0000-0000-111111111111";
        let field = Field {
            id: field_id.to_string(),
            namespace: "com.test".to_string(),
            name: "resolve-me".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: String::new(),
            ai_guidance: serde_json::Value::Null,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        };
        // Store field data at the primary package key
        store
            .add_definition_to_boundary(
                &None,
                DefinitionKind::Field,
                "fields/resolve-me-11111111.json",
            )
            .unwrap();
        store
            .save_instance_json(
                "package/fields/resolve-me-11111111.json",
                &serde_json::to_value(&field).unwrap(),
            )
            .unwrap();

        let owner = store
            .resolve_definition_owner(field_id, DefinitionKind::Field)
            .unwrap();
        assert!(owner.is_none(), "primary boundary owner should be None");
    }

    #[test]
    fn memory_store_resolve_definition_owner_sub_package() {
        use crate::package_types::DefinitionKind;
        use srs_core::types::field::{Field, ValueType};

        let store = MemoryStore::default();
        let selector = Some("pkg/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let field_id = "00000000-0000-0000-0000-222222222222";
        let field = Field {
            id: field_id.to_string(),
            namespace: "com.test".to_string(),
            name: "sub-field".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: String::new(),
            ai_guidance: serde_json::Value::Null,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        };
        store
            .add_definition_to_boundary(
                &selector,
                DefinitionKind::Field,
                "fields/sub-field-22222222.json",
            )
            .unwrap();
        store
            .save_instance_json(
                "pkg/ext/fields/sub-field-22222222.json",
                &serde_json::to_value(&field).unwrap(),
            )
            .unwrap();

        let owner = store
            .resolve_definition_owner(field_id, DefinitionKind::Field)
            .unwrap();
        assert_eq!(
            owner, selector,
            "sub-package owner should be Some(\"pkg/ext\")"
        );
    }

    #[test]
    fn file_store_package_boundary_maps_existing_layout() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Set up primary package
        std::fs::create_dir_all(root.join("package/fields")).unwrap();
        let manifest = serde_json::json!({
            "instanceIndex": [],
            "srsVersion": "2.0-draft",
            "repositoryId": "boundary-test",
            "namespace": "com.test",
            "packageRefs": [{"mode": "local", "path": "extensions/myext"}]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let primary_pkg = serde_json::json!({
            "id": "primary-pkg",
            "namespace": "com.test",
            "name": "primary",
            "version": "1.0.0",
            "fields": ["fields/field-aaa.json"],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&primary_pkg).unwrap(),
        )
        .unwrap();

        // Set up sub-package
        std::fs::create_dir_all(root.join("extensions/myext")).unwrap();
        let sub_pkg = serde_json::json!({
            "id": "ext-pkg",
            "namespace": "com.test.ext",
            "name": "myext",
            "version": "0.1.0",
            "fields": [],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            root.join("extensions/myext/package.json"),
            serde_json::to_string_pretty(&sub_pkg).unwrap(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 2, "should have primary + 1 sub-package");

        let primary = boundaries.iter().find(|b| b.selector.is_none()).unwrap();
        assert_eq!(primary.id, "primary-pkg");
        assert_eq!(primary.field_paths, vec!["fields/field-aaa.json"]);

        let ext = boundaries
            .iter()
            .find(|b| b.selector == Some("extensions/myext".to_string()))
            .unwrap();
        assert_eq!(ext.id, "ext-pkg");
    }
}
