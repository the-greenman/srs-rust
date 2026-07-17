use serde::{Deserialize, Serialize};

/// Manifest-level index entry for a source document (RFC-017).
///
/// Mirrors the `SourceDocumentIndexEntry` definition in the JSON schema's
/// `manifest.json#/$defs/SourceDocumentIndexEntry`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentIndexEntry {
    pub document_id: String,
    pub sidecar_path: String,
    pub content_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_document_index_entry_roundtrips() {
        let entry = SourceDocumentIndexEntry {
            document_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string(),
            sidecar_path: "my-doc.meta.json".to_string(),
            content_path: "my-doc.pdf".to_string(),
            title: Some("My Document".to_string()),
            sidecar_checksum: Some("sha256:abcdef".to_string()),
            content_checksum: Some("sha256:123456".to_string()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let reparsed: SourceDocumentIndexEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(reparsed, entry);
        assert!(json.contains("\"documentId\""));
        assert!(json.contains("\"sidecarPath\""));
        assert!(json.contains("\"contentPath\""));
        assert!(json.contains("\"sidecarChecksum\""));
        assert!(json.contains("\"contentChecksum\""));
    }

    #[test]
    fn source_document_index_entry_omits_none_fields() {
        let entry = SourceDocumentIndexEntry {
            document_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string(),
            sidecar_path: "doc.meta.json".to_string(),
            content_path: "doc.pdf".to_string(),
            title: None,
            sidecar_checksum: None,
            content_checksum: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("title"));
        assert!(!json.contains("sidecarChecksum"));
        assert!(!json.contains("contentChecksum"));
        let reparsed: SourceDocumentIndexEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(reparsed, entry);
    }
}
