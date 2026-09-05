//! Load-time deserialization of a Field file, including the RFC-032
//! data-model-revision 0 → 1 compatibility path.
//!
//! A field file on disk is one of two shapes:
//!
//! * **revision 1** (RFC-032) — carries `fieldType`.
//! * **revision 0** — carries the scalar `valueType` plus its companions
//!   (`contentFormat`, `allowedValues`, `vocabularyRef`, `validationRules`).
//!
//! Both load. A revision-0 field is upgraded in memory by RFC-032 Change H
//! ([`srs_core::types::field::FieldType::from_legacy`]), and
//! `srs repo apply-migration --id field-type` makes that upgrade durable by writing
//! the fields back. Without this path the migration would be unreachable: a
//! binary that cannot read a revision-0 repository cannot migrate one either.
//!
//! Whether a *repository* has been migrated is a separate, coarse question
//! answered by the manifest's `dataModelRevision` stamp (RFC-033 [R6]) — see
//! `field_type_migration_service`. The stamp is the gate; this module is the
//! reader that keeps unstamped repositories usable in the meantime.

use crate::error::RepositoryError;
use srs_core::types::field::{
    AiGuidance, EditorHint, Field, FieldType, LegacyContentFormat, LegacyFieldFacets,
    LegacyValidationRule, LegacyValueType, Lineage, Provenance,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FieldJson {
    /// Declared by `field.json` itself — not an unknown property.
    #[serde(rename = "$schema", default)]
    pub(crate) schema: Option<String>,
    pub(crate) id: String,
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) version: u32,
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    #[serde(default)]
    pub(crate) ai_guidance: Option<AiGuidance>,
    /// RFC-032 (data-model revision 1).
    #[serde(default)]
    pub(crate) field_type: Option<FieldType>,
    // RFC-040 [R4] (srs#477/#867): defaultValue/deprecatedAt no longer exist at any
    // definition-layer site — removed here too, so `deny_unknown_fields` now rejects
    // either key on load rather than silently accepting a retired shape.
    #[serde(default)]
    pub(crate) editor_hint: Option<EditorHint>,
    #[serde(default)]
    pub(crate) tags: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) lineage: Option<Lineage>,
    #[serde(default)]
    pub(crate) provenance: Option<Provenance>,
    pub(crate) created_at: Option<String>,

    // --- data-model revision 0 (pre-RFC-032) ---------------------------------
    #[serde(default)]
    pub(crate) value_type: Option<String>,
    #[serde(default)]
    pub(crate) content_format: Option<LegacyContentFormat>,
    #[serde(default)]
    pub(crate) allowed_values: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) vocabulary_ref: Option<String>,
    #[serde(default)]
    pub(crate) validation_rules: Option<Vec<LegacyValidationRule>>,
}

impl FieldJson {
    pub(crate) fn into_field(self, path: &std::path::Path) -> Result<Field, RepositoryError> {
        // A document carrying BOTH shapes is ambiguous, and preferring either
        // one silently discards the other's meaning — a half-migrated Field
        // (`fieldType` written, companions left behind) would lose its
        // cardinality, value domain and constraints, and
        // `apply-migration field-type` would then make that loss permanent.
        // Refuse to guess.
        if self.field_type.is_some() {
            let stray: Vec<&str> = [
                ("valueType", self.value_type.is_some()),
                ("contentFormat", self.content_format.is_some()),
                ("allowedValues", self.allowed_values.is_some()),
                ("vocabularyRef", self.vocabulary_ref.is_some()),
                ("validationRules", self.validation_rules.is_some()),
            ]
            .into_iter()
            .filter_map(|(k, present)| present.then_some(k))
            .collect();
            if !stray.is_empty() {
                return Err(RepositoryError::InvalidValueType {
                    path: path.to_path_buf(),
                    value_type: format!(
                        "field declares both the RFC-032 `fieldType` and the pre-RFC-032 {} — \
                         remove the pre-RFC-032 propert{} so the field's type has one definition",
                        stray
                            .iter()
                            .map(|k| format!("`{k}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        if stray.len() == 1 { "y" } else { "ies" }
                    ),
                });
            }
        }

        let field_type = match self.field_type {
            Some(ft) => ft,
            None => {
                let value_type = self.value_type.as_deref().ok_or_else(|| {
                    RepositoryError::InvalidValueType {
                        path: path.to_path_buf(),
                        value_type: "<missing>: a Field must declare `fieldType` (data-model \
                                     revision 1) or `valueType` (revision 0)"
                            .to_string(),
                    }
                })?;
                let legacy = parse_legacy_value_type(value_type, path)?;
                FieldType::from_legacy(
                    legacy,
                    &LegacyFieldFacets {
                        content_format: self.content_format,
                        allowed_values: self.allowed_values,
                        vocabulary_ref: self.vocabulary_ref,
                        validation_rules: self.validation_rules,
                    },
                )
            }
        };

        Ok(Field {
            schema: self.schema,
            id: self.id,
            namespace: self.namespace,
            name: self.name,
            version: self.version,
            description: self.description.unwrap_or_default(),
            instructions: self.instructions,
            // Absent stays absent, authored-but-empty stays authored (#768/#832).
            ai_guidance: self.ai_guidance,
            field_type,
            editor_hint: self.editor_hint,
            tags: self.tags,
            lineage: self.lineage,
            provenance: self.provenance,
            created_at: self.created_at.unwrap_or_default(),
        })
    }
}

/// `#[serde(deserialize_with = ...)]` adapter for any `Vec<Field>` member that
/// is read straight out of a serialized structure (package snapshots, embedded
/// bundles) rather than from individual field files.
///
/// Routing those through [`FieldJson`] keeps **one** answer to "what may a Field
/// document contain": unknown properties are rejected everywhere, and a
/// data-model-revision-0 document is upgraded everywhere. Without this, an
/// archive or bundle would be the one input that could not carry a pre-RFC-032
/// Field even though a repository directory could.
pub(crate) fn deserialize_fields_compat<'de, D>(deserializer: D) -> Result<Vec<Field>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let raw = Vec::<FieldJson>::deserialize(deserializer)?;
    raw.into_iter()
        .map(|f| {
            f.into_field(std::path::Path::new("<snapshot>"))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

/// Parse a data-model-revision-0 `valueType` string.
pub(crate) fn parse_legacy_value_type(
    s: &str,
    path: &std::path::Path,
) -> Result<LegacyValueType, RepositoryError> {
    match s {
        "string" => Ok(LegacyValueType::String),
        "text" => Ok(LegacyValueType::Text),
        "number" => Ok(LegacyValueType::Number),
        "boolean" => Ok(LegacyValueType::Boolean),
        "date" => Ok(LegacyValueType::Date),
        "url" => Ok(LegacyValueType::Url),
        "select" => Ok(LegacyValueType::Select),
        "multiselect" => Ok(LegacyValueType::Multiselect),
        _ => Err(RepositoryError::InvalidValueType {
            path: path.to_path_buf(),
            value_type: s.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::field::{AllowedValue, Cardinality, Datatype, StringFormat, ValueDomain};
    use std::path::Path;

    fn parse(json: serde_json::Value) -> Result<Field, RepositoryError> {
        let fj: FieldJson =
            serde_json::from_value(json).map_err(|e| RepositoryError::InvalidInput {
                message: e.to_string(),
            })?;
        fj.into_field(Path::new("f.json"))
    }

    #[test]
    fn loads_a_revision_1_field_verbatim() {
        let field = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "title", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "fieldType": {"datatype": "string", "format": "markdown"}
        }))
        .unwrap();
        assert_eq!(field.field_type, FieldType::markdown());
    }

    #[test]
    fn upgrades_a_revision_0_multiselect_with_its_companions() {
        let field = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "labels", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "valueType": "multiselect",
            "allowedValues": ["a", "b"]
        }))
        .unwrap();
        assert_eq!(field.field_type.datatype, Datatype::String);
        assert_eq!(field.field_type.cardinality, Some(Cardinality::List));
        assert_eq!(field.field_type.value_domain, Some(ValueDomain::Closed));
        assert_eq!(
            field.field_type.allowed_values(),
            Some(
                [
                    AllowedValue::String("a".to_string()),
                    AllowedValue::String("b".to_string())
                ]
                .as_slice()
            )
        );
        assert!(field.field_type.validate().is_empty());
    }

    #[test]
    fn upgrades_a_revision_0_text_field_with_content_format() {
        let field = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "body", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "valueType": "text",
            "contentFormat": "markdown"
        }))
        .unwrap();
        assert_eq!(field.field_type.format, Some(StringFormat::Markdown));
    }

    #[test]
    fn upgrades_revision_0_validation_rules_into_constraints() {
        let field = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "code", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "valueType": "string",
            "validationRules": [{"type": "maxLength", "value": 12}]
        }))
        .unwrap();
        assert_eq!(
            field.field_type.constraints.as_ref().unwrap().max_length,
            Some(12)
        );
    }

    #[test]
    fn a_field_with_neither_shape_is_an_error() {
        let err = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "x", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z"
        }));
        assert!(err.is_err());
    }

    #[test]
    fn an_unknown_property_is_rejected() {
        // srs-rust#767 — one policy on every code path.
        let err = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "x", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "fieldType": {"datatype": "string"},
            "xFutureHint": "nope"
        }));
        assert!(err.is_err(), "unknown Field property must be rejected");
    }

    #[test]
    fn a_document_carrying_both_shapes_is_rejected_not_silently_halved() {
        // A half-migrated Field (tool writes `fieldType`, leaves the companions)
        // is a plausible real state. Preferring `fieldType` and dropping the
        // rest would erase list-ness, the closed domain, the allowed values and
        // the length constraint — and `apply-migration field-type` would then
        // write that loss to disk with no diagnostic.
        for stray in [
            serde_json::json!({"valueType": "multiselect"}),
            serde_json::json!({"allowedValues": ["a", "b"]}),
            serde_json::json!({"contentFormat": "markdown"}),
            serde_json::json!({"vocabularyRef": "ns/v@1"}),
            serde_json::json!({"validationRules": [{"type": "maxLength", "value": 5}]}),
        ] {
            let mut doc = serde_json::json!({
                "id": "f-1", "namespace": "com.test", "name": "x", "version": 1,
                "description": "d", "createdAt": "2026-01-01T00:00:00Z",
                "fieldType": {"datatype": "string"}
            });
            for (k, v) in stray.as_object().unwrap() {
                doc[k] = v.clone();
            }
            let err = parse(doc.clone());
            assert!(
                err.is_err(),
                "a Field with both `fieldType` and {stray} must be rejected, not silently halved"
            );
            let message = format!("{}", err.unwrap_err());
            assert!(
                message.contains("both"),
                "the error must say what is ambiguous: {message}"
            );
        }
    }

    #[test]
    fn an_unknown_legacy_value_type_is_an_error() {
        let err = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "x", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "valueType": "quaternion"
        }));
        assert!(err.is_err());
    }

    #[test]
    fn absent_ai_guidance_is_not_manufactured_on_the_write_path() {
        // srs-rust#768 removed the injected `{"purpose": ""}` from the create
        // gate; it must not survive on the load/rewrite path either, or
        // `apply-migration field-type` would write it to disk and make "no
        // guidance" permanently indistinguishable from "empty guidance".
        let field = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "unguided", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "fieldType": {"datatype": "string"}
        }))
        .unwrap();
        let written = serde_json::to_value(&field).unwrap();
        assert!(
            written.get("aiGuidance").is_none(),
            "absent guidance must stay absent, not become {{\"purpose\": \"\"}}: {written}"
        );
    }

    #[test]
    fn authored_empty_ai_guidance_survives_the_write_path() {
        // srs-rust#832, the other half of #768's distinction. `aiGuidance` is
        // *required* by field.json, so dropping an authored `{"purpose": ""}` on
        // write turns a conforming Field into one that fails [R8] — and every
        // Type assigning it into an [R13] dangling reference. `repo copy`
        // round-trips every Field through this struct, so it turned a valid
        // repository into an unloadable bundle in one step, silently.
        let field = parse(serde_json::json!({
            "id": "f-1", "namespace": "com.test", "name": "amendment_rule", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "aiGuidance": {"purpose": ""},
            "fieldType": {"datatype": "string"}
        }))
        .unwrap();
        let written = serde_json::to_value(&field).unwrap();
        assert_eq!(
            written.get("aiGuidance"),
            Some(&serde_json::json!({"purpose": ""})),
            "an authored empty purpose must survive the round-trip: {written}"
        );
    }

    #[test]
    fn the_schema_pointer_survives_the_load() {
        let field = parse(serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
            "id": "f-1", "namespace": "com.test", "name": "x", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "fieldType": {"datatype": "string"}
        }))
        .unwrap();
        assert_eq!(
            field.schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/field.json")
        );
    }
}
