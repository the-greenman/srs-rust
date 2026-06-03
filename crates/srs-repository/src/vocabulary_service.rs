use crate::error::RepositoryError;
use crate::store::RepositoryStore;
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
