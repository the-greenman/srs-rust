//! # Theme Service
//!
//! Public API for Theme CRUD operations.
//! This module is the sole entry point for all theme logic.
//! CLI handlers and future API handlers must call these functions;
//! they must not call internal helpers or store I/O methods directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes `store: &dyn RepositoryStore` and returns a typed result.
//! - All validation, orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.

use crate::error::RepositoryError;
use crate::package_types::{DefinitionKind, PackageSelector};
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::theme::Theme;
use srs_core::types::view::DocumentView;
use srs_core::validation::theme::validate_theme;

// ── Result enums (read-only) ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum GetThemeResult {
    Found(Box<Theme>),
    NotFound,
}

// ── Summary types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub targets: Vec<String>,
    /// Boundary path of the package that owns this theme.
    /// `None` = primary package (`package/`); `Some(path)` = sub-package path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

// ── Result structs (mutating operations) ─────────────────────────────────────

pub struct CreateThemeResult {
    pub theme: Theme,
}

#[derive(Debug)]
pub struct UpdateThemeResult {
    pub theme: Theme,
}

#[derive(Debug)]
pub struct DeleteThemeResult {
    pub id: String,
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// Locate the package-relative path (e.g. `"themes/foo-abcd1234.json"`) for a Theme by ID.
fn find_theme_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::Theme) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let prefix = owner.as_deref().unwrap_or("package");
    let pkg_json = store.load_instance_json(&format!("{prefix}/package.json"))?;
    let paths = pkg_json["themes"].as_array().cloned().unwrap_or_default();
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

pub fn list_themes(store: &dyn RepositoryStore) -> Result<Vec<Theme>, RepositoryError> {
    let package = store.load_package()?;
    Ok(package.themes().to_vec())
}

pub fn get_theme_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetThemeResult, RepositoryError> {
    let package = store.load_package()?;
    match package.resolve_theme(id) {
        Some(theme) => Ok(GetThemeResult::Found(Box::new(theme.clone()))),
        None => Ok(GetThemeResult::NotFound),
    }
}

pub fn list_themes_summary(
    store: &dyn RepositoryStore,
) -> Result<Vec<ThemeSummary>, RepositoryError> {
    // Build provenance map: theme id -> boundary selector by scanning each boundary's package.json
    let mut provenance: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let boundaries = store.list_package_boundaries()?;
    for boundary in &boundaries {
        let prefix = boundary.selector.as_deref().unwrap_or("package");
        let pkg_json_path = format!("{prefix}/package.json");
        if let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) {
            if let Some(paths) = pkg_json["themes"].as_array() {
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

    Ok(list_themes(store)?
        .into_iter()
        .map(|t| {
            let source_package = provenance.get(&t.id).cloned().flatten();
            ThemeSummary {
                id: t.id,
                namespace: t.namespace,
                name: t.name,
                version: t.version,
                description: t.description,
                targets: t.targets,
                source_package,
            }
        })
        .collect())
}

// ── Theme CRUD ────────────────────────────────────────────────────────────────

/// Create a new Theme. Validates, writes file, registers in the boundary's `package.json` themes array.
/// Pass `selector = None` for the primary package; `Some(path)` for a sub-package.
pub fn create_theme(
    store: &dyn RepositoryStore,
    mut theme: Theme,
    selector: PackageSelector,
) -> Result<CreateThemeResult, RepositoryError> {
    // Validate the boundary exists before touching the filesystem.
    store.load_package_boundary(&selector)?;

    let boundary_path = selector.as_deref().unwrap_or("package");
    validate_theme(&theme).map_err(|e| RepositoryError::ThemeValidation {
        path: std::path::PathBuf::from(format!("{boundary_path}/themes")),
        source: e,
    })?;
    if theme.id.is_empty() {
        theme.id = new_instance_id();
    }
    store.ensure_themes_dir(&format!("{boundary_path}/themes"))?;
    let id_prefix = &theme.id[..theme.id.len().min(8)];
    let rel_filename = format!("themes/{}-{}.json", slugify(&theme.name), id_prefix);
    let full_path = format!("{boundary_path}/{rel_filename}");
    store.save_theme(&full_path, &theme)?;
    store.add_definition_to_boundary(&selector, DefinitionKind::Theme, &rel_filename)?;
    Ok(CreateThemeResult { theme })
}

/// Update an existing Theme (full replace). Validates, locates existing file, overwrites.
pub fn update_theme(
    store: &dyn RepositoryStore,
    theme_id: &str,
    theme: Theme,
) -> Result<UpdateThemeResult, RepositoryError> {
    validate_theme(&theme).map_err(|e| RepositoryError::ThemeValidation {
        path: std::path::PathBuf::from("package/themes"),
        source: e,
    })?;
    let (path, _owner) =
        find_theme_path(store, theme_id)?.ok_or_else(|| RepositoryError::ThemeNotFound {
            theme_id: theme_id.to_string(),
        })?;
    store.update_theme_file(&path, &theme)?;
    Ok(UpdateThemeResult { theme })
}

/// Returns the IDs of any DocumentViews whose `theme_ref.theme_id` references `theme_id`.
fn find_document_views_referencing_theme(
    store: &dyn RepositoryStore,
    theme_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let package = store.load_package()?;
    let refs: Vec<String> = package
        .document_views
        .iter()
        .filter(|dv| references_theme(dv, theme_id))
        .map(|dv| dv.id.clone())
        .collect();
    Ok(refs)
}

fn references_theme(dv: &DocumentView, theme_id: &str) -> bool {
    if let Some(theme_ref) = &dv.theme_ref {
        if theme_ref.theme_id.as_deref() == Some(theme_id) {
            return true;
        }
    }
    if let Some(variants) = &dv.theme_variants {
        for variant in variants {
            if variant.theme_ref.theme_id.as_deref() == Some(theme_id) {
                return true;
            }
        }
    }
    false
}

/// Delete a Theme by ID. Removes the file and unregisters from `package.json` themes array.
/// Returns `CannotDeleteInUse` if any DocumentView references this theme via `themeRef.themeId`.
pub fn delete_theme(
    store: &dyn RepositoryStore,
    theme_id: &str,
) -> Result<DeleteThemeResult, RepositoryError> {
    let refs = find_document_views_referencing_theme(store, theme_id)?;
    if !refs.is_empty() {
        return Err(RepositoryError::CannotDeleteInUse {
            entity_type: "theme".to_string(),
            id: theme_id.to_string(),
            used_by: refs,
        });
    }
    let (full_path, owner) =
        find_theme_path(store, theme_id)?.ok_or_else(|| RepositoryError::ThemeNotFound {
            theme_id: theme_id.to_string(),
        })?;
    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();
    let _ = store.delete_theme_file(&full_path); // best-effort; ignore if already gone
    store.remove_definition_from_boundary(&owner, DefinitionKind::Theme, &rel_path)?;
    Ok(DeleteThemeResult {
        id: theme_id.to_string(),
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{FileStore, RepositoryStore};
    use crate::view_service::{create_document_view, CreateDocumentViewResult};
    use srs_core::types::view::{DocumentSection, SectionSource, ThemeMode, ThemeReference};
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
                "documentViews": [],
                "themes": []
            })
            .to_string(),
        )
        .unwrap();
    }

    fn minimal_theme(name: &str) -> Theme {
        Theme {
            id: String::new(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "test theme".to_string(),
            targets: vec!["markdown".to_string()],
            assets: None,
            css_class_fields: None,
            page_templates: None,
            element_templates: None,
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn minimal_document_view_with_theme(
        name: &str,
        theme_id: &str,
    ) -> srs_core::types::view::DocumentView {
        srs_core::types::view::DocumentView {
            id: String::new(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "test doc view".to_string(),
            container_type: None,
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
            theme_ref: Some(ThemeReference {
                mode: ThemeMode::Bundled,
                path: None,
                url: None,
                theme_id: Some(theme_id.to_string()),
            }),
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn minimal_document_view_no_theme(name: &str) -> srs_core::types::view::DocumentView {
        srs_core::types::view::DocumentView {
            id: String::new(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "test doc view".to_string(),
            container_type: None,
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

    #[test]
    fn create_theme_assigns_id_and_registers_in_package_json() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = create_theme(&store, minimal_theme("my-theme"), None).unwrap();
        assert!(!result.theme.id.is_empty());

        let pkg_json = store.load_package_json().unwrap();
        let themes = pkg_json["themes"].as_array().unwrap();
        assert!(
            themes
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("my-theme")),
            "theme path should be registered in package.json"
        );
    }

    #[test]
    fn create_theme_fails_with_empty_targets() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let mut t = minimal_theme("bad");
        t.targets = vec![];
        assert!(create_theme(&store, t, None).is_err());
    }

    #[test]
    fn list_themes_summary_returns_created_theme() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_theme(&store, minimal_theme("listed-theme"), None).unwrap();
        let summaries = list_themes_summary(&store).unwrap();
        assert!(
            summaries.iter().any(|s| s.id == created.theme.id),
            "created theme should appear in summary list"
        );
    }

    #[test]
    fn get_theme_by_id_finds_created_theme() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_theme(&store, minimal_theme("get-me"), None).unwrap();
        match get_theme_by_id(&store, &created.theme.id).unwrap() {
            GetThemeResult::Found(t) => assert_eq!(t.name, "get-me"),
            GetThemeResult::NotFound => panic!("expected Found"),
        }
    }

    #[test]
    fn get_theme_by_id_not_found_returns_not_found() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        match get_theme_by_id(&store, "00000000-0000-0000-0000-000000000000").unwrap() {
            GetThemeResult::NotFound => {}
            GetThemeResult::Found(_) => panic!("expected NotFound"),
        }
    }

    #[test]
    fn update_theme_overwrites_description() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_theme(&store, minimal_theme("update-me"), None).unwrap();
        let mut updated = created.theme.clone();
        updated.description = "updated description".to_string();

        let result = update_theme(&store, &created.theme.id, updated).unwrap();
        assert_eq!(result.theme.description, "updated description");

        // Verify persisted
        match get_theme_by_id(&store, &created.theme.id).unwrap() {
            GetThemeResult::Found(t) => assert_eq!(t.description, "updated description"),
            GetThemeResult::NotFound => panic!("expected Found after update"),
        }
    }

    #[test]
    fn update_theme_not_found_returns_theme_not_found_error() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = update_theme(
            &store,
            "00000000-0000-0000-0000-000000000000",
            minimal_theme("x"),
        );
        assert!(
            matches!(result, Err(RepositoryError::ThemeNotFound { .. })),
            "expected ThemeNotFound, got {:?}",
            result.err()
        );
    }

    #[test]
    fn delete_theme_removes_from_package_json() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let created = create_theme(&store, minimal_theme("delete-me"), None).unwrap();
        let id = created.theme.id.clone();

        delete_theme(&store, &id).unwrap();

        let pkg_json = store.load_package_json().unwrap();
        let themes = pkg_json["themes"].as_array().unwrap();
        assert!(
            themes
                .iter()
                .all(|v| !v.as_str().unwrap_or("").contains(&id[..8])),
            "theme path should be removed from package.json"
        );
    }

    #[test]
    fn delete_theme_not_found_returns_theme_not_found_error() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let result = delete_theme(&store, "00000000-0000-0000-0000-000000000000");
        assert!(
            matches!(result, Err(RepositoryError::ThemeNotFound { .. })),
            "expected ThemeNotFound"
        );
    }

    #[test]
    fn delete_theme_blocked_when_document_view_references_it() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        let theme = create_theme(&store, minimal_theme("ref-target"), None).unwrap();
        let theme_id = theme.theme.id.clone();

        let dv = minimal_document_view_with_theme("referencing-dv", &theme_id);
        let CreateDocumentViewResult { document_view } =
            create_document_view(&store, dv, None).unwrap();

        let result = delete_theme(&store, &theme_id);
        match result {
            Err(RepositoryError::CannotDeleteInUse {
                entity_type,
                id,
                used_by,
            }) => {
                assert_eq!(entity_type, "theme");
                assert_eq!(id, theme_id);
                assert!(used_by.contains(&document_view.id));
            }
            other => panic!("expected CannotDeleteInUse, got {:?}", other),
        }
    }

    #[test]
    fn delete_theme_succeeds_when_no_document_view_references_it() {
        let temp = tempfile::TempDir::new().unwrap();
        setup_minimal_repo(temp.path());
        let store = FileStore::new(temp.path());

        // A document view with no theme_ref -- should not block deletion
        create_document_view(&store, minimal_document_view_no_theme("unrelated-dv"), None).unwrap();

        let theme = create_theme(&store, minimal_theme("free-theme"), None).unwrap();
        delete_theme(&store, &theme.theme.id).unwrap();
    }
}
