use srs_repository::manifest_service::{set_manifest_root_container, SetManifestRootContainerInput};
use srs_repository::FileStore;
use tempfile::TempDir;

fn create_minimal_repo(dir: &std::path::Path) {
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"srsVersion":"2.0-draft","repositoryId":"test-repo","instanceIndex":[]}"#,
    )
    .unwrap();
}

#[test]
fn set_root_container_writes_manifest() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo(tmp.path());
    let store = FileStore::new(tmp.path());

    let result = set_manifest_root_container(
        &store,
        SetManifestRootContainerInput {
            container_id: "cid-abc".to_string(),
            identity_instance_id: "iid-xyz".to_string(),
        },
    )
    .unwrap();

    assert_eq!(result.container_id, "cid-abc");
    assert_eq!(result.identity_instance_id, "iid-xyz");

    let manifest_str = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
    let manifest_val: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();

    assert_eq!(
        manifest_val["container"]["containerId"].as_str(),
        Some("cid-abc")
    );
    assert_eq!(
        manifest_val["container"]["identityInstanceId"].as_str(),
        Some("iid-xyz")
    );
}
