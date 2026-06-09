use crate::types::vocabulary::{Vocabulary, VocabularyMode};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct VocabularyDiagnostic {
    pub severity: VocabularyDiagnosticSeverity,
    pub code: VocabularyDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VocabularyDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VocabularyDiagnosticCode {
    /// V5: duplicate key/alias in closed vocabulary (error)
    V5DuplicateKeyClosed,
    /// V5: duplicate key/alias in open vocabulary (warning)
    V5DuplicateKeyOpen,
    /// Extension mismatch: extendsVocabularyId present but extendsVocabularyVersion absent (or vice versa)
    ExtensionVersionMissing,
}

/// V5: Validate vocabulary key/alias uniqueness and extension consistency.
///
/// - Closed vocab: key/alias collisions are errors.
/// - Open vocab: collisions are warnings.
/// - extendsVocabularyId and extendsVocabularyVersion must both be present or both absent.
pub fn validate_vocabulary(vocab: &Vocabulary) -> Vec<VocabularyDiagnostic> {
    let mut diags = Vec::new();

    // Check extension field consistency
    match (
        vocab.extends_vocabulary_id.as_ref(),
        vocab.extends_vocabulary_version.as_ref(),
    ) {
        (Some(_), None) | (None, Some(_)) => {
            diags.push(VocabularyDiagnostic {
                severity: VocabularyDiagnosticSeverity::Error,
                code: VocabularyDiagnosticCode::ExtensionVersionMissing,
                message: "extendsVocabularyId and extendsVocabularyVersion must both be present"
                    .to_string(),
            });
        }
        _ => {}
    }

    // Collect keys and aliases from effective terms (non-retired)
    let effective = vocab.effective_terms();
    let mut seen: HashSet<&str> = HashSet::new();
    let is_closed = matches!(vocab.mode, VocabularyMode::Closed);

    for term in &effective {
        if seen.contains(term.key.as_str()) {
            diags.push(VocabularyDiagnostic {
                severity: if is_closed {
                    VocabularyDiagnosticSeverity::Error
                } else {
                    VocabularyDiagnosticSeverity::Warning
                },
                code: if is_closed {
                    VocabularyDiagnosticCode::V5DuplicateKeyClosed
                } else {
                    VocabularyDiagnosticCode::V5DuplicateKeyOpen
                },
                message: format!(
                    "duplicate key '{}' in vocabulary '{}'",
                    term.key, vocab.name
                ),
            });
        } else {
            seen.insert(&term.key);
        }

        if let Some(aliases) = &term.aliases {
            for alias in aliases {
                if seen.contains(alias.as_str()) {
                    diags.push(VocabularyDiagnostic {
                        severity: if is_closed {
                            VocabularyDiagnosticSeverity::Error
                        } else {
                            VocabularyDiagnosticSeverity::Warning
                        },
                        code: if is_closed {
                            VocabularyDiagnosticCode::V5DuplicateKeyClosed
                        } else {
                            VocabularyDiagnosticCode::V5DuplicateKeyOpen
                        },
                        message: format!(
                            "duplicate alias '{}' for term '{}' in vocabulary '{}'",
                            alias, term.key, vocab.name
                        ),
                    });
                } else {
                    seen.insert(alias);
                }
            }
        }
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::term::{Term, VocabularyEntryStatus};
    use crate::types::vocabulary::{Vocabulary, VocabularyMode};

    fn make_term(id: &str, key: &str, aliases: Option<Vec<&str>>, retired: bool) -> Term {
        Term {
            id: id.to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            key: key.to_string(),
            label: None,
            description: None,
            aliases: aliases.map(|a| a.iter().map(|s| s.to_string()).collect()),
            roles: None,
            status: if retired {
                Some(VocabularyEntryStatus::Retired)
            } else {
                None
            },
            properties: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn make_vocab(mode: VocabularyMode, terms: Vec<Term>) -> Vocabulary {
        Vocabulary {
            id: "v1".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            mode,
            terms,
            extends_vocabulary_id: None,
            extends_vocabulary_version: None,
            promotion_window: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn closed_vocab_duplicate_key_is_error() {
        let v = make_vocab(
            VocabularyMode::Closed,
            vec![
                make_term("id1", "foo", None, false),
                make_term("id2", "foo", None, false),
            ],
        );
        let diags = validate_vocabulary(&v);
        assert!(diags
            .iter()
            .any(|d| d.code == VocabularyDiagnosticCode::V5DuplicateKeyClosed
                && d.severity == VocabularyDiagnosticSeverity::Error));
    }

    #[test]
    fn open_vocab_duplicate_key_is_warning() {
        let v = make_vocab(
            VocabularyMode::Open,
            vec![
                make_term("id1", "foo", None, false),
                make_term("id2", "foo", None, false),
            ],
        );
        let diags = validate_vocabulary(&v);
        assert!(diags
            .iter()
            .any(|d| d.code == VocabularyDiagnosticCode::V5DuplicateKeyOpen
                && d.severity == VocabularyDiagnosticSeverity::Warning));
    }

    #[test]
    fn no_duplicate_returns_empty() {
        let v = make_vocab(
            VocabularyMode::Closed,
            vec![
                make_term("id1", "foo", None, false),
                make_term("id2", "bar", None, false),
            ],
        );
        assert!(validate_vocabulary(&v).is_empty());
    }

    #[test]
    fn retired_terms_excluded_from_uniqueness_check() {
        // Two terms with same key, but first is retired — should not collide
        let v = make_vocab(
            VocabularyMode::Closed,
            vec![
                make_term("id1", "foo", None, true),  // retired — excluded
                make_term("id2", "foo", None, false), // active — only one
            ],
        );
        assert!(validate_vocabulary(&v).is_empty());
    }

    #[test]
    fn extension_version_mismatch_is_error() {
        let mut v = make_vocab(VocabularyMode::Closed, vec![]);
        v.extends_vocabulary_id = Some("upstream-id".to_string());
        // Missing extends_vocabulary_version
        let diags = validate_vocabulary(&v);
        assert!(diags
            .iter()
            .any(|d| d.code == VocabularyDiagnosticCode::ExtensionVersionMissing));
    }

    #[test]
    fn alias_collision_with_key_is_error_in_closed() {
        // term1.key = "foo", term2.alias = "foo" → collision
        let v = make_vocab(
            VocabularyMode::Closed,
            vec![
                make_term("id1", "foo", None, false),
                make_term("id2", "bar", Some(vec!["foo"]), false),
            ],
        );
        let diags = validate_vocabulary(&v);
        assert!(diags
            .iter()
            .any(|d| d.code == VocabularyDiagnosticCode::V5DuplicateKeyClosed));
    }
}
