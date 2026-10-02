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
    let mut d = McpDispatcher::new(SrsMcpApplication::open(store).unwrap());
    d.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion": MCP_PROTOCOL_VERSION}}));
    (dir, d)
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
    assert!(text(&r).starts_with("actor-supplied"), "{}", text(&r));
    let r = tool(
        &mut d,
        "relation_create",
        json!({"relationType":"evidences","sourceInstanceId":"a","targetInstanceId":"b",
               "createdBy":{"kind":"human","id":"x"}}),
    );
    assert!(text(&r).starts_with("actor-supplied"), "{}", text(&r));
    let r = tool(
        &mut d,
        "record_create",
        json!({"type":"com.semanticops.core/purpose","fieldValues":{"statement":"s"},
               "createdBy":{"kind":"human","id":"x"}}),
    );
    assert!(text(&r).starts_with("actor-supplied"), "{}", text(&r));
    // A malformed (non-object) createdBy is still actor-supplied, not a parse error.
    let mut malformed = NOTE();
    malformed["createdBy"] = json!("x");
    assert!(text(&tool(&mut d, "note_create", malformed)).starts_with("actor-supplied"));
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
    assert!(text(&r).starts_with("actor-invalid"), "{}", text(&r));
    d.application_mut().set_session_actor(None);
    let r = tool(&mut d, "note_create", NOTE());
    assert_eq!(r["isError"], false);
    assert!(r["structuredContent"].get("createdBy").is_none());
}
