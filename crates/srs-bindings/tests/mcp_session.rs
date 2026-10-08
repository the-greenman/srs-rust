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
fn note_update_replaces_authoring_fields() {
    let repo = open_repo();
    let mut session = repo.open_mcp_session().ok().unwrap();
    send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-06-18" }),
    );
    session.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let created = send(
        &mut session,
        2,
        "tools/call",
        json!({ "name": "note_create", "arguments": {
            "title": "Before", "sections": [{ "name": "body", "content": "one" }]
        }}),
    );
    let text = created["result"]["content"][0]["text"].as_str().unwrap();
    let id = serde_json::from_str::<serde_json::Value>(text).unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let updated = send(
        &mut session,
        3,
        "tools/call",
        json!({ "name": "note_update", "arguments": {
            "instanceId": id, "title": "After",
            "sections": [{ "name": "body", "content": "two" }]
        }}),
    );
    assert_eq!(updated["result"]["isError"], false, "{updated}");
    let after = repo.export_srsj().ok().unwrap();
    assert!(after.contains("After") && after.contains("two"));
    assert!(!after.contains("Before"));

    let missing = send(
        &mut session,
        4,
        "tools/call",
        json!({ "name": "note_update", "arguments": {
            "instanceId": "00000000-0000-4000-8000-000000000000", "sections": []
        }}),
    );
    assert_eq!(missing["result"]["isError"], true, "{missing}");
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

#[test]
fn mcp_container_create_then_add_members() {
    let repo = open_repo();
    let mut session = repo.open_mcp_session().ok().unwrap();
    send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-06-18" }),
    );
    let mut note = |id: i64, title: &str| -> String {
        let r = send(
            &mut session,
            id,
            "tools/call",
            json!({ "name": "note_create", "arguments": { "title": title, "sections": [] }}),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        r["result"]["structuredContent"]["instanceId"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (intro, body) = (note(2, "Intro"), note(3, "Body"));

    let created = send(
        &mut session,
        4,
        "tools/call",
        json!({ "name": "container_create", "arguments": {
            "title": "Essay", "containerType": "essay",
            "anchorInstanceId": intro, "identityInstanceId": intro,
            "memberInstanceIds": [{ "instanceId": intro }]
        }}),
    );
    assert_eq!(created["result"]["isError"], false, "{created}");
    let cid = created["result"]["structuredContent"]["containerId"]
        .as_str()
        .unwrap()
        .to_string();

    let added = send(
        &mut session,
        5,
        "tools/call",
        json!({ "name": "container_member_add",
                "arguments": { "containerId": cid, "instanceId": body, "depth": 0 }}),
    );
    assert_eq!(added["result"]["isError"], false, "{added}");
    let members = added["result"]["structuredContent"]["members"]
        .as_array()
        .unwrap();
    assert_eq!(members.len(), 2);

    // Validation failure: unresolvable member is a tool error, nothing created.
    let bad = send(
        &mut session,
        6,
        "tools/call",
        json!({ "name": "container_create", "arguments": {
            "title": "Bad", "memberInstanceIds": [{ "instanceId": "no-such-id" }] }}),
    );
    assert_eq!(bad["result"]["isError"], true, "{bad}");
}

/// srs-rust#1202 / ADR-049: dogfood sequence for `take_write_summary`.
#[test]
fn write_summary_reports_each_request_once() {
    let repo = open_repo();
    let mut session = repo.open_mcp_session().ok().unwrap();
    send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-06-18" }),
    );
    let mut n = 1;
    let mut call = |session: &mut srs_bindings::McpSession, name: &str, args: Value| -> Value {
        n += 1;
        send(
            session,
            n,
            "tools/call",
            json!({ "name": name, "arguments": args }),
        )
    };
    let summary = |s: &mut srs_bindings::McpSession| -> Option<Value> {
        s.take_write_summary()
            .map(|j| serde_json::from_str(&j).unwrap())
    };
    let id = |r: &Value, k: &str| {
        r["result"]["structuredContent"][k]
            .as_str()
            .unwrap()
            .to_string()
    };
    let kinds = |s: &Value| -> Vec<(String, String, String)> {
        let mut v: Vec<_> = s["changed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["target"].as_str().unwrap().into(),
                    c["id"].as_str().unwrap().into(),
                    c["kind"].as_str().unwrap().into(),
                )
            })
            .collect();
        v.sort();
        v
    };

    // reads write nothing
    call(&mut session, "repo_validate", json!({}));
    assert_eq!(summary(&mut session), None);

    // create
    let a = call(
        &mut session,
        "note_create",
        json!({ "title": "A", "sections": [] }),
    );
    let (a_id, b) = (
        id(&a, "instanceId"),
        call(
            &mut session,
            "note_create",
            json!({ "title": "B", "sections": [] }),
        ),
    );
    let s = summary(&mut session).unwrap();
    assert_eq!(s["tool"], "note_create");
    assert_eq!(
        kinds(&s),
        vec![("instance".into(), id(&b, "instanceId"), "created".into())]
    );
    assert_eq!(
        summary(&mut session),
        None,
        "drained: a second take is empty"
    );
    let b_id = id(&b, "instanceId");

    // relation
    let rel = call(
        &mut session,
        "relation_create",
        json!({
        "relationType": "depends-on", "sourceInstanceId": a_id, "targetInstanceId": b_id }),
    );
    assert_eq!(rel["result"]["isError"], false, "{rel}");
    let s = summary(&mut session).unwrap();
    assert_eq!(s["tool"], "relation_create");
    assert_eq!(
        kinds(&s),
        vec![("relation".into(), id(&rel, "relationId"), "created".into())]
    );

    // container create + member add + member move: containers are `updated`/`created`
    let c = call(
        &mut session,
        "container_create",
        json!({
        "title": "Essay", "containerType": "essay", "anchorInstanceId": a_id,
        "identityInstanceId": a_id, "memberInstanceIds": [{ "instanceId": a_id }] }),
    );
    assert_eq!(c["result"]["isError"], false, "{c}");
    let c_id = id(&c, "containerId");
    let s = summary(&mut session).unwrap();
    assert!(
        kinds(&s).contains(&("container".into(), c_id.clone(), "created".into())),
        "{s}"
    );
    call(
        &mut session,
        "container_member_add",
        json!({ "containerId": c_id, "instanceId": b_id, "depth": 0 }),
    );
    let s = summary(&mut session).unwrap();
    assert!(
        kinds(&s).contains(&("container".into(), c_id.clone(), "updated".into())),
        "{s}"
    );
    let mv = call(
        &mut session,
        "container_member_move",
        json!({ "containerId": c_id, "instanceId": b_id, "position": 0 }),
    );
    assert_eq!(mv["result"]["isError"], false, "{mv}");
    let s = summary(&mut session).unwrap();
    assert_eq!(s["tool"], "container_member_move");
    assert_eq!(
        kinds(&s),
        vec![("container".into(), c_id.clone(), "updated".into())]
    );

    // a UI write between requests is not attributed to the next agent request
    repo.delete_relation(&id(&rel, "relationId")).unwrap();
    call(&mut session, "repo_validate", json!({}));
    assert_eq!(summary(&mut session), None);

    // guard rejection: no summary, no epoch move
    session
        .set_write_guard(&json!({ "containerIds": [c_id] }).to_string())
        .ok()
        .unwrap();
    let e0 = session.write_epoch();
    let rej = call(
        &mut session,
        "container_member_remove",
        json!({ "containerId": c_id, "instanceId": b_id }),
    );
    assert_eq!(rej["result"]["isError"], true, "{rej}");
    assert_eq!(session.write_epoch(), e0);
    assert_eq!(summary(&mut session), None);
}
