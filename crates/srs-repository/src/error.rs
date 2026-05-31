use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RepositoryError {
    #[error("not found: {path:?}")]
    NotFound { path: PathBuf },

    #[error("manifest missing: {path:?}")]
    ManifestMissing { path: PathBuf },

    #[error("failed to load package at {path:?}: {source}")]
    PackageLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("type not found: {type_id}@{version}")]
    TypeNotFound { type_id: String, version: u32 },

    #[error("field not found: {field_id}")]
    FieldNotFound { field_id: String },

    #[error("failed to load record at {path:?}: {source}")]
    RecordLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("failed to write record at {path:?}: {source}")]
    RecordWrite {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("record validation failed at {path:?}: {source}")]
    RecordValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("manifest parse error at {path:?}: {source}")]
    ManifestParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("note load error at {path:?}: {source}")]
    NoteLoad {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("note validation error at {path:?}: {source}")]
    NoteValidation {
        path: PathBuf,
        #[source]
        source: srs_core::error::CoreError,
    },

    #[error("note write error at {path:?}: {source}")]
    NoteWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("note not found: {id} at {path:?}")]
    NoteNotFound { path: PathBuf, id: String },

    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("serialization error at {path:?}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to load tag definition at {path}: {source}")]
    TagDefinitionLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("tag definition validation failed at {path}: {source}")]
    TagDefinitionValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("relation type definition validation failed at {path:?}: {source}")]
    RelationTypeDefinitionValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("failed to write tag definition at {path}: {source}")]
    TagDefinitionWrite {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("schema validation error at {path:?}: {message}")]
    SchemaValidation { path: PathBuf, message: String },

    #[error("relation type conflict for '{relation_type}': definitions from {path_a:?} and {path_b:?} differ")]
    RelationTypeDefinitionConflict {
        relation_type: String,
        path_a: PathBuf,
        path_b: PathBuf,
    },

    #[error("relation validation failed for relation {relation_id}: {message}")]
    RelationValidation {
        relation_id: String,
        message: String,
    },

    #[error("container not found: {container_id}")]
    ContainerNotFound { container_id: String },

    #[error("container validation failed: {source}")]
    ContainerValidation { source: srs_core::error::CoreError },

    #[error("invalid valueType '{value_type}' in field definition at {path:?}")]
    InvalidValueType { path: PathBuf, value_type: String },

    #[error("failed to load view at {path:?}: {source}")]
    ViewLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("view validation failed at {path:?}: {source}")]
    ViewValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("failed to load document view at {path:?}: {source}")]
    DocumentViewLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("document view validation failed at {path:?}: {source}")]
    DocumentViewValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("document view not found: {view_id}")]
    DocumentViewNotFound { view_id: String },

    #[error("view not found: {view_id}")]
    ViewNotFound { view_id: String },

    #[error("document view not found: {document_view_id}")]
    DocumentViewNotFoundById { document_view_id: String },

    #[error("package ref path '{path}' is outside the repository root")]
    PackageRefOutsideRepo { path: String },

    #[error("package ref path '{path}' does not contain a package.json")]
    PackageRefMissing { path: String },

    #[error("package ref '{path}' contains a conflicting {kind} definition: id '{id}' (first loaded from {first_path:?}, conflict from {second_path:?})")]
    PackageRefConflict {
        path: String,
        kind: String,
        id: String,
        first_path: PathBuf,
        second_path: PathBuf,
    },

    #[error("repository already exists at {path:?}")]
    RepositoryAlreadyExists { path: PathBuf },

    #[error("invalid repository initialization: {message}")]
    InvalidRepositoryInitialization { message: String },

    #[error("repository target is not empty at {path:?}")]
    RepositoryNotEmpty { path: PathBuf },

    #[error("invalid snapshot data: {message}")]
    InvalidSnapshotData { message: String },

    #[error("package not found: {selector:?}")]
    PackageNotFound { selector: Option<String> },

    #[error("package already registered: id '{id}'")]
    PackageAlreadyRegistered { id: String },

    #[error("definition not found: {id}")]
    DefinitionNotFound { id: String },
}

impl PartialEq for RepositoryError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RepositoryError::NotFound { path: a }, RepositoryError::NotFound { path: b }) => {
                a == b
            }
            (
                RepositoryError::ManifestMissing { path: a },
                RepositoryError::ManifestMissing { path: b },
            ) => a == b,
            (
                RepositoryError::PackageLoad { path: a, source: _ },
                RepositoryError::PackageLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::TypeNotFound {
                    type_id: a,
                    version: va,
                },
                RepositoryError::TypeNotFound {
                    type_id: b,
                    version: vb,
                },
            ) => a == b && va == vb,
            (
                RepositoryError::FieldNotFound { field_id: a },
                RepositoryError::FieldNotFound { field_id: b },
            ) => a == b,
            (
                RepositoryError::RecordLoad { path: a, source: _ },
                RepositoryError::RecordLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::RecordWrite { path: a, source: _ },
                RepositoryError::RecordWrite { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::RecordValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::RecordValidation {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa == sb,
            (
                RepositoryError::ManifestParse { path: a, source: _ },
                RepositoryError::ManifestParse { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::NoteLoad { path: a, source: _ },
                RepositoryError::NoteLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::NoteValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::NoteValidation {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa == sb,
            (
                RepositoryError::NoteWrite { path: a, source: _ },
                RepositoryError::NoteWrite { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::Io { path: a, source: _ },
                RepositoryError::Io { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::Serialize { path: a, source: _ },
                RepositoryError::Serialize { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::TagDefinitionLoad { path: a, source: _ },
                RepositoryError::TagDefinitionLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::TagDefinitionValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::TagDefinitionValidation {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa == sb,
            (
                RepositoryError::TagDefinitionWrite { path: a, source: _ },
                RepositoryError::TagDefinitionWrite { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::RelationTypeDefinitionValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::RelationTypeDefinitionValidation {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa == sb,
            (
                RepositoryError::SchemaValidation {
                    path: a,
                    message: ma,
                },
                RepositoryError::SchemaValidation {
                    path: b,
                    message: mb,
                },
            ) => a == b && ma == mb,
            (
                RepositoryError::RelationTypeDefinitionConflict {
                    relation_type: rta,
                    path_a: aa,
                    path_b: ba,
                },
                RepositoryError::RelationTypeDefinitionConflict {
                    relation_type: rtb,
                    path_a: ab,
                    path_b: bb,
                },
            ) => rta == rtb && aa == ab && ba == bb,
            (
                RepositoryError::RelationValidation {
                    relation_id: ia,
                    message: ma,
                },
                RepositoryError::RelationValidation {
                    relation_id: ib,
                    message: mb,
                },
            ) => ia == ib && ma == mb,
            (
                RepositoryError::ContainerNotFound { container_id: a },
                RepositoryError::ContainerNotFound { container_id: b },
            ) => a == b,
            (
                RepositoryError::ContainerValidation { source: sa },
                RepositoryError::ContainerValidation { source: sb },
            ) => sa == sb,
            (
                RepositoryError::InvalidValueType {
                    path: ap,
                    value_type: av,
                },
                RepositoryError::InvalidValueType {
                    path: bp,
                    value_type: bv,
                },
            ) => ap == bp && av == bv,
            (
                RepositoryError::ViewLoad { path: a, source: _ },
                RepositoryError::ViewLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::ViewValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::ViewValidation {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa == sb,
            (
                RepositoryError::DocumentViewLoad { path: a, source: _ },
                RepositoryError::DocumentViewLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::DocumentViewValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::DocumentViewValidation {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa == sb,
            (
                RepositoryError::DocumentViewNotFound { view_id: a },
                RepositoryError::DocumentViewNotFound { view_id: b },
            ) => a == b,
            (
                RepositoryError::ViewNotFound { view_id: a },
                RepositoryError::ViewNotFound { view_id: b },
            ) => a == b,
            (
                RepositoryError::DocumentViewNotFoundById {
                    document_view_id: a,
                },
                RepositoryError::DocumentViewNotFoundById {
                    document_view_id: b,
                },
            ) => a == b,
            (
                RepositoryError::PackageRefOutsideRepo { path: a },
                RepositoryError::PackageRefOutsideRepo { path: b },
            ) => a == b,
            (
                RepositoryError::PackageRefMissing { path: a },
                RepositoryError::PackageRefMissing { path: b },
            ) => a == b,
            (
                RepositoryError::PackageRefConflict {
                    path: pa,
                    kind: ka,
                    id: ia,
                    ..
                },
                RepositoryError::PackageRefConflict {
                    path: pb,
                    kind: kb,
                    id: ib,
                    ..
                },
            ) => pa == pb && ka == kb && ia == ib,
            (
                RepositoryError::RepositoryAlreadyExists { path: a },
                RepositoryError::RepositoryAlreadyExists { path: b },
            ) => a == b,
            (
                RepositoryError::InvalidRepositoryInitialization { message: a },
                RepositoryError::InvalidRepositoryInitialization { message: b },
            ) => a == b,
            (
                RepositoryError::RepositoryNotEmpty { path: a },
                RepositoryError::RepositoryNotEmpty { path: b },
            ) => a == b,
            (
                RepositoryError::InvalidSnapshotData { message: a },
                RepositoryError::InvalidSnapshotData { message: b },
            ) => a == b,
            (
                RepositoryError::PackageNotFound { selector: a },
                RepositoryError::PackageNotFound { selector: b },
            ) => a == b,
            (
                RepositoryError::PackageAlreadyRegistered { id: a },
                RepositoryError::PackageAlreadyRegistered { id: b },
            ) => a == b,
            (
                RepositoryError::DefinitionNotFound { id: a },
                RepositoryError::DefinitionNotFound { id: b },
            ) => a == b,
            _ => false,
        }
    }
}
