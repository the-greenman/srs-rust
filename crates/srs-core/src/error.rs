use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("duplicate section name: {name}")]
    DuplicateSectionName { name: String },

    #[error("empty tag not allowed")]
    EmptyTag,

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
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
            _ => false,
        }
    }
}
