use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::view::{DocumentView, View};

#[derive(Debug, Clone)]
pub enum GetDocumentViewResult {
    Found(Box<DocumentView>),
    NotFound,
}

#[derive(Debug, Clone)]
pub enum GetViewResult {
    Found(Box<View>),
    NotFound,
}

pub fn list_document_views(
    store: &dyn RepositoryStore,
) -> Result<Vec<DocumentView>, RepositoryError> {
    let package = store.load_package()?;
    Ok(package.document_views)
}

pub fn get_document_view_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetDocumentViewResult, RepositoryError> {
    let package = store.load_package()?;
    match package.resolve_document_view(id) {
        Some(view) => Ok(GetDocumentViewResult::Found(Box::new(view.clone()))),
        None => Ok(GetDocumentViewResult::NotFound),
    }
}

pub fn list_views(store: &dyn RepositoryStore) -> Result<Vec<View>, RepositoryError> {
    let package = store.load_package()?;
    Ok(package.views)
}

pub fn get_view_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetViewResult, RepositoryError> {
    let package = store.load_package()?;
    match package.resolve_view(id) {
        Some(view) => Ok(GetViewResult::Found(Box::new(view.clone()))),
        None => Ok(GetViewResult::NotFound),
    }
}
