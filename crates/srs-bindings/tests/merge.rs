// The WASM merge surface over the same service (srs-rust#1580). A refusal throws a `JsValue`,
// which is not meaningful on a native target, so the refusals are tested on the service
// (`srs-repository/tests/merge_copies.rs`); the wasm32 build gate covers the binding layer.

use serde_json::{json, Value};
use srs_bindings::SrsRepository;
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::{archive_to_vec, services, FileStore, RepositoryStore};
use std::rc::Rc;

fn origin() -> FileStore {
    let store = FileStore::from_vfs(Rc::new(srs_repository::vfs::MemVfs::new()));
    store.set_session_actor(Some(json!({"kind": "ai", "id": "agent-1"})));
    create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "15800000-0000-4000-8000-000000000033".to_string(),
                namespace: "com.test.merge".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "15800000-0000-4000-8000-000000000034".to_string(),
                namespace: "com.test.merge".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    store
}

#[test]
fn preview_then_apply_over_archive_bytes() {
    let store = origin();
    let copy = archive_to_vec(&store).unwrap();
    let target = SrsRepository::load_archive(&copy).unwrap();

    let note: srs_core::types::note::Note = serde_json::from_value(json!({
        "title": "added",
        "sections": [{"name": "body", "content": "x"}],
    }))
    .unwrap();
    let id = services::create_note(&store, note)
        .unwrap()
        .note
        .instance_id;
    let archive = archive_to_vec(&store).unwrap();

    let preview: Value =
        serde_json::from_str(&target.merge_preview(&archive, None).unwrap()).unwrap();
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["bring"]["instances"][0]["instanceId"], id);

    let selection = json!({"instanceIds": [id]}).to_string();
    let checked: Value = serde_json::from_str(
        &target
            .merge_preview(&archive, Some(selection.clone()))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(checked["unavailable"], json!([]));

    let applied: Value =
        serde_json::from_str(&target.merge_apply(&archive, &selection).unwrap()).unwrap();
    assert_eq!(applied["applied"], true);
    assert_eq!(
        applied["bring"]["instances"][0]["createdBy"]["id"],
        "agent-1"
    );

    // Nothing is left to bring.
    let again: Value =
        serde_json::from_str(&target.merge_preview(&archive, None).unwrap()).unwrap();
    assert_eq!(again["bring"]["instances"], json!([]));
}
