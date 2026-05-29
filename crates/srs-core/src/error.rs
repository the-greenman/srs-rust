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
            _ => false,
        }
    }
}
