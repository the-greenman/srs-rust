use serde::{Deserialize, Serialize};

// Core RFC-017 sidecar format; not an extension-defined external catalog type (ADR-028),
// so types/ is correct per the Protocol precedent (ADR-016).
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
    /// Optional because sidecars written by add_attachment() omit this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

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
            "description": "Main SRS specification document.",
            "tags": ["specification", "core", "normative"],
            "createdAt": "2026-05-27T00:00:00Z",
            "importedAt": "2026-05-27T15:00:00Z",
            "meta": {
                "originalPath": "spec/srs-spec.md",
                "version": "2.0-draft"
            }
        }"#;
        let parsed: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.document_id, "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d");
        assert_eq!(parsed.content_type, "text/markdown");
        assert_eq!(parsed.created_at.as_deref(), Some("2026-05-27T00:00:00Z"));
        let tags = parsed.tags.as_ref().unwrap();
        assert_eq!(tags, &["specification", "core", "normative"]);
        let roundtripped: SourceDocumentMeta =
            serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
        assert_eq!(roundtripped, parsed);
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
            "tags": ["source-material", "origin", "chatgpt"],
            "createdAt": "2026-05-27T00:00:00Z",
            "importedAt": "2026-05-27T15:00:00Z"
        }"#;
        let parsed: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.document_id, "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e");
        assert_eq!(parsed.content_type, "text/markdown");
        let roundtripped: SourceDocumentMeta =
            serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
        assert_eq!(roundtripped, parsed);
    }

    #[test]
    fn source_document_meta_minimal_no_created_at() {
        // Sidecars written by add_attachment() omit createdAt — must deserialize without error.
        let json = r#"{
            "documentId": "aaaaaaaa-0000-4000-8000-000000000001",
            "contentPath": "doc.pdf",
            "contentType": "application/pdf"
        }"#;
        let parsed: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.document_id, "aaaaaaaa-0000-4000-8000-000000000001");
        assert!(parsed.created_at.is_none());
        assert!(parsed.schema.is_none());
    }

    #[test]
    fn source_document_meta_with_excerpt() {
        let json = r#"{
            "documentId": "aaaaaaaa-0000-4000-8000-000000000002",
            "contentPath": "doc.md",
            "contentType": "text/markdown",
            "excerpt": {
                "sourceDocumentId": "bbbbbbbb-0000-4000-8000-000000000003",
                "anchor": {
                    "kind": "line",
                    "value": "42",
                    "note": "Key insight"
                },
                "capturedAt": "2026-01-01T00:00:00Z"
            }
        }"#;
        let parsed: SourceDocumentMeta = serde_json::from_str(json).unwrap();
        let excerpt = parsed.excerpt.as_ref().unwrap();
        assert_eq!(
            excerpt.source_document_id,
            "bbbbbbbb-0000-4000-8000-000000000003"
        );
        assert_eq!(excerpt.anchor.as_ref().unwrap().kind, "line");
        let roundtripped: SourceDocumentMeta =
            serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
        assert_eq!(roundtripped, parsed);
    }
}
