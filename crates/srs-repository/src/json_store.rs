use crate::error::RepositoryError;
use crate::manifest::Manifest;
use crate::package::Package;
use crate::repository_lifecycle::{CreateRepositoryResult, InitializeRepositoryInput};
use crate::store::RepositoryStore;
use serde::de::Error as SerdeDeError;
use srs_core::types::field::{Field, ValueType};
use srs_core::types::lifecycle::Lifecycle;
use srs_core::types::record_type::{
    FieldAssignment, FieldAssignmentOverride, FieldGroup, RecordType, TypeLifecycle,
};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentView, View};
use srs_core::types::vocabulary::Vocabulary;
use srs_core::validation::relation_type_definition::validate_relation_type_definition;
use srs_core::validation::theme::validate_theme;
use srs_core::validation::view::{validate_document_view, validate_view};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonStoreFile {
    srsj: String,
    manifest: serde_json::Value,
    // BTreeMap (not HashMap) so the `.srsj` envelope serialises entries in
    // deterministic, sorted key order — minimal-diff, idempotent writes (ADR-017).
    data: BTreeMap<String, serde_json::Value>,
}

struct JsonStoreState {
    initialized: bool,
    manifest: Manifest,
    // BTreeMap for deterministic `.srsj` serialisation — see JsonStoreFile.data (ADR-017).
    data: BTreeMap<String, serde_json::Value>,
}

pub struct JsonStore {
    file_path: PathBuf,
    state: RefCell<JsonStoreState>,
}

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
    #[allow(dead_code)] // blueprints not yet loaded in JsonStore; see TODO(#223)
    blueprints: Vec<String>,
    #[serde(default)]
    protocols: Vec<String>,
    #[serde(default)]
    vocabularies: Vec<String>,
    #[serde(default)]
    lifecycles: Vec<String>,
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
    composite_renderer: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldAssignmentOverrideJson {
    field_id: String,
    display_label: Option<String>,
    display_hint: Option<String>,
    required: Option<bool>,
}

fn json_store_boundary_from_json(
    pkg_json: &serde_json::Value,
    selector: crate::package_types::PackageSelector,
) -> crate::package_types::PackageBoundary {
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
    crate::package_types::PackageBoundary {
        selector,
        id: pkg_json["id"].as_str().unwrap_or("").to_string(),
        namespace: pkg_json["namespace"].as_str().unwrap_or("").to_string(),
        name: pkg_json["name"].as_str().unwrap_or("").to_string(),
        version: pkg_json["version"].as_str().unwrap_or("").to_string(),
        field_paths,
        type_paths,
    }
}

impl JsonStore {
    pub fn create(file_path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let file_path = file_path.into();
        if file_path.exists() {
            return Err(RepositoryError::RepositoryAlreadyExists {
                path: file_path.clone(),
            });
        }
        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: file_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf(),
        };
        let store = Self {
            file_path: file_path.clone(),
            state: RefCell::new(JsonStoreState {
                initialized: false,
                manifest,
                data: BTreeMap::new(),
            }),
        };
        store.flush()?;
        Ok(store)
    }

    /// Load a repository from a `.srsj` JSON string without touching the filesystem.
    ///
    /// `manifest.root` is set to `"."` — acceptable for read-only use because the `.srsj`
    /// format embeds all package definitions inline and requires no external path resolution.
    pub fn from_srsj(content: &str) -> Result<Self, RepositoryError> {
        let mem_path = PathBuf::from("<memory>");
        let envelope: JsonStoreFile =
            serde_json::from_str(content).map_err(|source| RepositoryError::Serialize {
                path: mem_path.clone(),
                source,
            })?;
        if envelope.srsj != "1" {
            return Err(RepositoryError::InvalidSnapshotData {
                message: format!("unsupported srsj version '{}'", envelope.srsj),
            });
        }
        let mut manifest: Manifest =
            serde_json::from_value(envelope.manifest).map_err(|source| {
                RepositoryError::ManifestParse {
                    path: mem_path.clone(),
                    source,
                }
            })?;
        manifest.root = PathBuf::from(".");
        Ok(Self {
            file_path: mem_path,
            state: RefCell::new(JsonStoreState {
                initialized: true,
                manifest,
                data: envelope.data,
            }),
        })
    }

    pub fn open(file_path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let file_path = file_path.into();
        let raw = std::fs::read_to_string(&file_path).map_err(|source| RepositoryError::Io {
            path: file_path.clone(),
            source,
        })?;
        let mut store = Self::from_srsj(&raw).map_err(|e| match e {
            RepositoryError::Serialize { source, .. } => RepositoryError::Serialize {
                path: file_path.clone(),
                source,
            },
            RepositoryError::ManifestParse { source, .. } => RepositoryError::ManifestParse {
                path: file_path.clone(),
                source,
            },
            other => other,
        })?;
        store.file_path = file_path;
        Ok(store)
    }

    /// Returns the repository's current state as a `.srsj` JSON string.
    /// Pure: no filesystem access. Safe to call from WASM.
    pub fn to_srsj_string(&self) -> Result<String, RepositoryError> {
        let state = self.state.borrow();
        let manifest =
            serde_json::to_value(&state.manifest).map_err(|source| RepositoryError::Serialize {
                path: self.file_path.clone(),
                source,
            })?;
        let envelope = JsonStoreFile {
            srsj: "1".to_string(),
            manifest,
            data: state.data.clone(),
        };
        serde_json::to_string_pretty(&envelope).map_err(|source| RepositoryError::Serialize {
            path: self.file_path.clone(),
            source,
        })
    }

    fn flush(&self) -> Result<(), RepositoryError> {
        // In-memory stores (loaded from a string via `from_srsj`) use the sentinel
        // path "<memory>" and must not attempt file I/O. This is the normal
        // operating mode for the WASM browser binding.
        if self.file_path == std::path::Path::new("<memory>") {
            return Ok(());
        }
        let json = self.to_srsj_string()?;
        std::fs::write(&self.file_path, json).map_err(|source| RepositoryError::Io {
            path: self.file_path.clone(),
            source,
        })
    }

    fn not_found(path: &str) -> RepositoryError {
        RepositoryError::Io {
            path: PathBuf::from(path),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found in JsonStore"),
        }
    }

    fn data_get(&self, path: &str) -> Result<serde_json::Value, RepositoryError> {
        self.state
            .borrow()
            .data
            .get(path)
            .cloned()
            .ok_or_else(|| Self::not_found(path))
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
    fn load_package_from_prefix(
        &self,
        package_prefix: &str,
        rt_by_type: &mut HashMap<String, (RelationTypeDefinition, PathBuf)>,
    ) -> Result<
        (
            PackageMetadata,
            Vec<Field>,
            Vec<RecordType>,
            Vec<View>,
            Vec<DocumentView>,
            Vec<Theme>,
            Vec<crate::package::LoadedProtocol>,
            Vec<Vocabulary>,
            Vec<Lifecycle>,
        ),
        RepositoryError,
    > {
        let package_json_path = format!("{package_prefix}/package.json");
        let package_json = self.data_get(&package_json_path)?;
        let metadata: PackageMetadata = serde_json::from_value(package_json).map_err(|source| {
            RepositoryError::PackageLoad {
                path: PathBuf::from(&package_json_path),
                source,
            }
        })?;

        let mut fields = Vec::new();
        for rel_path in &metadata.fields {
            let full = format!("{package_prefix}/{rel_path}");
            let fj: FieldJson =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            fields.push(Field {
                id: fj.id,
                namespace: fj.namespace,
                name: fj.name,
                version: fj.version,
                value_type: Self::parse_value_type(&fj.value_type, &PathBuf::from(&full))?,
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
        for rel_path in &metadata.types {
            let full = format!("{package_prefix}/{rel_path}");
            let tj: TypeJson = serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                }
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
                        composite_renderer: g.composite_renderer,
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

        for rel_path in &metadata.relation_types {
            let full = format!("{package_prefix}/{rel_path}");
            let def: RelationTypeDefinition = serde_json::from_value(self.data_get(&full)?)
                .map_err(|source| RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                })?;
            validate_relation_type_definition(&def).map_err(|source| {
                RepositoryError::RelationTypeDefinitionValidation {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            if let Some((existing, existing_path)) = rt_by_type.get(&def.key) {
                if existing != &def {
                    return Err(RepositoryError::RelationTypeDefinitionConflict {
                        relation_type: def.key.clone(),
                        path_a: existing_path.clone(),
                        path_b: PathBuf::from(full),
                    });
                }
            } else {
                rt_by_type.insert(def.key.clone(), (def, PathBuf::from(full)));
            }
        }

        let mut views = Vec::new();
        for rel_path in &metadata.views {
            let full = format!("{package_prefix}/{rel_path}");
            let view: View = serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                RepositoryError::ViewLoad {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            validate_view(&view).map_err(|source| RepositoryError::ViewValidation {
                path: PathBuf::from(&full),
                source,
            })?;
            views.push(view);
        }

        let mut document_views = Vec::new();
        for rel_path in &metadata.document_views {
            let full = format!("{package_prefix}/{rel_path}");
            let view: DocumentView =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::DocumentViewLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            validate_document_view(&view).map_err(|source| {
                RepositoryError::DocumentViewValidation {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            document_views.push(view);
        }

        let mut themes = Vec::new();
        for rel_path in &metadata.themes {
            let full = format!("{package_prefix}/{rel_path}");
            let theme: Theme = serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            validate_theme(&theme).map_err(|source| RepositoryError::ThemeValidation {
                path: PathBuf::from(&full),
                source,
            })?;
            themes.push(theme);
        }

        let mut vocabularies = Vec::new();
        for rel_path in &metadata.vocabularies {
            let full = format!("{package_prefix}/{rel_path}");
            let vocab: Vocabulary =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            vocabularies.push(vocab);
        }

        let mut protocols = Vec::new();
        for rel_path in &metadata.protocols {
            let full = format!("{package_prefix}/{rel_path}");
            let raw: serde_json::Value = self.data_get(&full)?;
            let protocol: srs_core::types::protocol::Protocol = serde_json::from_value(raw.clone())
                .map_err(|source| RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                })?;
            protocols.push(crate::package::LoadedProtocol {
                protocol,
                raw,
                source_package: None,
            });
        }

        let mut lifecycles = Vec::new();
        for rel_path in &metadata.lifecycles {
            let full = format!("{package_prefix}/{rel_path}");
            let lc: Lifecycle =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            lifecycles.push(lc);
        }

        Ok((
            metadata,
            fields,
            record_types,
            views,
            document_views,
            themes,
            protocols,
            vocabularies,
            lifecycles,
        ))
    }
}

impl RepositoryStore for JsonStore {
    fn repository_root(&self) -> PathBuf {
        match self.file_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    fn repository_exists(&self) -> Result<bool, RepositoryError> {
        Ok(self.state.borrow().initialized)
    }

    fn initialize_repository(
        &self,
        input: &InitializeRepositoryInput,
    ) -> Result<CreateRepositoryResult, RepositoryError> {
        if self.repository_exists()? {
            return Err(RepositoryError::RepositoryAlreadyExists {
                path: self.file_path.clone(),
            });
        }
        // `extra` is a HashMap (insertion order non-deterministic), but that is safe for
        // `.srsj` determinism: `to_srsj_string` serialises the manifest via
        // `serde_json::to_value`, which normalises these flattened keys into sorted order
        // through serde_json's BTreeMap-backed Map. Only the top-level `data` map is
        // serialised directly, which is why it (and not this) had to become a BTreeMap (ADR-017).
        let mut extra = HashMap::new();
        extra.insert(
            "srsVersion".to_string(),
            serde_json::Value::String(input.repository.srs_version.clone()),
        );
        extra.insert(
            "repositoryId".to_string(),
            serde_json::Value::String(input.repository.repository_id.clone()),
        );
        extra.insert(
            "namespace".to_string(),
            serde_json::Value::String(input.repository.namespace.clone()),
        );
        if let Some(title) = &input.repository.title {
            extra.insert(
                "title".to_string(),
                serde_json::Value::String(title.clone()),
            );
        }
        if let Some(desc) = &input.repository.description {
            extra.insert(
                "description".to_string(),
                serde_json::Value::String(desc.clone()),
            );
        }
        let mut state = self.state.borrow_mut();
        state.manifest = Manifest {
            instance_index: vec![],
            extra,
            root: self.repository_root(),
        };
        state.data.insert(
            "package/package.json".to_string(),
            serde_json::json!({
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
            }),
        );
        state.initialized = true;
        drop(state);
        self.flush()?;
        Ok(CreateRepositoryResult {
            repo_root: self.repository_root(),
            repository_id: input.repository.repository_id.clone(),
            package_id: input.primary_package.id.clone(),
            root_note_id: None,
        })
    }

    fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
        let mut manifest = self.state.borrow().manifest.clone();
        manifest.root = self.repository_root();
        Ok(manifest)
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError> {
        self.state.borrow_mut().manifest = manifest.clone();
        self.flush()
    }

    fn load_package(&self) -> Result<Package, RepositoryError> {
        let manifest = self.load_manifest()?;
        let mut rt_by_type: HashMap<String, (RelationTypeDefinition, PathBuf)> = HashMap::new();
        let (
            root_meta,
            mut fields,
            mut record_types,
            mut views,
            mut document_views,
            mut themes,
            mut protocols,
            mut vocabularies,
            mut lifecycles,
        ) = self.load_package_from_prefix("package", &mut rt_by_type)?;

        if let Some(pkg_refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            let mut field_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut type_sources: HashMap<(String, u32), PathBuf> = HashMap::new();
            let mut view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut doc_view_sources: HashMap<String, PathBuf> = HashMap::new();
            for f in &fields {
                field_sources.insert(f.id.clone(), PathBuf::from("package"));
            }
            for rt in &record_types {
                type_sources.insert((rt.id.clone(), rt.version), PathBuf::from("package"));
            }
            for v in &views {
                view_sources.insert(v.id.clone(), PathBuf::from("package"));
            }
            for dv in &document_views {
                doc_view_sources.insert(dv.id.clone(), PathBuf::from("package"));
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
                let (
                    ..,
                    sub_fields,
                    sub_types,
                    sub_views,
                    sub_doc_views,
                    sub_themes,
                    sub_protocols,
                    sub_vocabs,
                    sub_lcs,
                ) = self.load_package_from_prefix(rel_path, &mut rt_by_type)?;

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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        field_sources.insert(field.id.clone(), PathBuf::from(rel_path));
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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        type_sources.insert(key, PathBuf::from(rel_path));
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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        view_sources.insert(view.id.clone(), PathBuf::from(rel_path));
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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        doc_view_sources.insert(dv.id.clone(), PathBuf::from(rel_path));
                        document_views.push(dv);
                    }
                }
                // Themes: first definition of each id wins (primary package takes precedence
                // over sub-packages). Silent skip matches the bundled-theme lookup model
                // where themes are identified by stable UUID — duplicate IDs in different
                // packages indicate a packaging error, not a semantic override.
                for theme in sub_themes {
                    if !themes.iter().any(|t| t.id == theme.id) {
                        themes.push(theme);
                    }
                }
                for mut lp in sub_protocols {
                    if !protocols
                        .iter()
                        .any(|p| p.protocol.protocol_id == lp.protocol.protocol_id)
                    {
                        lp.source_package = Some(rel_path.to_string());
                        protocols.push(lp);
                    }
                }
                for vocab in sub_vocabs {
                    if !vocabularies.iter().any(|v| v.id == vocab.id) {
                        vocabularies.push(vocab);
                    }
                }
                for lc in sub_lcs {
                    if !lifecycles.iter().any(|l| l.id == lc.id) {
                        lifecycles.push(lc);
                    }
                }
            }
        }

        // Sort by (key, id) so this Vec is deterministic regardless of HashMap
        // iteration order — keeps regenerated package indexes stable across runs.
        let mut relation_type_definitions: Vec<RelationTypeDefinition> =
            rt_by_type.into_values().map(|(def, _)| def).collect();
        relation_type_definitions.sort_by(|a, b| a.key.cmp(&b.key).then(a.id.cmp(&b.id)));

        Ok(Package {
            id: root_meta.id,
            namespace: root_meta.namespace,
            name: root_meta.name,
            version: root_meta.version,
            fields,
            record_types,
            relation_type_definitions,
            views,
            document_views,
            themes,
            blueprints: vec![], // TODO(#223): blueprints not yet loaded in JsonStore
            protocols,
            root: self.repository_root(),
            dependency_refs: vec![],
            vocabularies,
            lifecycles,
        })
    }

    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
        self.data_get("package/package.json")
    }

    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert("package/package.json".to_string(), value.clone());
        self.flush()
    }

    fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(field).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        self.save_field(relative_path, field)
    }

    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_fields_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(record_type).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        self.save_type(relative_path, record_type)
    }

    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_relation_type_definition(
        &self,
        relative_path: &str,
        relation_type: &RelationTypeDefinition,
    ) -> Result<(), RepositoryError> {
        let v =
            serde_json::to_value(relation_type).map_err(|source| RepositoryError::Serialize {
                path: PathBuf::from(relative_path),
                source,
            })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_relation_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(view).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        self.save_view(relative_path, view)
    }

    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(view).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        self.save_document_view(relative_path, view)
    }

    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_document_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_theme(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(theme).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_theme_file(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError> {
        if !self.state.borrow().data.contains_key(relative_path) {
            return Err(RepositoryError::NotFound {
                path: PathBuf::from(relative_path),
            });
        }
        self.save_theme(relative_path, theme)
    }

    fn delete_theme_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_themes_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_blueprint(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(blueprint).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_blueprint_file(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        self.save_blueprint(relative_path, blueprint)
    }

    fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_blueprints_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_vocabulary(
        &self,
        relative_path: &str,
        vocabulary: &srs_core::types::vocabulary::Vocabulary,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(vocabulary).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn ensure_vocabularies_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_lifecycle(
        &self,
        relative_path: &str,
        lifecycle: &srs_core::types::lifecycle::Lifecycle,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(lifecycle).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn ensure_lifecycles_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn load_instance_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.data_get(relative_path)
    }

    fn save_instance_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), value.clone());
        self.flush()
    }

    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_instance_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError> {
        let prefix = if relative_dir.ends_with('/') {
            relative_dir.to_string()
        } else {
            format!("{relative_dir}/")
        };
        let out = self
            .state
            .borrow()
            .data
            .keys()
            .filter(|k| {
                k.starts_with(&prefix) && k.ends_with(".json") && !k[prefix.len()..].contains('/')
            })
            .cloned()
            .collect();
        Ok(out)
    }

    fn load_relations_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.data_get(relative_path)
    }

    fn save_relations_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), value.clone());
        self.flush()
    }

    fn ensure_relations_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn load_container(
        &self,
        container_id: &str,
    ) -> Result<srs_core::types::container::Container, RepositoryError> {
        let key = format!("containers/{container_id}.json");
        let val = self
            .data_get(&key)
            .map_err(|_| RepositoryError::ContainerNotFound {
                container_id: container_id.to_string(),
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
        let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from(&key),
            source,
        })?;
        self.state.borrow_mut().data.insert(key, val);
        // Update index in manifest data
        let mut manifest_val = self
            .data_get("manifest.json")
            .unwrap_or_else(|_| serde_json::json!({}));
        let mut entries: Vec<serde_json::Value> = manifest_val["containerIndex"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        entries.retain(|e| e["containerId"].as_str() != Some(id));
        entries.push(serde_json::json!({ "containerId": id, "title": container.title }));
        if let Some(obj) = manifest_val.as_object_mut() {
            obj.insert("containerIndex".to_string(), serde_json::json!(entries));
        }
        self.state
            .borrow_mut()
            .data
            .insert("manifest.json".to_string(), manifest_val);
        self.flush()
    }

    fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError> {
        let key = format!("containers/{container_id}.json");
        if self.state.borrow_mut().data.remove(&key).is_none() {
            return Err(RepositoryError::ContainerNotFound {
                container_id: container_id.to_string(),
            });
        }
        // Remove from manifest index
        let mut manifest_val = self
            .data_get("manifest.json")
            .unwrap_or_else(|_| serde_json::json!({}));
        let mut entries: Vec<serde_json::Value> = manifest_val["containerIndex"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        entries.retain(|e| e["containerId"].as_str() != Some(container_id));
        if let Some(obj) = manifest_val.as_object_mut() {
            obj.insert("containerIndex".to_string(), serde_json::json!(entries));
        }
        self.state
            .borrow_mut()
            .data
            .insert("manifest.json".to_string(), manifest_val);
        self.flush()
    }

    fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
        let manifest_val = self
            .data_get("manifest.json")
            .unwrap_or_else(|_| serde_json::json!({}));
        let entries: Vec<serde_json::Value> = manifest_val["containerIndex"]
            .as_array()
            .cloned()
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
        self.data_get(relative_path)
    }

    #[allow(deprecated)]
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), value.clone());
        self.flush()
    }

    #[allow(deprecated)]
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    #[allow(deprecated)]
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String> {
        if relative_dir.is_empty() {
            return self.state.borrow().data.keys().cloned().collect();
        }
        let prefix = format!("{}/", relative_dir.trim_end_matches('/'));
        self.state
            .borrow()
            .data
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect()
    }

    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError> {
        if relative_path == "manifest.json" {
            let manifest = self.load_manifest()?;
            return serde_json::to_string_pretty(&manifest).map_err(|source| {
                RepositoryError::Serialize {
                    path: PathBuf::from(relative_path),
                    source,
                }
            });
        }

        self.state
            .borrow()
            .data
            .get(relative_path)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| Self::not_found(relative_path))
    }

    fn validate_package_ref_path(&self, _relative_path: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    // --- Package boundaries ---

    fn list_package_boundaries(
        &self,
    ) -> Result<Vec<crate::package_types::PackageBoundary>, RepositoryError> {
        let mut result = Vec::new();

        // Primary
        let primary_json = self.data_get("package/package.json")?;
        result.push(json_store_boundary_from_json(&primary_json, None));

        // Sub-packages from manifest
        let state = self.state.borrow();
        if let Some(refs) = state
            .manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
        {
            for pkg_ref in refs {
                if pkg_ref.get("mode").and_then(|m| m.as_str()) != Some("local") {
                    continue;
                }
                if let Some(path) = pkg_ref.get("path").and_then(|p| p.as_str()) {
                    let key = format!("{path}/package.json");
                    if let Some(pkg_json) = state.data.get(&key).cloned() {
                        result.push(json_store_boundary_from_json(
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
        selector: &crate::package_types::PackageSelector,
    ) -> Result<crate::package_types::PackageBoundary, RepositoryError> {
        let key = match selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let pkg_json = self
            .data_get(&key)
            .map_err(|_| RepositoryError::PackageNotFound {
                selector: selector.clone(),
            })?;
        Ok(json_store_boundary_from_json(&pkg_json, selector.clone()))
    }

    fn save_package_boundary_metadata(
        &self,
        boundary: &crate::package_types::PackageBoundary,
    ) -> Result<(), RepositoryError> {
        let key = match &boundary.selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let mut pkg_json = self.data_get(&key).unwrap_or_else(|_| {
            serde_json::json!({
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": []
            })
        });
        if let Some(obj) = pkg_json.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!(boundary.id));
            obj.insert(
                "namespace".to_string(),
                serde_json::json!(boundary.namespace),
            );
            obj.insert("name".to_string(), serde_json::json!(boundary.name));
            obj.insert("version".to_string(), serde_json::json!(boundary.version));
        }
        self.state.borrow_mut().data.insert(key, pkg_json);
        self.flush()
    }

    fn register_package_boundary(
        &self,
        selector: &crate::package_types::PackageSelector,
    ) -> Result<(), RepositoryError> {
        let path = match selector {
            None => return Ok(()),
            Some(p) => p.clone(),
        };
        let mut state = self.state.borrow_mut();
        let mut refs: Vec<serde_json::Value> = state
            .manifest
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
            state
                .manifest
                .extra
                .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
        }
        drop(state);
        self.flush()
    }

    fn add_definition_to_boundary(
        &self,
        selector: &crate::package_types::PackageSelector,
        kind: crate::package_types::DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let key = match selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let mut pkg_json = self.data_get(&key)?;
        let array_key = crate::store::definition_kind_key(kind);
        // Auto-initialize missing array keys so older package.json files remain compatible.
        if pkg_json[array_key].is_null() {
            pkg_json[array_key] = serde_json::json!([]);
        }
        let arr =
            pkg_json[array_key]
                .as_array_mut()
                .ok_or_else(|| RepositoryError::PackageLoad {
                    path: PathBuf::from(&key),
                    source: serde_json::Error::custom(format!("{array_key} is not an array")),
                })?;
        if !arr.iter().any(|e| e.as_str() == Some(path)) {
            arr.push(serde_json::json!(path));
        }
        self.state.borrow_mut().data.insert(key, pkg_json);
        self.flush()
    }

    fn remove_definition_from_boundary(
        &self,
        selector: &crate::package_types::PackageSelector,
        kind: crate::package_types::DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let key = match selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let mut pkg_json = self.data_get(&key)?;
        let array_key = crate::store::definition_kind_key(kind);
        if let Some(arr) = pkg_json[array_key].as_array_mut() {
            arr.retain(|e| e.as_str() != Some(path));
        }
        self.state.borrow_mut().data.insert(key, pkg_json);
        self.flush()
    }

    fn resolve_definition_owner(
        &self,
        id: &str,
        kind: crate::package_types::DefinitionKind,
    ) -> Result<crate::package_types::PackageSelector, RepositoryError> {
        let array_key = crate::store::definition_kind_key(kind);
        let boundaries = self.list_package_boundaries()?;
        for boundary in &boundaries {
            let prefix = match &boundary.selector {
                None => "package".to_string(),
                Some(p) => p.clone(),
            };
            let pkg_key = format!("{prefix}/package.json");
            if let Ok(pkg_json) = self.data_get(&pkg_key) {
                if let Some(paths) = pkg_json[array_key].as_array() {
                    for entry in paths {
                        if let Some(rel) = entry.as_str() {
                            let data_key = format!("{prefix}/{rel}");
                            if let Ok(val) = self.data_get(&data_key) {
                                if val["id"].as_str() == Some(id) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_lifecycle::{create_repository, get_repository_status};
    use crate::repository_portability::{copy_repository, export_repository_snapshot};
    use crate::store::memory::MemoryStore;
    use crate::store::FileStore;
    use tempfile::TempDir;

    fn init_input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "json-repo".to_string(),
                namespace: "com.semanticops.json".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg-json".to_string(),
                namespace: "com.semanticops.json".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    use crate::repository_lifecycle::{PrimaryPackageMetadata, RepositoryMetadata};

    #[test]
    fn json_store_create_then_open_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        assert!(!store.repository_exists().unwrap());
        create_repository(&store, &init_input()).unwrap();
        drop(store);
        let reopened = JsonStore::open(&path).unwrap();
        assert!(reopened.repository_exists().unwrap());
        let manifest = reopened.load_manifest().unwrap();
        assert_eq!(
            manifest.extra.get("namespace").and_then(|v| v.as_str()),
            Some("com.semanticops.json")
        );
    }

    #[test]
    fn json_store_create_rejects_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        std::fs::write(&path, "{}").unwrap();
        let result = JsonStore::create(&path);
        assert!(matches!(
            result,
            Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn json_store_open_rejects_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("missing.srsj");
        let result = JsonStore::open(&path);
        assert!(matches!(result, Err(RepositoryError::Io { .. })));
    }

    #[test]
    fn json_store_open_rejects_malformed_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.srsj");
        std::fs::write(&path, "{not-json").unwrap();
        let result = JsonStore::open(&path);
        assert!(matches!(result, Err(RepositoryError::Serialize { .. })));
    }

    #[test]
    fn json_store_initialize_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let second = create_repository(&store, &init_input());
        assert!(matches!(
            second,
            Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn json_store_flush_on_save_instance() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let val = serde_json::json!({"instanceId":"a","sections":[{"name":"b","content":"c"}]});
        store
            .save_instance_json("records/notes/a.json", &val)
            .unwrap();
        drop(store);
        let reopened = JsonStore::open(&path).unwrap();
        assert_eq!(
            reopened.load_instance_json("records/notes/a.json").unwrap(),
            val
        );
    }

    #[test]
    fn json_store_flush_on_delete() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let val = serde_json::json!({"k":"v"});
        store
            .save_instance_json("records/notes/a.json", &val)
            .unwrap();
        store.delete_instance_file("records/notes/a.json").unwrap();
        drop(store);
        let reopened = JsonStore::open(&path).unwrap();
        assert!(matches!(
            reopened.load_instance_json("records/notes/a.json"),
            Err(RepositoryError::Io { .. })
        ));
    }

    #[test]
    fn json_store_manifest_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("x".to_string(), serde_json::Value::String("y".to_string()));
        store.save_manifest(&manifest).unwrap();
        assert_eq!(
            store
                .load_manifest()
                .unwrap()
                .extra
                .get("x")
                .and_then(|v| v.as_str()),
            Some("y")
        );
    }

    #[test]
    fn json_store_package_json_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let package = serde_json::json!({"id":"p","namespace":"n","name":"x","version":"1","fields":[],"types":[],"relationTypes":[],"views":[],"documentViews":[]});
        store.save_package_json(&package).unwrap();
        assert_eq!(store.load_package_json().unwrap(), package);
    }

    #[test]
    fn json_store_list_instance_files_direct_children_only() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let v = serde_json::json!({"a":1});
        store
            .save_instance_json("records/notes/one.json", &v)
            .unwrap();
        store
            .save_instance_json("records/notes/deep/two.json", &v)
            .unwrap();
        let files = store.list_instance_files("records/notes").unwrap();
        assert!(files.contains(&"records/notes/one.json".to_string()));
        assert!(!files.contains(&"records/notes/deep/two.json".to_string()));
    }

    #[test]
    fn json_store_list_files_recursive_returns_all_depths() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let v = serde_json::json!({"a":1});
        store.save_instance_json("records/a.json", &v).unwrap();
        store.save_instance_json("records/b/c.json", &v).unwrap();
        let all = store.list_files_recursive("records");
        assert!(all.contains(&"records/a.json".to_string()));
        assert!(all.contains(&"records/b/c.json".to_string()));
    }

    #[test]
    fn json_store_load_text_file_returns_string_value() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        store
            .save_instance_json(
                "docs/readme.txt",
                &serde_json::Value::String("hello".to_string()),
            )
            .unwrap();
        assert_eq!(store.load_text_file("docs/readme.txt").unwrap(), "hello");
    }

    #[test]
    fn json_store_load_text_file_returns_manifest_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let manifest_text = store.load_text_file("manifest.json").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(manifest["repositoryId"], "json-repo");
    }

    #[test]
    fn json_store_copy_from_memory_store() {
        let source = MemoryStore::uninitialized();
        create_repository(&source, &init_input()).unwrap();
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let target = JsonStore::create(&path).unwrap();
        copy_repository(&source, &target).unwrap();
        let reopened = JsonStore::open(&path).unwrap();
        let snap = export_repository_snapshot(&reopened).unwrap();
        assert_eq!(snap.repository.repository_id, "json-repo");
    }

    #[test]
    fn json_store_copy_to_file_store() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let source = JsonStore::create(&path).unwrap();
        create_repository(&source, &init_input()).unwrap();
        source
            .save_instance_json(
                "records/notes/a.json",
                &serde_json::json!({"instanceId":"a","sections":[{"name":"b","content":"c"}]}),
            )
            .unwrap();

        let out = TempDir::new().unwrap();
        let target = FileStore::new(out.path());
        copy_repository(&source, &target).unwrap();
        assert!(out.path().join("manifest.json").is_file());
        assert!(out.path().join("package/package.json").is_file());
    }

    #[test]
    fn json_store_import_rejects_non_empty_target() {
        let source = MemoryStore::uninitialized();
        create_repository(&source, &init_input()).unwrap();

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let target = JsonStore::create(&path).unwrap();
        create_repository(&target, &init_input()).unwrap();
        let result = copy_repository(&source, &target);
        assert!(matches!(
            result,
            Err(RepositoryError::RepositoryNotEmpty { .. })
                | Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn json_store_repository_status_transitions() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        assert!(!get_repository_status(&store).unwrap().exists);
        create_repository(&store, &init_input()).unwrap();
        assert!(get_repository_status(&store).unwrap().exists);
    }

    // --- Package boundary tests for JsonStore ---

    #[test]
    fn json_store_package_boundaries_roundtrip() {
        use crate::package_service::{create_package, list_packages, CreatePackageInput};
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        // Primary boundary should be present after repo creation.
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 1, "primary boundary should exist");
        assert!(boundaries[0].selector.is_none());

        // Create a sub-package and verify it appears.
        create_package(
            &store,
            CreatePackageInput {
                id: "json-sub-001".to_string(),
                namespace: "com.json".to_string(),
                name: "sub".to_string(),
                version: "1.0.0".to_string(),
                boundary_path: Some("pkg/sub".to_string()),
            },
        )
        .unwrap();

        let packages = list_packages(&store).unwrap();
        assert_eq!(packages.len(), 2);
        assert!(packages.iter().any(|p| p.id == "json-sub-001"));
    }

    #[test]
    fn json_store_add_remove_definition_boundary() {
        use crate::package_types::DefinitionKind;
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/foo.json")
            .unwrap();
        let b = store.load_package_boundary(&None).unwrap();
        assert!(b.field_paths.contains(&"fields/foo.json".to_string()));

        store
            .remove_definition_from_boundary(&None, DefinitionKind::Field, "fields/foo.json")
            .unwrap();
        let b2 = store.load_package_boundary(&None).unwrap();
        assert!(!b2.field_paths.contains(&"fields/foo.json".to_string()));
    }

    #[test]
    fn json_store_save_boundary_metadata_preserves_paths() {
        use crate::package_service::{update_package_metadata, UpdatePackageMetadataInput};
        use crate::package_types::DefinitionKind;
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/keep.json")
            .unwrap();

        update_package_metadata(
            &store,
            None,
            UpdatePackageMetadataInput {
                name: Some("renamed".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let b = store.load_package_boundary(&None).unwrap();
        assert_eq!(b.name, "renamed");
        assert!(
            b.field_paths.contains(&"fields/keep.json".to_string()),
            "field_paths must survive save_package_boundary_metadata"
        );
    }

    #[test]
    fn json_store_resolve_definition_owner_returns_definition_not_found() {
        use crate::error::RepositoryError;
        use crate::package_types::DefinitionKind;
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let err = store
            .resolve_definition_owner("nonexistent-id", DefinitionKind::Field)
            .unwrap_err();
        assert!(
            matches!(err, RepositoryError::DefinitionNotFound { .. }),
            "should return DefinitionNotFound, got: {err:?}"
        );
    }

    // --- Container store tests for JsonStore ---

    #[test]
    fn json_store_container_operations_are_keyed_by_id() {
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let container = Container {
            container_id: "json-c-001".to_string(),
            title: "My Container".to_string(),
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
        };
        store.save_container(&container).unwrap();

        let loaded = store.load_container("json-c-001").unwrap();
        assert_eq!(loaded.container_id, "json-c-001");
        assert_eq!(loaded.title, "My Container");

        let summaries = store.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "json-c-001"));
    }

    #[test]
    fn json_store_container_persists_across_reopen() {
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        {
            let store = JsonStore::create(&path).unwrap();
            create_repository(&store, &init_input()).unwrap();
            store
                .save_container(&Container {
                    container_id: "persist-c".to_string(),
                    title: "Persisted".to_string(),
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
                })
                .unwrap();
        }
        let reopened = JsonStore::open(&path).unwrap();
        let loaded = reopened.load_container("persist-c").unwrap();
        assert_eq!(loaded.title, "Persisted");
        let summaries = reopened.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "persist-c"));
    }

    #[test]
    fn json_store_delete_container_removes_entry() {
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .save_container(&Container {
                container_id: "delete-me".to_string(),
                title: "Delete Me".to_string(),
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
            })
            .unwrap();
        store.delete_container("delete-me").unwrap();

        let err = store.load_container("delete-me").unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));

        let summaries = store.list_container_summaries().unwrap();
        assert!(!summaries.iter().any(|(id, _)| id == "delete-me"));
    }

    #[test]
    fn file_store_container_adapter_preserves_existing_layout() {
        use crate::repository_lifecycle::create_repository;
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let store = FileStore::new(tmp.path());
        create_repository(&store, &init_input()).unwrap();

        let container = Container {
            container_id: "fs-c-001".to_string(),
            title: "File Store Container".to_string(),
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
        };
        store.save_container(&container).unwrap();

        // File must exist under containers/ directory
        assert!(
            tmp.path().join("containers").is_dir(),
            "containers/ directory should exist"
        );
        let json_files: Vec<_> = std::fs::read_dir(tmp.path().join("containers"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        assert_eq!(json_files.len(), 1, "one container file should exist");

        // Load it back
        let loaded = store.load_container("fs-c-001").unwrap();
        assert_eq!(loaded.title, "File Store Container");

        let summaries = store.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "fs-c-001"));
    }

    #[test]
    fn from_str_roundtrip() {
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "mem-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [
                    {"instanceId": "inst-001", "tier": 0, "path": "records/a.json"}
                ],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "records/a.json": {"instanceId": "inst-001", "sections": []}
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).unwrap();
        let manifest = store.load_manifest().unwrap();
        assert_eq!(manifest.instance_index.len(), 1);
        assert_eq!(manifest.instance_index[0].instance_id(), "inst-001");
    }

    #[test]
    fn from_str_bad_version() {
        let srsj = serde_json::json!({
            "srsj": "2",
            "manifest": {},
            "data": {}
        })
        .to_string();

        match JsonStore::from_srsj(&srsj) {
            Err(RepositoryError::InvalidSnapshotData { .. }) => {}
            Err(e) => panic!("expected InvalidSnapshotData, got {:?}", e),
            Ok(_) => panic!("expected error but got Ok"),
        }
    }

    #[test]
    fn open_delegates_to_from_str() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        drop(store);

        let content = std::fs::read_to_string(&path).unwrap();
        let via_open = JsonStore::open(&path).unwrap();
        let via_from_str = JsonStore::from_srsj(&content).unwrap();

        let manifest_open = via_open.load_manifest().unwrap();
        let manifest_str = via_from_str.load_manifest().unwrap();
        assert_eq!(
            manifest_open.instance_index.len(),
            manifest_str.instance_index.len()
        );
    }

    #[test]
    fn to_srsj_string_returns_valid_srsj_envelope() {
        // Build a minimal valid .srsj in-memory and round-trip through to_srsj_string.
        let srsj_content = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "mem-repo-b2",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.b2",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-b2",
                    "namespace": "com.test.b2",
                    "name": "primary",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj_content).unwrap();

        // to_srsj_string must succeed and produce valid JSON with srsj == "1".
        let result = store.to_srsj_string();
        assert!(result.is_ok(), "to_srsj_string returned Err: {:?}", result);

        let serialized = result.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("to_srsj_string output must be valid JSON");
        assert_eq!(
            parsed["srsj"].as_str(),
            Some("1"),
            "srsj key must equal \"1\""
        );
    }

    #[test]
    fn json_store_srsj_write_is_deterministic_and_idempotent() {
        // (1) Source `.srsj` with `data` keys in DELIBERATELY non-sorted order. A raw
        // string literal (not `serde_json::json!`, which would pre-sort via its
        // BTreeMap-backed Map) is required so the disorder actually reaches `from_srsj`.
        // After the BTreeMap change, `to_srsj_string` must emit them in sorted order,
        // identically every time, and idempotently across a read→write round-trip
        // (ADR-017, issue #171).
        let srsj_content = r#"{
            "srsj": "1",
            "manifest": {
                "repositoryId": "det-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.det",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "records/zebra.json": {"instanceId": "z"},
                "records/alpha.json": {"instanceId": "a"},
                "package/package.json": {
                    "id": "pkg-det", "namespace": "com.test.det", "name": "primary",
                    "version": "1.0.0", "fields": [], "types": [], "relationTypes": [],
                    "views": [], "documentViews": []
                },
                "records/mike.json": {"instanceId": "m"}
            }
        }"#;

        let store = JsonStore::from_srsj(srsj_content).unwrap();

        // (2) Two writes of the same store are byte-identical.
        let s1 = store.to_srsj_string().unwrap();
        let s2 = store.to_srsj_string().unwrap();
        assert_eq!(
            s1, s2,
            "two writes of the same store must be byte-identical"
        );

        // (3) write(read(x)) == write(read(write(read(x)))) — idempotent across round-trip.
        let reloaded = JsonStore::from_srsj(&s1).unwrap();
        assert_eq!(
            reloaded.to_srsj_string().unwrap(),
            s1,
            "re-serialising a reloaded store must reproduce the same bytes"
        );

        // (4) Top-level `data` keys are emitted in sorted order — non-vacuous because the
        // source literal above lists them as zebra, alpha, package, mike.
        let parsed: serde_json::Value = serde_json::from_str(&s1).unwrap();
        let keys: Vec<String> = parsed["data"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "data keys must be serialised in sorted order");
        // Guard against a future reader "tidying" the source into sorted order.
        assert_eq!(
            keys,
            vec![
                "package/package.json",
                "records/alpha.json",
                "records/mike.json",
                "records/zebra.json"
            ],
            "expected the four keys in sorted order"
        );
    }

    #[test]
    fn copy_file_to_json_preserves_vocabularies_and_lifecycles() {
        use crate::repository_lifecycle::create_repository;
        use crate::repository_portability::copy_repository;

        let src_tmp = TempDir::new().unwrap();
        let src_store = FileStore::new(src_tmp.path());
        create_repository(&src_store, &init_input()).unwrap();

        // Write a vocabulary and a lifecycle directly as JSON files into the source
        // file-store, then register them in package.json.
        let vocab_json = serde_json::json!({
            "id": "voc-test-01",
            "version": 1,
            "namespace": "com.semanticops.json",
            "name": "test-vocab",
            "mode": "open",
            "terms": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::create_dir_all(src_tmp.path().join("package/vocabularies")).unwrap();
        std::fs::write(
            src_tmp
                .path()
                .join("package/vocabularies/test-vocab-voc-test-0.json"),
            serde_json::to_string_pretty(&vocab_json).unwrap(),
        )
        .unwrap();

        let lc_json = serde_json::json!({
            "id": "lc-test-01",
            "version": 1,
            "namespace": "com.semanticops.json",
            "name": "test-lifecycle",
            "states": [
                {"id": "s1", "key": "draft", "isInitial": true},
                {"id": "s2", "key": "active", "isFinal": true}
            ],
            "transitions": [{"name": "publish", "from": "draft", "to": "active"}],
            "initialState": "draft",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::create_dir_all(src_tmp.path().join("package/lifecycles")).unwrap();
        std::fs::write(
            src_tmp
                .path()
                .join("package/lifecycles/test-lifecycle-lc-test-0.json"),
            serde_json::to_string_pretty(&lc_json).unwrap(),
        )
        .unwrap();

        // Register both in package.json
        let pkg_path = src_tmp.path().join("package/package.json");
        let mut pkg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&pkg_path).unwrap()).unwrap();
        pkg["vocabularies"] = serde_json::json!(["vocabularies/test-vocab-voc-test-0.json"]);
        pkg["lifecycles"] = serde_json::json!(["lifecycles/test-lifecycle-lc-test-0.json"]);
        std::fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

        // Copy file→json
        let dst_tmp = TempDir::new().unwrap();
        let dst_path = dst_tmp.path().join("copy.srsj");
        let dst_store = JsonStore::create(&dst_path).unwrap();
        copy_repository(&src_store, &dst_store).unwrap();
        drop(dst_store);

        // Reopen the .srsj and verify vocabularies and lifecycles survive the round-trip
        let reopened = JsonStore::open(&dst_path).unwrap();
        let pkg = reopened.load_package().unwrap();
        assert_eq!(
            pkg.vocabularies.len(),
            1,
            "expected 1 vocabulary in srsj, got {}",
            pkg.vocabularies.len()
        );
        assert_eq!(pkg.vocabularies[0].name, "test-vocab");
        assert_eq!(
            pkg.lifecycles.len(),
            1,
            "expected 1 lifecycle in srsj, got {}",
            pkg.lifecycles.len()
        );
        assert_eq!(pkg.lifecycles[0].name, "test-lifecycle");
    }
}
