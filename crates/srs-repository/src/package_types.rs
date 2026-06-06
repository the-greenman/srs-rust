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
    Vocabulary,
}
