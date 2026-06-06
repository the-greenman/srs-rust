use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("duplicate section name: {name}")]
    DuplicateSectionName { name: String },

    #[error("empty tag not allowed")]
    EmptyTag,

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("missing required field: {field_id}")]
    MissingRequiredField { field_id: String },

    #[error("unknown field in record: {field_id}")]
    UnknownField { field_id: String },

    #[error("tag key must be non-empty")]
    EmptyTagKey,

    #[error("invalid relation type: {relation_type}")]
    InvalidRelationType { relation_type: String },

    #[error("invalid field value for {field_id}: {reason}")]
    InvalidFieldValue { field_id: String, reason: String },

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

    #[error("repeatable field {field_id} has {count} entries but minItems is {min}")]
    TooFewEntries {
        field_id: String,
        count: usize,
        min: u32,
    },

    #[error("repeatable field {field_id} has {count} entries but maxItems is {max}")]
    TooManyEntries {
        field_id: String,
        count: usize,
        max: u32,
    },

    #[error("non-repeatable field {field_id} must use `value`, not `entries`")]
    EntriesOnNonRepeatableField { field_id: String },

    #[error("required field group {group_id} is missing from record")]
    MissingRequiredFieldGroup { group_id: String },

    #[error("field group {group_id} has {count} entries but minItems is {min}")]
    TooFewGroupEntries {
        group_id: String,
        count: usize,
        min: u32,
    },

    #[error("field group {group_id} has {count} entries but maxItems is {max}")]
    TooManyGroupEntries {
        group_id: String,
        count: usize,
        max: u32,
    },

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
}

impl PartialEq for CoreError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                CoreError::DuplicateSectionName { name: a },
                CoreError::DuplicateSectionName { name: b },
            ) => a == b,
            (CoreError::EmptyTag, CoreError::EmptyTag) => true,
            // Json errors compared by their display representation
            (CoreError::Json(a), CoreError::Json(b)) => a.to_string() == b.to_string(),
            (
                CoreError::MissingRequiredField { field_id: a },
                CoreError::MissingRequiredField { field_id: b },
            ) => a == b,
            (CoreError::UnknownField { field_id: a }, CoreError::UnknownField { field_id: b }) => {
                a == b
            }
            (CoreError::EmptyTagKey, CoreError::EmptyTagKey) => true,
            (
                CoreError::InvalidRelationType { relation_type: a },
                CoreError::InvalidRelationType { relation_type: b },
            ) => a == b,
            (
                CoreError::InvalidFieldValue {
                    field_id: af,
                    reason: ar,
                },
                CoreError::InvalidFieldValue {
                    field_id: bf,
                    reason: br,
                },
            ) => af == bf && ar == br,
            (CoreError::EmptyDocumentViewSections, CoreError::EmptyDocumentViewSections) => true,
            (
                CoreError::DuplicateDocumentSectionId { section_id: a },
                CoreError::DuplicateDocumentSectionId { section_id: b },
            ) => a == b,
            (
                CoreError::DuplicateFieldViewId { field_id: a },
                CoreError::DuplicateFieldViewId { field_id: b },
            ) => a == b,
            (CoreError::EmptyViewFieldViews, CoreError::EmptyViewFieldViews) => true,
            (
                CoreError::DuplicateThemeVariantName { name: a },
                CoreError::DuplicateThemeVariantName { name: b },
            ) => a == b,
            (CoreError::ThemeTargetsEmpty, CoreError::ThemeTargetsEmpty) => true,
            (
                CoreError::DuplicateThemeSectionOverrideId { section_id: a },
                CoreError::DuplicateThemeSectionOverrideId { section_id: b },
            ) => a == b,
            (
                CoreError::DuplicateThemeRecordOverrideTypeId { type_id: a },
                CoreError::DuplicateThemeRecordOverrideTypeId { type_id: b },
            ) => a == b,
            (
                CoreError::TooFewEntries {
                    field_id: af,
                    count: ac,
                    min: am,
                },
                CoreError::TooFewEntries {
                    field_id: bf,
                    count: bc,
                    min: bm,
                },
            ) => af == bf && ac == bc && am == bm,
            (
                CoreError::TooManyEntries {
                    field_id: af,
                    count: ac,
                    max: am,
                },
                CoreError::TooManyEntries {
                    field_id: bf,
                    count: bc,
                    max: bm,
                },
            ) => af == bf && ac == bc && am == bm,
            (
                CoreError::EntriesOnNonRepeatableField { field_id: a },
                CoreError::EntriesOnNonRepeatableField { field_id: b },
            ) => a == b,
            (
                CoreError::MissingRequiredFieldGroup { group_id: a },
                CoreError::MissingRequiredFieldGroup { group_id: b },
            ) => a == b,
            (
                CoreError::TooFewGroupEntries {
                    group_id: ag,
                    count: ac,
                    min: am,
                },
                CoreError::TooFewGroupEntries {
                    group_id: bg,
                    count: bc,
                    min: bm,
                },
            ) => ag == bg && ac == bc && am == bm,
            (
                CoreError::TooManyGroupEntries {
                    group_id: ag,
                    count: ac,
                    max: am,
                },
                CoreError::TooManyGroupEntries {
                    group_id: bg,
                    count: bc,
                    max: bm,
                },
            ) => ag == bg && ac == bc && am == bm,
            (
                CoreError::InvalidLifecycleState { state: a },
                CoreError::InvalidLifecycleState { state: b },
            ) => a == b,
            (
                CoreError::InvalidLifecycleInitialState { state: a },
                CoreError::InvalidLifecycleInitialState { state: b },
            ) => a == b,
            (
                CoreError::InvalidLifecycleTransitionState {
                    state: sa,
                    transition_name: ta,
                },
                CoreError::InvalidLifecycleTransitionState {
                    state: sb,
                    transition_name: tb,
                },
            ) => sa == sb && ta == tb,
            (CoreError::InvalidTagValue { tag: a }, CoreError::InvalidTagValue { tag: b }) => {
                a == b
            }
            _ => false,
        }
    }
}
