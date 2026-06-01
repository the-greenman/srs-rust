use crate::error::RepositoryError;
use crate::manifest::load_manifest;
use srs_core::types::field::{Field, ValueType};
use srs_core::types::record_type::{FieldAssignment, FieldGroup, RecordType};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentView, View};
use srs_core::validation::relation_type_definition::validate_relation_type_definition;
use srs_core::validation::theme::validate_theme;
use srs_core::validation::view::{validate_document_view, validate_view};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A loaded package containing field definitions, record types, views, and themes.
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
    pub views: Vec<View>,
    pub document_views: Vec<DocumentView>,
    pub themes: Vec<Theme>,
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
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    document_views: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
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
    #[serde(default)]
    field_groups: Option<Vec<FieldGroupJson>>,
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
    #[serde(default)]
    repeatable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_items: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

    /// Resolve a view by its UUID id.
    pub fn resolve_view(&self, id: &str) -> Option<&View> {
        self.views.iter().find(|v| v.id == id)
    }

    /// Resolve a document view by its UUID id.
    pub fn resolve_document_view(&self, id: &str) -> Option<&DocumentView> {
        self.document_views.iter().find(|v| v.id == id)
    }

    /// Resolve a theme by its UUID id.
    pub fn resolve_theme(&self, theme_id: &str) -> Option<&Theme> {
        self.themes.iter().find(|theme| theme.id == theme_id)
    }

    /// Get all themes as a slice.
    pub fn themes(&self) -> &[Theme] {
        &self.themes
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

/// Load raw package content from a directory containing a package.json.
#[allow(clippy::type_complexity)]
fn load_package_from_dir(
    package_dir: &Path,
    rt_by_type: &mut HashMap<String, (RelationTypeDefinition, PathBuf)>,
) -> Result<
    (
        Vec<Field>,
        Vec<RecordType>,
        Vec<View>,
        Vec<DocumentView>,
        Vec<Theme>,
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
                path: full_path.clone(),
                source: e,
            })?;

        fields.push(Field {
            id: field_json.id,
            namespace: field_json.namespace,
            name: field_json.name,
            version: field_json.version,
            value_type: parse_value_type(&field_json.value_type, &full_path)?,
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

        let type_fields: Vec<FieldAssignment> = type_json
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
        let field_groups = type_json.field_groups.map(|groups| {
            groups
                .into_iter()
                .map(|group| FieldGroup {
                    group_id: group.group_id,
                    order: group.order,
                    fields: group
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
                    label: group.label,
                    description: group.description,
                    required: group.required,
                    repeatable: group.repeatable,
                    min_items: group.min_items,
                    max_items: group.max_items,
                })
                .collect()
        });

        record_types.push(RecordType {
            id: type_json.id,
            namespace: type_json.namespace,
            name: type_json.name,
            version: type_json.version,
            description: type_json.description.unwrap_or_default(),
            fields: type_fields,
            field_groups,
            created_at: type_json.created_at.unwrap_or_default(),
            extra: HashMap::new(),
        });
    }

    // Load all relation type definitions, detecting conflicts and coalescing identical defs.
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

    // Load all views (L1)
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

    // Load all document views (L2)
    let mut document_views = Vec::new();
    for document_view_path in &metadata.document_views {
        let full_path = package_dir.join(document_view_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
        let document_view: DocumentView =
            serde_json::from_str(&content).map_err(|source| RepositoryError::DocumentViewLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_document_view(&document_view).map_err(|source| {
            RepositoryError::DocumentViewValidation {
                path: full_path.clone(),
                source,
            }
        })?;
        document_views.push(document_view);
    }

    // Load all themes (ext:themes-l1)
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

    Ok((fields, record_types, views, document_views, themes))
}

/// Load a package from a repository's `package/` directory, merging any sub-packages
/// declared in `manifest.json` `packageRefs`.
///
/// The `repo_root` parameter is the path to the repository root (where the package/ directory is located).
pub fn load_package(repo_root: &Path) -> Result<Package, RepositoryError> {
    let package_dir = repo_root.join("package");
    let package_json_path = package_dir.join("package.json");

    // Read the primary package id/namespace/name/version from its manifest.
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

    // Shared relation-type map for conflict detection across all packages.
    let mut rt_by_type: HashMap<String, (RelationTypeDefinition, PathBuf)> = HashMap::new();

    // Load the primary package.
    let (mut fields, mut record_types, mut views, mut document_views, mut themes) =
        load_package_from_dir(&package_dir, &mut rt_by_type)?;

    // Merge sub-packages declared in manifest.json packageRefs.
    // Manifest load failure is an error — it means the repo is malformed.
    let manifest = load_manifest(repo_root)?;
    if let Some(pkg_refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
        // Track source paths for each collected id so we can report conflicts.
        let mut field_sources: HashMap<String, PathBuf> = HashMap::new();
        let mut type_sources: HashMap<(String, u32), PathBuf> = HashMap::new();
        let mut view_sources: HashMap<String, PathBuf> = HashMap::new();
        let mut doc_view_sources: HashMap<String, PathBuf> = HashMap::new();
        let mut theme_sources: HashMap<String, PathBuf> = HashMap::new();

        // Seed with items already loaded from the primary package.
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

        for pkg_ref in pkg_refs {
            let mode = pkg_ref.get("mode").and_then(|m| m.as_str()).unwrap_or("");
            if mode != "local" {
                continue;
            }
            let rel_path = match pkg_ref.get("path").and_then(|p| p.as_str()) {
                Some(p) => p,
                None => continue,
            };
            // packageRef path is relative to repo_root (e.g. "package/spec-authoring-core")
            let sub_package_dir = repo_root.join(rel_path);
            if !sub_package_dir.join("package.json").exists() {
                return Err(RepositoryError::PackageRefMissing {
                    path: rel_path.to_string(),
                });
            }
            let (sub_fields, sub_types, sub_views, sub_doc_views, sub_themes) =
                load_package_from_dir(&sub_package_dir, &mut rt_by_type)?;

            // Merge fields — conflict if same id but different content
            for field in sub_fields {
                if let Some(first_path) = field_sources.get(&field.id) {
                    // Already present: check for conflict by comparing against existing
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
                            second_path: sub_package_dir.clone(),
                        });
                    }
                    // Identical — coalesce silently
                } else {
                    field_sources.insert(field.id.clone(), sub_package_dir.clone());
                    fields.push(field);
                }
            }
            // Merge record types — conflict if same id+version but different content
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
                            second_path: sub_package_dir.clone(),
                        });
                    }
                } else {
                    type_sources.insert(key, sub_package_dir.clone());
                    record_types.push(rt);
                }
            }
            // Merge views — conflict if same id but different name
            for view in sub_views {
                if let Some(first_path) = view_sources.get(&view.id) {
                    let existing = views.iter().find(|v| v.id == view.id).unwrap();
                    if existing.name != view.name {
                        return Err(RepositoryError::PackageRefConflict {
                            path: rel_path.to_string(),
                            kind: "view".to_string(),
                            id: view.id.clone(),
                            first_path: first_path.clone(),
                            second_path: sub_package_dir.clone(),
                        });
                    }
                } else {
                    view_sources.insert(view.id.clone(), sub_package_dir.clone());
                    views.push(view);
                }
            }
            // Merge document views — conflict if same id but different name
            for dv in sub_doc_views {
                if let Some(first_path) = doc_view_sources.get(&dv.id) {
                    let existing = document_views.iter().find(|d| d.id == dv.id).unwrap();
                    if existing.name != dv.name {
                        return Err(RepositoryError::PackageRefConflict {
                            path: rel_path.to_string(),
                            kind: "document-view".to_string(),
                            id: dv.id.clone(),
                            first_path: first_path.clone(),
                            second_path: sub_package_dir.clone(),
                        });
                    }
                } else {
                    doc_view_sources.insert(dv.id.clone(), sub_package_dir.clone());
                    document_views.push(dv);
                }
            }
            // Merge themes — conflict if same id but different identity fields.
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
                            second_path: sub_package_dir.clone(),
                        });
                    }
                } else {
                    theme_sources.insert(theme.id.clone(), sub_package_dir.clone());
                    themes.push(theme);
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
        root: repo_root.to_path_buf(),
    })
}

fn parse_value_type(s: &str, path: &Path) -> Result<ValueType, RepositoryError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn srs_spec_repo() -> PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut dir = manifest.to_path_buf();
        loop {
            let candidate = dir.join("../srs/srs");
            if let Ok(c) = candidate.canonicalize() {
                if c.join(".srs").exists() {
                    return c;
                }
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        manifest.join("../../../srs/srs")
    }

    #[test]
    fn load_package_from_live_repo() {
        let srs_repo = srs_spec_repo();
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
        let srs_repo = srs_spec_repo();
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
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");

        let status_field = package
            .find_field_by_name("status")
            .expect("should find status field");

        assert_eq!(status_field.name, "status");
        assert_eq!(status_field.namespace, "com.semanticops.srs");
    }

    #[test]
    fn resolve_type_by_name_returns_none_for_unknown() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert!(package
            .resolve_type_by_name("unknown.namespace", "unknown-type")
            .is_none());
    }

    #[test]
    fn resolve_field_returns_none_for_unknown() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert!(package
            .resolve_field("00000000-0000-0000-0000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_loads_relation_types() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");

        assert!(
            package.relation_type_definitions.len() >= 7,
            "expected at least 7 relation types (canonical), got {}",
            package.relation_type_definitions.len()
        );
    }

    #[test]
    fn load_package_loads_document_views() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");
        assert!(
            !package.document_views.is_empty(),
            "expected at least one document view"
        );
    }

    #[test]
    fn resolve_document_view_finds_srs_spec_view() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");
        let view = package
            .resolve_document_view("ec34f54b-8636-5c8b-af5b-c9eb3df24fe6")
            .expect("should find srs spec document view");
        assert_eq!(view.name, "srs-spec-document-view");
    }

    #[test]
    fn resolve_document_view_returns_none_for_unknown() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");
        assert!(package
            .resolve_document_view("00000000-0000-0000-0000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_loads_themes() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let themes_dir = root.join("package/themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(
            themes_dir.join("basic-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000950",
                "namespace": "fixture.theme",
                "name": "basic-theme",
                "version": 1,
                "description": "Basic theme",
                "targets": ["markdown"],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();

        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/basic-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let package = load_package(root).expect("should load themed package");
        assert_eq!(package.themes.len(), 1);
        assert_eq!(package.themes[0].name, "basic-theme");
    }

    #[test]
    fn resolve_theme_finds_known_theme() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        std::fs::create_dir_all(root.join("package/themes")).unwrap();
        std::fs::write(
            root.join("package/themes/basic-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000951",
                "namespace": "fixture.theme",
                "name": "basic-theme",
                "version": 1,
                "description": "Basic theme",
                "targets": ["markdown"],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/basic-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let package = load_package(root).expect("should load themed package");
        let theme = package
            .resolve_theme("00000000-0000-4000-8000-000000000951")
            .expect("should resolve theme by id");
        assert_eq!(theme.name, "basic-theme");
    }

    #[test]
    fn resolve_theme_returns_none_for_unknown() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        std::fs::create_dir_all(root.join("package/themes")).unwrap();
        std::fs::write(
            root.join("package/themes/basic-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000952",
                "namespace": "fixture.theme",
                "name": "basic-theme",
                "version": 1,
                "description": "Basic theme",
                "targets": ["markdown"],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/basic-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let package = load_package(root).expect("should load themed package");
        assert!(package
            .resolve_theme("00000000-0000-4000-8000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_without_themes_key_loads_without_error() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let package = load_package(root).expect("should load package without themes key");
        assert!(package.themes.is_empty());
    }

    #[test]
    fn load_package_theme_validation_fails_on_empty_targets() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        std::fs::create_dir_all(root.join("package/themes")).unwrap();
        std::fs::write(
            root.join("package/themes/invalid-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000953",
                "namespace": "fixture.theme",
                "name": "invalid-theme",
                "version": 1,
                "description": "Invalid theme",
                "targets": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/invalid-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = load_package(root);
        assert!(
            matches!(result, Err(RepositoryError::ThemeValidation { .. })),
            "expected theme validation error, got {result:?}"
        );
    }

    #[test]
    fn resolve_canonical_relation_type_precedes() {
        let srs_repo = srs_spec_repo();
        let package = load_package(&srs_repo).expect("should load live srs package");

        let rt = package
            .resolve_relation_type("precedes")
            .expect("should find canonical 'precedes' relation type");

        assert_eq!(rt.namespace, "com.semanticops.srs");
        assert!(rt.is_active());
        assert!(rt.is_irreflexive());
    }

    /// Write a minimal SRS repo at `root` with a primary package at `root/package/`.
    fn create_minimal_repo(root: &Path) {
        // .srs marker
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        // manifest.json
        let manifest = serde_json::json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo-id",
            "namespace": "com.test",
            "instanceIndex": []
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        // primary package
        let pkg_dir = root.join("package");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        write_package_json(&pkg_dir, "primary-pkg-id", "com.test", "primary", &[], &[]);
    }

    /// Write a package.json for the given dir, listing optional field/type files.
    fn write_package_json(
        dir: &Path,
        id: &str,
        namespace: &str,
        name: &str,
        fields: &[&str],
        types: &[&str],
    ) {
        let pkg = serde_json::json!({
            "id": id,
            "namespace": namespace,
            "name": name,
            "version": "1.0.0",
            "fields": fields,
            "types": types,
            "relationTypes": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string_pretty(&pkg).unwrap(),
        )
        .unwrap();
    }

    fn write_field_json(dir: &Path, file: &str, id: &str, name: &str) {
        let field = serde_json::json!({
            "id": id,
            "namespace": "com.test",
            "name": name,
            "version": 1,
            "valueType": "string"
        });
        std::fs::write(
            dir.join(file),
            serde_json::to_string_pretty(&field).unwrap(),
        )
        .unwrap();
    }

    fn add_package_ref_to_manifest(root: &Path, rel_path: &str) {
        let manifest_path = root.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let refs = manifest
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut refs = refs;
        refs.push(serde_json::json!({"mode": "local", "path": rel_path}));
        manifest["packageRefs"] = serde_json::json!(refs);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_package_errors_on_missing_package_ref() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);
        add_package_ref_to_manifest(root, "package/nonexistent");

        let result = load_package(root);
        assert!(
            matches!(result, Err(RepositoryError::PackageRefMissing { .. })),
            "expected PackageRefMissing, got {result:?}"
        );
    }

    #[test]
    fn load_package_detects_conflicting_field_definitions() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        // Sub-package with a field using the same id as primary but different name.
        let sub_dir = root.join("package").join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        write_field_json(
            &root.join("package"),
            "field-a.json",
            "field-uuid-1",
            "original_name",
        );
        write_package_json(
            &root.join("package"),
            "primary-pkg-id",
            "com.test",
            "primary",
            &["field-a.json"],
            &[],
        );

        write_field_json(
            &sub_dir,
            "field-a-conflict.json",
            "field-uuid-1",
            "different_name",
        );
        write_package_json(
            &sub_dir,
            "sub-pkg-id",
            "com.test",
            "sub",
            &["field-a-conflict.json"],
            &[],
        );
        add_package_ref_to_manifest(root, "package/sub");

        let result = load_package(root);
        assert!(
            matches!(
                result,
                Err(RepositoryError::PackageRefConflict { ref kind, .. }) if kind == "field"
            ),
            "expected PackageRefConflict(field), got {result:?}"
        );
    }

    #[test]
    fn load_package_coalesces_identical_field_definitions() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let sub_dir = root.join("package").join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();

        // Same field in both primary and sub-package.
        write_field_json(
            &root.join("package"),
            "field-a.json",
            "field-uuid-1",
            "shared_field",
        );
        write_package_json(
            &root.join("package"),
            "primary-pkg-id",
            "com.test",
            "primary",
            &["field-a.json"],
            &[],
        );
        write_field_json(&sub_dir, "field-a.json", "field-uuid-1", "shared_field");
        write_package_json(
            &sub_dir,
            "sub-pkg-id",
            "com.test",
            "sub",
            &["field-a.json"],
            &[],
        );
        add_package_ref_to_manifest(root, "package/sub");

        let package = load_package(root).expect("identical fields should coalesce without error");
        // Field should appear exactly once.
        let count = package
            .fields
            .iter()
            .filter(|f| f.id == "field-uuid-1")
            .count();
        assert_eq!(count, 1, "expected exactly one copy of field-uuid-1");
    }

    #[test]
    fn deprecated_relation_types_loaded_with_correct_status() {
        let srs_repo = srs_spec_repo();
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
