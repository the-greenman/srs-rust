//! Owner ruling srs-rust#742: identity is a very explicit modification.
//!
//! `container delete` must refuse the RFC-013 root container; `repo
//! unset-root-container` is the only path that removes it, and afterward
//! `repo validate` reports the missing root as the expected diagnostic
//! (I-79) rather than the repository silently losing its identity.

use srs_repository::container_service::delete_container;
use srs_repository::error::RepositoryError;
use srs_repository::manifest_service::{
    set_manifest_root_container, unset_manifest_root_container, SetManifestRootContainerInput,
};
use srs_repository::validation::validate_repository;
use srs_repository::{FileStore, RepositoryStore};
use tempfile::TempDir;

fn create_minimal_repo(dir: &std::path::Path) {
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"srsVersion":"2.0-draft","repositoryId":"test-repo","title":"Test Repo","dataModelRevision":2}"#,
    )
    .unwrap();
}

const CONTAINER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
const IDENTITY_ID: &str = "aaaaaaaa-0000-4000-8000-aaaaaaaaaaaa";

/// RED: before this unit, `container delete` on the root silently deleted it
/// (asymmetric with `get`/`update`, which already refused via the embed
/// fallback). This test documents the fix: it now refuses instead.
#[test]
fn container_delete_refuses_the_root_container() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo(tmp.path());
    let store = FileStore::new(tmp.path());

    set_manifest_root_container(
        &store,
        SetManifestRootContainerInput {
            container_id: CONTAINER_ID.to_string(),
            identity_instance_id: IDENTITY_ID.to_string(),
            title: None,
        },
    )
    .unwrap();

    let err = delete_container(&store, CONTAINER_ID).unwrap_err();
    assert!(
        matches!(
            err,
            RepositoryError::ContainerIsRepositoryRoot { ref container_id } if container_id == CONTAINER_ID
        ),
        "expected ContainerIsRepositoryRoot, got {err:?}"
    );

    // The repository's identity must be intact after the refusal.
    let manifest = store.load_manifest().unwrap();
    assert_eq!(
        manifest.container.as_ref().map(|c| c.container_id.as_str()),
        Some(CONTAINER_ID)
    );
}

/// GREEN: `repo unset-root-container` succeeds where `container delete`
/// refuses, and leaves `repo validate` reporting RFC-013 I-79 (the manifest's
/// root container is absent) as the expected diagnostic.
#[test]
fn unset_root_container_succeeds_and_validate_reports_i79() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo(tmp.path());
    let store = FileStore::new(tmp.path());

    set_manifest_root_container(
        &store,
        SetManifestRootContainerInput {
            container_id: CONTAINER_ID.to_string(),
            identity_instance_id: IDENTITY_ID.to_string(),
            title: None,
        },
    )
    .unwrap();

    // Sanity: a fully-formed root passes validation clean before the unset.
    let report = validate_repository(&store).unwrap();
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("I-79")),
        "did not expect I-79 while the root container is set: {:?}",
        report.diagnostics
    );

    let result = unset_manifest_root_container(&store).unwrap();
    assert_eq!(result.container_id, CONTAINER_ID);
    assert_eq!(result.identity_instance_id.as_deref(), Some(IDENTITY_ID));

    // `container delete` on the now-former root works like any other container
    // once it is no longer the declared root... except it was never file-backed
    // here (embed-only), so it simply no longer exists to delete or refuse.
    let err = delete_container(&store, CONTAINER_ID).unwrap_err();
    assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));

    let report = validate_repository(&store).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("I-79")),
        "expected RFC-013 I-79 diagnostic after unset-root-container, got: {:?}",
        report.diagnostics
    );
}
