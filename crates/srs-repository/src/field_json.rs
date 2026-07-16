use crate::error::RepositoryError;
use srs_core::types::field::{Field, ValueType};
use std::collections::HashMap;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldJson {
    pub(crate) id: String,
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) version: u32,
    pub(crate) value_type: String,
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    pub(crate) ai_guidance: Option<serde_json::Value>,
    pub(crate) allowed_values: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) vocabulary_ref: Option<String>,
    pub(crate) default_value: Option<serde_json::Value>,
    pub(crate) created_at: Option<String>,
    #[serde(flatten)]
    pub(crate) extra_fields: HashMap<String, serde_json::Value>,
}

impl FieldJson {
    pub(crate) fn into_field(self, path: &std::path::Path) -> Result<Field, RepositoryError> {
        Ok(Field {
            id: self.id,
            namespace: self.namespace,
            name: self.name,
            version: self.version,
            value_type: parse_value_type(&self.value_type, path)?,
            description: self.description.unwrap_or_default(),
            instructions: self.instructions,
            ai_guidance: self.ai_guidance.unwrap_or(serde_json::Value::Null),
            allowed_values: self.allowed_values,
            vocabulary_ref: self.vocabulary_ref,
            default_value: self.default_value,
            created_at: self.created_at.unwrap_or_default(),
            extra: self.extra_fields,
        })
    }
}

pub(crate) fn parse_value_type(
    s: &str,
    path: &std::path::Path,
) -> Result<ValueType, RepositoryError> {
    match s {
        "string" => Ok(ValueType::String),
        "text" => Ok(ValueType::Text),
        "number" => Ok(ValueType::Number),
        "boolean" => Ok(ValueType::Boolean),
        "date" => Ok(ValueType::Date),
        "url" => Ok(ValueType::Url),
        "select" => Ok(ValueType::Select),
        "multiselect" => Ok(ValueType::Multiselect),
        _ => Err(RepositoryError::InvalidValueType {
            path: path.to_path_buf(),
            value_type: s.to_string(),
        }),
    }
}
