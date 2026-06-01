use jsonschema::Validator;
use serde_json::Value;
use std::sync::OnceLock;
use thiserror::Error;

pub const CONTAINER_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/container.json";
pub const DOCUMENT_VIEW_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/document-view.json";
pub const DOCUMENT_VIEW_OUTPUT_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/document-view-output.json";
pub const FIELD_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/field.json";
pub const FEDERATION_EVENTS_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/federation-events.json";
pub const FEDERATION_REGISTRY_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/federation-registry.json";
pub const MANIFEST_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/manifest.json";
pub const NOTE_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/note.json";
pub const PACKAGE_BUNDLE_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/package-bundle.json";
pub const PACKAGE_MANIFEST_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/package-manifest.json";
pub const RECORD_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/record.json";
pub const RELATION_TYPE_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/relation-type.json";
pub const RELATIONS_COLLECTION_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/relations-collection.json";
pub const SOURCE_DOCUMENT_META_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/source-document-meta.json";
pub const THEME_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/theme.json";
pub const TYPE_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/type.json";
pub const TYPED_RECORD_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/typed-record.json";
pub const VIEW_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/view.json";

pub const ALL_SCHEMA_IDS: &[&str] = &[
    CONTAINER_SCHEMA_ID,
    DOCUMENT_VIEW_SCHEMA_ID,
    DOCUMENT_VIEW_OUTPUT_SCHEMA_ID,
    FIELD_SCHEMA_ID,
    FEDERATION_EVENTS_SCHEMA_ID,
    FEDERATION_REGISTRY_SCHEMA_ID,
    MANIFEST_SCHEMA_ID,
    NOTE_SCHEMA_ID,
    PACKAGE_BUNDLE_SCHEMA_ID,
    PACKAGE_MANIFEST_SCHEMA_ID,
    RECORD_SCHEMA_ID,
    RELATION_TYPE_SCHEMA_ID,
    RELATIONS_COLLECTION_SCHEMA_ID,
    SOURCE_DOCUMENT_META_SCHEMA_ID,
    THEME_SCHEMA_ID,
    TYPE_SCHEMA_ID,
    TYPED_RECORD_SCHEMA_ID,
    VIEW_SCHEMA_ID,
];

macro_rules! include_schema {
    ($filename:literal) => {
        include_str!(concat!("../schemas/2.0/", $filename))
    };
}

static SCHEMA_SOURCES: &[(&str, &str)] = &[
    (CONTAINER_SCHEMA_ID, include_schema!("container.json")),
    (
        DOCUMENT_VIEW_SCHEMA_ID,
        include_schema!("document-view.json"),
    ),
    (
        DOCUMENT_VIEW_OUTPUT_SCHEMA_ID,
        include_schema!("document-view-output.json"),
    ),
    (FIELD_SCHEMA_ID, include_schema!("field.json")),
    (
        FEDERATION_EVENTS_SCHEMA_ID,
        include_schema!("federation-events.json"),
    ),
    (
        FEDERATION_REGISTRY_SCHEMA_ID,
        include_schema!("federation-registry.json"),
    ),
    (MANIFEST_SCHEMA_ID, include_schema!("manifest.json")),
    (NOTE_SCHEMA_ID, include_schema!("note.json")),
    (
        PACKAGE_BUNDLE_SCHEMA_ID,
        include_schema!("package-bundle.json"),
    ),
    (
        PACKAGE_MANIFEST_SCHEMA_ID,
        include_schema!("package-manifest.json"),
    ),
    (RECORD_SCHEMA_ID, include_schema!("record.json")),
    (
        RELATION_TYPE_SCHEMA_ID,
        include_schema!("relation-type.json"),
    ),
    (
        RELATIONS_COLLECTION_SCHEMA_ID,
        include_schema!("relations-collection.json"),
    ),
    (
        SOURCE_DOCUMENT_META_SCHEMA_ID,
        include_schema!("source-document-meta.json"),
    ),
    (THEME_SCHEMA_ID, include_schema!("theme.json")),
    (TYPE_SCHEMA_ID, include_schema!("type.json")),
    (TYPED_RECORD_SCHEMA_ID, include_schema!("typed-record.json")),
    (VIEW_SCHEMA_ID, include_schema!("view.json")),
];

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("schema id not registered: {0}")]
    UnknownSchemaId(String),

    #[error("instance does not declare $schema")]
    MissingDeclaredSchema,

    #[error("validation failed for schema {schema_id}: {errors}")]
    ValidationFailed { schema_id: String, errors: String },
}

pub type SchemaResult<T> = Result<T, SchemaError>;

struct CompiledEntry {
    schema_id: &'static str,
    validator: Validator,
}

pub struct SchemaRegistry {
    entries: Vec<CompiledEntry>,
}

impl SchemaRegistry {
    fn build() -> Self {
        let mut entries = Vec::with_capacity(SCHEMA_SOURCES.len());
        for (schema_id, src) in SCHEMA_SOURCES {
            let schema_value: Value = serde_json::from_str(src)
                .unwrap_or_else(|e| panic!("srs-schema: failed to parse {schema_id}: {e}"));
            let validator = jsonschema::validator_for(&schema_value)
                .unwrap_or_else(|e| panic!("srs-schema: failed to compile {schema_id}: {e}"));
            entries.push(CompiledEntry {
                schema_id,
                validator,
            });
        }
        SchemaRegistry { entries }
    }

    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<SchemaRegistry> = OnceLock::new();
        INSTANCE.get_or_init(SchemaRegistry::build)
    }

    pub fn schema_ids(&self) -> &[&'static str] {
        ALL_SCHEMA_IDS
    }

    pub fn validate_by_id(&self, schema_id: &str, value: &Value) -> SchemaResult<()> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.schema_id == schema_id)
            .ok_or_else(|| SchemaError::UnknownSchemaId(schema_id.to_string()))?;

        let errors: Vec<String> = entry
            .validator
            .iter_errors(value)
            .map(|e| format!("[{}] {}", e.instance_path, e))
            .collect();
        if !errors.is_empty() {
            return Err(SchemaError::ValidationFailed {
                schema_id: schema_id.to_string(),
                errors: errors.join("; "),
            });
        }
        Ok(())
    }

    pub fn validate_declared_schema(&self, value: &Value) -> SchemaResult<&'static str> {
        let declared = value
            .get("$schema")
            .and_then(|v| v.as_str())
            .ok_or(SchemaError::MissingDeclaredSchema)?;

        let entry = self
            .entries
            .iter()
            .find(|e| e.schema_id == declared)
            .ok_or_else(|| SchemaError::UnknownSchemaId(declared.to_string()))?;

        let errors: Vec<String> = entry
            .validator
            .iter_errors(value)
            .map(|e| format!("[{}] {}", e.instance_path, e))
            .collect();
        if !errors.is_empty() {
            return Err(SchemaError::ValidationFailed {
                schema_id: entry.schema_id.to_string(),
                errors: errors.join("; "),
            });
        }
        Ok(entry.schema_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_builds_and_has_all_schema_ids() {
        let reg = SchemaRegistry::global();
        assert_eq!(reg.schema_ids().len(), 18);
        for id in ALL_SCHEMA_IDS {
            assert!(reg.schema_ids().contains(id), "missing: {id}");
        }
    }

    #[test]
    fn valid_note_passes() {
        let reg = SchemaRegistry::global();
        let note = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
            "instanceId": "00000000-0000-4000-8000-000000000001",
            "sections": [{"name": "body", "content": "Hello"}]
        });
        reg.validate_by_id(NOTE_SCHEMA_ID, &note).unwrap();
    }

    #[test]
    fn note_missing_sections_fails() {
        let reg = SchemaRegistry::global();
        let note = json!({
            "instanceId": "00000000-0000-4000-8000-000000000001"
        });
        assert!(reg.validate_by_id(NOTE_SCHEMA_ID, &note).is_err());
    }

    #[test]
    fn valid_record_passes() {
        let reg = SchemaRegistry::global();
        let record = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": "00000000-0000-4000-8000-000000000002",
            "typeId": "00000000-0000-4000-8000-000000000003",
            "typeVersion": 1,
            "typeNamespace": "test.ns",
            "typeName": "test-type",
            "fieldValues": []
        });
        reg.validate_by_id(RECORD_SCHEMA_ID, &record).unwrap();
    }

    #[test]
    fn record_missing_type_namespace_fails() {
        let reg = SchemaRegistry::global();
        let record = json!({
            "instanceId": "00000000-0000-4000-8000-000000000002",
            "typeId": "00000000-0000-4000-8000-000000000003",
            "typeVersion": 1,
            "fieldValues": []
        });
        assert!(reg.validate_by_id(RECORD_SCHEMA_ID, &record).is_err());
    }

    #[test]
    fn validate_declared_schema_uses_dollar_schema_field() {
        let reg = SchemaRegistry::global();
        let note = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
            "instanceId": "00000000-0000-4000-8000-000000000001",
            "sections": [{"name": "body", "content": "Hello"}]
        });
        let schema_id = reg.validate_declared_schema(&note).unwrap();
        assert_eq!(schema_id, NOTE_SCHEMA_ID);
    }

    #[test]
    fn missing_dollar_schema_returns_error() {
        let reg = SchemaRegistry::global();
        let val = json!({"instanceId": "00000000-0000-4000-8000-000000000001", "sections": []});
        assert!(matches!(
            reg.validate_declared_schema(&val),
            Err(SchemaError::MissingDeclaredSchema)
        ));
    }

    #[test]
    fn unknown_schema_id_returns_error() {
        let reg = SchemaRegistry::global();
        let val = json!({});
        assert!(matches!(
            reg.validate_by_id("https://example.com/unknown.json", &val),
            Err(SchemaError::UnknownSchemaId(_))
        ));
    }

    #[test]
    fn valid_field_passes() {
        let reg = SchemaRegistry::global();
        let field = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test",
            "name": "summary",
            "version": 1,
            "description": "A short summary",
            "aiGuidance": {"purpose": "captures the summary"},
            "valueType": "text",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        reg.validate_by_id(FIELD_SCHEMA_ID, &field).unwrap();
    }

    #[test]
    fn valid_type_passes() {
        let reg = SchemaRegistry::global();
        let t = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
            "id": "00000000-0000-4000-8000-000000000020",
            "namespace": "test",
            "name": "decision",
            "version": 1,
            "description": "A decision record type",
            "fields": [
                {"fieldId": "00000000-0000-4000-8000-000000000010", "order": 0, "required": true}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        reg.validate_by_id(TYPE_SCHEMA_ID, &t).unwrap();
    }
}
