use crate::error::RepositoryError;
use crate::package_types::DefinitionKind;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::term::Term;
use srs_core::types::vocabulary::Vocabulary;

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

    // Determine the file path by scanning the package.json vocabularies array
    let pkg_json = store.load_package_json()?;
    let vocab_paths: Vec<String> = pkg_json["vocabularies"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    let vocab_path = vocab_paths
        .iter()
        .find(|rel| {
            // Load the file and check if it matches the vocabulary id
            let full = format!("package/{rel}");
            store
                .load_instance_json(&full)
                .map(|v| v["id"].as_str() == Some(vocabulary_id))
                .unwrap_or(false)
        })
        .map(|rel| format!("package/{rel}"))
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("vocabulary file for {}", vocabulary_id)),
        })?;

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
