//! RFC-046 MCP session actor (srs-rust#1171): host-set only, never from tool arguments.

use serde_json::{json, Value};
use srs_mcp_core::{McpDispatcher, SrsMcpApplication, MCP_PROTOCOL_VERSION};
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::FileStore;

fn setup() -> (
    tempfile::TempDir,
    McpDispatcher<SrsMcpApplication<FileStore>>,
) {
    let (dir, mut d) = setup_uninit();
    init(&mut d, json!({"protocolVersion": MCP_PROTOCOL_VERSION}));
    (dir, d)
}

fn init(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>, params: Value) -> Option<Value> {
    d.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":params}))
}

fn setup_uninit() -> (
    tempfile::TempDir,
    McpDispatcher<SrsMcpApplication<FileStore>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "actor-mcp".into(),
                namespace: "com.example.actor".into(),
                srs_version: "2.0".into(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg".into(),
                namespace: "com.example.actor".into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
    (
        dir,
        McpDispatcher::new(SrsMcpApplication::open(store).unwrap()),
    )
}

fn tool(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>, name: &str, args: Value) -> Value {
    d.dispatch(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name": name, "arguments": args}}))
        .unwrap()["result"]
        .clone()
}

fn text(r: &Value) -> String {
    r["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

const NOTE: fn() -> Value = || json!({"title":"T","sections":[{"name":"i","content":"x"}]});

#[test]
fn host_set_actor_is_stamped_and_tool_args_cannot_forge_one() {
    let (_d, mut d) = setup();
    d.application_mut()
        .set_session_actor(Some(json!({"kind":"ai","id":"agent-1"})));

    let ok = tool(&mut d, "note_create", NOTE());
    assert_eq!(ok["isError"], false, "{ok}");
    assert_eq!(
        ok["structuredContent"]["createdBy"],
        json!({"kind":"ai","id":"agent-1"})
    );

    // A createdBy tool argument is `actor-supplied`, on every creating tool.
    let mut forged = NOTE();
    forged["createdBy"] = json!({"kind":"human","id":"someone-else"});
    let r = tool(&mut d, "note_create", forged);
    assert_eq!(r["isError"], true);
    assert!(text(&r).contains("may not carry createdBy"), "{}", text(&r));
    let r = tool(
        &mut d,
        "relation_create",
        json!({"relationType":"evidences","sourceInstanceId":"a","targetInstanceId":"b",
               "createdBy":{"kind":"human","id":"x"}}),
    );
    assert!(text(&r).contains("may not carry createdBy"), "{}", text(&r));
    let r = tool(
        &mut d,
        "record_create",
        json!({"type":"com.semanticops.core/purpose","fieldValues":{"statement":"s"},
               "createdBy":{"kind":"human","id":"x"}}),
    );
    assert!(text(&r).contains("may not carry createdBy"), "{}", text(&r));
    // A malformed (non-object) createdBy is still actor-supplied, not a parse error.
    let mut malformed = NOTE();
    malformed["createdBy"] = json!("x");
    assert!(text(&tool(&mut d, "note_create", malformed)).contains("may not carry createdBy"));
    // Update with a createdBy that is not the stored value: actor-changed.
    let r = tool(
        &mut d,
        "record_update",
        json!({"instanceId":"00000000-0000-4000-8000-000000000000","fieldValues":{},
               "createdBy":{"kind":"human","id":"x"}}),
    );
    assert_eq!(r["isError"], true);
}

#[test]
fn clearing_or_invalidating_the_host_actor_changes_the_outcome() {
    let (_d, mut d) = setup();
    d.application_mut()
        .set_session_actor(Some(json!({"kind":"ai","id":""})));
    let r = tool(&mut d, "note_create", NOTE());
    assert!(text(&r).contains("not a valid Actor"), "{}", text(&r));
    d.application_mut().set_session_actor(None);
    let r = tool(&mut d, "note_create", NOTE());
    assert_eq!(r["isError"], false);
    assert!(r["structuredContent"].get("createdBy").is_none());
}

/// The actor as stamped on a fresh note (`createdBy`, absent => `Null`).
fn stamped(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>) -> Value {
    tool(d, "note_create", NOTE())["structuredContent"]["createdBy"].clone()
}

fn handle_session(
    actor: Value,
    client_info: Value,
) -> (
    tempfile::TempDir,
    McpDispatcher<SrsMcpApplication<FileStore>>,
) {
    let (dir, mut d) = setup_uninit();
    d.application_mut().set_session_actor(Some(actor));
    init(
        &mut d,
        json!({"protocolVersion": MCP_PROTOCOL_VERSION, "clientInfo": client_info}),
    );
    (dir, d)
}

#[test]
fn client_handle_becomes_name_and_stamp_keeps_host_id() {
    let (_t, mut d) = handle_session(
        json!({"kind":"ai","id":"agent-1"}),
        json!({"name":"  claude-code  ","version":"1"}),
    );
    assert_eq!(
        stamped(&mut d),
        json!({"kind":"ai","id":"agent-1","name":"claude-code"})
    );
}

#[test]
fn handle_is_capped_and_missing_or_empty_leaves_name_absent() {
    let (_t, mut d) = handle_session(
        json!({"kind":"ai","id":"a"}),
        json!({"name":"x".repeat(500)}),
    );
    assert_eq!(stamped(&mut d)["name"].as_str().unwrap().len(), 120);
    for info in [json!({"name":"   "}), json!({}), json!({"name": 5})] {
        let (_t, mut d) = handle_session(json!({"kind":"ai","id":"a"}), info);
        assert_eq!(stamped(&mut d), json!({"kind":"ai","id":"a"}));
    }
    // No clientInfo at all.
    let (_t, mut d) = setup_uninit();
    d.application_mut()
        .set_session_actor(Some(json!({"kind":"ai","id":"a"})));
    init(&mut d, json!({"protocolVersion": MCP_PROTOCOL_VERSION}));
    assert_eq!(stamped(&mut d), json!({"kind":"ai","id":"a"}));
}

#[test]
fn host_fixed_name_wins_and_client_cannot_touch_id() {
    let (_t, mut d) = handle_session(
        json!({"kind":"human","id":"u-1","name":"Host Name"}),
        json!({"name":"evil","id":"x","kind":"ai"}),
    );
    assert_eq!(
        stamped(&mut d),
        json!({"kind":"human","id":"u-1","name":"Host Name"})
    );
}

#[test]
fn reinitialize_refreshes_the_handle() {
    let (_t, mut d) = handle_session(json!({"kind":"ai","id":"a"}), json!({"name":"first"}));
    let r = init(
        &mut d,
        json!({"protocolVersion": "1999-01-01", "clientInfo":{"name":"second"}}),
    )
    .unwrap();
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(r["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
    assert_eq!(stamped(&mut d)["name"], "second");
}

#[test]
fn reinitialize_keeps_a_host_fixed_name() {
    let (_t, mut d) = handle_session(
        json!({"kind":"human","id":"u","name":"Host"}),
        json!({"name":"first"}),
    );
    init(
        &mut d,
        json!({"protocolVersion": MCP_PROTOCOL_VERSION, "clientInfo":{"name":"second"}}),
    );
    assert_eq!(stamped(&mut d)["name"], "Host");
}

#[test]
fn no_session_actor_means_no_handle_stamp() {
    let (_t, mut d) = setup_uninit();
    init(
        &mut d,
        json!({"protocolVersion": MCP_PROTOCOL_VERSION, "clientInfo":{"name":"c"}}),
    );
    assert!(stamped(&mut d).is_null());
}

#[test]
fn multibyte_cap_and_control_chars() {
    let (_t, mut d) = handle_session(
        json!({"kind":"ai","id":"a"}),
        json!({"name":"é".repeat(200)}),
    );
    assert_eq!(
        stamped(&mut d)["name"].as_str().unwrap().chars().count(),
        120
    );
    let (_t, mut d) = handle_session(json!({"kind":"ai","id":"a"}), json!({"name":"a\nb\u{0}c"}));
    assert_eq!(stamped(&mut d)["name"], "abc");
    let (_t, mut d) = handle_session(json!({"kind":"ai","id":"a"}), json!({"name":"\n\u{0}\t"}));
    assert_eq!(stamped(&mut d), json!({"kind":"ai","id":"a"}));
}

#[test]
fn null_or_empty_host_name_is_not_fixed() {
    for n in [Value::Null, json!("")] {
        let (_t, mut d) =
            handle_session(json!({"kind":"ai","id":"a","name":n}), json!({"name":"h"}));
        assert_eq!(stamped(&mut d)["name"], "h");
    }
}

#[test]
fn later_set_actor_replaces_actor_and_drops_handle() {
    let (_t, mut d) = handle_session(json!({"kind":"ai","id":"a"}), json!({"name":"h"}));
    d.application_mut()
        .set_session_actor(Some(json!({"kind":"ai","id":"b"})));
    assert_eq!(stamped(&mut d), json!({"kind":"ai","id":"b"}));
}

#[test]
fn initialized_notification_and_tool_args_leave_actor_unchanged() {
    let (_t, mut d) = handle_session(json!({"kind":"ai","id":"a"}), json!({"name":"h"}));
    d.dispatch(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let mut args = NOTE();
    args["actor"] = json!({"kind":"human","id":"x","name":"evil"});
    args["name"] = json!("evil");
    let _ = tool(&mut d, "note_create", args);
    assert_eq!(stamped(&mut d), json!({"kind":"ai","id":"a","name":"h"}));
}

#[test]
fn record_create_stamps_host_id_and_handle() {
    let (_t, mut d) = handle_session(json!({"kind":"ai","id":"agent-1"}), json!({"name":"cc"}));
    let r = tool(
        &mut d,
        "record_create",
        json!({"type":"com.semanticops.core/purpose","fieldValues":{"statement":"s"}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(
        r["structuredContent"]["createdBy"],
        json!({"kind":"ai","id":"agent-1","name":"cc"})
    );
}
