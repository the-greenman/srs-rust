//! Full MCP surface through the raw JSON-RPC dispatcher over a real store:
//! the same path the browser session drives (no rmcp, no transport).

use serde_json::{json, Value};
use srs_core::types::blueprint::{Blueprint, TypeRef};
use srs_mcp_core::{McpDispatcher, SrsMcpApplication, MCP_PROTOCOL_VERSION};
use srs_repository::blueprint_service::create_blueprint;
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::FileStore;

const REPO_ID: &str = "core-surface";

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
                repository_id: REPO_ID.into(),
                namespace: "com.example.surface".into(),
                srs_version: "2.0".into(),
                title: Some("Surface fixture".into()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg".into(),
                namespace: "com.example.surface".into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
    let mut dispatcher = McpDispatcher::new(SrsMcpApplication::open(store).unwrap());
    let init = rpc(
        &mut dispatcher,
        "initialize",
        json!({ "protocolVersion": MCP_PROTOCOL_VERSION }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "srs-mcp");
    (dir, dispatcher)
}

fn rpc(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>, method: &str, params: Value) -> Value {
    d.dispatch(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .unwrap()
}

fn tool(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>, name: &str, args: Value) -> Value {
    rpc(d, "tools/call", json!({ "name": name, "arguments": args }))
}

#[test]
fn application_reads_repository_id_from_manifest() {
    let (_dir, d) = setup();
    assert_eq!(d.application().repository_id(), REPO_ID);
}

#[test]
fn tool_catalogue_has_all_nineteen_tools_and_core_owns_the_schemas() {
    let (_dir, mut d) = setup();
    let listed = rpc(&mut d, "tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 19);
    assert!(tools
        .iter()
        .all(|t| t["description"].is_string() && t["inputSchema"]["type"] == "object"));
}

#[test]
fn validated_write_then_read_resource_and_find() {
    let (_dir, mut d) = setup();
    let created = tool(
        &mut d,
        "note_create",
        json!({ "title": "Browser note", "sections": [{ "name": "body", "content": "hello" }] }),
    );
    assert_eq!(created["result"]["isError"], false, "{created}");
    let id = created["result"]["structuredContent"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let map = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": format!("srs://{REPO_ID}/map") }),
    );
    let content = &map["result"]["contents"][0];
    assert_eq!(content["mimeType"], "application/json");
    assert!(serde_json::from_str::<Value>(content["text"].as_str().unwrap()).is_ok());

    let tree = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": format!("srs://{REPO_ID}/tree/{id}") }),
    );
    assert!(tree["result"]["contents"][0]["text"].is_string(), "{tree}");

    let validate = tool(&mut d, "repo_validate", json!({}));
    assert_eq!(validate["result"]["isError"], false);
}

#[test]
fn service_rejection_is_a_tool_result_not_a_protocol_error() {
    let (_dir, mut d) = setup();
    let rejected = tool(
        &mut d,
        "record_create",
        json!({ "type": "no.such/type", "fieldValues": {} }),
    );
    assert!(rejected.get("error").is_none(), "{rejected}");
    assert_eq!(rejected["result"]["isError"], true);

    assert_eq!(
        tool(&mut d, "no_such_tool", json!({}))["error"]["code"],
        -32602
    );
    assert_eq!(
        tool(&mut d, "find", json!({ "bogus": 1 }))["error"]["code"],
        -32602,
        "unknown argument keys are invalid params"
    );
    assert_eq!(
        rpc(&mut d, "bogus/method", json!({}))["error"]["code"],
        -32601
    );
}

#[test]
fn resource_read_errors_preserve_mcp_codes() {
    let (_dir, mut d) = setup();
    let missing = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": format!("srs://{REPO_ID}/record/missing") }),
    );
    assert_eq!(missing["error"]["code"], -32002);
    let foreign = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": "srs://other/map" }),
    );
    assert_eq!(foreign["error"]["code"], -32602);
    assert_eq!(
        rpc(&mut d, "resources/read", json!({}))["error"]["code"],
        -32602
    );
}

fn blueprint(name: &str) -> Blueprint {
    Blueprint {
        schema: None,
        id: String::new(),
        namespace: "test.ns".into(),
        name: name.into(),
        version: 1,
        description: format!("{name} description"),
        root_types: vec![TypeRef {
            type_id: "placeholder-type-id".into(),
            type_version: None,
        }],
        structure: vec![],
        required_types: vec![],
        ai_guidance: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        lineage: None,
        provenance: None,
    }
}

#[test]
fn prompts_list_and_get_one_per_blueprint() {
    let (dir, mut d) = setup();
    let created = create_blueprint(&FileStore::new(dir.path()), blueprint("my-bp"), None).unwrap();
    let id = created.blueprint.id;

    let listed = rpc(&mut d, "prompts/list", json!({}));
    let prompts = listed["result"]["prompts"].as_array().unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0]["name"], id);
    assert!(prompts[0]["description"]
        .as_str()
        .unwrap()
        .contains("test.ns/my-bp"));

    let got = rpc(&mut d, "prompts/get", json!({ "name": id }));
    let message = &got["result"]["messages"][0];
    assert_eq!(message["role"], "user");
    assert_eq!(message["content"]["type"], "text");
    assert!(message["content"]["text"]
        .as_str()
        .unwrap()
        .contains("my-bp"));
}

#[test]
fn prompt_errors_use_invalid_params() {
    let (_dir, mut d) = setup();
    let unknown = rpc(&mut d, "prompts/get", json!({ "name": "no-such-id" }));
    assert_eq!(unknown["error"]["code"], -32602);
    assert!(unknown["error"]["message"]
        .as_str()
        .unwrap()
        .contains("prompt not found"));
    let with_args = rpc(
        &mut d,
        "prompts/get",
        json!({ "name": "x", "arguments": { "foo": "bar" } }),
    );
    assert_eq!(with_args["error"]["code"], -32602);
}
