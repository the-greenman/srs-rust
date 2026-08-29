use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VocabularyEntryStatus {
    #[default]
    Active,
    Deprecated,
    Tombstone,
    Retired,
}

impl VocabularyEntryStatus {
    pub fn resolves_for_reads(&self) -> bool {
        !matches!(self, VocabularyEntryStatus::Retired)
    }

    pub fn accepts_new_writes(&self) -> bool {
        matches!(self, VocabularyEntryStatus::Active)
    }

    pub fn is_retired(&self) -> bool {
        matches!(self, VocabularyEntryStatus::Retired)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Term {
    #[serde(default)]
    pub id: String,
    pub version: u32,
    pub namespace: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<VocabularyEntryStatus>,
    /// Substrate escape-bag (rfc-decision-6fc7e142 / srs#433 / srs PR #510).
    /// Serializes as `meta`; still accepts the pre-rev-5 `properties` key on
    /// read (monotonic support, RFC-033) — the `substrate-properties-to-meta`
    /// migration (#5) persists the rename on disk.
    #[serde(skip_serializing_if = "Option::is_none", alias = "properties")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Term {
    pub fn is_retired(&self) -> bool {
        self.status
            .as_ref()
            .map(|s| s.is_retired())
            .unwrap_or(false)
    }

    pub fn resolves_for_reads(&self) -> bool {
        self.status
            .as_ref()
            .map(|s| s.resolves_for_reads())
            .unwrap_or(true)
    }

    pub fn accepts_new_writes(&self) -> bool {
        self.status
            .as_ref()
            .map(|s| s.accepts_new_writes())
            .unwrap_or(true)
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles
            .as_ref()
            .map(|rs| rs.iter().any(|r| r == role))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_term() -> Term {
        Term {
            id: "a1000001-0000-4000-b000-000000000001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            key: "foundation".to_string(),
            label: None,
            description: None,
            aliases: None,
            roles: None,
            status: None,
            meta: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn term_roundtrips_json() {
        let term = Term {
            id: "a1000001-0000-4000-b000-000000000001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            key: "foundation".to_string(),
            label: Some("Foundation".to_string()),
            description: Some("Core tags".to_string()),
            aliases: Some(vec!["core".to_string()]),
            roles: Some(vec!["foundation".to_string()]),
            status: Some(VocabularyEntryStatus::Active),
            meta: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
        };
        let json = serde_json::to_string(&term).unwrap();
        let parsed: Term = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.key, "foundation");
        assert_eq!(parsed.label, Some("Foundation".to_string()));
        assert_eq!(parsed.aliases, Some(vec!["core".to_string()]));
    }

    #[test]
    fn term_minimal_omits_optional_fields() {
        let term = minimal_term();
        let json = serde_json::to_string(&term).unwrap();
        assert!(!json.contains("label"));
        assert!(!json.contains("description"));
        assert!(!json.contains("status"));
        let parsed: Term = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.key, "foundation");
        assert_eq!(parsed.status, None);
    }

    #[test]
    fn vocabulary_entry_status_serde() {
        assert_eq!(
            serde_json::to_string(&VocabularyEntryStatus::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&VocabularyEntryStatus::Deprecated).unwrap(),
            "\"deprecated\""
        );
        assert_eq!(
            serde_json::to_string(&VocabularyEntryStatus::Tombstone).unwrap(),
            "\"tombstone\""
        );
        assert_eq!(
            serde_json::to_string(&VocabularyEntryStatus::Retired).unwrap(),
            "\"retired\""
        );
    }

    #[test]
    fn resolves_for_reads_and_accepts_new_writes() {
        assert!(VocabularyEntryStatus::Active.resolves_for_reads());
        assert!(VocabularyEntryStatus::Active.accepts_new_writes());
        assert!(VocabularyEntryStatus::Deprecated.resolves_for_reads());
        assert!(!VocabularyEntryStatus::Deprecated.accepts_new_writes());
        assert!(VocabularyEntryStatus::Tombstone.resolves_for_reads());
        assert!(!VocabularyEntryStatus::Tombstone.accepts_new_writes());
        assert!(!VocabularyEntryStatus::Retired.resolves_for_reads());
        assert!(!VocabularyEntryStatus::Retired.accepts_new_writes());
    }

    #[test]
    fn term_is_retired_checks_status() {
        let mut t = minimal_term();
        assert!(!t.is_retired());
        t.status = Some(VocabularyEntryStatus::Retired);
        assert!(t.is_retired());
    }

    #[test]
    fn term_has_role() {
        let mut t = minimal_term();
        assert!(!t.has_role("foundation"));
        t.roles = Some(vec!["foundation".to_string()]);
        assert!(t.has_role("foundation"));
        assert!(!t.has_role("navigation"));
    }

    #[test]
    fn term_deny_unknown_fields() {
        let json = r#"{"id":"x","version":1,"namespace":"ns","key":"k","unknownField":"oops"}"#;
        assert!(serde_json::from_str::<Term>(json).is_err());
    }

    /// srs-rust#894 (srs#433, srs PR #510): a rev-4 repo still carrying the
    /// pre-rename `properties` key must load (monotonic support, RFC-033) —
    /// tolerated via serde alias, not rejected. Writes always use `meta`;
    /// the `substrate-properties-to-meta` migration (#5) persists the rename.
    #[test]
    fn term_tolerates_legacy_properties_key_on_read() {
        let json =
            r#"{"id":"x","version":1,"namespace":"ns","key":"k","properties":{"color":"blue"}}"#;
        let t: Term = serde_json::from_str(json).expect("legacy `properties` key must tolerate");
        assert_eq!(
            t.meta.as_ref().and_then(|m| m.get("color")),
            Some(&serde_json::json!("blue"))
        );
        let serialized = serde_json::to_string(&t).unwrap();
        assert!(serialized.contains("\"meta\""), "writes must use `meta`");
        assert!(
            !serialized.contains("\"properties\""),
            "writes must not reintroduce the retired `properties` key"
        );
    }
}
