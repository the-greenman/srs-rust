//! Owner ruling srs-rust#742 and RFC-052: identity is a very explicit modification.
//!
//! `container delete` refuses the root container (`container-root-delete`,
//! RFC-052 [R6]), and since a revision-10 manifest must name its root
//! (RFC-052 [R1]) `repo unset-root-container` is refused too: both leave the
//! repository exactly as it was.

use srs_repository::container_service::delete_container;
use srs_repository::manifest_service::{
    set_manifest_root_container, unset_manifest_root_container, SetManifestRootContainerInput,
};
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

fn repo_with_root() -> (TempDir, FileStore) {
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
    (tmp, store)
}

fn snapshot(dir: &std::path::Path) -> (String, String) {
    (
        std::fs::read_to_string(dir.join("manifest.json")).unwrap(),
        std::fs::read_to_string(dir.join(format!("containers/{CONTAINER_ID}.json"))).unwrap(),
    )
}

#[test]
fn container_delete_refuses_the_root_container() {
    let (tmp, store) = repo_with_root();
    let before = snapshot(tmp.path());

    let err = delete_container(&store, CONTAINER_ID).unwrap_err();
    assert_eq!(err.code(), "container-root-delete", "{err:?}");

    // Nothing changed: the manifest still names the root and its file is intact.
    assert_eq!(snapshot(tmp.path()), before);
    assert_eq!(
        store.load_manifest().unwrap().root_container_id(),
        Some(CONTAINER_ID)
    );
}

#[test]
fn unset_root_container_is_refused_and_changes_nothing() {
    let (tmp, store) = repo_with_root();
    let before = snapshot(tmp.path());

    let err = unset_manifest_root_container(&store).unwrap_err();
    assert_eq!(err.code(), "manifest-root-container-missing", "{err:?}");
    assert_eq!(snapshot(tmp.path()), before);
}
