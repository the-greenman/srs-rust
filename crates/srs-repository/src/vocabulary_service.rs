use crate::error::RepositoryError;
use crate::package_types::DefinitionKind;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::term::{Term, VocabularyEntryStatus};
use srs_core::types::vocabulary::{Vocabulary, VocabularyMode};
use std::collections::HashMap;

/// Load package, returning empty result if no package exists.
fn load_package_optional(
    store: &dyn RepositoryStore,
) -> Result<crate::package::Package, RepositoryError> {
    store.load_package()
}

pub fn list_vocabularies(store: &dyn RepositoryStore) -> Result<Vec<Vocabulary>, RepositoryError> {
    match load_package_optional(store) {
        Ok(package) => Ok(package.vocabularies),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(vec![]),
        Err(e) => Err(e),
    }
}

pub fn get_vocabulary_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Vocabulary>, RepositoryError> {
    match load_package_optional(store) {
        Ok(package) => Ok(package.vocabularies.into_iter().find(|v| v.id == id)),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Collect all Terms across all vocabularies in the package.
pub fn list_terms(store: &dyn RepositoryStore) -> Result<Vec<Term>, RepositoryError> {
    match load_package_optional(store) {
        Ok(package) => Ok(package
            .vocabularies
            .into_iter()
            .flat_map(|v| v.terms)
            .collect()),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(vec![]),
        Err(e) => Err(e),
    }
}

/// Find a Term by id across all vocabularies.
pub fn get_term_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Term>, RepositoryError> {
    let terms = list_terms(store)?;
    Ok(terms.into_iter().find(|t| t.id == id))
}

/// Result of `create_vocabulary`.
pub struct CreateVocabularyResult {
    pub vocabulary: Vocabulary,
}

/// Create a new Vocabulary in the primary package.
///
/// Writes `package/vocabularies/{slug}-{id}.json` and adds the path to
/// `package/package.json` → `vocabularies[]`. If `vocabulary.id` is empty,
/// a new UUID is generated. If `vocabulary.created_at` is empty, the
/// current timestamp is used.
pub fn create_vocabulary(
    store: &dyn RepositoryStore,
    mut vocabulary: Vocabulary,
) -> Result<CreateVocabularyResult, RepositoryError> {
    store.load_package_boundary(&None)?;

    if vocabulary.id.trim().is_empty() {
        vocabulary.id = new_instance_id();
    }
    if vocabulary.created_at.trim().is_empty() {
        vocabulary.created_at = chrono::Utc::now().to_rfc3339();
    }

    // Assign IDs to any terms that are missing one
    for term in &mut vocabulary.terms {
        if term.id.trim().is_empty() {
            term.id = new_instance_id();
        }
    }

    let slug = vocabulary
        .name
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-");
    let rel_filename = format!("vocabularies/{}-{}.json", slug, &vocabulary.id[..8]);
    let full_path = format!("package/{rel_filename}");

    store.ensure_vocabularies_dir("package/vocabularies")?;
    store.save_vocabulary(&full_path, &vocabulary)?;
    store.add_definition_to_boundary(&None, DefinitionKind::Vocabulary, &rel_filename)?;

    Ok(CreateVocabularyResult { vocabulary })
}

/// Result of `create_term`.
pub struct CreateTermResult {
    pub term: Term,
    pub vocabulary: Vocabulary,
}

/// Add a new Term to an existing Vocabulary (identified by `vocabulary_id`).
///
/// This is a read-modify-write of the vocabulary file: loads the vocabulary,
/// appends the term, writes the file back. The `package.json` entry is
/// unchanged (the file path stays the same).
///
/// Returns an error if no vocabulary with `vocabulary_id` exists in the package.
pub fn create_term(
    store: &dyn RepositoryStore,
    vocabulary_id: &str,
    mut term: Term,
) -> Result<CreateTermResult, RepositoryError> {
    let package = store.load_package()?;

    // Find the vocabulary and its file path
    let vocab = package
        .vocabularies
        .into_iter()
        .find(|v| v.id == vocabulary_id)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("vocabulary/{}", vocabulary_id)),
        })?;

    let vocab_path = find_vocabulary_file_path(store, vocabulary_id)?;

    if term.id.trim().is_empty() {
        term.id = new_instance_id();
    }
    if term.created_at.is_none() {
        term.created_at = Some(chrono::Utc::now().to_rfc3339());
    }
    // Inherit namespace from vocabulary if not set
    if term.namespace.trim().is_empty() {
        term.namespace = vocab.namespace.clone();
    }

    let mut updated_vocab = vocab;
    updated_vocab.terms.push(term.clone());

    store.save_vocabulary(&vocab_path, &updated_vocab)?;

    Ok(CreateTermResult {
        term,
        vocabulary: updated_vocab,
    })
}

/// Find the repo-root-relative path for a vocabulary file by scanning the package.json index.
fn find_vocabulary_file_path(
    store: &dyn RepositoryStore,
    vocabulary_id: &str,
) -> Result<String, RepositoryError> {
    let pkg_json = store.load_package_json()?;
    let vocab_paths: Vec<String> = pkg_json["vocabularies"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    vocab_paths
        .iter()
        .find(|rel| {
            let full = format!("package/{rel}");
            store
                .load_instance_json(&full)
                .map(|v| v["id"].as_str() == Some(vocabulary_id))
                .unwrap_or(false)
        })
        .map(|rel| format!("package/{rel}"))
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("vocabulary file for {}", vocabulary_id)),
        })
}

/// Collect all tag string counts from the manifest instance index.
/// Returns a map of tag_key → usage count across all instances.
fn collect_tag_key_counts(
    store: &dyn RepositoryStore,
) -> Result<HashMap<String, usize>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in &manifest.instance_index {
        if let Some(tags) = &entry.tags {
            for tag in tags {
                *counts.entry(tag.clone()).or_insert(0) += 1;
            }
        }
    }
    Ok(counts)
}

/// How a tag key in use relates to the vocabulary after promotion.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TagSetEntryClassification {
    /// Key has no active term in the vocabulary; reads will break after close.
    WillBeInvalid,
    /// Key resolves to a deprecated or tombstone term; existing reads survive, new writes rejected.
    ReadOnlyAfterClose,
    /// Key resolves to an active term; fine after promotion.
    UsedAndActive,
}

/// A single entry in the derived tag set.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSetEntry {
    pub key: String,
    pub usage_count: usize,
    pub classification: TagSetEntryClassification,
}

/// Input for `derive_tag_set`.
pub struct DeriveTagSetInput {
    pub vocabulary_id: String,
}

/// Result of `derive_tag_set`.
pub struct DeriveTagSetResult {
    /// The resolved vocabulary the tag set was derived against.
    pub vocabulary: Vocabulary,
    pub entries: Vec<TagSetEntry>,
}

/// Build the derived tag set for a vocabulary: lists every in-use tag key and
/// classifies it according to V10 (will-be-invalid / read-only-after-close / used-and-active).
pub fn derive_tag_set(
    store: &dyn RepositoryStore,
    input: DeriveTagSetInput,
) -> Result<DeriveTagSetResult, RepositoryError> {
    let vocab = get_vocabulary_by_id(store, &input.vocabulary_id)?.ok_or_else(|| {
        RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("vocabulary/{}", input.vocabulary_id)),
        }
    })?;
    let counts = collect_tag_key_counts(store)?;
    let mut entries = Vec::new();
    for (key, usage_count) in &counts {
        let classification = classify_key_against_vocabulary(key, &vocab);
        entries.push(TagSetEntry {
            key: key.clone(),
            usage_count: *usage_count,
            classification,
        });
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(DeriveTagSetResult {
        vocabulary: vocab,
        entries,
    })
}

fn classify_key_against_vocabulary(key: &str, vocab: &Vocabulary) -> TagSetEntryClassification {
    // Check all terms (including retired) to determine classification
    let all_terms = &vocab.terms;
    // First: check effective (non-retired) terms
    let effective_match = all_terms.iter().find(|t| {
        let status = t.status.as_ref().unwrap_or(&VocabularyEntryStatus::Active);
        !status.is_retired()
            && (t.key == key
                || t.aliases
                    .as_ref()
                    .map(|a| a.iter().any(|al| al == key))
                    .unwrap_or(false))
    });
    match effective_match {
        Some(term) => {
            let status = term
                .status
                .as_ref()
                .unwrap_or(&VocabularyEntryStatus::Active);
            match status {
                VocabularyEntryStatus::Deprecated | VocabularyEntryStatus::Tombstone => {
                    TagSetEntryClassification::ReadOnlyAfterClose
                }
                _ => TagSetEntryClassification::UsedAndActive,
            }
        }
        None => TagSetEntryClassification::WillBeInvalid,
    }
}

/// Input for `promote_vocabulary`.
pub struct PromoteVocabularyInput {
    pub vocabulary_id: String,
}

/// Result of `promote_vocabulary`.
pub struct PromoteVocabularyResult {
    pub vocabulary: Vocabulary,
}

/// Promote a vocabulary from open → closed mode (V10 pre-flight).
///
/// V10 rules:
/// - Collects all in-use tag keys from manifest instance index.
/// - Classifies each key against the vocabulary's effective terms.
/// - Keys that `WillBeInvalid` block promotion unless:
///   - The vocabulary has a `promotionWindow.until` date that has not yet passed.
/// - If not blocked, the vocabulary's mode is set to `Closed` and saved.
pub fn promote_vocabulary(
    store: &dyn RepositoryStore,
    input: PromoteVocabularyInput,
) -> Result<PromoteVocabularyResult, RepositoryError> {
    let vocab = get_vocabulary_by_id(store, &input.vocabulary_id)?.ok_or_else(|| {
        RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("vocabulary/{}", input.vocabulary_id)),
        }
    })?;

    let tag_counts = collect_tag_key_counts(store)?;

    // Classify all in-use keys
    let will_be_invalid: Vec<String> = tag_counts
        .keys()
        .filter(|key| {
            classify_key_against_vocabulary(key, &vocab) == TagSetEntryClassification::WillBeInvalid
        })
        .cloned()
        .collect();

    // V10 pre-flight: check if promotion window covers will-be-invalid keys
    let blocked = if will_be_invalid.is_empty() {
        false
    } else {
        match &vocab.promotion_window {
            None => true, // No grace window → block immediately
            Some(window) => {
                let today = chrono::Utc::now().date_naive();
                // A malformed date is treated as already expired (conservative: blocks promotion).
                let until = window
                    .until
                    .parse::<chrono::NaiveDate>()
                    .unwrap_or(chrono::NaiveDate::MIN);
                today > until // blocked if today is past the window
            }
        }
    };

    if blocked {
        let mut sorted_keys = will_be_invalid;
        sorted_keys.sort();
        // Note: usage counts (how many instances use each key) are intentionally omitted
        // from the error — callers that need counts should call derive_tag_set first.
        return Err(RepositoryError::VocabularyPromotionBlocked {
            vocabulary_id: input.vocabulary_id,
            unresolvable_keys: sorted_keys,
        });
    }

    let vocab_path = find_vocabulary_file_path(store, &input.vocabulary_id)?;

    let mut promoted = vocab;
    promoted.mode = VocabularyMode::Closed;

    store.save_vocabulary(&vocab_path, &promoted)?;

    Ok(PromoteVocabularyResult {
        vocabulary: promoted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use srs_core::types::vocabulary::VocabularyMode;

    fn make_vocab(name: &str) -> Vocabulary {
        Vocabulary {
            id: String::new(),
            version: 1,
            namespace: "com.test".to_string(),
            name: name.to_string(),
            mode: VocabularyMode::Open,
            terms: vec![],
            extends_vocabulary_id: None,
            extends_vocabulary_version: None,
            promotion_window: None,
            description: None,
            created_at: String::new(),
        }
    }

    fn make_term(key: &str) -> srs_core::types::term::Term {
        srs_core::types::term::Term {
            id: String::new(),
            version: 1,
            namespace: "com.test".to_string(),
            key: key.to_string(),
            label: None,
            description: None,
            aliases: None,
            roles: None,
            status: None,
            properties: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn list_vocabularies_empty_when_no_package() {
        let store = MemoryStore::default();
        let result = list_vocabularies(&store).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn create_vocabulary_assigns_id_and_writes_file() {
        let store = MemoryStore::default();
        let result = create_vocabulary(&store, make_vocab("my-vocab")).unwrap();
        assert!(!result.vocabulary.id.is_empty());
        let found = get_vocabulary_by_id(&store, &result.vocabulary.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "my-vocab");
    }

    #[test]
    fn create_vocabulary_roundtrips_via_file_store() {
        let store = MemoryStore::default();
        let result = create_vocabulary(&store, make_vocab("roundtrip-vocab")).unwrap();
        let vocab = result.vocabulary;
        let found = get_vocabulary_by_id(&store, &vocab.id).unwrap().unwrap();
        assert_eq!(found.id, vocab.id);
        assert_eq!(found.name, vocab.name);
        assert_eq!(found.mode, VocabularyMode::Open);
        assert!(found.terms.is_empty());
    }

    #[test]
    fn create_term_appends_to_vocabulary() {
        let store = MemoryStore::default();
        let vocab_result = create_vocabulary(&store, make_vocab("vocab-with-terms")).unwrap();
        let vocab_id = vocab_result.vocabulary.id.clone();
        create_term(&store, &vocab_id, make_term("my-key")).unwrap();
        let terms = list_terms(&store).unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].key, "my-key");
    }

    #[test]
    fn get_vocabulary_by_id_finds_created() {
        let store = MemoryStore::default();
        let created = create_vocabulary(&store, make_vocab("find-me")).unwrap();
        let id = created.vocabulary.id.clone();
        let found = get_vocabulary_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(found.id, id);
    }

    #[test]
    fn list_terms_returns_terms_across_vocabularies() {
        let store = MemoryStore::default();
        let v1 = create_vocabulary(&store, make_vocab("vocab-a")).unwrap();
        let v2 = create_vocabulary(&store, make_vocab("vocab-b")).unwrap();
        create_term(&store, &v1.vocabulary.id, make_term("term-a")).unwrap();
        create_term(&store, &v2.vocabulary.id, make_term("term-b")).unwrap();
        let terms = list_terms(&store).unwrap();
        assert_eq!(terms.len(), 2);
        let keys: Vec<&str> = terms.iter().map(|t| t.key.as_str()).collect();
        assert!(keys.contains(&"term-a"));
        assert!(keys.contains(&"term-b"));
    }
}
