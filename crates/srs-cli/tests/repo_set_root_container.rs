use srs_repository::manifest_service::{
    set_manifest_root_container, SetManifestRootContainerInput,
};
use srs_repository::FileStore;
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

#[test]
fn set_root_container_writes_canonical_manifest_embed() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo(tmp.path());
    let store = FileStore::new(tmp.path());

    let result = set_manifest_root_container(
        &store,
        SetManifestRootContainerInput {
            container_id: CONTAINER_ID.to_string(),
            identity_instance_id: IDENTITY_ID.to_string(),
            title: None,
        },
    )
    .unwrap();

    assert_eq!(result.container_id, CONTAINER_ID);
    assert_eq!(result.identity_instance_id, IDENTITY_ID);
    assert_eq!(result.title, "Test Repo");
    assert_eq!(result.member_instance_ids, vec![IDENTITY_ID]);

    let manifest_str = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
    let manifest_val: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();

    assert_eq!(
        manifest_val["container"]["containerId"].as_str(),
        Some(CONTAINER_ID)
    );
    assert_eq!(
        manifest_val["container"]["identityInstanceId"].as_str(),
        Some(IDENTITY_ID)
    );
    // Canonical embed shape (RFC-013): non-empty title + identity in memberInstanceIds.
    assert_eq!(
        manifest_val["container"]["title"].as_str(),
        Some("Test Repo")
    );
    assert_eq!(
        manifest_val["container"]["memberInstanceIds"],
        serde_json::json!([IDENTITY_ID])
    );
}

#[test]
fn set_root_container_explicit_title_flag_wins() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo(tmp.path());
    let store = FileStore::new(tmp.path());

    let result = set_manifest_root_container(
        &store,
        SetManifestRootContainerInput {
            container_id: CONTAINER_ID.to_string(),
            identity_instance_id: IDENTITY_ID.to_string(),
            title: Some("Explicit".to_string()),
        },
    )
    .unwrap();
    assert_eq!(result.title, "Explicit");
}
