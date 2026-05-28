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

    #[error("failed to write tag definition at {path}: {source}")]
    TagDefinitionWrite {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("schema validation error at {path:?}: {message}")]
    SchemaValidation { path: PathBuf, message: String },
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
                RepositoryError::SchemaValidation {
                    path: a,
                    message: ma,
                },
                RepositoryError::SchemaValidation {
                    path: b,
                    message: mb,
                },
            ) => a == b && ma == mb,
            _ => false,
        }
    }
}
