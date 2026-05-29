use crate::error::RepositoryError;
use crate::package::load_package;
use srs_core::types::view::{DocumentView, View};
use std::path::Path;

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

pub fn list_document_views(repo_root: &Path) -> Result<Vec<DocumentView>, RepositoryError> {
    let package = load_package(repo_root)?;
    Ok(package.document_views)
}

pub fn get_document_view_by_id(
    repo_root: &Path,
    id: &str,
) -> Result<GetDocumentViewResult, RepositoryError> {
    let package = load_package(repo_root)?;
    match package.resolve_document_view(id) {
        Some(view) => Ok(GetDocumentViewResult::Found(Box::new(view.clone()))),
        None => Ok(GetDocumentViewResult::NotFound),
    }
}

pub fn list_views(repo_root: &Path) -> Result<Vec<View>, RepositoryError> {
    let package = load_package(repo_root)?;
    Ok(package.views)
}

pub fn get_view_by_id(repo_root: &Path, id: &str) -> Result<GetViewResult, RepositoryError> {
    let package = load_package(repo_root)?;
    match package.resolve_view(id) {
        Some(view) => Ok(GetViewResult::Found(Box::new(view.clone()))),
        None => Ok(GetViewResult::NotFound),
    }
}
