use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// AI-facing guidance for a Field: what it captures, how to extract or
/// populate it, what to avoid, and worked examples. Mirrors `$defs/AiGuidance`
/// in `field.json`. `purpose` is the schema's only required sub-property.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGuidance {
    #[serde(default)]
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_guidance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<AiGuidanceExample>>,
}

/// One worked example in `AiGuidance.examples`. Mirrors `$defs/AiGuidanceExample`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGuidanceExample {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    pub output: String,
}

/// Distribution/fork tracking for a Field. Mirrors `$defs/Lineage`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_definition_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_definition_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_version: Option<u32>,
}

/// Publisher/package attribution for a Field. Mirrors `$defs/Provenance`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
}

/// Content encoding of a `string`/`text` value. See I-38 and
/// [`Field::effective_content_format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentFormat {
    Plain,
    Markdown,
}

/// Suggested UI control for editing this Field's value. Implementations and
/// Views may override; this is a hint, not a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditorHint {
    Singleline,
    Textarea,
    RichText,
    DatePicker,
    Dropdown,
    MultiSelect,
    Voice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    // Required by field.json (`required: ["id", ...]`) — no `#[serde(default)]`.
    // A document with no `id` is malformed and must fail to deserialize, not
    // silently produce a Field with `id: ""` (srs-rust#769).
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub ai_guidance: AiGuidance,
    pub value_type: ValueType,
    // Meaningful only when `value_type` is `String`/`Text` (I-38) — use
    // `effective_content_format()` rather than reading this directly.
    //
    // Property name intentionally kept as `contentFormat` (not the JSON
    // Schema standard `contentMediaType`): a rename was floated in
    // srs-rust#769's discussion but is a spec-level decision that belongs to
    // the-greenman/srs#234, not decided unilaterally here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_format: Option<ContentFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocabulary_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_hint: Option<EditorHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<Lineage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    // Decision (srs-rust#769): `id` and `created_at` stay plain `String` for
    // now rather than gaining parsed newtypes. `format: "uuid"`/`"date-time"`
    // in field.json are annotations only until srs-schema opts into format
    // assertion (the-greenman/srs#236); a Rust-side newtype would assert a
    // guarantee the schema itself does not yet give. Revisit once #236 lands.
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Field {
    /// I-38 (spec section "core — Field.contentFormat"): `contentFormat` is
    /// only meaningful when `valueType` is `string` or `text`; implementations
    /// must ignore it for every other `valueType`. Consumers that want to
    /// honour `contentFormat` (e.g. a renderer choosing markdown vs
    /// plain-text treatment) must go through this accessor instead of
    /// reading `self.content_format` directly, so the invariant cannot be
    /// silently bypassed by a value type that shouldn't carry one.
    pub fn effective_content_format(&self) -> Option<ContentFormat> {
        match self.value_type {
            ValueType::String | ValueType::Text => self.content_format,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    String,
    Text,
    Number,
    Boolean,
    Date,
    Url,
    Select,
    Multiselect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn field_roundtrips_json() {
        let field = Field {
            id: "00000000-0000-4000-8000-000000000010".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-field".to_string(),
            version: 1,
            description: "A test field".to_string(),
            instructions: None,
            ai_guidance: AiGuidance {
                purpose: "captures test data".to_string(),
                ..Default::default()
            },
            value_type: ValueType::Select,
            content_format: None,
            allowed_values: Some(vec!["a".to_string(), "b".to_string()]),
            vocabulary_ref: None,
            default_value: Some(json!("a")),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let json_str = serde_json::to_string(&field).unwrap();
        let parsed: Field = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, field.id);
        assert_eq!(parsed.value_type, ValueType::Select);
        assert_eq!(parsed.ai_guidance.purpose, "captures test data");
        assert_eq!(
            parsed.allowed_values,
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn field_extra_fields_survive_roundtrip() {
        let json_str = r#"{
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test.ns",
            "name": "test-field",
            "version": 1,
            "description": "A field",
            "aiGuidance": {"purpose": "test"},
            "valueType": "string",
            "createdAt": "2026-01-01T00:00:00Z",
            "unknownFutureField": "preserved",
            "anotherExtra": 42
        }"#;

        let field: Field = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            field.extra.get("unknownFutureField"),
            Some(&json!("preserved"))
        );
        assert_eq!(field.extra.get("anotherExtra"), Some(&json!(42)));

        let serialized = serde_json::to_string(&field).unwrap();
        assert!(serialized.contains("unknownFutureField"));
        assert!(serialized.contains("anotherExtra"));
    }

    #[test]
    fn value_type_serializes_to_lowercase() {
        assert_eq!(
            serde_json::to_string(&ValueType::String).unwrap(),
            "\"string\""
        );
        assert_eq!(
            serde_json::to_string(&ValueType::Multiselect).unwrap(),
            "\"multiselect\""
        );
    }

    #[test]
    fn minimal_field_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let field = Field {
            id: "00000000-0000-4000-8000-000000000010".to_string(),
            namespace: "test".to_string(),
            name: "summary".to_string(),
            version: 1,
            description: "A short summary".to_string(),
            instructions: None,
            ai_guidance: AiGuidance {
                purpose: "captures the summary".to_string(),
                ..Default::default()
            },
            value_type: ValueType::Text,
            content_format: None,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let mut value = serde_json::to_value(&field).unwrap();
        value["$schema"] = json!("https://srs.semanticops.com/schema/2.0/field.json");
        reg.validate_by_id(srs_schema::FIELD_SCHEMA_ID, &value)
            .expect("minimal Field must pass field.json schema");
    }

    #[test]
    fn missing_id_fails_to_deserialize() {
        // Regression for srs-rust#769: `id` is required by field.json and
        // must no longer default to "" when absent.
        let json_str = r#"{
            "namespace": "test.ns",
            "name": "test-field",
            "version": 1,
            "description": "A field with no id",
            "aiGuidance": {"purpose": "test"},
            "valueType": "string",
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let result: Result<Field, _> = serde_json::from_str(json_str);
        assert!(
            result.is_err(),
            "a Field with no `id` must fail to deserialize, not default id to \"\""
        );
    }

    #[test]
    fn ai_guidance_and_content_format_are_typed() {
        // Regression for srs-rust#769: aiGuidance and contentFormat must be
        // real typed fields, not opaque Value / entries in `extra`.
        let json_str = r#"{
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test.ns",
            "name": "test-field",
            "version": 1,
            "description": "A field",
            "aiGuidance": {"purpose": "captures test data", "extraction": "extract verbatim"},
            "valueType": "string",
            "contentFormat": "markdown",
            "editorHint": "rich-text",
            "tags": ["draft", "reviewed"],
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let field: Field = serde_json::from_str(json_str).unwrap();
        assert_eq!(field.ai_guidance.purpose, "captures test data");
        assert_eq!(
            field.ai_guidance.extraction.as_deref(),
            Some("extract verbatim")
        );
        assert_eq!(field.content_format, Some(ContentFormat::Markdown));
        assert_eq!(field.editor_hint, Some(EditorHint::RichText));
        assert_eq!(
            field.tags,
            Some(vec!["draft".to_string(), "reviewed".to_string()])
        );
        assert!(
            !field.extra.contains_key("contentFormat"),
            "contentFormat must no longer fall through to `extra`"
        );
    }

    #[test]
    fn i38_content_format_honoured_only_for_string_and_text() {
        let mut field = Field {
            id: "00000000-0000-4000-8000-000000000010".to_string(),
            namespace: "test.ns".to_string(),
            name: "test-field".to_string(),
            version: 1,
            description: "A field".to_string(),
            instructions: None,
            ai_guidance: AiGuidance::default(),
            value_type: ValueType::String,
            content_format: Some(ContentFormat::Markdown),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        // Honoured for `string`.
        assert_eq!(
            field.effective_content_format(),
            Some(ContentFormat::Markdown)
        );

        // Honoured for `text`.
        field.value_type = ValueType::Text;
        assert_eq!(
            field.effective_content_format(),
            Some(ContentFormat::Markdown)
        );

        // Ignored for every other valueType, even though `content_format`
        // is still set on the struct — I-38 requires implementations to
        // ignore it, not require authors to omit it.
        for vt in [
            ValueType::Number,
            ValueType::Boolean,
            ValueType::Date,
            ValueType::Url,
            ValueType::Select,
            ValueType::Multiselect,
        ] {
            field.value_type = vt;
            assert_eq!(
                field.effective_content_format(),
                None,
                "contentFormat must be ignored for valueType {vt:?}"
            );
        }
    }
}
