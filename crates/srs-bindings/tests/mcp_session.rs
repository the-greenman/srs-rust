//! Browser MCP session over an open repository (srs-rust#1057).
//!
//! Native test (no wasm-pack): the session's `handle`/`export_srsj` paths return
//! plain strings, so the `#[wasm_bindgen]` surface is exercisable off-wasm.

use serde_json::{json, Value};
use srs_bindings::SrsRepository;
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::srsj::to_srsj_string;
use srs_repository::tree_session::new_tree_session;

fn open_repo() -> SrsRepository {
    let store = new_tree_session();
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "browser-mcp".into(),
                namespace: "com.example.browser".into(),
                srs_version: "2.0".into(),
                title: Some("Browser MCP".into()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg".into(),
                namespace: "com.example.browser".into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
    SrsRepository::load(&to_srsj_string(&store).unwrap())
        .ok()
        .unwrap()
}

fn send(session: &mut srs_bindings::McpSession, id: i64, method: &str, params: Value) -> Value {
    let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    serde_json::from_str(&session.handle(&message.to_string()).unwrap()).unwrap()
}

#[test]
fn mcp_write_is_visible_to_export_without_any_implicit_save() {
    let repo = open_repo();
    let before = repo.export_srsj().ok().unwrap();
    let mut session = repo.open_mcp_session().ok().unwrap();

    let init = send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-06-18" }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "srs-mcp");
    assert_eq!(
        session.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None
    );
    assert!(session.is_initialized());

    let resources = send(&mut session, 2, "resources/list", json!({}));
    assert!(resources["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["uri"] == "srs://browser-mcp/map"));

    let created = send(
        &mut session,
        3,
        "tools/call",
        json!({ "name": "note_create", "arguments": {
            "title": "Written over browser MCP",
            "sections": [{ "name": "body", "content": "hello" }]
        }}),
    );
    assert_eq!(created["result"]["isError"], false, "{created}");

    let after = repo.export_srsj().ok().unwrap();
    assert_ne!(before, after);
    assert!(after.contains("Written over browser MCP"));
}
