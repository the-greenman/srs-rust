use serde::{Deserialize, Serialize};

/// Metadata sidecar for a source document (`.meta.json` file).
///
/// Mirrors the `source-document-meta.json` JSON Schema (v2.0). No `deny_unknown_fields` —
/// forward-compatible with future schema additions (RFC-017 Rev 3+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentMeta {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub document_id: String,
    pub content_path: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<SourceDocumentExcerpt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// An excerpt reference embedded in a source document sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentExcerpt {
    pub source_document_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<SourceAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_checksum_at_capture: Option<String>,
}

/// An anchor within a source document (e.g. line range, heading, byte offset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAnchor {
    pub kind: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_document_meta_roundtrip_spec() {
        let json = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/source-document.json",
            "documentId": "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
            "contentPath": "srs-spec.md",
            "contentType": "text/markdown",
            "encoding": "utf-8",
            "title": "SRS Specification",
            "description": "Main SRS specification document - the canonical source of truth for the Semantic Record System specification.",
            "tags": ["specification", "core", "normative"],
            "createdAt": "2026-05-27T00:00:00Z",
            "importedAt": "2026-05-27T15:00:00Z",
            "meta": {
                "originalPath": "spec/srs-spec.md",
                "version": "2.0-draft",
                "status": "active-draft"
            }
        }"#;
        let meta: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(
            meta.document_id,
            "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"
        );
        assert_eq!(meta.content_type, "text/markdown");
        assert_eq!(meta.title.as_deref(), Some("SRS Specification"));
        let tags = meta.tags.as_ref().unwrap();
        assert_eq!(tags, &["specification", "core", "normative"]);
        let roundtripped: SourceDocumentMeta =
            serde_json::from_value(serde_json::to_value(&meta).unwrap()).unwrap();
        assert_eq!(meta, roundtripped);
    }

    #[test]
    fn source_document_meta_roundtrip_ai_session() {
        let json = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/source-document.json",
            "documentId": "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e",
            "contentPath": "chatgpt-origin.md",
            "contentType": "text/markdown",
            "encoding": "utf-8",
            "title": "ChatGPT Origin Session",
            "description": "Original AI session transcript documenting the initial conception and design of the SRS specification.",
            "tags": ["source-material", "origin", "chatgpt"],
            "createdAt": "2026-05-27T00:00:00Z",
            "importedAt": "2026-05-27T15:00:00Z",
            "meta": {
                "originalPath": "source-documents/ai-sessions/chatgpt-origin.md",
                "sessionType": "origin"
            }
        }"#;
        let meta: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(
            meta.document_id,
            "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e"
        );
        assert_eq!(meta.content_type, "text/markdown");
        assert_eq!(meta.title.as_deref(), Some("ChatGPT Origin Session"));
        let roundtripped: SourceDocumentMeta =
            serde_json::from_value(serde_json::to_value(&meta).unwrap()).unwrap();
        assert_eq!(meta, roundtripped);
    }

    #[test]
    fn source_document_meta_minimal_required_fields() {
        let json = r#"{
            "documentId": "cccccccc-0000-4000-8000-000000000001",
            "contentPath": "doc.pdf",
            "contentType": "application/pdf",
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let meta: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.document_id, "cccccccc-0000-4000-8000-000000000001");
        assert!(meta.title.is_none());
        assert!(meta.tags.is_none());
        assert!(meta.schema.is_none());
        let v = serde_json::to_value(&meta).unwrap();
        assert!(v.get("$schema").is_none(), "schema should be omitted when None");
        assert!(v.get("title").is_none(), "title should be omitted when None");
    }

    #[test]
    fn source_document_excerpt_roundtrip() {
        let json = r#"{
            "documentId": "aaaaaaaa-0000-4000-8000-000000000001",
            "contentPath": "excerpt.md",
            "contentType": "text/markdown",
            "createdAt": "2026-01-01T00:00:00Z",
            "excerpt": {
                "sourceDocumentId": "bbbbbbbb-0000-4000-8000-000000000002",
                "anchor": {
                    "kind": "line-range",
                    "value": "10-25",
                    "note": "key paragraph"
                },
                "capturedAt": "2026-01-02T00:00:00Z",
                "capturedBy": "claude-sonnet-4",
                "sourceChecksumAtCapture": "sha256:abc123"
            }
        }"#;
        let meta: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        let excerpt = meta.excerpt.as_ref().unwrap();
        assert_eq!(
            excerpt.source_document_id,
            "bbbbbbbb-0000-4000-8000-000000000002"
        );
        let anchor = excerpt.anchor.as_ref().unwrap();
        assert_eq!(anchor.kind, "line-range");
        assert_eq!(anchor.note.as_deref(), Some("key paragraph"));
        let roundtripped: SourceDocumentMeta =
            serde_json::from_value(serde_json::to_value(&meta).unwrap()).unwrap();
        assert_eq!(meta, roundtripped);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{
            "documentId": "aaaaaaaa-0000-4000-8000-000000000001",
            "contentPath": "doc.md",
            "contentType": "text/markdown",
            "createdAt": "2026-01-01T00:00:00Z",
            "futureField": "will be ignored"
        }"#;
        let meta: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.document_id, "aaaaaaaa-0000-4000-8000-000000000001");
    }
}
