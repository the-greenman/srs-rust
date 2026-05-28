use serde::{Deserialize, Serialize};

/// The live manifest uses a legacy string-array `instanceIndex`. The formal schema uses an
/// object array. Both must deserialize correctly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InstanceIndexEntry {
    Path(String),
    Object(InstanceIndexObject),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceIndexObject {
    pub instance_id: String,
    pub tier: u8,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl InstanceIndexEntry {
    pub fn path(&self) -> &str {
        match self {
            InstanceIndexEntry::Path(p) => p,
            InstanceIndexEntry::Object(o) => &o.path,
        }
    }

    pub fn instance_id(&self) -> Option<&str> {
        match self {
            InstanceIndexEntry::Path(_) => None,
            InstanceIndexEntry::Object(o) => Some(&o.instance_id),
        }
    }

    pub fn tier(&self) -> Option<u8> {
        match self {
            InstanceIndexEntry::Path(_) => None,
            InstanceIndexEntry::Object(o) => Some(o.tier),
        }
    }

    pub fn title(&self) -> Option<String> {
        match self {
            InstanceIndexEntry::Path(_) => None,
            InstanceIndexEntry::Object(o) => o.title.as_ref().map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                _ => v.to_string(),
            }),
        }
    }

    /// Returns true if this entry is a Note (tier 0)
    pub fn is_note(&self) -> bool {
        match self {
            InstanceIndexEntry::Path(_) => true, // Legacy entries are assumed to be notes
            InstanceIndexEntry::Object(o) => o.tier == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy_string_entry_deserializes() {
        let json = r#""records/notes/foo.json""#;
        let entry: InstanceIndexEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.path(), "records/notes/foo.json");
        assert_eq!(entry.instance_id(), None);
        assert_eq!(entry.tier(), None);
    }

    #[test]
    fn test_object_entry_deserializes() {
        let json = r#"{"instanceId": "abc-123", "tier": 0, "path": "records/notes/bar.json", "title": "Bar Note"}"#;
        let entry: InstanceIndexEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.path(), "records/notes/bar.json");
        assert_eq!(entry.instance_id(), Some("abc-123"));
        assert_eq!(entry.tier(), Some(0));
        assert_eq!(entry.title(), Some("Bar Note".to_string()));
    }

    #[test]
    fn test_is_note_for_tier_0() {
        let note_json = r#"{"instanceId": "abc-123", "tier": 0, "path": "records/notes/bar.json"}"#;
        let note: InstanceIndexEntry = serde_json::from_str(note_json).unwrap();
        assert!(note.is_note());
    }

    #[test]
    fn test_is_note_for_non_zero_tier() {
        let spec_json =
            r#"{"instanceId": "spec-123", "tier": 1, "path": "specifications/spec.json"}"#;
        let spec: InstanceIndexEntry = serde_json::from_str(spec_json).unwrap();
        assert!(!spec.is_note());
    }

    #[test]
    fn test_is_note_for_legacy_path() {
        let legacy: InstanceIndexEntry =
            serde_json::from_str("\"records/notes/legacy.json\"").unwrap();
        assert!(legacy.is_note()); // Legacy entries assumed to be notes
    }
}
