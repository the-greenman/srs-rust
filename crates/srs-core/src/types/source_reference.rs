use serde::{Deserialize, Serialize};

/// A reference to a source document, transcript chunk, or other external
/// material that supports or produced a given entity.
///
/// Used on `Note::source_refs`, `Relation::source_refs`, and `Revision::source_refs`.
/// No `deny_unknown_fields` — forward-compatible with future schema additions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub source_type: SourceType,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_standard: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_type: Option<SourceRelationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The kind of source material being referenced.
///
/// `Copy` is intentional — `note.rs` had it; `relation.rs` was missing it without
/// documented reason. `Copy` is additive and all callers compile without change.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    TranscriptChunk,
    TranscriptSegment,
    ExternalDocument,
    RepositoryDocument,
}

/// The role the source plays relative to the entity it supports.
///
/// Merged from `note.rs::RelationType` (same enum, different name) and
/// `relation.rs::SourceRelationType`. Both already had identical five variants;
/// `SourceRelationType` is the canonical name going forward.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceRelationType {
    Evidence,
    DerivedFrom,
    QuotedFrom,
    InspiredBy,
    SupersedesContext,
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
            relation_type: None,
            confidence: None,
            note: None,
        };
        let v = serde_json::to_value(&sr).unwrap();
        assert_eq!(v["sourceType"], json!("transcript-chunk"));
        assert_eq!(v["sourceId"], json!("chunk-1"));
        assert!(v.get("relationType").is_none());
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
            relation_type: Some(SourceRelationType::Evidence),
            confidence: Some(0.9),
            note: Some("primary source".to_string()),
        };
        let v = serde_json::to_value(&sr).unwrap();
        assert_eq!(v["sourceType"], json!("repository-document"));
        assert_eq!(v["relationType"], json!("evidence"));
        assert_eq!(v["confidence"], json!(0.9));
        let parsed: SourceReference = serde_json::from_value(v).unwrap();
        assert_eq!(parsed, sr);
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
    fn source_relation_type_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SourceRelationType::DerivedFrom).unwrap(),
            "\"derived-from\""
        );
        assert_eq!(
            serde_json::to_string(&SourceRelationType::SupersedesContext).unwrap(),
            "\"supersedes-context\""
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{"sourceType":"transcript-chunk","sourceId":"x","unknownFuture":"val"}"#;
        let parsed: SourceReference = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.source_id, "x");
    }
}
