/// Identifies a package boundary within a repository.
///
/// `None` = primary package (`package/`); `Some(path)` = sub-package at `path/`.
///
/// This is **not** test-gated — it is used in `RepositoryStore` trait methods
/// that must be available in production code.
pub type PackageSelector = Option<String>;

/// Validate that a package boundary selector is safe to use as a path prefix.
///
/// This is the **single canonical validation** for `--package` boundary selectors,
/// shared by every definition-create service (fields, types, views, document views,
/// themes, blueprints, protocols, relation types, lifecycles). Do not add per-service
/// selector rules — extend this function instead (#507).
///
/// The canonical selector form is the boundary's repo-root-relative directory path,
/// exactly as registered by `package create --path <path>` (e.g. `packages/governance`
/// or `package/ext`). `None` selects the primary package (`package/`). No particular
/// path prefix is required — whether the boundary actually exists is checked separately
/// via `RepositoryStore::load_package_boundary`.
///
/// Rules:
/// - `None` (primary package) is always valid.
/// - Must not be empty or whitespace-only.
/// - Must not be an absolute path.
/// - Must not contain `".."` path components.
pub fn validate_package_selector(
    selector: &PackageSelector,
) -> Result<(), crate::error::RepositoryError> {
    use crate::error::RepositoryError;
    let Some(path) = selector.as_deref() else {
        return Ok(());
    };
    if path.trim().is_empty() {
        return Err(RepositoryError::InvalidPackageSelector {
            message: "selector must not be empty".to_string(),
        });
    }
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
    Ok(())
}

/// Metadata describing one package boundary.
#[derive(Debug, Clone)]
pub struct PackageBoundary {
    /// `None` for the primary package; `Some(path)` for sub-packages.
    pub selector: PackageSelector,
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    /// Paths of field files, relative to the boundary directory (e.g. `"fields/foo.json"`).
    pub field_paths: Vec<String>,
    /// Paths of type files, relative to the boundary directory.
    pub type_paths: Vec<String>,
    /// Paths of blueprint files, relative to the boundary directory.
    pub blueprint_paths: Vec<String>,
    /// Paths of protocol files, relative to the boundary directory.
    pub protocol_paths: Vec<String>,
}

impl PackageBoundary {
    /// Build a `PackageBoundary` from a parsed `package.json` value and its selector.
    ///
    /// All fields default to empty string / empty vec when absent rather than returning an error —
    /// validation of required fields (e.g. `id`) is the caller's responsibility.
    ///
    /// This constructor belongs in `srs-repository`, not `srs-core`. Constructing a
    /// `PackageBoundary` from a raw parsed `package.json` blob is a storage-adapter concern
    /// (ADR-001, ADR-009); the core type layer constructs types with explicit, validated fields
    /// rather than interpreting on-disk JSON shapes.
    pub fn from_pkg_json(
        pkg_json: &serde_json::Value,
        selector: PackageSelector,
    ) -> PackageBoundary {
        let str_paths = |key: &str| -> Vec<String> {
            pkg_json[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        PackageBoundary {
            selector,
            id: pkg_json["id"].as_str().unwrap_or("").to_string(),
            namespace: pkg_json["namespace"].as_str().unwrap_or("").to_string(),
            name: pkg_json["name"].as_str().unwrap_or("").to_string(),
            version: pkg_json["version"].as_str().unwrap_or("").to_string(),
            field_paths: str_paths("fields"),
            type_paths: str_paths("types"),
            blueprint_paths: str_paths("blueprints"),
            protocol_paths: str_paths("protocols"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_pkg_json_extracts_all_fields() {
        let pkg = serde_json::json!({
            "id": "com.example/pkg",
            "namespace": "com.example",
            "name": "My Package",
            "version": "1.0.0",
            "fields": ["fields/a.json", "fields/b.json"],
            "types": ["types/t.json"],
            "blueprints": ["blueprints/bp.json"],
            "protocols": ["protocols/p.json"]
        });
        let b = PackageBoundary::from_pkg_json(&pkg, Some("sub/pkg".to_string()));
        assert_eq!(b.id, "com.example/pkg");
        assert_eq!(b.namespace, "com.example");
        assert_eq!(b.name, "My Package");
        assert_eq!(b.version, "1.0.0");
        assert_eq!(b.field_paths, vec!["fields/a.json", "fields/b.json"]);
        assert_eq!(b.type_paths, vec!["types/t.json"]);
        assert_eq!(b.blueprint_paths, vec!["blueprints/bp.json"]);
        assert_eq!(b.protocol_paths, vec!["protocols/p.json"]);
        assert_eq!(b.selector, Some("sub/pkg".to_string()));
    }

    #[test]
    fn from_pkg_json_missing_arrays_default_to_empty() {
        let pkg = serde_json::json!({ "id": "com.example/minimal", "namespace": "com.example", "name": "Min", "version": "0.1.0" });
        let b = PackageBoundary::from_pkg_json(&pkg, None);
        assert!(b.field_paths.is_empty());
        assert!(b.type_paths.is_empty());
        assert!(b.blueprint_paths.is_empty());
        assert!(b.protocol_paths.is_empty());
    }

    #[test]
    fn validate_package_selector_accepts_primary() {
        assert!(validate_package_selector(&None).is_ok());
    }

    #[test]
    fn validate_package_selector_accepts_package_prefixed_path() {
        assert!(validate_package_selector(&Some("package/ext".to_string())).is_ok());
    }

    #[test]
    fn validate_package_selector_accepts_packages_prefixed_path() {
        // Regression for #507: the boundary form created by
        // `package create --path packages/governance` must be accepted everywhere.
        assert!(validate_package_selector(&Some("packages/governance".to_string())).is_ok());
    }

    #[test]
    fn validate_package_selector_accepts_arbitrary_relative_path() {
        // The convention is "any repo-root-relative registered boundary path" —
        // existence is checked by load_package_boundary, not the selector parser.
        assert!(validate_package_selector(&Some("pkg/sub".to_string())).is_ok());
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
    fn validate_package_selector_rejects_empty() {
        assert!(validate_package_selector(&Some(String::new())).is_err());
        assert!(validate_package_selector(&Some("   ".to_string())).is_err());
    }

    #[test]
    fn from_pkg_json_non_string_array_entries_are_silently_skipped() {
        let pkg = serde_json::json!({
            "id": "com.example/pkg",
            "namespace": "com.example",
            "name": "Pkg",
            "version": "1.0.0",
            "fields": [1, null, "fields/valid.json"],
            "types": []
        });
        let b = PackageBoundary::from_pkg_json(&pkg, None);
        assert_eq!(b.field_paths, vec!["fields/valid.json"]);
        assert!(b.type_paths.is_empty());
    }
}

/// A field merged from all boundaries, carrying its source boundary.
#[derive(Debug, Clone)]
pub struct OwnedField {
    pub field: srs_core::types::field::Field,
    pub owner: PackageSelector,
}

/// A record type merged from all boundaries, carrying its source boundary.
#[derive(Debug, Clone)]
pub struct OwnedType {
    pub record_type: srs_core::types::record_type::RecordType,
    pub owner: PackageSelector,
}

/// Discriminates the kind of definition stored in a package boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind {
    Field,
    Type,
    View,
    DocumentView,
    RelationType,
    Blueprint,
    Protocol,
    Vocabulary,
    Lifecycle,
    Theme,
}
