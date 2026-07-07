/// Identifies a package boundary within a repository.
///
/// `None` = primary package (`package/`); `Some(path)` = sub-package at `path/`.
///
/// This is **not** test-gated — it is used in `RepositoryStore` trait methods
/// that must be available in production code.
pub type PackageSelector = Option<String>;

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
    /// This constructor accepts `serde_json::Value` because it lives in `srs-repository` (which
    /// already depends on `serde_json`). It must not be moved to `srs-core`, which has no
    /// `serde_json` dependency.
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
