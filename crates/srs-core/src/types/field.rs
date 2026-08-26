use serde::{Deserialize, Serialize};

pub use crate::types::field_type::{
    Cardinality, Datatype, ExactTypeRef, FieldType, FieldTypeConstraints, FieldTypeViolation,
    LegacyContentFormat, LegacyFieldFacets, LegacyValidationRule, LegacyValueType, MapValueRange,
    RefMode, StringFormat, ValueDomain,
};

/// AI-facing guidance for a Field: what it captures, how to extract or
/// populate it, what to avoid, and worked examples. Mirrors `$defs/AiGuidance`
/// in `field.json`. `purpose` is the schema's only required sub-property.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

impl AiGuidance {
    /// True when no author-supplied guidance is present.
    ///
    /// `purpose` carries the model's central claim — that a Field declares what
    /// it means — so "absent" and "empty" must stay distinguishable rather than
    /// being papered over with a manufactured default (srs-rust#768).
    pub fn is_empty(&self) -> bool {
        self.purpose.trim().is_empty()
            && self.extraction.is_none()
            && self.negative_guidance.is_none()
            && self.examples.is_none()
    }
}

/// One worked example in `AiGuidance.examples`. Mirrors `$defs/AiGuidanceExample`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiGuidanceExample {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    pub output: String,
}

/// Distribution/fork tracking for a Field. Mirrors `$defs/Lineage`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

/// Suggested UI control for editing this Field's value. Presentation only — not
/// part of the RFC-032 type model. Implementations and Views may override.
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

/// The atomic semantic unit of SRS: a reusable, versioned field definition.
///
/// Property order and optionality mirror `docs/schema/2.0/field.json` exactly.
/// Since RFC-032 the value type is the decomposed [`FieldType`] rather than a
/// scalar `valueType` enum with untyped companions.
///
/// **Forward compatibility (srs-rust#767).** `field.json` sets
/// `additionalProperties: false`, and the self-hosted meta-model (`field` Type
/// in `com.semanticops.srs/metamodel`) declares no extension bag — so this
/// struct denies unknown properties too. Engine and schema now implement one
/// policy on every code path, rather than the previous split where loading
/// preserved unknown properties that the create gate rejected. `$schema` is a
/// declared property, not an unknown one, so it is modelled explicitly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Field {
    /// The `$schema` pointer a field file may carry. Declared by `field.json`
    /// itself; preserved so a loaded-then-written Field keeps it.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
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
    /// Absent guidance stays absent through a round-trip (srs-rust#768).
    /// Serializing a manufactured `{"purpose": ""}` for a Field that never had
    /// one is the exact artifact that issue removed from the create gate — it
    /// makes "no guidance written" indistinguishable from "guidance written and
    /// empty", and writing it back during a migration would make the loss
    /// permanent. A Field with no guidance therefore fails `field.json`'s
    /// `required` check, which is the visible outcome the issue asks for.
    ///
    /// `Option` is what carries that distinction (srs-rust#832). Skipping on an
    /// *all-empty* `AiGuidance` conflated the two cases the paragraph above says
    /// must stay apart: an authored `{"purpose": ""}` serialized as nothing, so
    /// `repo copy` — which round-trips every Field through this struct — turned a
    /// conforming repository into a bundle that fails `field.json`'s `required`
    /// check and cascades into [R13] for every Type assigning the dropped Field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<AiGuidance>,
    /// RFC-032 — the decomposed value type: datatype × cardinality ×
    /// value-domain × format × constraints.
    pub field_type: FieldType,
    // RFC-040 [R4] (srs#477/#867): no `defaultValue` mechanism exists at any
    // definition-layer site, and no `deprecatedAt` on definitions — a definition
    // retires by deletion with version history (`5f8204bc`). Both fields removed.
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
}

impl Field {
    /// A Field with only the schema-required properties set — the base every
    /// construction site starts from.
    pub fn new(
        id: impl Into<String>,
        namespace: impl Into<String>,
        name: impl Into<String>,
        field_type: FieldType,
    ) -> Self {
        Field {
            schema: None,
            id: id.into(),
            namespace: namespace.into(),
            name: name.into(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: None,
            field_type,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: String::new(),
        }
    }

    /// The base datatype facet — the successor of asking `valueType`.
    pub fn datatype(&self) -> Datatype {
        self.field_type.datatype
    }

    /// Whether this field holds an ordered list of values.
    pub fn is_list(&self) -> bool {
        self.field_type.is_list()
    }

    /// The inline closed vocabulary, when this field declares one.
    pub fn allowed_values(&self) -> Option<&[String]> {
        self.field_type.allowed_values()
    }

    /// The named Vocabulary this field's closed domain draws from, if any.
    pub fn vocabulary_ref(&self) -> Option<&str> {
        self.field_type.vocabulary_ref.as_deref()
    }

    /// I-38, restated in RFC-032 terms: a content format is only meaningful for
    /// `datatype: string`. The invariant used to need an explicit accessor
    /// because `contentFormat` was a companion property that could contradict
    /// `valueType`; `format` now lives inside `fieldType`, where R5 forbids the
    /// contradiction outright. This accessor remains the single read path so
    /// consumers cannot reintroduce it.
    pub fn effective_format(&self) -> Option<StringFormat> {
        match self.field_type.datatype {
            Datatype::String => self.field_type.format,
            _ => None,
        }
    }

    /// True when values should be rendered as CommonMark rather than plain text.
    pub fn is_markdown(&self) -> bool {
        self.effective_format() == Some(StringFormat::Markdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Field {
        Field {
            description: "A test field".to_string(),
            ai_guidance: Some(AiGuidance {
                purpose: "captures test data".to_string(),
                ..Default::default()
            }),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            ..Field::new(
                "00000000-0000-4000-8000-000000000010",
                "test.ns",
                "test_field",
                FieldType::select(["a", "b"]),
            )
        }
    }

    #[test]
    fn field_roundtrips_json() {
        let field = sample();
        let parsed: Field = serde_json::from_str(&serde_json::to_string(&field).unwrap()).unwrap();
        assert_eq!(parsed, field);
        assert_eq!(parsed.datatype(), Datatype::String);
        assert_eq!(
            parsed.allowed_values(),
            Some(["a".to_string(), "b".to_string()].as_slice())
        );
    }

    #[test]
    fn unknown_properties_are_rejected() {
        // srs-rust#767: `field.json` sets `additionalProperties: false` and the
        // create gate enforces it. Deserialization must not implement the
        // opposite policy — a Field's forward-compatibility answer may not
        // depend on which code path it entered through.
        let json_str = r#"{
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test.ns",
            "name": "test_field",
            "version": 1,
            "description": "A field",
            "aiGuidance": {"purpose": "test"},
            "fieldType": {"datatype": "string"},
            "createdAt": "2026-01-01T00:00:00Z",
            "unknownFutureField": "not preserved"
        }"#;
        let result: Result<Field, _> = serde_json::from_str(json_str);
        assert!(
            result.is_err(),
            "an unknown Field property must be rejected, matching additionalProperties: false"
        );
    }

    #[test]
    fn declared_schema_pointer_survives_a_roundtrip() {
        // `$schema` is a *declared* property of field.json, so it must not be
        // mistaken for an unknown one by `deny_unknown_fields`.
        let json_str = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test.ns",
            "name": "test_field",
            "version": 1,
            "description": "A field",
            "aiGuidance": {"purpose": "test"},
            "fieldType": {"datatype": "string"},
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let field: Field = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            field.schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/field.json")
        );
        let back = serde_json::to_value(&field).unwrap();
        assert_eq!(
            back["$schema"],
            json!("https://srs.semanticops.com/schema/2.0/field.json")
        );
    }

    #[test]
    fn serialized_property_order_matches_the_frozen_seed() {
        let mut field = sample();
        field.schema = Some("https://srs.semanticops.com/schema/2.0/field.json".to_string());
        field.instructions = Some("fill this in".to_string());
        field.tags = Some(vec!["x".to_string()]);
        let s = serde_json::to_string(&field).unwrap();
        let order: Vec<&str> = [
            "\"$schema\"",
            "\"id\"",
            "\"namespace\"",
            "\"name\"",
            "\"version\"",
            "\"description\"",
            "\"instructions\"",
            "\"aiGuidance\"",
            "\"fieldType\"",
            "\"tags\"",
            "\"createdAt\"",
        ]
        .to_vec();
        let mut cursor = 0usize;
        for key in order {
            let at = s[cursor..]
                .find(key)
                .unwrap_or_else(|| panic!("{key} missing or out of order in {s}"));
            cursor += at + key.len();
        }
    }

    #[test]
    fn missing_id_fails_to_deserialize() {
        // Regression for srs-rust#769: `id` is required by field.json and
        // must no longer default to "" when absent.
        let json_str = r#"{
            "namespace": "test.ns",
            "name": "test_field",
            "version": 1,
            "description": "A field with no id",
            "aiGuidance": {"purpose": "test"},
            "fieldType": {"datatype": "string"},
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let result: Result<Field, _> = serde_json::from_str(json_str);
        assert!(
            result.is_err(),
            "a Field with no `id` must fail to deserialize, not default id to \"\""
        );
    }

    #[test]
    fn ai_guidance_and_field_type_are_typed() {
        let json_str = r#"{
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "test.ns",
            "name": "test_field",
            "version": 1,
            "description": "A field",
            "aiGuidance": {"purpose": "captures test data", "extraction": "extract verbatim"},
            "fieldType": {"datatype": "string", "format": "markdown"},
            "editorHint": "rich-text",
            "tags": ["draft", "reviewed"],
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let field: Field = serde_json::from_str(json_str).unwrap();
        let guidance = field.ai_guidance.as_ref().expect("aiGuidance was present");
        assert_eq!(guidance.purpose, "captures test data");
        assert_eq!(guidance.extraction.as_deref(), Some("extract verbatim"));
        assert_eq!(field.editor_hint, Some(EditorHint::RichText));
        assert_eq!(
            field.tags,
            Some(vec!["draft".to_string(), "reviewed".to_string()])
        );
        assert!(field.is_markdown());
    }

    #[test]
    fn i38_format_honoured_only_for_string_datatype() {
        let mut field = Field::new("f-1", "test.ns", "test_field", FieldType::markdown());
        assert_eq!(field.effective_format(), Some(StringFormat::Markdown));

        // R5 forbids a non-string carrying a format, but should one arrive from
        // a hand-edited file the accessor still ignores it (I-38's "must
        // ignore", not "must reject").
        field.field_type.datatype = Datatype::Number;
        assert_eq!(field.effective_format(), None);
        assert!(!field.is_markdown());
    }

    #[test]
    fn empty_ai_guidance_is_distinguishable_from_authored_guidance() {
        // srs-rust#768: an auto-filled `purpose: ""` used to be indistinguishable
        // from authored guidance. It must not be.
        assert!(AiGuidance::default().is_empty());
        assert!(AiGuidance {
            purpose: "   ".to_string(),
            ..Default::default()
        }
        .is_empty());
        assert!(!AiGuidance {
            purpose: "captures the summary".to_string(),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn minimal_field_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let field = Field {
            description: "A short summary".to_string(),
            ai_guidance: Some(AiGuidance {
                purpose: "captures the summary".to_string(),
                ..Default::default()
            }),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            ..Field::new(
                "00000000-0000-4000-8000-000000000010",
                "test",
                "summary",
                FieldType::markdown(),
            )
        };
        let mut value = serde_json::to_value(&field).unwrap();
        value["$schema"] = json!("https://srs.semanticops.com/schema/2.0/field.json");
        reg.validate_by_id(srs_schema::FIELD_SCHEMA_ID, &value)
            .expect("minimal Field must pass field.json schema");
    }

    #[test]
    fn every_field_type_shape_passes_the_schema_contract() {
        // The struct and the frozen seed must agree across the whole RFC-032
        // surface, not just the scalar happy path.
        let reg = srs_schema::SchemaRegistry::global();
        let shapes = [
            FieldType::string(),
            FieldType::markdown(),
            FieldType::uri(),
            FieldType::number(),
            FieldType::integer(),
            FieldType::boolean(),
            FieldType::date(),
            FieldType::date_time(),
            FieldType::select(["a", "b"]),
            FieldType::multiselect(["a", "b"]),
            FieldType::inline_ref(ExactTypeRef {
                type_id: "4c000007-0000-4000-a000-000000000007".to_string(),
                type_version: 1,
            }),
            FieldType::instance_ref(ExactTypeRef {
                type_id: "4c000007-0000-4000-a000-000000000007".to_string(),
                type_version: 1,
            }),
        ];
        for shape in shapes {
            let field = Field {
                description: "d".to_string(),
                ai_guidance: Some(AiGuidance {
                    purpose: "p".to_string(),
                    ..Default::default()
                }),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                ..Field::new(
                    "00000000-0000-4000-8000-000000000010",
                    "test",
                    "f",
                    shape.clone(),
                )
            };
            assert!(
                field.field_type.validate().is_empty(),
                "{shape:?} must satisfy RFC-032 conformance"
            );
            let value = serde_json::to_value(&field).unwrap();
            reg.validate_by_id(srs_schema::FIELD_SCHEMA_ID, &value)
                .unwrap_or_else(|e| panic!("{shape:?} must pass field.json: {e}"));
        }
    }
}
