//! # View Service
//!
//! Public API for View (L1) and DocumentView (L2) CRUD operations.
//! This module is the sole entry point for all view and document-view logic.
//! CLI handlers and future API handlers must call these functions;
//! they must not call internal helpers or store I/O methods directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes `store: &dyn RepositoryStore` and returns a typed result.
//! - All validation, orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI handler — this is the entire function body
//! let view: View = serde_json::from_reader(io::stdin())?;
//! match with_store(&ctx, |store| Ok(view_service::create_view(store, view)?)) {
//!     Ok(CreateViewResult { view }) => Ok(output::ok("view create", json!({ "view": view }))),
//!     Err(e) => Ok(output::err("view create", vec![e.to_string()])),
//! }
//! ```

use crate::container_service;
use crate::error::RepositoryError;
use crate::package_types::{DefinitionKind, PackageSelector};
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::view::{DocumentView, ExactTypeRef, View};
use srs_core::validation::view::{validate_document_view, validate_view};

// ── Result enums (read-only) ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum GetDocumentViewResult {
    Found(Box<DocumentView>),
    NotFound,
}

#[derive(Debug, Clone)]
pub enum GetViewResult {
    Found(Box<View>),
    NotFound,
}

// ── Summary types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_types: Option<Vec<String>>,
    /// Boundary path of the package that owns this view.
    /// `None` = primary package (`package/`); `Some(path)` = sub-package path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    /// RFC-009: version-exact Type anchors this DocumentView applies to (OR semantics).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_type_refs: Option<Vec<ExactTypeRef>>,
    /// Boundary path of the package that owns this document view.
    /// `None` = primary package (`package/`); `Some(path)` = sub-package path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

/// Filter for [`list_document_views_summary`]. All criteria are AND-combined; a `None`
/// field imposes no constraint. `root_type_id` matches when the view's `root_type_refs`
/// contains an `ExactTypeRef` with that `type_id` (any version).
#[derive(Debug, Clone, Default)]
pub struct DocumentViewListFilter {
    pub namespace: Option<String>,
    pub container_type: Option<String>,
    pub root_type_id: Option<String>,
}

// ── Result structs (mutating operations) ─────────────────────────────────────

pub struct CreateViewResult {
    pub view: View,
}

#[derive(Debug)]
pub struct UpdateViewResult {
    pub view: View,
}

#[derive(Debug)]
pub struct DeleteViewResult {
    pub id: String,
}

pub struct CreateDocumentViewResult {
    pub document_view: DocumentView,
}

pub struct UpdateDocumentViewResult {
    pub document_view: DocumentView,
}

pub struct DeleteDocumentViewResult {
    pub id: String,
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// Locate the package-relative path (e.g. `"views/foo-abcd1234.json"`) for a View by ID.
/// Uses `resolve_definition_owner` to find the boundary, then scans the `package.json` views
/// array and checks each file's `id` field.
fn find_view_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, crate::package_types::PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::View) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    // PackageBoundary only carries field_paths/type_paths, not view_paths.
    // Load the *owner's* package.json (not always the primary one).
    let prefix = owner.as_deref().unwrap_or("package");
    let pkg_json = store.load_instance_json(&format!("{prefix}/package.json"))?;
    let paths = pkg_json["views"].as_array().cloned().unwrap_or_default();
    for entry in &paths {
        if let Some(rel) = entry.as_str() {
            let full = format!("{prefix}/{rel}");
            if let Ok(val) = store.load_instance_json(&full) {
                if val["id"].as_str() == Some(id) {
                    return Ok(Some((full, owner)));
                }
            }
        }
    }
    Ok(None)
}

/// Same as `find_view_path` but scans the `documentViews` array.
fn find_document_view_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, crate::package_types::PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::DocumentView) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    // Load the *owner's* package.json (not always the primary one).
    let prefix = owner.as_deref().unwrap_or("package");
    let pkg_json = store.load_instance_json(&format!("{prefix}/package.json"))?;
    let paths = pkg_json["documentViews"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for entry in &paths {
        if let Some(rel) = entry.as_str() {
            let full = format!("{prefix}/{rel}");
            if let Ok(val) = store.load_instance_json(&full) {
                if val["id"].as_str() == Some(id) {
                    return Ok(Some((full, owner)));
                }
            }
        }
    }
    Ok(None)
}

// ── Read-only service functions ───────────────────────────────────────────────

pub fn list_document_views(
    store: &dyn RepositoryStore,
) -> Result<Vec<DocumentView>, RepositoryError> {
    let package = store.load_package()?;
    Ok(package.document_views)
}

pub fn get_document_view_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetDocumentViewResult, RepositoryError> {
    let package = store.load_package()?;
    match package.resolve_document_view(id) {
        Some(view) => Ok(GetDocumentViewResult::Found(Box::new(view.clone()))),
        None => Ok(GetDocumentViewResult::NotFound),
    }
}

pub fn list_views(store: &dyn RepositoryStore) -> Result<Vec<View>, RepositoryError> {
    let package = store.load_package()?;
    Ok(package.views)
}

pub fn get_view_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetViewResult, RepositoryError> {
    let package = store.load_package()?;
    match package.resolve_view(id) {
        Some(view) => Ok(GetViewResult::Found(Box::new(view.clone()))),
        None => Ok(GetViewResult::NotFound),
    }
}

pub fn list_views_summary(
    store: &dyn RepositoryStore,
) -> Result<Vec<ViewSummary>, RepositoryError> {
    // Build provenance map: view id → boundary selector by scanning each boundary's package.json
    let mut provenance: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let boundaries = store.list_package_boundaries()?;
    for boundary in &boundaries {
        let prefix = boundary.selector.as_deref().unwrap_or("package");
        let pkg_json_path = format!("{prefix}/package.json");
        if let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) {
            if let Some(paths) = pkg_json["views"].as_array() {
                for entry in paths {
                    if let Some(rel) = entry.as_str() {
                        let full = format!("{prefix}/{rel}");
                        if let Ok(val) = store.load_instance_json(&full) {
                            if let Some(id) = val["id"].as_str() {
                                provenance
                                    .entry(id.to_string())
                                    .or_insert_with(|| boundary.selector.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(list_views(store)?
        .into_iter()
        .map(|v| {
            let source_package = provenance.get(&v.id).cloned().flatten();
            ViewSummary {
                id: v.id,
                namespace: v.namespace,
                name: v.name,
                version: v.version,
                description: v.description,
                compatible_types: v.compatible_types,
                source_package,
            }
        })
        .collect())
}

pub fn list_document_views_summary(
    store: &dyn RepositoryStore,
    filter: &DocumentViewListFilter,
) -> Result<Vec<DocumentViewSummary>, RepositoryError> {
    // Build provenance map: document view id → boundary selector
    let mut provenance: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let boundaries = store.list_package_boundaries()?;
    for boundary in &boundaries {
        let prefix = boundary.selector.as_deref().unwrap_or("package");
        let pkg_json_path = format!("{prefix}/package.json");
        if let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) {
            if let Some(paths) = pkg_json["documentViews"].as_array() {
                for entry in paths {
                    if let Some(rel) = entry.as_str() {
                        let full = format!("{prefix}/{rel}");
                        if let Ok(val) = store.load_instance_json(&full) {
                            if let Some(id) = val["id"].as_str() {
                                provenance
                                    .entry(id.to_string())
                                    .or_insert_with(|| boundary.selector.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(list_document_views(store)?
        .into_iter()
        .filter(|v| {
            // namespace: exact match
            if let Some(ns) = &filter.namespace {
                if &v.namespace != ns {
                    return false;
                }
            }
            // container_type: exact match against the (optional) hint
            if let Some(ct) = &filter.container_type {
                if v.container_type.as_deref() != Some(ct.as_str()) {
                    return false;
                }
            }
            // root_type_id: at least one ExactTypeRef with this type_id (any version)
            if let Some(rt) = &filter.root_type_id {
                let matches = v
                    .root_type_refs
                    .as_ref()
                    .is_some_and(|refs| refs.iter().any(|r| &r.type_id == rt));
                if !matches {
                    return false;
                }
            }
            true
        })
        .map(|v| {
            let source_package = provenance.get(&v.id).cloned().flatten();
            DocumentViewSummary {
                id: v.id,
                namespace: v.namespace,
                name: v.name,
                version: v.version,
                description: v.description,
                container_type: v.container_type,
                root_type_refs: v.root_type_refs,
                source_package,
            }
        })
        .collect())
}

/// Given a container ID, resolves its first root instance's `typeId`/`typeVersion`,
/// then returns all DocumentViews whose `rootTypeRefs` contains an `ExactTypeRef`
/// matching that exact type binding (both `typeId` and `typeVersion` must match).
///
/// Returns an empty vec — not an error — when:
/// - the container has no `rootInstanceIds`
/// - the root instance has no `typeId` (Tier 0 Note or Tier 1 TypedRecord)
/// - no DocumentViews match the type binding
///
/// Returns `RepositoryError` when:
/// - the container is not found
/// - the root instance is not found in the manifest index
pub fn document_views_for_container(
    store: &dyn RepositoryStore,
    container_id: &str,
) -> Result<Vec<DocumentView>, RepositoryError> {
    let container = container_service::get_container(store, container_id)?;

    // Get the first root instance ID; if none, no DocumentView can match.
    let root_id = match container
        .root_instance_ids
        .as_ref()
        .and_then(|ids| ids.first())
    {
        Some(id) => id.clone(),
        None => return Ok(vec![]),
    };

    // Find the instance path in the manifest index.
    let manifest = store.load_manifest()?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == root_id)
        .ok_or_else(|| RepositoryError::InstanceLoad {
            instance_id: root_id.clone(),
            path: std::path::PathBuf::from(&root_id),
            source: Box::from(format!(
                "root instance '{root_id}' in container '{container_id}' not found in manifest index"
            )),
        })?;

    // Load the raw JSON to extract typeId / typeVersion without committing to a tier type.
    let instance_json = store.load_instance_json(entry.path())?;
    let type_id = match instance_json
        .get("typeId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        Some(id) => id,
        None => return Ok(vec![]), // Tier 0 or Tier 1 — no type binding
    };
    let type_version = match instance_json.get("typeVersion").and_then(|v| v.as_u64()) {
        Some(v) => v as u32,
        None => return Ok(vec![]), // typeId present but typeVersion missing — not a valid Tier 2
    };

    // Filter all DocumentViews to those that reference this exact type binding.
    let all_views = list_document_views(store)?;
    let matched = all_views
        .into_iter()
        .filter(|dv| {
            dv.root_type_refs.as_ref().is_some_and(|refs| {
                refs.iter()
                    .any(|r| r.type_id == type_id && r.type_version == type_version)
            })
        })
        .collect();

    Ok(matched)
}

// ── View CRUD ─────────────────────────────────────────────────────────────────

/// Create a new View. Validates, writes file, registers in the boundary's `package.json` views array.
/// Pass `selector = None` for the primary package; `Some(path)` for a sub-package.
pub fn create_view(
    store: &dyn RepositoryStore,
    mut view: View,
    selector: PackageSelector,
) -> Result<CreateViewResult, RepositoryError> {
    // Validate the boundary exists before touching the filesystem.
    store.load_package_boundary(&selector)?;

    let boundary_path = selector.as_deref().unwrap_or("package");
    validate_view(&view).map_err(|e| RepositoryError::ViewValidation {
        path: std::path::PathBuf::from(format!("{boundary_path}/views")),
        source: e,
    })?;
    if view.id.is_empty() {
        view.id = new_instance_id();
    }
    store.ensure_views_dir(&format!("{boundary_path}/views"))?;
    let id_prefix = &view.id[..view.id.len().min(8)];
    let rel_filename = format!("views/{}-{}.json", slugify(&view.name), id_prefix);
    let full_path = format!("{boundary_path}/{rel_filename}");
    store.save_view(&full_path, &view)?;
    store.add_definition_to_boundary(&selector, DefinitionKind::View, &rel_filename)?;
    Ok(CreateViewResult { view })
}

/// Update an existing View (full replace). Validates, locates existing file, overwrites.
pub fn update_view(
    store: &dyn RepositoryStore,
    view_id: &str,
    view: View,
) -> Result<UpdateViewResult, RepositoryError> {
    validate_view(&view).map_err(|e| RepositoryError::ViewValidation {
        path: std::path::PathBuf::from("package/views"),
        source: e,
    })?;
    let (path, _owner) =
        find_view_path(store, view_id)?.ok_or_else(|| RepositoryError::ViewNotFound {
            view_id: view_id.to_string(),
        })?;
    store.update_view_file(&path, &view)?;
    Ok(UpdateViewResult { view })
}

/// Returns the IDs of any DocumentViews whose sections reference `view_id` via `render_view_id`.
fn find_document_views_referencing_view(
    store: &dyn RepositoryStore,
    view_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let refs: Vec<String> = list_document_views(store)?
        .into_iter()
        .filter(|dv| {
            dv.sections
                .iter()
                .any(|s| s.render_view_id.as_deref() == Some(view_id))
        })
        .map(|dv| dv.id)
        .collect();
    Ok(refs)
}

/// Delete a View by ID. Removes the file and unregisters from `package.json` views array.
/// Returns `CannotDeleteInUse` if any DocumentView section references this view.
pub fn delete_view(
    store: &dyn RepositoryStore,
    view_id: &str,
) -> Result<DeleteViewResult, RepositoryError> {
    let refs = find_document_views_referencing_view(store, view_id)?;
    if !refs.is_empty() {
        return Err(RepositoryError::CannotDeleteInUse {
            entity_type: "view".to_string(),
            id: view_id.to_string(),
            used_by: refs,
        });
    }
    let (full_path, owner) =
        find_view_path(store, view_id)?.ok_or_else(|| RepositoryError::ViewNotFound {
            view_id: view_id.to_string(),
        })?;
    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();
    let _ = store.delete_view_file(&full_path); // best-effort; ignore if already gone
    store.remove_definition_from_boundary(&owner, DefinitionKind::View, &rel_path)?;
    Ok(DeleteViewResult {
        id: view_id.to_string(),
    })
}

// ── DocumentView CRUD ─────────────────────────────────────────────────────────

/// Create a new DocumentView. Validates, writes file, registers in the boundary's `package.json` documentViews array.
/// Pass `selector = None` for the primary package; `Some(path)` for a sub-package.
pub fn create_document_view(
    store: &dyn RepositoryStore,
    mut document_view: DocumentView,
    selector: PackageSelector,
) -> Result<CreateDocumentViewResult, RepositoryError> {
    // Validate the boundary exists before touching the filesystem.
    store.load_package_boundary(&selector)?;

    let boundary_path = selector.as_deref().unwrap_or("package");
    validate_document_view(&document_view).map_err(|e| {
        RepositoryError::DocumentViewValidation {
            path: std::path::PathBuf::from(format!("{boundary_path}/document-views")),
            source: e,
        }
    })?;
    if document_view.id.is_empty() {
        document_view.id = new_instance_id();
    }
    store.ensure_document_views_dir(&format!("{boundary_path}/document-views"))?;
    let id_prefix = &document_view.id[..document_view.id.len().min(8)];
    let rel_filename = format!(
        "document-views/{}-{}.json",
        slugify(&document_view.name),
        id_prefix
    );
    let full_path = format!("{boundary_path}/{rel_filename}");
    store.save_document_view(&full_path, &document_view)?;
    store.add_definition_to_boundary(&selector, DefinitionKind::DocumentView, &rel_filename)?;
    Ok(CreateDocumentViewResult { document_view })
}

/// Update an existing DocumentView (full replace). Validates, locates existing file, overwrites.
/// The `id` field of the written file is always set to `document_view_id` — the caller's JSON may
/// omit or leave it blank; the positional argument is authoritative.
pub fn update_document_view(
    store: &dyn RepositoryStore,
    document_view_id: &str,
    mut document_view: DocumentView,
) -> Result<UpdateDocumentViewResult, RepositoryError> {
    document_view.id = document_view_id.to_string();
    validate_document_view(&document_view).map_err(|e| {
        RepositoryError::DocumentViewValidation {
            path: std::path::PathBuf::from("package/document-views"),
            source: e,
        }
    })?;
    let (path, _owner) = find_document_view_path(store, document_view_id)?.ok_or_else(|| {
        RepositoryError::DocumentViewNotFoundById {
            document_view_id: document_view_id.to_string(),
        }
    })?;
    store.update_document_view_file(&path, &document_view)?;
    Ok(UpdateDocumentViewResult { document_view })
}

/// Delete a DocumentView by ID. Removes the file and unregisters from `package.json`.
pub fn delete_document_view(
    store: &dyn RepositoryStore,
    document_view_id: &str,
) -> Result<DeleteDocumentViewResult, RepositoryError> {
    let (full_path, owner) =
        find_document_view_path(store, document_view_id)?.ok_or_else(|| {
            RepositoryError::DocumentViewNotFoundById {
                document_view_id: document_view_id.to_string(),
            }
        })?;
    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();
    let _ = store.delete_document_view_file(&full_path); // best-effort
    store.remove_definition_from_boundary(&owner, DefinitionKind::DocumentView, &rel_path)?;
    Ok(DeleteDocumentViewResult {
        id: document_view_id.to_string(),
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use crate::store::{FileStore, RepositoryStore};
    use srs_core::types::view::{DocumentSection, FieldView, SectionSource};
    use std::collections::HashMap;

    fn setup_minimal_repo(root: &std::path::Path) {
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        std::fs::write(root.join("manifest.json"), r#"{"instanceIndex":[]}"#).unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "pkg",
                "namespace": "com.test",
                "name": "test",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": []
            })
            .to_string(),
        )
        .unwrap();
    }

    fn minimal_view(name: &str) -> View {
        View {
            id: String::new(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "test view".to_string(),
            field_views: vec![FieldView {
                field_id: "f1".to_string(),
                order: 0,
                required: None,
                visible: None,
                display_label: None,
            }],
            compatible_types: None,
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn minimal_document_view(name: &str) -> DocumentView {
        DocumentView {
            id: String::new(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "test doc view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                section_id: "s1".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                render_view_id: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
            }],
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    // ── View tests ────────────────────────────────────────────────────────────

    #[test]
    fn create_view_assigns_id_and_registers_in_package_json() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = create_view(&store, minimal_view("my-view"), None).unwrap();
        assert!(!result.view.id.is_empty());

        let pkg_json = store.load_package_json().unwrap();
        let views = pkg_json["views"].as_array().unwrap();
        assert!(
            views
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("my-view")),
            "view path should be registered in package.json"
        );
    }

    #[test]
    fn create_view_fails_with_empty_field_views() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let mut v = minimal_view("bad");
        v.field_views = vec![];
        assert!(create_view(&store, v, None).is_err());
    }

    #[test]
    fn list_views_summary_returns_created_view() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_view(&store, minimal_view("listed-view"), None).unwrap();
        let summaries = list_views_summary(&store).unwrap();
        assert!(
            summaries.iter().any(|s| s.id == created.view.id),
            "created view should appear in summary list"
        );
    }

    #[test]
    fn get_view_by_id_finds_created_view() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_view(&store, minimal_view("get-me"), None).unwrap();
        match get_view_by_id(&store, &created.view.id).unwrap() {
            GetViewResult::Found(v) => assert_eq!(v.name, "get-me"),
            GetViewResult::NotFound => panic!("expected Found"),
        }
    }

    #[test]
    fn get_view_by_id_not_found_returns_not_found() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        match get_view_by_id(&store, "00000000-0000-0000-0000-000000000000").unwrap() {
            GetViewResult::NotFound => {}
            GetViewResult::Found(_) => panic!("expected NotFound"),
        }
    }

    #[test]
    fn update_view_overwrites_description() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_view(&store, minimal_view("update-me"), None).unwrap();
        let mut updated = created.view.clone();
        updated.description = "updated description".to_string();

        let result = update_view(&store, &created.view.id, updated).unwrap();
        assert_eq!(result.view.description, "updated description");

        // Verify persisted
        match get_view_by_id(&store, &created.view.id).unwrap() {
            GetViewResult::Found(v) => assert_eq!(v.description, "updated description"),
            GetViewResult::NotFound => panic!("expected Found after update"),
        }
    }

    #[test]
    fn update_view_not_found_returns_view_not_found_error() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = update_view(
            &store,
            "00000000-0000-0000-0000-000000000000",
            minimal_view("x"),
        );
        assert!(
            matches!(result, Err(RepositoryError::ViewNotFound { .. })),
            "expected ViewNotFound, got {:?}",
            result.err()
        );
    }

    #[test]
    fn delete_view_removes_from_package_json() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_view(&store, minimal_view("delete-me"), None).unwrap();
        let id = created.view.id.clone();

        delete_view(&store, &id).unwrap();

        let pkg_json = store.load_package_json().unwrap();
        let views = pkg_json["views"].as_array().unwrap();
        assert!(
            views
                .iter()
                .all(|v| !v.as_str().unwrap_or("").contains(&id[..8])),
            "view path should be removed from package.json"
        );
    }

    #[test]
    fn delete_view_not_found_returns_view_not_found_error() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = delete_view(&store, "00000000-0000-0000-0000-000000000000");
        assert!(
            matches!(result, Err(RepositoryError::ViewNotFound { .. })),
            "expected ViewNotFound"
        );
    }

    #[test]
    fn delete_view_blocked_when_document_view_references_it() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let view = create_view(&store, minimal_view("ref-target"), None).unwrap();
        let view_id = view.view.id.clone();

        let mut dv = minimal_document_view("referencing-dv");
        dv.sections[0].render_view_id = Some(view_id.clone());
        let dv_result = create_document_view(&store, dv, None).unwrap();

        let result = delete_view(&store, &view_id);
        match result {
            Err(RepositoryError::CannotDeleteInUse {
                entity_type,
                id,
                used_by,
            }) => {
                assert_eq!(entity_type, "view");
                assert_eq!(id, view_id);
                assert!(used_by.contains(&dv_result.document_view.id));
            }
            other => panic!("expected CannotDeleteInUse, got {:?}", other),
        }
    }

    #[test]
    fn delete_view_succeeds_when_no_document_view_references_it() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        // A document view with no render_view_id — should not block deletion
        create_document_view(&store, minimal_document_view("unrelated-dv"), None).unwrap();

        let view = create_view(&store, minimal_view("free-view"), None).unwrap();
        delete_view(&store, &view.view.id).unwrap();
    }

    // ── DocumentView tests ────────────────────────────────────────────────────

    #[test]
    fn create_document_view_assigns_id_and_registers_in_package_json() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result =
            create_document_view(&store, minimal_document_view("my-doc-view"), None).unwrap();
        assert!(!result.document_view.id.is_empty());

        let pkg_json = store.load_package_json().unwrap();
        let dviews = pkg_json["documentViews"].as_array().unwrap();
        assert!(
            dviews
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("my-doc-view")),
            "document view path should be registered in package.json"
        );
    }

    #[test]
    fn create_document_view_fails_with_empty_sections() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let mut dv = minimal_document_view("bad");
        dv.sections = vec![];
        assert!(create_document_view(&store, dv, None).is_err());
    }

    #[test]
    fn list_document_views_summary_returns_created() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created =
            create_document_view(&store, minimal_document_view("listed-dv"), None).unwrap();
        let summaries =
            list_document_views_summary(&store, &DocumentViewListFilter::default()).unwrap();
        assert!(summaries.iter().any(|s| s.id == created.document_view.id));
    }

    #[test]
    fn list_document_views_summary_filters_by_root_type() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let type_id = "00000000-0000-4000-8000-00000000aaaa";
        let mut anchored = minimal_document_view("anchored-dv");
        anchored.root_type_refs = Some(vec![ExactTypeRef {
            type_id: type_id.to_string(),
            type_version: 1,
        }]);
        let anchored = create_document_view(&store, anchored, None).unwrap();
        // A second view with no rootTypeRefs — must be excluded by the filter.
        create_document_view(&store, minimal_document_view("unanchored-dv"), None).unwrap();

        // Matching root_type_id returns only the anchored view; its summary carries rootTypeRefs.
        let filter = DocumentViewListFilter {
            root_type_id: Some(type_id.to_string()),
            ..Default::default()
        };
        let matched = list_document_views_summary(&store, &filter).unwrap();
        assert_eq!(matched.len(), 1, "expected exactly the anchored view");
        assert_eq!(matched[0].id, anchored.document_view.id);
        assert_eq!(
            matched[0].root_type_refs.as_ref().unwrap()[0].type_id,
            type_id
        );

        // A non-matching type id returns nothing.
        let none_filter = DocumentViewListFilter {
            root_type_id: Some("00000000-0000-4000-8000-00000000ffff".to_string()),
            ..Default::default()
        };
        assert!(list_document_views_summary(&store, &none_filter)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn get_document_view_by_id_finds_created() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created =
            create_document_view(&store, minimal_document_view("get-me-dv"), None).unwrap();
        match get_document_view_by_id(&store, &created.document_view.id).unwrap() {
            GetDocumentViewResult::Found(dv) => assert_eq!(dv.name, "get-me-dv"),
            GetDocumentViewResult::NotFound => panic!("expected Found"),
        }
    }

    #[test]
    fn get_document_view_by_id_not_found_returns_not_found() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        match get_document_view_by_id(&store, "00000000-0000-0000-0000-000000000000").unwrap() {
            GetDocumentViewResult::NotFound => {}
            GetDocumentViewResult::Found(_) => panic!("expected NotFound"),
        }
    }

    #[test]
    fn update_document_view_overwrites_description() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created =
            create_document_view(&store, minimal_document_view("update-dv"), None).unwrap();
        let mut updated = created.document_view.clone();
        updated.description = "updated dv description".to_string();

        let result = update_document_view(&store, &created.document_view.id, updated).unwrap();
        assert_eq!(result.document_view.description, "updated dv description");
    }

    #[test]
    fn update_document_view_preserves_id_when_input_id_is_empty() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created =
            create_document_view(&store, minimal_document_view("id-preserve-dv"), None).unwrap();
        let authoritative_id = created.document_view.id.clone();

        // Simulate caller sending update JSON with id stripped out
        let mut updated = created.document_view.clone();
        updated.id = String::new();

        let result = update_document_view(&store, &authoritative_id, updated).unwrap();
        assert_eq!(
            result.document_view.id, authoritative_id,
            "update must restore the id from the positional argument even when input id is empty"
        );

        // Confirm the file on disk has the correct id
        let fetched = get_document_view_by_id(&store, &authoritative_id).unwrap();
        assert!(
            matches!(fetched, GetDocumentViewResult::Found(dv) if dv.id == authoritative_id),
            "persisted file must have the authoritative id"
        );
    }

    #[test]
    fn update_document_view_not_found_returns_error() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = update_document_view(
            &store,
            "00000000-0000-0000-0000-000000000000",
            minimal_document_view("x"),
        );
        assert!(
            matches!(
                result,
                Err(RepositoryError::DocumentViewNotFoundById { .. })
            ),
            "expected DocumentViewNotFoundById"
        );
    }

    #[test]
    fn delete_document_view_removes_from_package_json() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created =
            create_document_view(&store, minimal_document_view("delete-dv"), None).unwrap();
        let id = created.document_view.id.clone();

        delete_document_view(&store, &id).unwrap();

        let pkg_json = store.load_package_json().unwrap();
        let dviews = pkg_json["documentViews"].as_array().unwrap();
        assert!(dviews
            .iter()
            .all(|v| !v.as_str().unwrap_or("").contains(&id[..8])));
    }

    #[test]
    fn delete_document_view_not_found_returns_error() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = delete_document_view(&store, "00000000-0000-0000-0000-000000000000");
        assert!(matches!(
            result,
            Err(RepositoryError::DocumentViewNotFoundById { .. })
        ));
    }

    // ── Phase B: sub-package selector + provenance tests ─────────────────────

    #[test]
    fn create_view_in_sub_package() {
        let store = MemoryStore::default();
        let selector = Some("package/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let result = create_view(&store, minimal_view("sub-view"), selector.clone()).unwrap();
        assert!(!result.view.id.is_empty());

        // File should be stored at sub-package path
        let data = store.all_data();
        let has_sub_path = data
            .keys()
            .any(|k| k.starts_with("package/ext/views/") && k.contains("sub-view"));
        assert!(
            has_sub_path,
            "view should be stored under package/ext/views/; keys: {:?}",
            data.keys().collect::<Vec<_>>()
        );

        // Primary boundary should NOT have it
        let primary = store.load_package_boundary(&None).unwrap();
        assert!(
            primary.field_paths.is_empty(),
            "primary boundary should be unaffected"
        );
    }

    #[test]
    fn create_document_view_in_sub_package() {
        let store = MemoryStore::default();
        let selector = Some("package/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let result =
            create_document_view(&store, minimal_document_view("sub-dv"), selector.clone())
                .unwrap();
        assert!(!result.document_view.id.is_empty());

        let data = store.all_data();
        let has_sub_path = data
            .keys()
            .any(|k| k.starts_with("package/ext/document-views/") && k.contains("sub-dv"));
        assert!(
            has_sub_path,
            "document view should be stored under package/ext/document-views/; keys: {:?}",
            data.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn list_views_includes_source_package() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        create_view(&store, minimal_view("primary-view"), None).unwrap();

        let summaries = list_views_summary(&store).unwrap();
        assert!(
            summaries.iter().any(|s| s.source_package.is_none()),
            "primary package views should have source_package = None"
        );
    }

    #[test]
    fn list_document_views_includes_source_package() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        create_document_view(&store, minimal_document_view("primary-dv"), None).unwrap();

        let summaries =
            list_document_views_summary(&store, &DocumentViewListFilter::default()).unwrap();
        assert!(
            summaries.iter().any(|s| s.source_package.is_none()),
            "primary package document views should have source_package = None"
        );
    }

    // ── Regression: find_view_path must read owner's package.json, not primary ─

    #[test]
    fn update_view_in_sub_package_finds_correct_file() {
        // Regression for find_view_path reading only primary package/package.json.
        // A view in a sub-package must be findable for update/delete.
        let store = MemoryStore::default();
        let selector = Some("package/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let created = create_view(&store, minimal_view("sub-view"), selector).unwrap();
        let mut updated = created.view.clone();
        updated.description = "updated description".to_string();

        let result = update_view(&store, &created.view.id, updated);
        assert!(
            result.is_ok(),
            "update_view should find view in sub-package: {:?}",
            result
        );
    }

    #[test]
    fn delete_view_in_sub_package_finds_correct_file() {
        let store = MemoryStore::default();
        let selector = Some("package/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let created = create_view(&store, minimal_view("del-sub-view"), selector).unwrap();
        let result = delete_view(&store, &created.view.id);
        assert!(
            result.is_ok(),
            "delete_view should find view in sub-package: {:?}",
            result
        );
    }

    // ── document_views_for_container tests ────────────────────────────────────

    /// Build a MemoryStore containing:
    /// - one DocumentView with `rootTypeRefs` pointing at the given `type_id`/`type_version`
    /// - one Tier-2 instance JSON at `instance_path` with the given `type_id`/`type_version`
    /// - one instance index entry pointing at that path
    fn make_store_with_dv_and_instance(
        type_id: &str,
        type_version: u32,
        instance_id: &str,
        instance_path: &str,
    ) -> MemoryStore {
        use crate::index::InstanceIndexEntry;
        use crate::manifest::Manifest;
        use crate::package::Package;
        use std::path::PathBuf;

        let dv = DocumentView {
            id: "dv-test-id".to_string(),
            namespace: "com.test".to_string(),
            name: "test-dv".to_string(),
            version: 1,
            description: "test document view".to_string(),
            container_type: None,
            root_type_refs: Some(vec![ExactTypeRef {
                type_id: type_id.to_string(),
                type_version,
            }]),
            sections: vec![srs_core::types::view::DocumentSection {
                section_id: "s1".to_string(),
                title: None,
                description: None,
                order: 0,
                source: srs_core::types::view::SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                render_view_id: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
            }],
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: instance_id.to_string(),
                tier: 2,
                path: instance_path.to_string(),
                title: None,
                tags: None,
            }],
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
            document_views: vec![dv],
            themes: vec![],
            blueprints: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        let instance_json = serde_json::json!({
            "instanceId": instance_id,
            "tier": 2,
            "typeId": type_id,
            "typeVersion": type_version,
            "fieldValues": {}
        });

        MemoryStore::new(manifest, package).with_data(instance_path, instance_json)
    }

    #[test]
    fn document_views_for_container_root_present_returns_matching_view() {
        use crate::container_service;
        use srs_core::types::container::Container;

        let type_id = "00000000-0000-4000-8000-00000000aaaa";
        let type_version = 1u32;
        let instance_id = "11111111-1111-4111-8111-111111111111";
        let container_id = "550e8400-e29b-41d4-a716-446655440000";

        let store = make_store_with_dv_and_instance(
            type_id,
            type_version,
            instance_id,
            "records/inst.json",
        );

        // Create a container with the instance as root
        let container = Container {
            container_id: container_id.to_string(),
            title: "Test Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: Some(vec![instance_id.to_string()]),
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        };
        container_service::create_container(&store, container).unwrap();

        let result = document_views_for_container(&store, container_id).unwrap();
        assert_eq!(
            result.len(),
            1,
            "expected exactly one matching DocumentView"
        );
        assert_eq!(result[0].id, "dv-test-id");
        assert_eq!(
            result[0].root_type_refs.as_ref().unwrap()[0].type_id,
            type_id
        );
    }

    #[test]
    fn document_views_for_container_no_root_returns_empty() {
        use crate::container_service;
        use srs_core::types::container::Container;

        let type_id = "00000000-0000-4000-8000-00000000aaaa";
        let container_id = "550e8400-e29b-41d4-a716-446655440000";
        let instance_id = "11111111-1111-4111-8111-111111111111";

        let store = make_store_with_dv_and_instance(type_id, 1, instance_id, "records/inst.json");

        // Container with no rootInstanceIds
        let container = Container {
            container_id: container_id.to_string(),
            title: "No-Root Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        };
        container_service::create_container(&store, container).unwrap();

        let result = document_views_for_container(&store, container_id).unwrap();
        assert!(
            result.is_empty(),
            "expected empty vec when container has no rootInstanceIds"
        );
    }

    #[test]
    fn document_views_for_container_untyped_root_returns_empty() {
        use crate::container_service;
        use crate::index::InstanceIndexEntry;
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::container::Container;
        use std::path::PathBuf;

        let type_id = "00000000-0000-4000-8000-00000000aaaa";
        let instance_id = "11111111-1111-4111-8111-111111111111";
        let container_id = "550e8400-e29b-41d4-a716-446655440000";

        // DocumentView expects a typed instance, but instance is Tier 0 (Note — no typeId)
        let dv = DocumentView {
            id: "dv-test-id".to_string(),
            namespace: "com.test".to_string(),
            name: "test-dv".to_string(),
            version: 1,
            description: "test document view".to_string(),
            container_type: None,
            root_type_refs: Some(vec![ExactTypeRef {
                type_id: type_id.to_string(),
                type_version: 1,
            }]),
            sections: vec![srs_core::types::view::DocumentSection {
                section_id: "s1".to_string(),
                title: None,
                description: None,
                order: 0,
                source: srs_core::types::view::SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                render_view_id: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
            }],
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: instance_id.to_string(),
                tier: 0,
                path: "records/note.json".to_string(),
                title: None,
                tags: None,
            }],
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
            document_views: vec![dv],
            themes: vec![],
            blueprints: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        // Tier-0 Note JSON: no typeId or typeVersion
        let note_json = serde_json::json!({
            "instanceId": instance_id,
            "tier": 0,
            "title": "A note"
        });

        let store = MemoryStore::new(manifest, package).with_data("records/note.json", note_json);

        let container = Container {
            container_id: container_id.to_string(),
            title: "Note Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: Some(vec![instance_id.to_string()]),
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        };
        container_service::create_container(&store, container).unwrap();

        let result = document_views_for_container(&store, container_id).unwrap();
        assert!(
            result.is_empty(),
            "expected empty vec when root instance has no typeId"
        );
    }

    #[test]
    fn document_views_for_container_not_found_returns_error() {
        let store = MemoryStore::default();
        let err = document_views_for_container(&store, "missing-container").unwrap_err();
        assert!(
            matches!(err, crate::error::RepositoryError::ContainerNotFound { .. }),
            "expected ContainerNotFound, got {:?}",
            err
        );
    }

    #[test]
    fn document_views_for_container_type_version_mismatch_returns_empty() {
        use crate::container_service;
        use srs_core::types::container::Container;

        let type_id = "00000000-0000-4000-8000-00000000aaaa";
        let instance_id = "11111111-1111-4111-8111-111111111111";
        let container_id = "550e8400-e29b-41d4-a716-446655440000";

        // Instance has typeVersion=1, DocumentView requires typeVersion=2
        let store = make_store_with_dv_and_instance(
            type_id,
            2, // DV expects v2
            instance_id,
            "records/inst.json",
        );

        // Overwrite the instance JSON so it has typeVersion=1 instead
        let store = store.with_data(
            "records/inst.json",
            serde_json::json!({
                "instanceId": instance_id,
                "tier": 2,
                "typeId": type_id,
                "typeVersion": 1,  // mismatched
                "fieldValues": {}
            }),
        );

        let container = Container {
            container_id: container_id.to_string(),
            title: "Mismatched Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: Some(vec![instance_id.to_string()]),
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        };
        container_service::create_container(&store, container).unwrap();

        let result = document_views_for_container(&store, container_id).unwrap();
        assert!(
            result.is_empty(),
            "expected empty vec when typeVersion does not match"
        );
    }
}
