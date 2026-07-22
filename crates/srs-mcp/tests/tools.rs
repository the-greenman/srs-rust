//! Phase 3 integration tests: tools, exercised through a real rmcp client
//! over an in-process duplex transport. Fixture repos are built inline per
//! test with `srs-repository` writer services.

use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::ServiceExt;
use srs_mcp::SrsMcpServer;
use srs_repository::discovery_service::{self, DiscoveryQuery};
use srs_repository::package_service::{
    create_field_normalized, create_relation_type_normalized, create_type_normalized,
};
use srs_repository::record_store::list_records_by_type;
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::store::FileStore;

const NS: &str = "com.example.mcptest";

struct Fixture {
    dir: tempfile::TempDir,
    title_field_id: String,
    body_field_id: String,
}

/// Repo with one type `com.example.mcptest/decision`:
/// title (string, required) + body (text, optional).
fn make_fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: uuid::Uuid::new_v4().to_string(),
                namespace: NS.into(),
                srs_version: "2.0-draft".into(),
                title: Some("MCP Tools Test Repo".into()),
                description: Some("Fixture repository for srs-mcp tool tests".into()),
            },
            primary_package: PrimaryPackageMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                namespace: NS.into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();

    let title_field_id = uuid::Uuid::new_v4().to_string();
    let body_field_id = uuid::Uuid::new_v4().to_string();
    for (id, name, value_type) in [
        (&title_field_id, "title", "string"),
        (&body_field_id, "body", "text"),
    ] {
        create_field_normalized(
            &store,
            serde_json::json!({
                "id": id,
                "namespace": NS,
                "name": name,
                "version": 1,
                "valueType": value_type
            }),
            None,
        )
        .unwrap();
    }

    let type_id = uuid::Uuid::new_v4().to_string();
    create_type_normalized(
        &store,
        serde_json::json!({
            "id": type_id,
            "namespace": NS,
            "name": "decision",
            "version": 1,
            "fields": [
                { "fieldId": title_field_id, "order": 1, "required": true },
                { "fieldId": body_field_id, "order": 2, "required": false }
            ]
        }),
        None,
    )
    .unwrap();

    // Install `depends-on` explicitly: the implicit core merge (ADR-025)
    // currently carries fields + record types only, not relation types, so a
    // fresh repo resolves no canonical relation vocabulary (R3/RFC-005 gap —
    // affects the CLI identically; tracked as a filed issue from this plan).
    create_relation_type_normalized(
        &store,
        serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "version": 1,
            "key": "depends-on",
            "namespace": NS,
            "label": "Depends on",
            "description": "Source depends on target",
            "category": "dependency"
        }),
        None,
    )
    .unwrap();

    Fixture {
        dir,
        title_field_id,
        body_field_id,
    }
}

async fn connect(fixture: &Fixture) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let (client_stream, server_stream) = tokio::io::duplex(1 << 20);
    let server = SrsMcpServer::new(fixture.dir.path().to_path_buf()).unwrap();
    tokio::spawn(async move {
        if let Ok(service) = server.serve(server_stream).await {
            let _ = service.waiting().await;
        }
    });
    ().serve(client_stream).await.unwrap()
}

fn args(value: serde_json::Value) -> rmcp::model::JsonObject {
    match value {
        serde_json::Value::Object(map) => map,
        _ => panic!("args must be an object"),
    }
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    arguments: serde_json::Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(args(arguments)))
        .await
        .unwrap()
}

fn text_of(result: &rmcp::model::CallToolResult) -> &str {
    match &result.content[0] {
        ContentBlock::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    }
}

#[tokio::test]
async fn tool_repo_validate_clean_fixture_zero_diagnostics() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let result = call(&client, "repo_validate", serde_json::json!({})).await;
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(
        structured["diagnostics"].as_array().unwrap().len(),
        0,
        "clean fixture should have zero diagnostics: {structured}"
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_record_create_happy_then_validate() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let result = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.title_field_id, "value": "First decision" },
                { "fieldId": fx.body_field_id, "value": "Because reasons." }
            ],
            "tags": ["mcp-test"]
        }),
    )
    .await;
    assert_eq!(result.is_error, Some(false), "create failed: {result:?}");
    let record = result.structured_content.as_ref().unwrap();
    let instance_id = record["instanceId"].as_str().unwrap();

    // The write is real: the service read path sees exactly this record.
    let store = FileStore::new(fx.dir.path());
    let listed = list_records_by_type(&store, NS, "decision").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].instance_id, instance_id);

    // And the repository is still consistent.
    let validate = call(&client, "repo_validate", serde_json::json!({})).await;
    assert_eq!(
        validate.structured_content.as_ref().unwrap()["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_record_create_missing_required_is_error_no_write() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let result = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.body_field_id, "value": "No title supplied" }
            ]
        }),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let message = text_of(&result);
    assert!(
        message.contains("title") || message.to_lowercase().contains("required"),
        "diagnostic should name the missing field: {message}"
    );

    // Nothing was written.
    let store = FileStore::new(fx.dir.path());
    assert!(list_records_by_type(&store, NS, "decision")
        .unwrap()
        .is_empty());

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_relation_create_unknown_type_rejected() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    // Create two notes to relate.
    let mut ids = Vec::new();
    for title in ["A", "B"] {
        let result = call(
            &client,
            "note_create",
            serde_json::json!({
                "title": title,
                "sections": [{ "name": "body", "content": "x" }]
            }),
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        ids.push(
            result.structured_content.as_ref().unwrap()["instanceId"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    // Unknown relation type → rejected, nothing written (R3 / RFC-005).
    let rejected = call(
        &client,
        "relation_create",
        serde_json::json!({
            "relationType": "com.example/not-installed",
            "sourceInstanceId": ids[0],
            "targetInstanceId": ids[1]
        }),
    )
    .await;
    assert_eq!(rejected.is_error, Some(true));
    assert!(
        text_of(&rejected).contains("not-installed"),
        "error should name the unresolvable type: {}",
        text_of(&rejected)
    );

    // Canonical type from the implicitly-merged core package works.
    let ok = call(
        &client,
        "relation_create",
        serde_json::json!({
            "relationType": "depends-on",
            "sourceInstanceId": ids[0],
            "targetInstanceId": ids[1]
        }),
    )
    .await;
    assert_eq!(ok.is_error, Some(false), "canonical type failed: {ok:?}");
    let relation = ok.structured_content.as_ref().unwrap();
    assert!(!relation["relationId"].as_str().unwrap().is_empty());

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_note_create_and_find_roundtrip() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    // note_create works (Tier 0 capture)...
    let created = call(
        &client,
        "note_create",
        serde_json::json!({
            "title": "Meeting capture",
            "sections": [
                { "name": "body", "content": "We discussed the quarterly budget.", "label": "Body" }
            ],
            "tags": ["meeting"]
        }),
    )
    .await;
    assert_eq!(created.is_error, Some(false));
    assert!(!created.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .is_empty());

    // ...but discovery serves Tier 2 (Phase 1 of ext:discovery), so the
    // find roundtrip goes through a typed record with matching content.
    let record = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.title_field_id, "value": "Budget decision" },
                { "fieldId": fx.body_field_id, "value": "We approved the quarterly budget." }
            ]
        }),
    )
    .await;
    assert_eq!(record.is_error, Some(false), "create failed: {record:?}");
    let record_id = record.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let found = call(
        &client,
        "find",
        serde_json::json!({ "contentMatch": "quarterly budget" }),
    )
    .await;
    assert_eq!(found.is_error, Some(false), "find failed: {found:?}");
    let structured = found.structured_content.as_ref().unwrap();

    // Equal to calling the service directly with the same query.
    let store = FileStore::new(fx.dir.path());
    let direct = discovery_service::find(
        &store,
        DiscoveryQuery {
            content_match: Some("quarterly budget".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(structured, &serde_json::to_value(&direct).unwrap());
    let hits = structured["hits"].as_array().unwrap();
    assert!(
        hits.iter()
            .any(|h| h["instanceId"].as_str() == Some(record_id.as_str())),
        "created record should be found: {structured}"
    );

    // Tier-0 discovery is deferred by the service: diagnostic + zero hits.
    let tier0 = call(
        &client,
        "find",
        serde_json::json!({ "contentMatch": "quarterly budget", "tier": 0 }),
    )
    .await;
    let tier0_structured = tier0.structured_content.as_ref().unwrap();
    assert_eq!(tier0_structured["total"], 0);
    assert!(!tier0_structured["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_call_malformed_args_invalid_params() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    // Unknown field on a deny_unknown_fields input → protocol-level invalid params.
    let err = client
        .call_tool(
            CallToolRequestParams::new("find")
                .with_arguments(args(serde_json::json!({ "noSuchAxis": true }))),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("noSuchAxis"),
        "error should cite the bad field: {err}"
    );

    // Unknown tool name → invalid params, server keeps serving.
    let err = client
        .call_tool(CallToolRequestParams::new("no_such_tool"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"), "got: {err}");

    let still_alive = call(&client, "repo_validate", serde_json::json!({})).await;
    assert_eq!(still_alive.is_error, Some(false));

    client.cancel().await.unwrap();
}
