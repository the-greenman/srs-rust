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

#[test]
fn write_epoch_tracks_mutation_only() {
    let repo = open_repo();
    let mut session = repo.open_mcp_session().ok().unwrap();
    send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-06-18" }),
    );
    session.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let e0 = session.write_epoch();
    send(&mut session, 2, "resources/list", json!({}));
    send(
        &mut session,
        3,
        "resources/read",
        json!({ "uri": "srs://browser-mcp/map" }),
    );
    send(
        &mut session,
        4,
        "tools/call",
        json!({ "name": "repo_validate", "arguments": {} }),
    );
    assert_eq!(session.write_epoch(), e0, "reads must not bump the epoch");

    let bad = send(
        &mut session,
        5,
        "tools/call",
        json!({ "name": "record_create", "arguments": {
            "typeId": "00000000-0000-0000-0000-000000000000", "fieldValues": {}
        }}),
    );
    assert!(
        bad.get("error").is_some() || bad["result"]["isError"] == true,
        "{bad}"
    );
    assert_eq!(
        session.write_epoch(),
        e0,
        "rejected write must not bump the epoch"
    );

    let bad_rel = send(
        &mut session,
        7,
        "tools/call",
        json!({ "name": "relation_create", "arguments": {
            "relationType": "depends-on",
            "sourceInstanceId": "00000000-0000-0000-0000-000000000001",
            "targetInstanceId": "00000000-0000-0000-0000-000000000002"
        }}),
    );
    assert!(
        bad_rel.get("error").is_some() || bad_rel["result"]["isError"] == true,
        "{bad_rel}"
    );
    assert_eq!(
        session.write_epoch(),
        e0,
        "rejected relation must not bump the epoch"
    );

    let ok = send(
        &mut session,
        6,
        "tools/call",
        json!({ "name": "note_create", "arguments": {
            "title": "n", "sections": [{ "name": "body", "content": "x" }]
        }}),
    );
    assert_eq!(ok["result"]["isError"], false, "{ok}");
    assert!(
        session.write_epoch() > e0,
        "validated write must bump the epoch"
    );
}
