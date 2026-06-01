//! # Blueprint Service
//!
//! Public API for Blueprint definition CRUD operations. This module is the sole entry point
//! for all blueprint logic. CLI handlers must call these functions; they must not call
//! internal helpers or store I/O methods directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes `store: &dyn RepositoryStore` and returns a typed result.
//! - All validation, orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Atomicity notes
//!
//! - **Create**: blueprint file is written first, then `package.json` is updated.
//!   If the `package.json` update fails, the orphaned file is left on disk and an error is
//!   returned including the orphaned path. Repair by deleting the file or re-running create.
//!
//! - **Delete**: `package.json` is updated first (entry removed), then the file is deleted.
//!   If file deletion fails after index removal, the entry is gone but the file remains as an
//!   orphan. The error includes the orphaned path.

use crate::error::RepositoryError;
use crate::package_types::{DefinitionKind, PackageSelector};
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::blueprint::Blueprint;
use srs_core::validation::blueprint::validate_blueprint;

// ── Selector validation ───────────────────────────────────────────────────────

/// Validate that a sub-package selector is safe to use as a path prefix.
///
/// Rules:
/// - `None` (primary package) is always valid.
/// - Must start with `"package/"`.
/// - Must not contain `".."` path components.
/// - Must not start with `"/"` (absolute path).
pub fn validate_package_selector(selector: &PackageSelector) -> Result<(), RepositoryError> {
    let Some(path) = selector.as_deref() else {
        return Ok(());
    };
    if path.starts_with('/') {
        return Err(RepositoryError::InvalidPackageSelector {
            message: format!("selector '{path}' must not be an absolute path"),
        });
    }
    if path.split('/').any(|c| c == "..") {
        return Err(RepositoryError::InvalidPackageSelector {
            message: format!("selector '{path}' must not contain '..' components"),
        });
    }
    if !path.starts_with("package/") && path != "package" {
        return Err(RepositoryError::InvalidPackageSelector {
            message: format!("selector '{path}' must start with 'package/'"),
        });
    }
    Ok(())
}

// ── Summary types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub root_type_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

// ── List result (carries provenance diagnostics) ─────────────────────────────

#[derive(Debug, Clone)]
pub struct BlueprintListResult {
    pub summaries: Vec<BlueprintSummary>,
    /// WARN-level diagnostics from provenance scan (missing files, duplicate IDs).
    pub diagnostics: Vec<String>,
}

// ── Mutating result types ─────────────────────────────────────────────────────

pub struct CreateBlueprintResult {
    pub blueprint: Blueprint,
}

#[derive(Debug, Clone)]
pub enum GetBlueprintResult {
    Found(Box<Blueprint>),
    NotFound,
}

pub struct UpdateBlueprintResult {
    pub blueprint: Blueprint,
}

pub struct DeleteBlueprintResult {
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct ValidateBlueprintResult {
    pub id: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// Locate the full repo-root-relative path for a blueprint by ID.
fn find_blueprint_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    let owner = match store.resolve_definition_owner(id, DefinitionKind::Blueprint) {
        Ok(sel) => sel,
        Err(RepositoryError::DefinitionNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let prefix = owner.as_deref().unwrap_or("package");
    let pkg_json = store.load_instance_json(&format!("{prefix}/package.json"))?;
    let paths = pkg_json["blueprints"]
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

fn load_blueprint_from_value(value: &serde_json::Value) -> Option<Blueprint> {
    serde_json::from_value(value.clone()).ok()
}

// ── Read-only service functions ───────────────────────────────────────────────

/// List blueprint summaries across all package boundaries.
///
/// Scans each boundary's `package.json` `blueprints[]` array directly (no `load_package` call).
/// Emits WARN diagnostics for missing files and duplicate IDs (first boundary wins).
pub fn list_blueprints_summary(
    store: &dyn RepositoryStore,
) -> Result<BlueprintListResult, RepositoryError> {
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut diagnostics: Vec<String> = vec![];
    let mut summaries: Vec<BlueprintSummary> = vec![];

    let boundaries = store.list_package_boundaries()?;

    for boundary in &boundaries {
        let prefix = boundary.selector.as_deref().unwrap_or("package");
        let pkg_json_path = format!("{prefix}/package.json");
        let Ok(pkg_json) = store.load_instance_json(&pkg_json_path) else {
            continue;
        };
        let Some(paths) = pkg_json["blueprints"].as_array() else {
            continue;
        };
        for entry in paths.clone() {
            let Some(rel) = entry.as_str() else { continue };
            let full = format!("{prefix}/{rel}");
            match store.load_instance_json(&full) {
                Ok(val) => {
                    if let Some(bp) = load_blueprint_from_value(&val) {
                        if seen_ids.contains(&bp.id) {
                            diagnostics.push(format!(
                                "[WARN] duplicate blueprint id '{}' found in '{prefix}'; first boundary wins",
                                bp.id
                            ));
                        } else {
                            seen_ids.insert(bp.id.clone());
                            summaries.push(BlueprintSummary {
                                root_type_count: bp.root_types.len(),
                                source_package: boundary.selector.clone(),
                                id: bp.id,
                                namespace: bp.namespace,
                                name: bp.name,
                                version: bp.version,
                                description: bp.description,
                            });
                        }
                    }
                }
                Err(_) => {
                    diagnostics.push(format!(
                        "[WARN] blueprint file '{full}' is indexed in {prefix}/package.json but missing on disk"
                    ));
                }
            }
        }
    }

    Ok(BlueprintListResult {
        summaries,
        diagnostics,
    })
}

/// Get a blueprint by its definition ID.
pub fn get_blueprint_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetBlueprintResult, RepositoryError> {
    match find_blueprint_path(store, id)? {
        Some((path, _owner)) => {
            let val = store.load_instance_json(&path)?;
            match load_blueprint_from_value(&val) {
                Some(bp) => Ok(GetBlueprintResult::Found(Box::new(bp))),
                None => Ok(GetBlueprintResult::NotFound),
            }
        }
        None => Ok(GetBlueprintResult::NotFound),
    }
}

/// Return the `structure` (RelationSpec list) for a blueprint, sorted deterministically.
///
/// Sort order: `(source_type_id, target_type_id, relation_type)` ascending.
pub fn list_blueprint_structure(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Vec<srs_core::types::blueprint::RelationSpec>, RepositoryError> {
    match get_blueprint_by_id(store, id)? {
        GetBlueprintResult::Found(bp) => {
            let mut structure = bp.structure;
            structure.sort_by(|a, b| {
                a.source_type
                    .type_id
                    .cmp(&b.source_type.type_id)
                    .then(a.target_type.type_id.cmp(&b.target_type.type_id))
                    .then(a.relation_type.cmp(&b.relation_type))
            });
            Ok(structure)
        }
        GetBlueprintResult::NotFound => Err(RepositoryError::BlueprintNotFound {
            blueprint_id: id.to_string(),
        }),
    }
}

// ── Mutating service functions ────────────────────────────────────────────────

/// Create a new Blueprint definition.
///
/// Validates the selector, validates the blueprint, assigns an ID if empty,
/// writes the file, then registers it in the boundary's `package.json`.
pub fn create_blueprint(
    store: &dyn RepositoryStore,
    mut blueprint: Blueprint,
    selector: PackageSelector,
) -> Result<CreateBlueprintResult, RepositoryError> {
    validate_package_selector(&selector)?;
    store.load_package_boundary(&selector)?;

    // Assign ID before validation so the ID-not-empty check passes.
    if blueprint.id.is_empty() {
        blueprint.id = new_instance_id();
    }

    let validation = validate_blueprint(&blueprint);
    if !validation.valid {
        let messages: Vec<String> = validation
            .diagnostics
            .into_iter()
            .map(|d| d.message)
            .collect();
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: messages.join("; "),
        });
    }

    let boundary_path = selector.as_deref().unwrap_or("package");
    store.ensure_blueprints_dir(&format!("{boundary_path}/blueprints"))?;

    let id_prefix = &blueprint.id[..blueprint.id.len().min(8)];
    let rel_filename = format!("blueprints/{}-{}.json", slugify(&blueprint.name), id_prefix);
    let full_path = format!("{boundary_path}/{rel_filename}");

    // Write file first (atomicity: file before index).
    store.save_blueprint(&full_path, &blueprint)?;

    // Then register in the boundary's package.json.
    if let Err(e) =
        store.add_definition_to_boundary(&selector, DefinitionKind::Blueprint, &rel_filename)
    {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "blueprint file written to '{full_path}' but package.json update failed: {e}; \
                 repair by deleting the orphaned file or re-running create"
            ),
        });
    }

    Ok(CreateBlueprintResult { blueprint })
}

/// Update an existing Blueprint definition (full replace).
///
/// Preserves the original `created_at` from the stored value — ignores any `created_at`
/// in the incoming blueprint.
pub fn update_blueprint(
    store: &dyn RepositoryStore,
    blueprint_id: &str,
    mut blueprint: Blueprint,
) -> Result<UpdateBlueprintResult, RepositoryError> {
    let (path, _owner) = find_blueprint_path(store, blueprint_id)?.ok_or_else(|| {
        RepositoryError::BlueprintNotFound {
            blueprint_id: blueprint_id.to_string(),
        }
    })?;

    // Preserve the original created_at.
    let stored_value = store.load_instance_json(&path)?;
    if let Some(original_created_at) = stored_value["createdAt"].as_str() {
        blueprint.created_at = original_created_at.to_string();
    }

    let validation = validate_blueprint(&blueprint);
    if !validation.valid {
        let messages: Vec<String> = validation
            .diagnostics
            .into_iter()
            .map(|d| d.message)
            .collect();
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: messages.join("; "),
        });
    }

    store.update_blueprint_file(&path, &blueprint)?;
    Ok(UpdateBlueprintResult { blueprint })
}

/// Delete a Blueprint by ID.
///
/// Removes the entry from `package.json` first, then deletes the file.
/// If file deletion fails, an error is returned including the orphaned path.
pub fn delete_blueprint(
    store: &dyn RepositoryStore,
    blueprint_id: &str,
) -> Result<DeleteBlueprintResult, RepositoryError> {
    let (full_path, owner) = find_blueprint_path(store, blueprint_id)?.ok_or_else(|| {
        RepositoryError::BlueprintNotFound {
            blueprint_id: blueprint_id.to_string(),
        }
    })?;

    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();

    // Remove from package.json first (atomicity: index before file).
    store.remove_definition_from_boundary(&owner, DefinitionKind::Blueprint, &rel_path)?;

    // Then delete the file.
    if let Err(e) = store.delete_blueprint_file(&full_path) {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "[WARN] removed '{full_path}' from package index but file deletion failed: {e}; \
                 orphaned file may remain at '{full_path}'"
            ),
        });
    }

    Ok(DeleteBlueprintResult {
        id: blueprint_id.to_string(),
    })
}

/// Validate a Blueprint definition by ID.
pub fn validate_blueprint_by_id(
    store: &dyn RepositoryStore,
    blueprint_id: &str,
) -> Result<ValidateBlueprintResult, RepositoryError> {
    match get_blueprint_by_id(store, blueprint_id)? {
        GetBlueprintResult::Found(bp) => {
            let result = validate_blueprint(&bp);
            let diagnostics: Vec<String> = result
                .diagnostics
                .into_iter()
                .map(|d| {
                    let severity = match d.severity {
                        srs_core::types::blueprint::BlueprintDiagnosticSeverity::Error => "ERROR",
                        srs_core::types::blueprint::BlueprintDiagnosticSeverity::Warning => {
                            "WARNING"
                        }
                    };
                    format!("[{severity}] {}", d.message)
                })
                .collect();
            Ok(ValidateBlueprintResult {
                id: blueprint_id.to_string(),
                valid: result.valid,
                diagnostics,
            })
        }
        GetBlueprintResult::NotFound => Err(RepositoryError::BlueprintNotFound {
            blueprint_id: blueprint_id.to_string(),
        }),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use srs_core::types::blueprint::{Blueprint, RelationSpec, TypeRef};

    fn minimal_blueprint(name: &str) -> Blueprint {
        Blueprint {
            id: String::new(),
            namespace: "test".to_string(),
            name: name.to_string(),
            version: 1,
            description: "test blueprint".to_string(),
            root_types: vec![TypeRef {
                type_id: "core/decision".to_string(),
                type_version: None,
            }],
            structure: vec![],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    #[test]
    fn validate_package_selector_rejects_absolute_path() {
        let result = validate_package_selector(&Some("/abs/path".to_string()));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("absolute path"),
            "expected 'absolute path' in: {msg}"
        );
    }

    #[test]
    fn validate_package_selector_rejects_path_traversal() {
        let result = validate_package_selector(&Some("package/../evil".to_string()));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains(".."), "expected '..' in: {msg}");
    }

    #[test]
    fn validate_package_selector_rejects_outside_package() {
        let result = validate_package_selector(&Some("other/sub".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn validate_package_selector_accepts_primary() {
        assert!(validate_package_selector(&None).is_ok());
    }

    #[test]
    fn validate_package_selector_accepts_sub_package() {
        assert!(validate_package_selector(&Some("package/ext".to_string())).is_ok());
    }

    #[test]
    fn create_blueprint_in_primary_package() {
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();

        let result = create_blueprint(&store, minimal_blueprint("my-bp"), None).unwrap();
        assert!(!result.blueprint.id.is_empty());

        let data = store.all_data();
        let has_blueprint = data
            .keys()
            .any(|k| k.starts_with("package/blueprints/") && k.ends_with(".json"));
        assert!(
            has_blueprint,
            "blueprint file not found in package/blueprints/"
        );
    }

    #[test]
    fn create_blueprint_in_sub_package() {
        let store = MemoryStore::default();
        let selector = Some("package/ext".to_string());
        store.register_package_boundary(&None).unwrap();
        store.register_package_boundary(&selector).unwrap();

        let result =
            create_blueprint(&store, minimal_blueprint("sub-bp"), selector.clone()).unwrap();
        assert!(!result.blueprint.id.is_empty());

        let data = store.all_data();
        let has_sub_path = data
            .keys()
            .any(|k| k.starts_with("package/ext/blueprints/") && k.ends_with(".json"));
        assert!(
            has_sub_path,
            "blueprint file not found under package/ext/blueprints/"
        );
    }

    #[test]
    fn create_blueprint_rejects_path_traversal() {
        let store = MemoryStore::default();
        let result = create_blueprint(
            &store,
            minimal_blueprint("evil"),
            Some("package/../evil".to_string()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn create_blueprint_rejects_absolute_path() {
        let store = MemoryStore::default();
        let result = create_blueprint(
            &store,
            minimal_blueprint("abs"),
            Some("/abs/path".to_string()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn list_blueprints_includes_source_package() {
        let store = MemoryStore::default();
        let selector = Some("package/ext".to_string());
        store.register_package_boundary(&None).unwrap();
        store.register_package_boundary(&selector).unwrap();

        create_blueprint(&store, minimal_blueprint("primary-bp"), None).unwrap();
        create_blueprint(&store, minimal_blueprint("sub-bp"), selector).unwrap();

        let result = list_blueprints_summary(&store).unwrap();
        assert_eq!(result.summaries.len(), 2);
        assert!(result.diagnostics.is_empty());

        let primary = result
            .summaries
            .iter()
            .find(|s| s.name == "primary-bp")
            .unwrap();
        assert!(primary.source_package.is_none());

        let sub = result
            .summaries
            .iter()
            .find(|s| s.name == "sub-bp")
            .unwrap();
        assert_eq!(sub.source_package.as_deref(), Some("package/ext"));
    }

    #[test]
    fn update_blueprint_preserves_created_at() {
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();

        let bp = create_blueprint(&store, minimal_blueprint("keep-date"), None)
            .unwrap()
            .blueprint;
        let bp_id = bp.id.clone();

        let mut updated = bp.clone();
        updated.description = "updated description".to_string();
        updated.created_at = "2099-01-01T00:00:00Z".to_string(); // should be ignored

        let result = update_blueprint(&store, &bp_id, updated).unwrap();
        assert_eq!(result.blueprint.created_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn validate_blueprint_valid() {
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();
        let bp = create_blueprint(&store, minimal_blueprint("valid"), None)
            .unwrap()
            .blueprint;
        let result = validate_blueprint_by_id(&store, &bp.id).unwrap();
        assert!(result.valid);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn validate_blueprint_missing_root_types() {
        let mut bp = minimal_blueprint("no-roots");
        bp.root_types.clear();
        // Bypass service validation to store a bad blueprint directly.
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();
        bp.id = "test-id-abc12345".to_string();
        store
            .save_blueprint("package/blueprints/no-roots-test-id-a.json", &bp)
            .unwrap();
        store
            .add_definition_to_boundary(
                &None,
                DefinitionKind::Blueprint,
                "blueprints/no-roots-test-id-a.json",
            )
            .unwrap();
        let result = validate_blueprint_by_id(&store, &bp.id).unwrap();
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.contains("root_types")));
    }

    #[test]
    fn validate_blueprint_required_type_not_in_universe() {
        let mut bp = minimal_blueprint("bad-required");
        bp.required_types = vec![TypeRef {
            type_id: "nonexistent/type".to_string(),
            type_version: None,
        }];
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();
        bp.id = "test-id-bad12345".to_string();
        store
            .save_blueprint("package/blueprints/bad-required-test-id-b.json", &bp)
            .unwrap();
        store
            .add_definition_to_boundary(
                &None,
                DefinitionKind::Blueprint,
                "blueprints/bad-required-test-id-b.json",
            )
            .unwrap();
        let result = validate_blueprint_by_id(&store, &bp.id).unwrap();
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.contains("required_type")));
    }

    #[test]
    fn validate_blueprint_type_version_zero() {
        let mut bp = minimal_blueprint("zero-version");
        bp.root_types[0].type_version = Some(0);
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();
        bp.id = "test-id-zero1234".to_string();
        store
            .save_blueprint("package/blueprints/zero-version-test-id-z.json", &bp)
            .unwrap();
        store
            .add_definition_to_boundary(
                &None,
                DefinitionKind::Blueprint,
                "blueprints/zero-version-test-id-z.json",
            )
            .unwrap();
        let result = validate_blueprint_by_id(&store, &bp.id).unwrap();
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.contains("version 0")));
    }

    #[test]
    fn blueprint_structure_is_deterministically_ordered() {
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();

        let mut bp = minimal_blueprint("ordered");
        bp.structure = vec![
            RelationSpec {
                relation_type: "depends-on".to_string(),
                source_type: TypeRef {
                    type_id: "b/type".to_string(),
                    type_version: None,
                },
                target_type: TypeRef {
                    type_id: "a/type".to_string(),
                    type_version: None,
                },
                cardinality: None,
                required: None,
            },
            RelationSpec {
                relation_type: "contains".to_string(),
                source_type: TypeRef {
                    type_id: "a/type".to_string(),
                    type_version: None,
                },
                target_type: TypeRef {
                    type_id: "a/type".to_string(),
                    type_version: None,
                },
                cardinality: None,
                required: None,
            },
        ];
        bp.root_types.push(TypeRef {
            type_id: "b/type".to_string(),
            type_version: None,
        });
        bp.root_types.push(TypeRef {
            type_id: "a/type".to_string(),
            type_version: None,
        });

        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let structure = list_blueprint_structure(&store, &bp.id).unwrap();

        assert_eq!(structure[0].source_type.type_id, "a/type");
        assert_eq!(structure[1].source_type.type_id, "b/type");
    }

    #[test]
    fn delete_blueprint_removes_from_store() {
        let store = MemoryStore::default();
        store.register_package_boundary(&None).unwrap();

        let bp = create_blueprint(&store, minimal_blueprint("to-delete"), None)
            .unwrap()
            .blueprint;
        let bp_id = bp.id.clone();

        delete_blueprint(&store, &bp_id).unwrap();
        assert!(matches!(
            get_blueprint_by_id(&store, &bp_id).unwrap(),
            GetBlueprintResult::NotFound
        ));
    }
}
