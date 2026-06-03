use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::lifecycle::Lifecycle;

pub fn list_lifecycles(store: &dyn RepositoryStore) -> Result<Vec<Lifecycle>, RepositoryError> {
    match store.load_package() {
        Ok(package) => Ok(package.lifecycles),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(vec![]),
        Err(e) => Err(e),
    }
}

pub fn get_lifecycle_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Lifecycle>, RepositoryError> {
    match store.load_package() {
        Ok(package) => Ok(package.lifecycles.into_iter().find(|lc| lc.id == id)),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}
