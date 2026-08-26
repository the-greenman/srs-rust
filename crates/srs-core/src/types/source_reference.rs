use serde::{Deserialize, Serialize};

/// A reference to a source document, transcript chunk, or other external
/// material that supports or produced a given entity.
///
/// Used on `Note::source_refs`, `Relation::source_refs`, and `Revision::source_refs`.
/// No `deny_unknown_fields` — forward-compatible with future schema additions.
///
/// RFC-023: `source_role` is the sole provenance-role field (serialized as `sourceRole`).
/// The legacy `relationType` alias's migration window closed with srs#480 — the schema no
/// longer accepts it (a hard rejection under `additionalProperties: false`), so the field is
/// removed here too rather than kept as dead read-compatibility code (srs-rust#869).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub source_type: SourceType,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_standard: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    /// RFC-023: the sole provenance-role field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_role: Option<SourceRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The kind of source material being referenced.
///
/// `Copy` is intentional — these are value-like, fieldless enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    TranscriptChunk,
    TranscriptSegment,
    ExternalDocument,
    RepositoryDocument,
}

/// The provenance role a source plays for the referencing entity (RFC-023).
///
/// This is the canonical vocabulary for `SourceReference.source_role`. The value set is
/// disjoint from installed `RelationTypeDefinition` keys per RFC-023 [R5].
/// `Attaches` added by RFC-017: material attachment of a source document to a record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceRole {
    Evidence,
    ExtractedFrom,
    QuotedFrom,
    InspiredBy,
    Attaches,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_reference_minimal_roundtrip() {
        let sr = SourceReference {
            source_type: SourceType::TranscriptChunk,
            source_id: "chunk-1".to_string(),
            source_standard: None,
            stream_id: None,
            source_role: None,
            confidence: None,
            note: None,
        };
        let v = serde_json::to_value(&sr).unwrap();
        assert_eq!(v["sourceType"], json!("transcript-chunk"));
        assert_eq!(v["sourceId"], json!("chunk-1"));
        assert!(v.get("relationType").is_none());
        assert!(v.get("sourceRole").is_none());
        let parsed: SourceReference = serde_json::from_value(v).unwrap();
        assert_eq!(parsed, sr);
    }

    #[test]
    fn source_reference_full_roundtrip() {
        let sr = SourceReference {
            source_type: SourceType::RepositoryDocument,
            source_id: "doc-abc".to_string(),
            source_standard: Some("ISO-999".to_string()),
            stream_id: Some("stream-1".to_string()),
            source_role: Some(SourceRole::Evidence),
            confidence: Some(0.9),
            note: Some("primary source".to_string()),
        };
        let v = serde_json::to_value(&sr).unwrap();
        assert_eq!(v["sourceType"], json!("repository-document"));
        assert_eq!(v["sourceRole"], json!("evidence"));
        assert_eq!(v["confidence"], json!(0.9));
        assert!(v.get("relationType").is_none());
        let parsed: SourceReference = serde_json::from_value(v).unwrap();
        assert_eq!(parsed, sr);
    }

    #[test]
    fn source_role_attaches_roundtrip() {
        let sr = SourceReference {
            source_type: SourceType::RepositoryDocument,
            source_id: "doc-xyz".to_string(),
            source_standard: None,
            stream_id: None,
            source_role: Some(SourceRole::Attaches),
            confidence: None,
            note: None,
        };
        let v = serde_json::to_value(&sr).unwrap();
        assert_eq!(v["sourceRole"], json!("attaches"));
        assert!(
            v.get("relationType").is_none(),
            "relationType is no longer a field — it must never be emitted"
        );
        let parsed: SourceReference = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.source_role, Some(SourceRole::Attaches));
    }

    #[test]
    fn source_role_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SourceRole::ExtractedFrom).unwrap(),
            "\"extracted-from\""
        );
        assert_eq!(
            serde_json::to_string(&SourceRole::InspiredBy).unwrap(),
            "\"inspired-by\""
        );
        assert_eq!(
            serde_json::to_string(&SourceRole::Attaches).unwrap(),
            "\"attaches\""
        );
    }

    #[test]
    fn source_type_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SourceType::RepositoryDocument).unwrap(),
            "\"repository-document\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::TranscriptChunk).unwrap(),
            "\"transcript-chunk\""
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{"sourceType":"transcript-chunk","sourceId":"x","unknownFuture":"val"}"#;
        let parsed: SourceReference = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.source_id, "x");
    }

    /// srs#480/#869: the legacy `relationType` field's migration window closed — the schema
    /// now rejects it outright. Rust's reader has no `deny_unknown_fields` on this struct
    /// (instance-layer tolerance, ruled behaviour), so a stray `relationType` key is silently
    /// ignored rather than rejected — verify that tolerance, not a resurrection of the field.
    #[test]
    fn legacy_relation_type_key_is_tolerated_and_dropped() {
        let json = r#"{"sourceType":"transcript-chunk","sourceId":"x","relationType":"evidence"}"#;
        let parsed: SourceReference = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.source_id, "x");
        assert!(parsed.source_role.is_none());
    }
}
