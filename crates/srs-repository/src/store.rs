use crate::error::RepositoryError;
use crate::manifest::Manifest;
use crate::package::Package;
use srs_core::types::field::{Field, ValueType};
use srs_core::types::record_type::{FieldAssignment, FieldGroup, RecordType};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::view::{DocumentView, View};
use srs_core::validation::relation_type_definition::validate_relation_type_definition;
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
    fn ensure_fields_dir(&self) -> Result<(), RepositoryError>;

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
    fn ensure_types_dir(&self) -> Result<(), RepositoryError>;

    // --- Views (L1) ---

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_views_dir(&self) -> Result<(), RepositoryError>;

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
    fn ensure_document_views_dir(&self) -> Result<(), RepositoryError>;

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

    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError>;
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError>;

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

    /// Resolve a path relative to the package/ subdirectory.
    fn pkg_abs(&self, relative_path: &str) -> PathBuf {
        self.repo_root.join("package").join(relative_path)
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
) -> Result<(Vec<Field>, Vec<RecordType>, Vec<View>, Vec<DocumentView>), RepositoryError> {
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
        record_types.push(RecordType {
            id: tj.id,
            namespace: tj.namespace,
            name: tj.name,
            version: tj.version,
            description: tj.description.unwrap_or_default(),
            fields: type_fields,
            field_groups,
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
        if let Some((existing, existing_path)) = rt_by_type.get(&def.relation_type) {
            if existing != &def {
                return Err(RepositoryError::RelationTypeDefinitionConflict {
                    relation_type: def.relation_type.clone(),
                    path_a: existing_path.clone(),
                    path_b: full_path,
                });
            }
        } else {
            rt_by_type.insert(def.relation_type.clone(), (def, full_path));
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

    Ok((fields, record_types, views, document_views))
}

impl RepositoryStore for FileStore {
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
        let (mut fields, mut record_types, mut views, mut document_views) =
            load_package_from_dir(&package_dir, &mut rt_by_type)?;

        // Merge sub-packages from manifest packageRefs
        let manifest = self.load_manifest()?;
        if let Some(pkg_refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            let mut field_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut type_sources: HashMap<(String, u32), PathBuf> = HashMap::new();
            let mut view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut doc_view_sources: HashMap<String, PathBuf> = HashMap::new();
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
                let (sub_fields, sub_types, sub_views, sub_doc_views) =
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
            root: self.repo_root.clone(),
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
            path: self.pkg_abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.pkg_abs(relative_path), &value)
    }

    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        self.save_field(relative_path, field)
    }

    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.pkg_abs(relative_path))
    }

    fn ensure_fields_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.pkg_abs("fields"))
    }

    // --- Types ---

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(record_type).map_err(|e| RepositoryError::Serialize {
            path: self.pkg_abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.pkg_abs(relative_path), &value)
    }

    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        self.save_type(relative_path, record_type)
    }

    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.pkg_abs(relative_path))
    }

    fn ensure_types_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.pkg_abs("types"))
    }

    // --- Views (L1) ---

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(view).map_err(|e| RepositoryError::Serialize {
            path: self.pkg_abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.pkg_abs(relative_path), &value)
    }

    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        self.save_view(relative_path, view)
    }

    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.pkg_abs(relative_path))
    }

    fn ensure_views_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.pkg_abs("views"))
    }

    // --- Document Views (L2) ---

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(view).map_err(|e| RepositoryError::Serialize {
            path: self.pkg_abs(relative_path),
            source: e,
        })?;
        self.write_json(&self.pkg_abs(relative_path), &value)
    }

    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        self.save_document_view(relative_path, view)
    }

    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.pkg_abs(relative_path))
    }

    fn ensure_document_views_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.pkg_abs("document-views"))
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

    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(&self.abs(relative_path))
    }

    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(&self.abs(relative_path), value)
    }

    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(&self.abs(relative_path))
    }

    fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir(&self.repo_root.join("containers"))
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
    }

    impl MemoryStore {
        pub fn new(manifest: Manifest, package: Package) -> Self {
            let pkg_json = Self::package_to_json(&package);
            let store = Self {
                manifest: RefCell::new(manifest),
                package: RefCell::new(package),
                data: RefCell::new(HashMap::new()),
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
                root: PathBuf::from("/memory"),
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
            // Update package.json index
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
            // Store the field data file
            let field_val = serde_json::to_value(&field).unwrap();
            store.data.borrow_mut().insert(filename, field_val);
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
            store.data.borrow_mut().insert(filename, type_val);
            store
        }

        fn package_to_json(pkg: &Package) -> serde_json::Value {
            serde_json::json!({
                "id": pkg.id,
                "namespace": pkg.namespace,
                "name": pkg.name,
                "version": pkg.version,
                "fields": [],
                "types": [],
                "views": [],
                "documentViews": []
            })
        }

        /// Pre-populate with a JSON value at the given relative path.
        pub fn with_data(self, path: &str, value: serde_json::Value) -> Self {
            self.data.borrow_mut().insert(path.to_string(), value);
            self
        }

        /// Return a clone of all stored data (for assertions).
        pub fn all_data(&self) -> HashMap<String, serde_json::Value> {
            self.data.borrow().clone()
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

        fn ensure_fields_dir(&self) -> Result<(), RepositoryError> {
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

        fn ensure_types_dir(&self) -> Result<(), RepositoryError> {
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

        fn ensure_views_dir(&self) -> Result<(), RepositoryError> {
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

        fn ensure_document_views_dir(&self) -> Result<(), RepositoryError> {
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
            let paths = self
                .data
                .borrow()
                .keys()
                .filter(|k| k.starts_with(&prefix) && k.ends_with(".json"))
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

        fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

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
            root: repo_root.to_path_buf(),
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
}
