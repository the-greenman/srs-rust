use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceIndexEntry {
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
        &self.path
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn tier(&self) -> u8 {
        self.tier
    }

    pub fn title(&self) -> Option<String> {
        self.title.as_ref().map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            _ => v.to_string(),
        })
    }

    pub fn is_note(&self) -> bool {
        self.tier == 0
    }

    pub fn is_tag_definition(&self) -> bool {
        self.tier == 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_entry_deserializes() {
        let json = r#"{"instanceId": "abc-123", "tier": 0, "path": "records/notes/bar.json", "title": "Bar Note"}"#;
        let entry: InstanceIndexEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.path(), "records/notes/bar.json");
        assert_eq!(entry.instance_id(), "abc-123");
        assert_eq!(entry.tier(), 0);
        assert_eq!(entry.title(), Some("Bar Note".to_string()));
    }

    #[test]
    fn string_entry_is_rejected() {
        let result: Result<InstanceIndexEntry, _> =
            serde_json::from_str(r#""records/notes/foo.json""#);
        assert!(result.is_err());
    }

    #[test]
    fn is_note_for_tier_0() {
        let note_json = r#"{"instanceId": "abc-123", "tier": 0, "path": "records/notes/bar.json"}"#;
        let note: InstanceIndexEntry = serde_json::from_str(note_json).unwrap();
        assert!(note.is_note());
    }

    #[test]
    fn is_note_false_for_non_zero_tier() {
        let spec_json =
            r#"{"instanceId": "spec-123", "tier": 1, "path": "specifications/spec.json"}"#;
        let spec: InstanceIndexEntry = serde_json::from_str(spec_json).unwrap();
        assert!(!spec.is_note());
    }

    #[test]
    fn is_tag_definition_for_tier_3() {
        let td_json = r#"{"instanceId": "td-123", "tier": 3, "path": "records/tag-definitions/purpose.json"}"#;
        let td: InstanceIndexEntry = serde_json::from_str(td_json).unwrap();
        assert!(td.is_tag_definition());
        assert!(!td.is_note());
    }
}
