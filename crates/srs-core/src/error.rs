use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("duplicate section name: {name}")]
    DuplicateSectionName { name: String },

    #[error("empty tag not allowed")]
    EmptyTag,

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("missing required field key: {key}")]
    MissingRequiredField { key: String },

    /// RFC-039 [R1]: a `fieldValues` key that names no Field in the Type's
    /// effective field set.
    #[error(
        "unknown fieldValues key '{key}': no Field of that name in the Type's effective field set"
    )]
    UnknownFieldKey { key: String },

    #[error("tag key must be non-empty")]
    EmptyTagKey,

    #[error("invalid relation type: {relation_type}")]
    InvalidRelationType { relation_type: String },

    #[error("invalid field value for {key}: {reason}")]
    InvalidFieldValue { key: String, reason: String },

    /// RFC-039 [R3]/[R16]: the value at a key does not conform to the Change-B
    /// rule for that Field's `fieldType`.
    #[error("value shape mismatch at '{key}': expected {expected}, got {got}")]
    ValueShape {
        key: String,
        expected: String,
        got: String,
    },

    /// RFC-039 [R5]: `null` is not a value — key absence is the sole
    /// representation of "unset".
    #[error("null value at '{key}': omit the key instead (RFC-039 [R5])")]
    NullFieldValue { key: String },

    /// RFC-039 [R6]: a `fieldMeta` key with no corresponding `fieldValues` key.
    #[error("fieldMeta key '{key}' has no corresponding fieldValues key (RFC-039 [R6])")]
    FieldMetaUnknownKey { key: String },

    /// RFC-039 [R14]: a `mode: "reference"` value whose target is not in the
    /// repository's instanceIndex.
    #[error("dangling reference at '{key}': instance {target} not found in instanceIndex (RFC-039 [R14])")]
    DanglingReference { key: String, target: String },

    /// RFC-039 [R14]: a `mode: "reference"` target of the wrong Type/version.
    #[error("reference type mismatch at '{key}': instance {target} is not a {expected_type}@{expected_version} (RFC-039 [R14])")]
    ReferenceTypeMismatch {
        key: String,
        target: String,
        expected_type: String,
        expected_version: u32,
    },

    /// RFC-039 [R4]: two Fields in one Type's effective field set share a name.
    #[error("duplicate Field.name '{name}' in the Type's effective field set (RFC-039 [R4])")]
    DuplicateEffectiveFieldName { name: String },

    /// RFC-039 [R7]: a removed construct present in a `dataModelRevision >= 2`
    /// document.
    #[error("removed construct '{construct}' in {location}: rejected at dataModelRevision >= 2 (RFC-039 [R7])")]
    RemovedConstruct { construct: String, location: String },

    /// RFC-039 [R15]: a `dataModelRevision >= 2` manifest declaring a retired
    /// extension.
    #[error("retired extension '{extension}' declared: ext:field-groups and ext:repeatable-fields are removed at dataModelRevision >= 2 (RFC-039 [R15])")]
    RetiredExtensionDeclared { extension: String },

    /// RFC-039 [R9]: a document of a generation this reader does not support.
    #[error("unsupported data-model generation in {document}: expected dataModelRevision {expected_revision} (RFC-039 [R9]); migrate with `srs repo apply-migration --id rfc039-carrier`")]
    UnsupportedGeneration {
        document: String,
        expected_revision: u32,
    },

    #[error("document view must contain at least one section")]
    EmptyDocumentViewSections,

    #[error("duplicate document section id: {section_id}")]
    DuplicateDocumentSectionId { section_id: String },

    #[error("duplicate field view id: {field_id}")]
    DuplicateFieldViewId { field_id: String },

    #[error("view must contain at least one field view")]
    EmptyViewFieldViews,

    #[error("duplicate theme variant name: {name}")]
    DuplicateThemeVariantName { name: String },

    #[error("theme must declare at least one target")]
    ThemeTargetsEmpty,

    #[error("duplicate theme section wrapper override id: {section_id}")]
    DuplicateThemeSectionOverrideId { section_id: String },

    #[error("duplicate theme record wrapper override type id: {type_id}")]
    DuplicateThemeRecordOverrideTypeId { type_id: String },

    /// Record.tags value must be a non-empty string.
    #[error("invalid tag value '{tag}': tag strings must be non-empty")]
    InvalidTagValue { tag: String },

    // ── ext:lifecycle errors ──────────────────────────────────────────────────
    /// Invariant 6: Record.lifecycleState names a state not in the Type's lifecycle.
    #[error("invalid lifecycle state '{state}': not defined in the Type's lifecycle")]
    InvalidLifecycleState { state: String },

    /// Invariant 4: Type.lifecycle.initialState does not reference a state with isInitial: true.
    #[error("invalid lifecycle initialState '{state}': must name a state with isInitial: true")]
    InvalidLifecycleInitialState { state: String },

    /// Invariant 5: A transition from/to references a state name not in lifecycle.states[].
    #[error("invalid lifecycle transition '{transition_name}': state '{state}' is not defined")]
    InvalidLifecycleTransitionState {
        state: String,
        transition_name: String,
    },

    // ── ext:cross-field-validation errors ─────────────────────────────────────
    #[error("cross-field rule (conditional-required): field '{target_field_id}' is required when field '{predicate_field_id}' equals '{predicate_value}'")]
    CrossFieldConditionalRequired {
        predicate_field_id: String,
        predicate_value: String,
        target_field_id: String,
    },

    #[error("cross-field rule (field-ordering): field '{target_field_id}' must {effect} field '{predicate_field_id}'")]
    CrossFieldOrdering {
        target_field_id: String,
        effect: String,
        predicate_field_id: String,
    },

    /// RFC-040 Change F (srs#477/#486): the if/then/not counterpart to
    /// `CrossFieldConditionalRequired` — the target field must be ABSENT when the
    /// predicate holds.
    #[error("cross-field rule (conditional-forbidden): field '{target_field_id}' is forbidden when field '{predicate_field_id}' equals '{predicate_value}'")]
    CrossFieldConditionalForbidden {
        predicate_field_id: String,
        predicate_value: String,
        target_field_id: String,
    },

    #[error("cross-field rule (mutual-exclusion): at most one of [{field_ids}] may have a non-empty value")]
    CrossFieldMutualExclusion { field_ids: String },

    #[error("cross-field rule misconfigured: {reason}")]
    CrossFieldRuleMisconfigured { reason: String },
}

impl PartialEq for CoreError {
    fn eq(&self, other: &Self) -> bool {
        // Every variant's payload is fully rendered in its Display message, so
        // discriminant + message equality is exactly payload equality (and the
        // only comparison available for the non-PartialEq serde_json::Error).
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.to_string() == other.to_string()
    }
}
