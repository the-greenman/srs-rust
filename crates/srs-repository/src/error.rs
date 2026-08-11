use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RepositoryError {
    #[error("not found: {path:?}")]
    NotFound { path: PathBuf },

    #[error("instance not found: {id}")]
    InstanceNotFound { id: String },

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

    #[error("failed to load instance '{instance_id}' from path {path:?}: {source}")]
    InstanceLoad {
        instance_id: String,
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("relation type definition validation failed at {path:?}: {source}")]
    RelationTypeDefinitionValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
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

    #[error("relation not found: {relation_id}")]
    RelationNotFound { relation_id: String },

    /// The relationId is not a canonical lowercase hyphenated UUID. Required
    /// because the id is the standalone object's filename component
    /// (`relations/<relationId>.json`, RFC-038 Change E): anything else is a
    /// path-escape write primitive (`../manifest`) or an [R11] filename
    /// mismatch by construction.
    #[error("invalid relationId '{relation_id}': must be a canonical lowercase hyphenated UUID (RFC-038 Change E)")]
    InvalidRelationId { relation_id: String },

    /// The instanceId is not a canonical lowercase hyphenated UUID. Required
    /// because a caller-supplied instanceId can become part of the saved
    /// entity's filename (`catalog_save_instance`'s full-id collision fallback,
    /// `{tier_dir}/{instance_id}.json`): anything else is a path-escape write
    /// primitive (`../manifest`) — the same class of bug `InvalidRelationId`
    /// guards against for relations.
    #[error("invalid instanceId '{instance_id}': must be a canonical lowercase hyphenated UUID")]
    InvalidInstanceId { instance_id: String },

    /// RFC-038 [R11]: a standalone relation object's filename disagrees with its
    /// in-file `relationId`. The in-file id is authoritative; the error names both.
    #[error("relation file {path:?} names relationId '{file_relation_id}' — the filename must match the in-file relationId (RFC-038 [R11])")]
    RelationFilenameMismatch {
        path: PathBuf,
        file_relation_id: String,
    },

    /// RFC-038 [R12]: the same `relationId` was discovered at more than one locator
    /// (standalone objects and/or relations-collection entries). Names every locator.
    #[error("duplicate relationId '{relation_id}' found at: {}", locators.join(", "))]
    DuplicateRelationId {
        relation_id: String,
        locators: Vec<String>,
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

    #[error("failed to load theme at {path:?}: {source}")]
    ThemeLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("failed to load source document metadata at {path:?}: {source}")]
    SourceDocumentMetaLoad {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("theme validation failed at {path:?}: {source}")]
    ThemeValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("document view not found: {view_id}")]
    DocumentViewNotFound { view_id: String },

    #[error("view not found: {view_id}")]
    ViewNotFound { view_id: String },

    #[error("theme not found: {theme_id}")]
    ThemeNotFound { theme_id: String },

    #[error("blueprint not found: {blueprint_id}")]
    BlueprintNotFound { blueprint_id: String },

    #[error("blueprint validation failed at {path:?}: {source}")]
    BlueprintValidation {
        path: PathBuf,
        source: srs_core::error::CoreError,
    },

    #[error("invalid package selector: {message}")]
    InvalidPackageSelector { message: String },

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

    #[error("invalid archive: {message}")]
    InvalidArchive { message: String },

    #[error("invalid export bundle: {message}")]
    InvalidExportBundle { message: String },

    #[error("package not found: {selector:?}")]
    PackageNotFound { selector: Option<String> },

    #[error("package already registered: id '{id}'")]
    PackageAlreadyRegistered { id: String },

    #[error(
        "package install aborted (strict): {count} same-key/different-UUID conflict(s): {keys}"
    )]
    PackageInstallConflicts { count: usize, keys: String },

    #[error("definition not found: {id}")]
    DefinitionNotFound { id: String },

    #[error("cannot delete {entity_type} '{id}': still referenced by [{used_by}]",
            used_by = used_by.join(", "))]
    CannotDeleteInUse {
        entity_type: String,
        id: String,
        used_by: Vec<String>,
    },

    // ── ext:type-inheritance errors ───────────────────────────────────────────
    #[error("type inheritance cycle detected involving type '{type_id}'")]
    TypeInheritanceCycle { type_id: String },

    #[error(
        "inherited field duplicate: field '{field_id}' appears in both base type '{base_type_id}' and specializing type '{type_id}'"
    )]
    InheritedFieldDuplicate {
        type_id: String,
        base_type_id: String,
        field_id: String,
    },

    #[error(
        "fieldOrder for type '{type_id}' is incomplete: field '{field_id}' is in the effective field set but not in fieldOrder"
    )]
    FieldOrderMismatch { type_id: String, field_id: String },

    #[error(
        "fieldAssignmentOverride in type '{type_id}' targets field '{field_id}' which is in the type's own fields[], not an inherited field"
    )]
    OverrideTargetsOwnField { type_id: String, field_id: String },

    #[error(
        "fieldAssignmentOverride in type '{type_id}' tries to relax required on field '{field_id}' (base: required=true, override: required=false)"
    )]
    OverrideRelaxesRequired { type_id: String, field_id: String },

    // ── ext:lifecycle errors ──────────────────────────────────────────────────
    #[error("record '{id}' has no lifecycle defined on its Type")]
    LifecycleNotDefined { id: String },

    #[error("no transition from '{from}' to '{to}' in Type lifecycle")]
    LifecycleTransitionNotAllowed { from: String, to: String },

    #[error("lifecycle state '{state}' is not defined in Type lifecycle")]
    LifecycleStateNotDefined { state: String },

    // ── RFC-022 relational lifecycle states ──────────────────────────────────
    #[error(
        "LIFECYCLE_RELATION_REQUIRED: state '{state}' requires a satisfying '{direction}' relation of type {relation_types:?}; supply fulfillment.newRecord or fulfillment.existingInstanceId, or assert the relation first",
    )]
    LifecycleRelationRequired {
        state: String,
        relation_types: Vec<String>,
        direction: String,
    },

    #[error("LIFECYCLE_FULFILLMENT_NOT_APPLICABLE: target state '{state}' declares no requiresRelation — fulfillment must be omitted")]
    LifecycleFulfillmentNotApplicable { state: String },

    #[error("LIFECYCLE_FULFILLMENT_RELATION_TYPE_MISMATCH: fulfillment.relationType '{relation_type}' is not among the declared types {declared:?} for state '{state}'")]
    LifecycleFulfillmentRelationTypeMismatch {
        state: String,
        relation_type: String,
        declared: Vec<String>,
    },

    #[error("LIFECYCLE_STATE_UNREACHABLE: state '{state}' is not reachable from initial state '{initial}' via declared transitions")]
    LifecycleStateUnreachable { state: String, initial: String },

    #[error("type version {version} not found for type '{type_id}'")]
    TypeVersionNotFound { type_id: String, version: u32 },

    #[error(
        "vocabulary '{vocabulary_id}' promotion blocked: {count} in-use key(s) have no active term in the vocabulary",
        count = unresolvable_keys.len()
    )]
    VocabularyPromotionBlocked {
        vocabulary_id: String,
        unresolvable_keys: Vec<String>,
    },

    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    #[error(
        "repository defines its own {kind} '{qualified_name}' (id: {id}) which conflicts with \
         a definition already reserved by the embedded core package"
    )]
    CorePackageConflict {
        kind: String,
        id: String,
        qualified_name: String,
    },

    // ── ext:registry errors ───────────────────────────────────────────────────
    #[error("failed to load registry at {path:?}: {source}")]
    RegistryLoad {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("registry parse error: {source}")]
    RegistryParse {
        #[source]
        source: serde_json::Error,
    },

    #[error("registry entry not found: {package_name}")]
    RegistryEntryNotFound { package_name: String },

    #[error("failed to read registry at {path:?}: {message}")]
    RegistryIo { path: PathBuf, message: String },

    // ── ext:federation errors ─────────────────────────────────────────────────
    #[error("failed to load federation registry at {path:?}: {source}")]
    FederationRegistryLoad {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("federation registry parse error: {source}")]
    FederationRegistryParse {
        #[source]
        source: serde_json::Error,
    },

    #[error("federation registry cycle detected at registry '{registry_id}'")]
    FederationRegistryCycle { registry_id: String },

    #[error("failed to load federation events at {path:?}: {source}")]
    FederationEventsLoad {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to write federation events at {path:?}: {source}")]
    FederationEventsWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ── ext:protocol run errors ───────────────────────────────────────────────
    #[error("protocol run '{run_id}' is not in a valid state for this operation: {message}")]
    RunInvalidState { run_id: String, message: String },

    // ── RFC-038 catalog (srs-rust#783 Phase 1) ───────────────────────────────
    /// [R24]: an `error` diagnostic under a reserved repository location is
    /// fatal to the load — no partial catalog is reported as complete. The
    /// complete diagnostic list (errors and warnings) travels with the error.
    #[error("catalog load failed: {fatal} fatal diagnostic(s); first: {first}")]
    CatalogLoad {
        fatal: usize,
        first: String,
        diagnostics: Vec<crate::catalog::CatalogDiagnostic>,
    },

    /// The store does not implement RFC-038 catalog enumeration — the trait
    /// default, for a backend outside the contract (#706's database adapter).
    #[error("this store does not support catalog enumeration")]
    CatalogUnsupported,

    /// RFC-038 [R2]: `manifest.json` must not contain the retired index/
    /// checksum/path properties. Feature-inactive until the Phase-6 flip;
    /// fired only under the crate-internal test activation until then.
    #[error("manifest.json declares retired property '{property}' — removed by RFC-038 [R2]; run the rfc038-storage migration")]
    RetiredManifestProperty { property: String },

    /// RFC-038 [R21]: a repository below storage generation 2 is not
    /// supported. Feature-inactive until the Phase-6 flip; fired only under
    /// the crate-internal test activation until then.
    #[error("manifest.json declares dataModelRevision {declared}; this build requires storage generation >= 2 (RFC-038 [R21]) — run the rfc038-storage migration")]
    StorageGenerationUnsupported { declared: u64 },
}

impl From<zip::result::ZipError> for RepositoryError {
    fn from(e: zip::result::ZipError) -> Self {
        RepositoryError::InvalidArchive {
            message: e.to_string(),
        }
    }
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
                RepositoryError::InstanceLoad {
                    instance_id: a,
                    path: pa,
                    ..
                },
                RepositoryError::InstanceLoad {
                    instance_id: b,
                    path: pb,
                    ..
                },
            ) => a == b && pa == pb,
            (
                RepositoryError::Serialize { path: a, source: _ },
                RepositoryError::Serialize { path: b, source: _ },
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
                RepositoryError::ThemeLoad {
                    path: a,
                    source: sa,
                },
                RepositoryError::ThemeLoad {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa.to_string() == sb.to_string(),
            (
                RepositoryError::SourceDocumentMetaLoad {
                    path: a,
                    source: sa,
                },
                RepositoryError::SourceDocumentMetaLoad {
                    path: b,
                    source: sb,
                },
            ) => a == b && sa.to_string() == sb.to_string(),
            (
                RepositoryError::ThemeValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::ThemeValidation {
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
                RepositoryError::ThemeNotFound { theme_id: a },
                RepositoryError::ThemeNotFound { theme_id: b },
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
                RepositoryError::InvalidArchive { message: a },
                RepositoryError::InvalidArchive { message: b },
            ) => a == b,
            (
                RepositoryError::InvalidExportBundle { message: a },
                RepositoryError::InvalidExportBundle { message: b },
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
            (
                RepositoryError::CannotDeleteInUse {
                    entity_type: eta,
                    id: ia,
                    used_by: ua,
                },
                RepositoryError::CannotDeleteInUse {
                    entity_type: etb,
                    id: ib,
                    used_by: ub,
                },
            ) => eta == etb && ia == ib && ua == ub,
            (
                RepositoryError::TypeInheritanceCycle { type_id: a },
                RepositoryError::TypeInheritanceCycle { type_id: b },
            ) => a == b,
            (
                RepositoryError::InheritedFieldDuplicate {
                    type_id: ta,
                    base_type_id: ba,
                    field_id: fa,
                },
                RepositoryError::InheritedFieldDuplicate {
                    type_id: tb,
                    base_type_id: bb,
                    field_id: fb,
                },
            ) => ta == tb && ba == bb && fa == fb,
            (
                RepositoryError::FieldOrderMismatch {
                    type_id: ta,
                    field_id: fa,
                },
                RepositoryError::FieldOrderMismatch {
                    type_id: tb,
                    field_id: fb,
                },
            ) => ta == tb && fa == fb,
            (
                RepositoryError::OverrideTargetsOwnField {
                    type_id: ta,
                    field_id: fa,
                },
                RepositoryError::OverrideTargetsOwnField {
                    type_id: tb,
                    field_id: fb,
                },
            ) => ta == tb && fa == fb,
            (
                RepositoryError::OverrideRelaxesRequired {
                    type_id: ta,
                    field_id: fa,
                },
                RepositoryError::OverrideRelaxesRequired {
                    type_id: tb,
                    field_id: fb,
                },
            ) => ta == tb && fa == fb,
            (
                RepositoryError::LifecycleNotDefined { id: a },
                RepositoryError::LifecycleNotDefined { id: b },
            ) => a == b,
            (
                RepositoryError::LifecycleTransitionNotAllowed { from: fa, to: ta },
                RepositoryError::LifecycleTransitionNotAllowed { from: fb, to: tb },
            ) => fa == fb && ta == tb,
            (
                RepositoryError::LifecycleStateNotDefined { state: a },
                RepositoryError::LifecycleStateNotDefined { state: b },
            ) => a == b,
            (
                RepositoryError::LifecycleRelationRequired {
                    state: sa,
                    relation_types: ra,
                    direction: da,
                },
                RepositoryError::LifecycleRelationRequired {
                    state: sb,
                    relation_types: rb,
                    direction: db,
                },
            ) => sa == sb && ra == rb && da == db,
            (
                RepositoryError::LifecycleFulfillmentNotApplicable { state: a },
                RepositoryError::LifecycleFulfillmentNotApplicable { state: b },
            ) => a == b,
            (
                RepositoryError::LifecycleFulfillmentRelationTypeMismatch {
                    state: sa,
                    relation_type: ra,
                    declared: da,
                },
                RepositoryError::LifecycleFulfillmentRelationTypeMismatch {
                    state: sb,
                    relation_type: rb,
                    declared: db,
                },
            ) => sa == sb && ra == rb && da == db,
            (
                RepositoryError::LifecycleStateUnreachable {
                    state: sa,
                    initial: ia,
                },
                RepositoryError::LifecycleStateUnreachable {
                    state: sb,
                    initial: ib,
                },
            ) => sa == sb && ia == ib,
            (
                RepositoryError::TypeVersionNotFound {
                    type_id: ia,
                    version: va,
                },
                RepositoryError::TypeVersionNotFound {
                    type_id: ib,
                    version: vb,
                },
            ) => ia == ib && va == vb,
            (
                RepositoryError::VocabularyPromotionBlocked {
                    vocabulary_id: va,
                    unresolvable_keys: ka,
                },
                RepositoryError::VocabularyPromotionBlocked {
                    vocabulary_id: vb,
                    unresolvable_keys: kb,
                },
            ) => va == vb && ka == kb,
            (
                RepositoryError::InvalidInput { message: a },
                RepositoryError::InvalidInput { message: b },
            ) => a == b,
            (
                RepositoryError::CorePackageConflict {
                    kind: ka,
                    id: ia,
                    qualified_name: qa,
                },
                RepositoryError::CorePackageConflict {
                    kind: kb,
                    id: ib,
                    qualified_name: qb,
                },
            ) => ka == kb && ia == ib && qa == qb,
            (
                RepositoryError::RegistryLoad { path: a, .. },
                RepositoryError::RegistryLoad { path: b, .. },
            ) => a == b,
            (RepositoryError::RegistryParse { .. }, RepositoryError::RegistryParse { .. }) => true,
            (
                RepositoryError::RegistryEntryNotFound { package_name: a },
                RepositoryError::RegistryEntryNotFound { package_name: b },
            ) => a == b,
            (
                RepositoryError::RegistryIo {
                    path: a,
                    message: ma,
                },
                RepositoryError::RegistryIo {
                    path: b,
                    message: mb,
                },
            ) => a == b && ma == mb,
            (
                RepositoryError::FederationRegistryLoad { path: a, .. },
                RepositoryError::FederationRegistryLoad { path: b, .. },
            ) => a == b,
            (
                RepositoryError::FederationRegistryParse { .. },
                RepositoryError::FederationRegistryParse { .. },
            ) => true,
            (
                RepositoryError::FederationRegistryCycle { registry_id: a },
                RepositoryError::FederationRegistryCycle { registry_id: b },
            ) => a == b,
            (
                RepositoryError::FederationEventsLoad { path: a, .. },
                RepositoryError::FederationEventsLoad { path: b, .. },
            ) => a == b,
            (
                RepositoryError::FederationEventsWrite { path: a, .. },
                RepositoryError::FederationEventsWrite { path: b, .. },
            ) => a == b,
            (
                RepositoryError::RunInvalidState {
                    run_id: a,
                    message: ma,
                },
                RepositoryError::RunInvalidState {
                    run_id: b,
                    message: mb,
                },
            ) => a == b && ma == mb,
            _ => false,
        }
    }
}

impl RepositoryError {
    /// Returns true for both `NotFound` (MemoryStore) and `Io` where
    /// `source.kind() == NotFound` (FileStore, either Vfs backend).
    pub fn is_not_found(&self) -> bool {
        matches!(self, RepositoryError::NotFound { .. })
            || matches!(self, RepositoryError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound)
    }
}
