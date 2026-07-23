use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceIndexEntry {
    pub instance_id: String,
    pub tier: u8,
    /// Adapter-private key (ADR-041 G5, ADR-042) — the same contract-opaque status
    /// `ContainerIndexEntry.path` has. Migrated service code addresses instances by
    /// logical id via the store's typed methods (`load_record_by_id`, `find_instance`,
    /// `list_instances`, …), not by this path. Only the FileStore/JsonStore adapters and
    /// the explicitly-deferred readers (tracked in srs-rust#725) still read it directly.
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
}

/// A lightweight, index-answerable summary of an instance — the columns a
/// `RepositoryStore` can return without loading the entity body (ADR-041 G5,
/// ADR-042). Mirrors [`InstanceIndexEntry`] minus its adapter-private `path`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRef {
    pub instance_id: String,
    pub tier: u8,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

impl InstanceRef {
    pub(crate) fn from_index_entry(entry: &InstanceIndexEntry) -> Self {
        InstanceRef {
            instance_id: entry.instance_id.clone(),
            tier: entry.tier,
            title: entry.title(),
            tags: entry.tags.clone().unwrap_or_default(),
        }
    }
}

/// Index-answerable predicate for [`RepositoryStore::list_instances`]. Only the
/// axes a backend can satisfy from its index live here (ADR-042); richer
/// predicates (type, lifecycle, content) stay in the service layer.
///
/// `tier` is an exact match; `tag` is a **single contains-predicate** (the
/// instance's tags must contain this value), matching the existing singular
/// `RecordListFilter.tag` / `ListNotesFilter.tag`. Both `None` ⇒ match all.
#[derive(Debug, Clone, Default)]
pub struct InstanceQuery {
    pub tier: Option<u8>,
    pub tag: Option<String>,
}

impl InstanceQuery {
    /// Does `entry` satisfy this query? See the struct doc for combinator semantics.
    pub fn matches(&self, entry: &InstanceIndexEntry) -> bool {
        if let Some(tier) = self.tier {
            if entry.tier != tier {
                return false;
            }
        }
        if let Some(ref tag) = self.tag {
            let has_tag = entry
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|t| t == tag));
            if !has_tag {
                return false;
            }
        }
        true
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
}
