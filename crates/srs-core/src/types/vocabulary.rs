use super::term::Term;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VocabularyMode {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionWindow {
    pub until: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vocabulary {
    #[serde(default)]
    pub id: String,
    pub version: u32,
    pub namespace: String,
    pub name: String,
    pub mode: VocabularyMode,
    pub terms: Vec<Term>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends_vocabulary_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends_vocabulary_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_window: Option<PromotionWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Vocabulary {
    /// Returns terms whose status is not Retired (i.e., active/deprecated/tombstone or absent).
    pub fn effective_terms(&self) -> Vec<&Term> {
        self.terms.iter().filter(|t| !t.is_retired()).collect()
    }

    /// Resolve a term by key or alias (skips retired).
    /// Key match takes priority over alias match.
    /// Among alias matches, lexicographically smallest `id` wins (V2 tie-break).
    pub fn resolve_term_by_key(&self, key: &str) -> Option<&Term> {
        let effective: Vec<&Term> = self.effective_terms();

        // Key match (highest priority)
        for term in &effective {
            if term.key == key {
                return Some(term);
            }
        }

        // Alias match — lowest id wins on tie
        let mut alias_matches: Vec<&Term> = effective
            .iter()
            .filter(|t| {
                t.aliases
                    .as_ref()
                    .map(|a| a.iter().any(|al| al == key))
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if alias_matches.is_empty() {
            return None;
        }

        alias_matches.sort_by(|a, b| a.id.cmp(&b.id));
        Some(alias_matches[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::term::VocabularyEntryStatus;

    fn make_term(id: &str, key: &str, aliases: Option<Vec<&str>>) -> Term {
        Term {
            id: id.to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            key: key.to_string(),
            label: None,
            description: None,
            aliases: aliases.map(|a| a.iter().map(|s| s.to_string()).collect()),
            roles: None,
            status: None,
            properties: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn make_vocab(mode: VocabularyMode, terms: Vec<Term>) -> Vocabulary {
        Vocabulary {
            id: "v1000001-0000-4000-b000-000000000001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "test-vocab".to_string(),
            mode,
            terms,
            extends_vocabulary_id: None,
            extends_vocabulary_version: None,
            promotion_window: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn vocabulary_mode_serde() {
        assert_eq!(
            serde_json::to_string(&VocabularyMode::Open).unwrap(),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&VocabularyMode::Closed).unwrap(),
            "\"closed\""
        );
        let m: VocabularyMode = serde_json::from_str("\"open\"").unwrap();
        assert_eq!(m, VocabularyMode::Open);
    }

    #[test]
    fn vocabulary_roundtrips_json() {
        let v = make_vocab(VocabularyMode::Open, vec![make_term("id1", "foo", None)]);
        let json = serde_json::to_string(&v).unwrap();
        let parsed: Vocabulary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.mode, VocabularyMode::Open);
        assert_eq!(parsed.terms.len(), 1);
        assert_eq!(parsed.terms[0].key, "foo");
    }

    #[test]
    fn vocabulary_effective_terms_excludes_retired() {
        let mut t_retired = make_term("id2", "old", None);
        t_retired.status = Some(VocabularyEntryStatus::Retired);
        let v = make_vocab(
            VocabularyMode::Closed,
            vec![make_term("id1", "active", None), t_retired],
        );
        let eff = v.effective_terms();
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].key, "active");
    }

    #[test]
    fn resolve_term_by_key_primary_key() {
        let v = make_vocab(VocabularyMode::Closed, vec![make_term("id1", "foo", None)]);
        let t = v.resolve_term_by_key("foo");
        assert!(t.is_some());
        assert_eq!(t.unwrap().key, "foo");
    }

    #[test]
    fn resolve_term_by_key_alias() {
        let v = make_vocab(
            VocabularyMode::Open,
            vec![make_term("id1", "foundation", Some(vec!["core"]))],
        );
        let t = v.resolve_term_by_key("core");
        assert!(t.is_some());
        assert_eq!(t.unwrap().key, "foundation");
    }

    #[test]
    fn resolve_term_by_key_excludes_retired() {
        let mut t = make_term("id1", "old", None);
        t.status = Some(VocabularyEntryStatus::Retired);
        let v = make_vocab(VocabularyMode::Closed, vec![t]);
        assert!(v.resolve_term_by_key("old").is_none());
    }

    #[test]
    fn resolve_term_alias_secondary_to_key() {
        // term1 has key="foo", term2 has alias="foo" — key match wins
        let term1 = make_term("id1", "foo", None);
        let term2 = make_term("id2", "bar", Some(vec!["foo"]));
        let v = make_vocab(VocabularyMode::Closed, vec![term1, term2]);
        let resolved = v.resolve_term_by_key("foo").unwrap();
        assert_eq!(resolved.key, "foo");
    }

    #[test]
    fn resolve_term_alias_tie_break_by_id() {
        // Both terms have alias "shared-alias"; lexicographically smaller id wins
        let t1 = make_term("b-id", "key1", Some(vec!["shared-alias"]));
        let t2 = make_term("a-id", "key2", Some(vec!["shared-alias"]));
        let v = make_vocab(VocabularyMode::Open, vec![t1.clone(), t2.clone()]);
        let resolved = v.resolve_term_by_key("shared-alias").unwrap();
        assert_eq!(resolved.id, "a-id");
    }

    #[test]
    fn vocabulary_accepts_schema_key() {
        let json = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/vocabulary.json",
            "id": "v-test",
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "open",
            "terms": [],
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        let v: Vocabulary = serde_json::from_str(json).expect("must accept $schema");
        assert_eq!(v.id, "v-test");
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(
            !serialized.contains("\"extra\""),
            "flatten must not emit an 'extra' key"
        );
    }

    #[test]
    fn vocabulary_absorbs_unknown_fields() {
        let json = r#"{
            "id": "v-test",
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "open",
            "terms": [],
            "createdAt": "2026-01-01T00:00:00Z",
            "futureExtension": "some-value"
        }"#;
        let v: Vocabulary = serde_json::from_str(json).expect("unknown fields must be absorbed");
        assert_eq!(v.id, "v-test");
    }
}
