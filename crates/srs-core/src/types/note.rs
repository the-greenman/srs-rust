use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    #[serde(default)]
    pub instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub sections: Vec<NoteSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graduated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<Vec<SourceReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSection {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hint: Option<ContentHint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentHint {
    Text,
    Markdown,
    Plain,
}

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
    pub relation_type: Option<RelationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    TranscriptChunk,
    TranscriptSegment,
    ExternalDocument,
    RepositoryDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationType {
    Evidence,
    DerivedFrom,
    QuotedFrom,
    InspiredBy,
    SupersedesContext,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_roundtrips_json() {
        let note = Note {
            instance_id: "test-id".to_string(),
            title: Some("Test Title".to_string()),
            tags: Some(vec!["tag1".to_string(), "tag2".to_string()]),
            sections: vec![NoteSection {
                name: "section1".to_string(),
                label: Some("Section 1".to_string()),
                content: "Content here".to_string(),
                content_hint: Some(ContentHint::Markdown),
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            meta: None,
        };

        let json = serde_json::to_string(&note).unwrap();
        let deserialized: Note = serde_json::from_str(&json).unwrap();
        assert_eq!(note, deserialized);
    }

    #[test]
    fn origin_purpose_deserializes() {
        const ORIGIN_PURPOSE_JSON: &str = r#"
        {
          "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
          "instanceId": "d5c7e536-5f7d-491a-8166-5ee25a954377",
          "title": "Origin: Purpose of SRS",
          "tags": ["purpose", "origin", "thesis", "architecture"],
          "sections": [
            {
              "name": "problem",
              "tags": ["origin", "purpose"],
              "content": "The system emerged from a concrete problem..."
            },
            {
              "name": "core_thesis",
              "tags": ["thesis", "principles"],
              "content": "Documents are not primarily text..."
            },
            {
              "name": "what_srs_is_not",
              "tags": ["constraints", "boundaries", "non-goals"],
              "content": "SRS is not a document schema..."
            },
            {
              "name": "the_key_separations",
              "tags": ["architecture", "principles"],
              "content": "The architecture depends on keeping these concerns separate..."
            },
            {
              "name": "mutable_understanding",
              "tags": ["principles", "provenance", "history"],
              "content": "Understanding evolves..."
            },
            {
              "name": "what_success_looks_like",
              "tags": ["objectives", "interoperability"],
              "content": "The same semantic state generates..."
            }
          ],
          "sourceRefs": [
            {
              "sourceType": "repository-document",
              "sourceId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
              "relationType": "evidence",
              "note": "ChatGPT origin session..."
            }
          ],
          "createdAt": "2026-05-28T00:00:00Z"
        }
        "#;

        let note: Note = serde_json::from_str(ORIGIN_PURPOSE_JSON).unwrap();
        assert_eq!(note.instance_id, "d5c7e536-5f7d-491a-8166-5ee25a954377");
        assert_eq!(note.sections.len(), 6);
    }

    #[test]
    fn source_type_serializes_hyphenated() {
        let val = SourceType::RepositoryDocument;
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "\"repository-document\"");
    }

    #[test]
    fn relation_type_serializes_hyphenated() {
        let val = RelationType::DerivedFrom;
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "\"derived-from\"");
    }

    #[test]
    fn content_hint_serializes_lowercase() {
        let val = ContentHint::Markdown;
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "\"markdown\"");
    }

    #[test]
    fn minimal_note_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let note = Note {
            instance_id: "00000000-0000-4000-8000-000000000001".to_string(),
            title: None,
            tags: None,
            sections: vec![NoteSection {
                name: "body".to_string(),
                label: None,
                content: "Hello".to_string(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            meta: None,
        };
        let mut value = serde_json::to_value(&note).unwrap();
        value["$schema"] = serde_json::json!("https://srs.semanticops.com/schema/2.0/note.json");
        reg.validate_by_id(srs_schema::NOTE_SCHEMA_ID, &value)
            .expect("minimal Note must pass note.json schema");
    }
}
