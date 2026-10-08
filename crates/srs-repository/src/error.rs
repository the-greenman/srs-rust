use crate::relation_service::RelationSummary;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug, serde::Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
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
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("type not found: {type_id}@{version}")]
    TypeNotFound { type_id: String, version: u32 },

    #[error("field not found: {field_id}")]
    FieldNotFound { field_id: String },

    #[error("lifecycle not found: {id}")]
    LifecycleNotFound { id: String },

    #[error("lifecycle id in body ({body_id}) does not match argument ({argument_id})")]
    LifecycleIdMismatch {
        argument_id: String,
        body_id: String,
    },

    #[error("lifecycle validation failed: {}", violations.join("; "))]
    LifecycleValidation { violations: Vec<String> },

    #[error("failed to load record at {path:?}: {source}")]
    RecordLoad {
        path: PathBuf,
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("failed to write record at {path:?}: {source}")]
    RecordWrite {
        path: PathBuf,
        #[serde(skip)]
        source: std::io::Error,
    },

    #[error("record validation failed at {path:?}: {source}")]
    RecordValidation {
        path: PathBuf,
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    /// srs-rust#1025: `record delete` used to cascade-delete every relation
    /// incident to the record with no diagnostic — a record that is the
    /// **target** of inbound relations (e.g. a successor's `derived-from`
    /// edge) had its provenance silently severed. Refusal is the default;
    /// `--cascade` (`cascade_inbound: true` at the service layer) opts in.
    #[error(
        "record '{instance_id}' is the target of {count} inbound relation(s); deleting it would silently drop them. Pass --cascade to delete the record and its incident relations, or resolve the relations first: {relations:?}"
    )]
    RecordHasInboundRelations {
        instance_id: String,
        count: usize,
        relations: Vec<RelationSummary>,
    },

    #[error("manifest parse error at {path:?}: {source}")]
    ManifestParse {
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("note load error at {path:?}: {source}")]
    NoteLoad {
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("note validation error at {path:?}: {source}")]
    NoteValidation {
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    #[error("note write error at {path:?}: {source}")]
    NoteWrite {
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: std::io::Error,
    },

    #[error("note not found: {id} at {path:?}")]
    NoteNotFound { path: PathBuf, id: String },

    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: std::io::Error,
    },

    #[error("serialization error at {path:?}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("failed to load instance '{instance_id}' from path {path:?}: {source}")]
    InstanceLoad {
        instance_id: String,
        path: PathBuf,
        #[source]
        #[serde(skip)]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("relation type definition validation failed at {path:?}: {source}")]
    RelationTypeDefinitionValidation {
        path: PathBuf,
        #[serde(skip)]
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

    /// srs-rust#1167: `container create` is a create, not an upsert. A caller that means
    /// "replace" must go through `container_service::update_container` instead.
    #[error("container '{container_id}' already exists; use `container update` to replace it")]
    ContainerAlreadyExists { container_id: String },

    /// Owner ruling srs-rust#742 (2026-09-08): identity is a very explicit modification.
    /// `container delete` must refuse the repository's root container (the RFC-013
    /// `manifest.container` embed) rather than making the repository rootless as a side
    /// effect of a generic CRUD verb — `repo unset-root-container` is the only path.
    #[error(
        "container '{container_id}' is the repository's root container; use `repo unset-root-container` to remove repository identity, not `container delete`"
    )]
    ContainerIsRepositoryRoot { container_id: String },

    #[error("container validation failed: {source}")]
    ContainerValidation {
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    #[error("invalid valueType '{value_type}' in field definition at {path:?}")]
    InvalidValueType { path: PathBuf, value_type: String },

    #[error("failed to load view at {path:?}: {source}")]
    ViewLoad {
        path: PathBuf,
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("view validation failed at {path:?}: {source}")]
    ViewValidation {
        path: PathBuf,
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    #[error("failed to load document view at {path:?}: {source}")]
    CompositionLoad {
        path: PathBuf,
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("document view validation failed at {path:?}: {source}")]
    CompositionValidation {
        path: PathBuf,
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    #[error("failed to load theme at {path:?}: {source}")]
    ThemeLoad {
        path: PathBuf,
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("failed to load source document metadata at {path:?}: {source}")]
    SourceDocumentMetaLoad {
        path: PathBuf,
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("theme validation failed at {path:?}: {source}")]
    ThemeValidation {
        path: PathBuf,
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    #[error("composition not found: {view_id}")]
    CompositionNotFound { view_id: String },

    #[error("view not found: {view_id}")]
    ViewNotFound { view_id: String },

    #[error("theme not found: {theme_id}")]
    ThemeNotFound { theme_id: String },

    #[error("blueprint not found: {blueprint_id}")]
    BlueprintNotFound { blueprint_id: String },

    #[error("blueprint validation failed at {path:?}: {source}")]
    BlueprintValidation {
        path: PathBuf,
        #[serde(skip)]
        source: srs_core::error::CoreError,
    },

    #[error("invalid package selector: {message}")]
    InvalidPackageSelector { message: String },

    #[error("composition not found: {composition_id}")]
    CompositionNotFoundById { composition_id: String },

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

    #[error(
        "{store} cannot guarantee batch rollback: an aborted operation would leave partial writes on disk (srs-rust#813 tracks real write-staging); refusing before the first write"
    )]
    BatchSeamUnsupported { store: String },

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
        "state '{state}' requires a satisfying '{direction}' relation of type {relation_types:?}; supply fulfillment.newRecord or fulfillment.existingInstanceId, or assert the relation first",
    )]
    LifecycleRelationRequired {
        state: String,
        relation_types: Vec<String>,
        direction: String,
    },

    #[error("{}", successor_undetermined_detail(candidates))]
    SuccessorRelationTypeUndetermined { candidates: Vec<String> },

    #[error("target state '{state}' declares no requiresRelation — fulfillment must be omitted")]
    LifecycleFulfillmentNotApplicable { state: String },

    #[error("fulfillment.relationType '{relation_type}' is not among the declared types {declared:?} for state '{state}'")]
    LifecycleFulfillmentRelationTypeMismatch {
        state: String,
        relation_type: String,
        declared: Vec<String>,
    },

    #[error(
        "state '{state}' is not reachable from initial state '{initial}' via declared transitions"
    )]
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
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("registry parse error: {source}")]
    RegistryParse {
        #[source]
        #[serde(skip)]
        source: serde_json::Error,
    },

    #[error("registry entry not found: {package_name}")]
    RegistryEntryNotFound { package_name: String },

    #[error("failed to read registry at {path:?}: {message}")]
    RegistryIo { path: PathBuf, message: String },

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

    /// RFC-043 [R16]: a revision-8 binary does not interpret revision-7 container shapes
    /// (`rootInstanceIds`, string `memberInstanceIds`); it names the registry migration instead.
    #[error("manifest.json carries a dataModelRevision 7 container shape (rootInstanceIds or bare-id memberInstanceIds); this build reads dataModelRevision 8 (RFC-043 [R16]) — run `srs repo apply-migration --id rfc043-container-entries`")]
    Rfc043MigrationNeeded,

    /// RFC-046 actor-provenance refusal. `code` is one of `actor-invalid`,
    /// `actor-supplied`, `actor-changed`, `revision-too-old` ([R4]/[R5]/[R11]-[R13]);
    /// nothing was written.
    #[error("{message}")]
    ActorProvenance {
        #[serde(skip)]
        code: &'static str,
        message: String,
    },

    /// A `.srspkg` Package Bundle was refused (ADR-050). `code` is one of
    /// `bundle-not-json`, `bundle-readme-unsupported`, `bundle-revision-too-new`,
    /// `bundle-migration-refused`, `bundle-schema-invalid`, `bundle-definition-invalid`
    /// (reader); `bundle-published-at-invalid`, `bundle-boundary-unreadable`,
    /// `bundle-schema-invalid` (writer); coded like `ActorProvenance` so clients
    /// can branch on the reason.
    #[error("{message}")]
    InvalidPackageBundle {
        #[serde(skip)]
        code: &'static str,
        message: String,
    },

    /// An RFC-026 container slice export was refused (ADR-051). `code` is one of
    /// `slice-root-identity-invalid` (the boundary's identity entry is not a
    /// depth-0 entry without descendants), `slice-exported-at-invalid`,
    /// `slice-repository-id-reused`.
    #[error("{message}")]
    SliceRefused {
        #[serde(skip)]
        code: &'static str,
        message: String,
    },

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
                RepositoryError::LifecycleNotFound { id: a },
                RepositoryError::LifecycleNotFound { id: b },
            ) => a == b,
            (
                RepositoryError::LifecycleIdMismatch {
                    argument_id: aa,
                    body_id: ab,
                },
                RepositoryError::LifecycleIdMismatch {
                    argument_id: ba,
                    body_id: bb,
                },
            ) => aa == ba && ab == bb,
            (
                RepositoryError::LifecycleValidation { violations: a },
                RepositoryError::LifecycleValidation { violations: b },
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
                RepositoryError::RecordHasInboundRelations {
                    instance_id: a,
                    count: ca,
                    relations: ra,
                },
                RepositoryError::RecordHasInboundRelations {
                    instance_id: b,
                    count: cb,
                    relations: rb,
                },
            ) => a == b && ca == cb && ra == rb,
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
                RepositoryError::ContainerIsRepositoryRoot { container_id: a },
                RepositoryError::ContainerIsRepositoryRoot { container_id: b },
            ) => a == b,
            (
                RepositoryError::ContainerAlreadyExists { container_id: a },
                RepositoryError::ContainerAlreadyExists { container_id: b },
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
                RepositoryError::CompositionLoad { path: a, source: _ },
                RepositoryError::CompositionLoad { path: b, source: _ },
            ) => a == b,
            (
                RepositoryError::CompositionValidation {
                    path: a,
                    source: sa,
                },
                RepositoryError::CompositionValidation {
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
                RepositoryError::CompositionNotFound { view_id: a },
                RepositoryError::CompositionNotFound { view_id: b },
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
                RepositoryError::CompositionNotFoundById { composition_id: a },
                RepositoryError::CompositionNotFoundById { composition_id: b },
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
                RepositoryError::BatchSeamUnsupported { store: a },
                RepositoryError::BatchSeamUnsupported { store: b },
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
                RepositoryError::SuccessorRelationTypeUndetermined { candidates: a },
                RepositoryError::SuccessorRelationTypeUndetermined { candidates: b },
            ) => a == b,
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

/// Code carried by failures with no more specific classification (ADR-053).
pub const UNCLASSIFIED: &str = "unclassified";

/// A failure as clients consume it: stable `code`, human `message`, optional
/// open `details` (ADR-053). Adapters carry it unchanged.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ErrorReport {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ErrorReport {
    pub fn unclassified(message: impl Into<String>) -> Self {
        Self {
            code: UNCLASSIFIED.to_string(),
            message: message.into(),
            details: None,
        }
    }
}

impl RepositoryError {
    /// Stable kebab-case code. Explicit and exhaustive: never derived from the
    /// Rust variant name, so renaming a variant cannot change the wire (ADR-053).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not-found",
            Self::InstanceNotFound { .. } => "instance-not-found",
            Self::ManifestMissing { .. } => "manifest-missing",
            Self::PackageLoad { .. } => "package-load",
            Self::TypeNotFound { .. } => "type-not-found",
            Self::FieldNotFound { .. } => "field-not-found",
            Self::LifecycleNotFound { .. } => "lifecycle-not-found",
            Self::LifecycleIdMismatch { .. } => "lifecycle-id-mismatch",
            Self::LifecycleValidation { .. } => "lifecycle-validation",
            Self::RecordLoad { .. } => "record-load",
            Self::RecordWrite { .. } => "record-write",
            Self::RecordValidation { .. } => "record-validation",
            Self::RecordHasInboundRelations { .. } => "record-has-inbound-relations",
            Self::ManifestParse { .. } => "manifest-parse",
            Self::NoteLoad { .. } => "note-load",
            Self::NoteValidation { .. } => "note-validation",
            Self::NoteWrite { .. } => "note-write",
            Self::NoteNotFound { .. } => "note-not-found",
            Self::Io { .. } => "io",
            Self::Serialize { .. } => "serialize",
            Self::InstanceLoad { .. } => "instance-load",
            Self::RelationTypeDefinitionValidation { .. } => "relation-type-definition-validation",
            Self::SchemaValidation { .. } => "schema-validation",
            Self::RelationTypeDefinitionConflict { .. } => "relation-type-definition-conflict",
            Self::RelationValidation { .. } => "relation-validation",
            Self::RelationNotFound { .. } => "relation-not-found",
            Self::InvalidRelationId { .. } => "invalid-relation-id",
            Self::InvalidInstanceId { .. } => "invalid-instance-id",
            Self::RelationFilenameMismatch { .. } => "relation-filename-mismatch",
            Self::DuplicateRelationId { .. } => "duplicate-relation-id",
            Self::ContainerNotFound { .. } => "container-not-found",
            Self::ContainerAlreadyExists { .. } => "container-already-exists",
            Self::ContainerIsRepositoryRoot { .. } => "container-is-repository-root",
            Self::ContainerValidation { .. } => "container-validation",
            Self::InvalidValueType { .. } => "invalid-value-type",
            Self::ViewLoad { .. } => "view-load",
            Self::ViewValidation { .. } => "view-validation",
            Self::CompositionLoad { .. } => "composition-load",
            Self::CompositionValidation { .. } => "composition-validation",
            Self::ThemeLoad { .. } => "theme-load",
            Self::SourceDocumentMetaLoad { .. } => "source-document-meta-load",
            Self::ThemeValidation { .. } => "theme-validation",
            Self::CompositionNotFound { .. } => "composition-not-found",
            Self::ViewNotFound { .. } => "view-not-found",
            Self::ThemeNotFound { .. } => "theme-not-found",
            Self::BlueprintNotFound { .. } => "blueprint-not-found",
            Self::BlueprintValidation { .. } => "blueprint-validation",
            Self::InvalidPackageSelector { .. } => "invalid-package-selector",
            Self::CompositionNotFoundById { .. } => "composition-not-found-by-id",
            Self::PackageRefOutsideRepo { .. } => "package-ref-outside-repo",
            Self::PackageRefMissing { .. } => "package-ref-missing",
            Self::PackageRefConflict { .. } => "package-ref-conflict",
            Self::RepositoryAlreadyExists { .. } => "repository-already-exists",
            Self::InvalidRepositoryInitialization { .. } => "invalid-repository-initialization",
            Self::RepositoryNotEmpty { .. } => "repository-not-empty",
            Self::InvalidSnapshotData { .. } => "invalid-snapshot-data",
            Self::BatchSeamUnsupported { .. } => "batch-seam-unsupported",
            Self::InvalidArchive { .. } => "invalid-archive",
            Self::InvalidExportBundle { .. } => "invalid-export-bundle",
            Self::PackageNotFound { .. } => "package-not-found",
            Self::PackageAlreadyRegistered { .. } => "package-already-registered",
            Self::PackageInstallConflicts { .. } => "package-install-conflicts",
            Self::DefinitionNotFound { .. } => "definition-not-found",
            Self::CannotDeleteInUse { .. } => "cannot-delete-in-use",
            Self::TypeInheritanceCycle { .. } => "type-inheritance-cycle",
            Self::InheritedFieldDuplicate { .. } => "inherited-field-duplicate",
            Self::FieldOrderMismatch { .. } => "field-order-mismatch",
            Self::OverrideTargetsOwnField { .. } => "override-targets-own-field",
            Self::OverrideRelaxesRequired { .. } => "override-relaxes-required",
            Self::LifecycleNotDefined { .. } => "lifecycle-not-defined",
            Self::LifecycleTransitionNotAllowed { .. } => "lifecycle-transition-not-allowed",
            Self::LifecycleStateNotDefined { .. } => "lifecycle-state-not-defined",
            Self::LifecycleRelationRequired { .. } => "lifecycle-relation-required",
            Self::SuccessorRelationTypeUndetermined { .. } => {
                "successor-relation-type-undetermined"
            }
            Self::LifecycleFulfillmentNotApplicable { .. } => {
                "lifecycle-fulfillment-not-applicable"
            }
            Self::LifecycleFulfillmentRelationTypeMismatch { .. } => {
                "lifecycle-fulfillment-relation-type-mismatch"
            }
            Self::LifecycleStateUnreachable { .. } => "lifecycle-state-unreachable",
            Self::TypeVersionNotFound { .. } => "type-version-not-found",
            Self::VocabularyPromotionBlocked { .. } => "vocabulary-promotion-blocked",
            Self::InvalidInput { .. } => "invalid-input",
            Self::CorePackageConflict { .. } => "core-package-conflict",
            Self::RegistryLoad { .. } => "registry-load",
            Self::RegistryParse { .. } => "registry-parse",
            Self::RegistryEntryNotFound { .. } => "registry-entry-not-found",
            Self::RegistryIo { .. } => "registry-io",
            Self::RunInvalidState { .. } => "run-invalid-state",
            Self::CatalogLoad { .. } => "catalog-load",
            Self::CatalogUnsupported { .. } => "catalog-unsupported",
            Self::RetiredManifestProperty { .. } => "retired-manifest-property",
            Self::Rfc043MigrationNeeded { .. } => "rfc043-migration-needed",
            Self::ActorProvenance { code, .. } => code,
            Self::InvalidPackageBundle { code, .. } => code,
            Self::SliceRefused { code, .. } => code,
            Self::StorageGenerationUnsupported { .. } => "storage-generation-unsupported",
        }
    }

    pub fn report(&self) -> ErrorReport {
        let details = serde_json::to_value(self)
            .ok()
            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
        ErrorReport {
            code: self.code().to_string(),
            message: self.to_string(),
            details,
        }
    }
}

/// Message tail for `SuccessorRelationTypeUndetermined` (srs-rust#1238).
fn successor_undetermined_detail(candidates: &[String]) -> String {
    if candidates.is_empty() {
        "the predecessor's lifecycle declares no hard incoming requiresRelation; pass relationType explicitly".to_string()
    } else {
        format!("the predecessor's lifecycle declares several candidate relation types {candidates:?}; pass relationType explicitly")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn je() -> serde_json::Error {
        serde_json::from_str::<u8>("x").unwrap_err()
    }
    fn ioe() -> std::io::Error {
        std::io::Error::other("e")
    }
    fn ce() -> srs_core::error::CoreError {
        srs_core::error::CoreError::EmptyTag
    }

    /// One value of every variant. Keep in step with the exhaustive match in
    /// `RepositoryError::code`; `codes_are_kebab_case` fails if the counts drift.
    fn all_variants() -> Vec<RepositoryError> {
        use RepositoryError as R;
        vec![
            R::NotFound {
                path: PathBuf::new(),
            },
            R::InstanceNotFound { id: "x".into() },
            R::ManifestMissing {
                path: PathBuf::new(),
            },
            R::PackageLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::TypeNotFound {
                type_id: "x".into(),
                version: 1,
            },
            R::FieldNotFound {
                field_id: "x".into(),
            },
            R::LifecycleNotFound { id: "x".into() },
            R::LifecycleIdMismatch {
                argument_id: "x".into(),
                body_id: "x".into(),
            },
            R::LifecycleValidation { violations: vec![] },
            R::RecordLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::RecordWrite {
                path: PathBuf::new(),
                source: ioe(),
            },
            R::RecordValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::RecordHasInboundRelations {
                instance_id: "x".into(),
                count: 1,
                relations: vec![],
            },
            R::ManifestParse {
                path: PathBuf::new(),
                source: je(),
            },
            R::NoteLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::NoteValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::NoteWrite {
                path: PathBuf::new(),
                source: ioe(),
            },
            R::NoteNotFound {
                path: PathBuf::new(),
                id: "x".into(),
            },
            R::Io {
                path: PathBuf::new(),
                source: ioe(),
            },
            R::Serialize {
                path: PathBuf::new(),
                source: je(),
            },
            R::InstanceLoad {
                instance_id: "x".into(),
                path: PathBuf::new(),
                source: Box::new(ioe()),
            },
            R::RelationTypeDefinitionValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::SchemaValidation {
                path: PathBuf::new(),
                message: "x".into(),
            },
            R::RelationTypeDefinitionConflict {
                relation_type: "x".into(),
                path_a: PathBuf::new(),
                path_b: PathBuf::new(),
            },
            R::RelationValidation {
                relation_id: "x".into(),
                message: "x".into(),
            },
            R::RelationNotFound {
                relation_id: "x".into(),
            },
            R::InvalidRelationId {
                relation_id: "x".into(),
            },
            R::InvalidInstanceId {
                instance_id: "x".into(),
            },
            R::RelationFilenameMismatch {
                path: PathBuf::new(),
                file_relation_id: "x".into(),
            },
            R::DuplicateRelationId {
                relation_id: "x".into(),
                locators: vec![],
            },
            R::ContainerNotFound {
                container_id: "x".into(),
            },
            R::ContainerAlreadyExists {
                container_id: "x".into(),
            },
            R::ContainerIsRepositoryRoot {
                container_id: "x".into(),
            },
            R::ContainerValidation { source: ce() },
            R::InvalidValueType {
                path: PathBuf::new(),
                value_type: "x".into(),
            },
            R::ViewLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::ViewValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::CompositionLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::CompositionValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::ThemeLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::SourceDocumentMetaLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::ThemeValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::CompositionNotFound {
                view_id: "x".into(),
            },
            R::ViewNotFound {
                view_id: "x".into(),
            },
            R::ThemeNotFound {
                theme_id: "x".into(),
            },
            R::BlueprintNotFound {
                blueprint_id: "x".into(),
            },
            R::BlueprintValidation {
                path: PathBuf::new(),
                source: ce(),
            },
            R::InvalidPackageSelector {
                message: "x".into(),
            },
            R::CompositionNotFoundById {
                composition_id: "x".into(),
            },
            R::PackageRefOutsideRepo { path: "x".into() },
            R::PackageRefMissing { path: "x".into() },
            R::PackageRefConflict {
                path: "x".into(),
                kind: "x".into(),
                id: "x".into(),
                first_path: PathBuf::new(),
                second_path: PathBuf::new(),
            },
            R::RepositoryAlreadyExists {
                path: PathBuf::new(),
            },
            R::InvalidRepositoryInitialization {
                message: "x".into(),
            },
            R::RepositoryNotEmpty {
                path: PathBuf::new(),
            },
            R::InvalidSnapshotData {
                message: "x".into(),
            },
            R::BatchSeamUnsupported { store: "x".into() },
            R::InvalidArchive {
                message: "x".into(),
            },
            R::InvalidExportBundle {
                message: "x".into(),
            },
            R::PackageNotFound { selector: None },
            R::PackageAlreadyRegistered { id: "x".into() },
            R::PackageInstallConflicts {
                count: 1,
                keys: "x".into(),
            },
            R::DefinitionNotFound { id: "x".into() },
            R::CannotDeleteInUse {
                entity_type: "x".into(),
                id: "x".into(),
                used_by: vec![],
            },
            R::TypeInheritanceCycle {
                type_id: "x".into(),
            },
            R::InheritedFieldDuplicate {
                type_id: "x".into(),
                base_type_id: "x".into(),
                field_id: "x".into(),
            },
            R::FieldOrderMismatch {
                type_id: "x".into(),
                field_id: "x".into(),
            },
            R::OverrideTargetsOwnField {
                type_id: "x".into(),
                field_id: "x".into(),
            },
            R::OverrideRelaxesRequired {
                type_id: "x".into(),
                field_id: "x".into(),
            },
            R::LifecycleNotDefined { id: "x".into() },
            R::LifecycleTransitionNotAllowed {
                from: "x".into(),
                to: "x".into(),
            },
            R::LifecycleStateNotDefined { state: "x".into() },
            R::LifecycleRelationRequired {
                state: "x".into(),
                relation_types: vec![],
                direction: "x".into(),
            },
            R::SuccessorRelationTypeUndetermined { candidates: vec![] },
            R::LifecycleFulfillmentNotApplicable { state: "x".into() },
            R::LifecycleFulfillmentRelationTypeMismatch {
                state: "x".into(),
                relation_type: "x".into(),
                declared: vec![],
            },
            R::LifecycleStateUnreachable {
                state: "x".into(),
                initial: "x".into(),
            },
            R::TypeVersionNotFound {
                type_id: "x".into(),
                version: 1,
            },
            R::VocabularyPromotionBlocked {
                vocabulary_id: "x".into(),
                unresolvable_keys: vec![],
            },
            R::InvalidInput {
                message: "x".into(),
            },
            R::CorePackageConflict {
                kind: "x".into(),
                id: "x".into(),
                qualified_name: "x".into(),
            },
            R::RegistryLoad {
                path: PathBuf::new(),
                source: je(),
            },
            R::RegistryParse { source: je() },
            R::RegistryEntryNotFound {
                package_name: "x".into(),
            },
            R::RegistryIo {
                path: PathBuf::new(),
                message: "x".into(),
            },
            R::RunInvalidState {
                run_id: "x".into(),
                message: "x".into(),
            },
            R::CatalogLoad {
                fatal: 1,
                first: "x".into(),
                diagnostics: vec![],
            },
            R::CatalogUnsupported,
            R::RetiredManifestProperty {
                property: "x".into(),
            },
            R::Rfc043MigrationNeeded,
            R::ActorProvenance {
                code: "actor-x",
                message: "x".into(),
            },
            R::InvalidPackageBundle {
                code: "bundle-x",
                message: "x".into(),
            },
            R::SliceRefused {
                code: "slice-y",
                message: "x".into(),
            },
            R::StorageGenerationUnsupported { declared: 1 },
        ]
    }

    #[test]
    fn codes_are_kebab_case() {
        let all = all_variants();
        assert_eq!(all.len(), 94, "all_variants() is out of step with the enum");
        let mut seen = std::collections::HashSet::new();
        for e in &all {
            let c = e.code();
            assert!(
                !c.is_empty()
                    && c.split('-').all(|p| {
                        !p.is_empty()
                            && p.bytes()
                                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    }),
                "not kebab-case: {c}"
            );
            assert!(seen.insert(c), "duplicate code {c}");
        }
    }

    #[test]
    fn report_carries_details() {
        let r = RepositoryError::CannotDeleteInUse {
            entity_type: "type".into(),
            id: "x".into(),
            used_by: vec!["a".into(), "b".into()],
        }
        .report();
        assert_eq!(r.code, "cannot-delete-in-use");
        assert_eq!(r.details.unwrap()["usedBy"], serde_json::json!(["a", "b"]));

        let r = RepositoryError::SuccessorRelationTypeUndetermined {
            candidates: vec!["supersedes".into()],
        }
        .report();
        assert_eq!(
            r.details.unwrap()["candidates"],
            serde_json::json!(["supersedes"])
        );

        let r = RepositoryError::ActorProvenance {
            code: "actor-supplied",
            message: "m".into(),
        }
        .report();
        assert_eq!(r.code, "actor-supplied");
        assert_eq!(r.message, "m");
        assert!(r.details.is_none_or(|d| d.get("code").is_none()));

        assert!(RepositoryError::CatalogUnsupported
            .report()
            .details
            .is_none());
    }

    #[test]
    fn display_has_no_code_prefix() {
        use RepositoryError as R;
        let msgs = [
            R::RecordHasInboundRelations {
                instance_id: "x".into(),
                count: 1,
                relations: vec![],
            },
            R::LifecycleRelationRequired {
                state: "s".into(),
                relation_types: vec![],
                direction: "d".into(),
            },
            R::SuccessorRelationTypeUndetermined { candidates: vec![] },
            R::LifecycleFulfillmentNotApplicable { state: "s".into() },
            R::LifecycleFulfillmentRelationTypeMismatch {
                state: "s".into(),
                relation_type: "r".into(),
                declared: vec![],
            },
            R::LifecycleStateUnreachable {
                state: "s".into(),
                initial: "i".into(),
            },
        ]
        .map(|e| e.to_string());
        for m in msgs {
            let head = m.split(':').next().unwrap();
            assert!(
                !head.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "{m}"
            );
        }
        let m = RepositoryError::SliceRefused {
            code: "slice-x",
            message: "msg".into(),
        }
        .to_string();
        assert_eq!(m, "msg");
    }
}
