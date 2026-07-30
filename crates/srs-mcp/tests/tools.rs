//! Phase 3 integration tests: tools, exercised through a real rmcp client
//! over an in-process duplex transport. Fixture repos are built inline per
//! test with `srs-repository` writer services.

use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::ServiceExt;
use srs_core::types::container::Container;
use srs_mcp::SrsMcpServer;
use srs_repository::container_service;
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
use std::collections::HashMap;

const NS: &str = "com.example.mcptest";

struct Fixture {
    dir: tempfile::TempDir,
    title_field_id: String,
    body_field_id: String,
    type_id: String,
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
                "valueType": value_type,
                "aiGuidance": {"purpose": format!("captures the {name}")}
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
        type_id,
    }
}

/// Extended fixture: base fixture + a lifecycle-enabled type + supersedes/refines
/// relation types + a container. Used by second-wave integration tests.
struct LifecycleFixture {
    base: Fixture,
    /// A pre-created container for membership tests.
    container_id: String,
}

fn make_lifecycle_fixture() -> LifecycleFixture {
    let base = make_fixture();
    let store = FileStore::new(base.dir.path());

    // Type with inline lifecycle (draft → active → archived)
    let lifecycle_type_id = uuid::Uuid::new_v4().to_string();
    create_type_normalized(
        &store,
        serde_json::json!({
            "id": lifecycle_type_id,
            "namespace": NS,
            "name": "proposal",
            "version": 1,
            "fields": [
                { "fieldId": base.title_field_id, "order": 1, "required": true }
            ],
            "lifecycle": {
                "states": [
                    {"key": "draft", "isInitial": true},
                    {"key": "active"},
                    {"key": "archived", "isFinal": true}
                ],
                "transitions": [
                    {"name": "promote", "from": "draft", "to": "active"},
                    {"name": "archive", "from": "active", "to": "archived"}
                ],
                "initialState": "draft"
            }
        }),
        None,
    )
    .unwrap();

    // Install supersedes and refines relation types (required by record_successor)
    for (key, label, category) in [
        ("supersedes", "Supersedes", "refinement"),
        ("refines", "Refines", "refinement"),
    ] {
        create_relation_type_normalized(
            &store,
            serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "version": 1,
                "key": key,
                "namespace": NS,
                "label": label,
                "description": "",
                "category": category
            }),
            None,
        )
        .unwrap();
    }

    // Create a container for membership tests
    let container_id = uuid::Uuid::new_v4().to_string();
    container_service::create_container(
        &store,
        Container {
            container_id: container_id.clone(),
            title: "MCP Test Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        },
    )
    .unwrap();

    LifecycleFixture { base, container_id }
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

    // Unknown field on record_update (second-wave deny_unknown_fields) → invalid params.
    let err = client
        .call_tool(
            CallToolRequestParams::new("record_update").with_arguments(args(serde_json::json!({
                "instanceId": "some-id",
                "fieldValues": [],
                "unknownExtra": true
            }))),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("unknownExtra"),
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

#[tokio::test]
async fn tool_type_schema_happy_and_unknown_id() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let result = call(
        &client,
        "type_schema",
        serde_json::json!({ "typeId": fx.type_id }),
    )
    .await;
    assert_eq!(
        result.is_error,
        Some(false),
        "type_schema failed: {result:?}"
    );
    let structured = result.structured_content.as_ref().unwrap();

    let direct = srs_repository::type_schema_service::type_schema(
        &FileStore::new(fx.dir.path()),
        srs_repository::type_schema_service::TypeSchemaInput {
            type_id: fx.type_id.clone(),
            type_version: None,
        },
    )
    .unwrap();
    assert_eq!(structured, &serde_json::to_value(&direct).unwrap());
    let text = serde_json::to_string(structured).unwrap();
    assert!(text.contains("x-srs-field-id"));
    assert!(
        text.contains(&fx.title_field_id),
        "fieldId discoverable in schema"
    );

    // Unknown id → tool-level error, server keeps serving.
    let missing = uuid::Uuid::new_v4().to_string();
    let bad = call(
        &client,
        "type_schema",
        serde_json::json!({ "typeId": missing }),
    )
    .await;
    assert_eq!(bad.is_error, Some(true));
    assert!(
        text_of(&bad).to_lowercase().contains("not found") || text_of(&bad).contains(&missing),
        "expected TypeNotFound text: {}",
        text_of(&bad)
    );
    let alive = call(&client, "repo_validate", serde_json::json!({})).await;
    assert_eq!(alive.is_error, Some(false));

    client.cancel().await.unwrap();
}

// ── Second-wave integration tests (#680) ─────────────────────────────────────

#[tokio::test]
async fn tool_record_update_replaces_field_values() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    // Create a record.
    let create = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.title_field_id, "value": "Original Title" }
            ]
        }),
    )
    .await;
    assert_eq!(create.is_error, Some(false), "create failed: {create:?}");
    let instance_id = create.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Update with new field values (full replace).
    let update = call(
        &client,
        "record_update",
        serde_json::json!({
            "instanceId": instance_id,
            "fieldValues": [
                { "fieldId": fx.title_field_id, "value": "Updated Title" }
            ]
        }),
    )
    .await;
    assert_eq!(update.is_error, Some(false), "update failed: {update:?}");
    let updated = update.structured_content.as_ref().unwrap();
    let updated_title = updated["fieldValues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fv| fv["fieldId"].as_str() == Some(fx.title_field_id.as_str()))
        .and_then(|fv| fv["value"].as_str());
    assert_eq!(
        updated_title,
        Some("Updated Title"),
        "field value not updated: {updated}"
    );

    // Write confirmed by the service read path.
    let store = FileStore::new(fx.dir.path());
    let loaded = srs_repository::record_store::get_record_by_id(&store, &instance_id)
        .unwrap()
        .unwrap();
    assert!(
        loaded
            .field_values
            .iter()
            .any(|fv| fv.field_id == fx.title_field_id
                && fv.value == serde_json::json!("Updated Title")),
        "direct service read should see updated title"
    );

    // Repository still consistent.
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
async fn tool_record_allowed_transitions_returns_initial_state() {
    let fx = make_lifecycle_fixture();
    let client = connect(&fx.base).await;

    // Create a record of the lifecycle type (starts in draft).
    let create = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/proposal"),
            "fieldValues": [
                { "fieldId": fx.base.title_field_id, "value": "My Proposal" }
            ]
        }),
    )
    .await;
    assert_eq!(create.is_error, Some(false), "create failed: {create:?}");
    let instance_id = create.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Query allowed transitions from initial state.
    let result = call(
        &client,
        "record_allowed_transitions",
        serde_json::json!({ "instanceId": instance_id }),
    )
    .await;
    assert_eq!(
        result.is_error,
        Some(false),
        "record_allowed_transitions failed: {result:?}"
    );
    let body = result.structured_content.as_ref().unwrap();
    assert_eq!(
        body["currentState"].as_str(),
        Some("draft"),
        "initial state must be draft: {body}"
    );
    let transitions = body["transitions"].as_array().unwrap();
    assert!(
        transitions
            .iter()
            .any(|t| t["name"].as_str() == Some("promote") && t["to"].as_str() == Some("active")),
        "promote→active must be listed: {transitions:?}"
    );
    assert_eq!(body["isImmutable"].as_bool(), Some(false));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_record_transition_promotes_draft_to_active() {
    let fx = make_lifecycle_fixture();
    let client = connect(&fx.base).await;

    // Create a record of the lifecycle type.
    let create = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/proposal"),
            "fieldValues": [
                { "fieldId": fx.base.title_field_id, "value": "My Proposal" }
            ]
        }),
    )
    .await;
    assert_eq!(create.is_error, Some(false), "create failed: {create:?}");
    let instance_id = create.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Transition via named transition "promote".
    let transition = call(
        &client,
        "record_transition",
        serde_json::json!({
            "instanceId": instance_id,
            "byTransition": "promote"
        }),
    )
    .await;
    assert_eq!(
        transition.is_error,
        Some(false),
        "record_transition failed: {transition:?}"
    );
    let result = transition.structured_content.as_ref().unwrap();
    assert_eq!(
        result["record"]["lifecycleState"].as_str(),
        Some("active"),
        "lifecycle state must be active after promote: {result}"
    );

    // Repository still consistent.
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
async fn tool_record_successor_creates_linked_pair() {
    let fx = make_lifecycle_fixture();
    let client = connect(&fx.base).await;

    // Create a predecessor.
    let create = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.base.title_field_id, "value": "Original Decision" }
            ]
        }),
    )
    .await;
    assert_eq!(create.is_error, Some(false), "create failed: {create:?}");
    let predecessor_id = create.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Create a successor linked by supersedes.
    let successor = call(
        &client,
        "record_successor",
        serde_json::json!({
            "predecessorId": predecessor_id,
            "relationType": "supersedes",
            "fieldValues": [
                { "fieldId": fx.base.title_field_id, "value": "Revised Decision" }
            ]
        }),
    )
    .await;
    assert_eq!(
        successor.is_error,
        Some(false),
        "record_successor failed: {successor:?}"
    );
    let result = successor.structured_content.as_ref().unwrap();
    let successor_id = result["record"]["instanceId"].as_str().unwrap();
    assert!(
        !successor_id.is_empty(),
        "successor instanceId must be non-empty"
    );
    assert_eq!(
        result["relation"]["relationType"].as_str(),
        Some("supersedes"),
        "linking relation must be supersedes: {result}"
    );
    assert_eq!(
        result["relation"]["sourceInstanceId"].as_str(),
        Some(successor_id),
        "relation source is the new successor: {result}"
    );
    assert_eq!(
        result["relation"]["targetInstanceId"].as_str(),
        Some(predecessor_id.as_str()),
        "relation target is the predecessor: {result}"
    );

    // Repository still consistent.
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
async fn tool_note_graduate_promotes_to_record() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    // Capture a Tier-0 note.
    let note = call(
        &client,
        "note_create",
        serde_json::json!({
            "title": "Meeting Notes",
            "sections": [
                { "name": "body", "content": "Discussed the quarterly review." }
            ]
        }),
    )
    .await;
    assert_eq!(note.is_error, Some(false), "note_create failed: {note:?}");
    let note_id = note.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Graduate the note to a typed Record.
    let graduate = call(
        &client,
        "note_graduate",
        serde_json::json!({
            "noteId": note_id,
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.title_field_id, "value": "Decision from meeting" }
            ]
        }),
    )
    .await;
    assert_eq!(
        graduate.is_error,
        Some(false),
        "note_graduate failed: {graduate:?}"
    );
    let result = graduate.structured_content.as_ref().unwrap();
    assert!(
        result["note"]["graduatedAt"].as_str().is_some(),
        "note must carry graduatedAt timestamp: {result}"
    );
    let record_id = result["record"]["instanceId"].as_str().unwrap();
    assert!(!record_id.is_empty(), "record instanceId must be non-empty");

    // Repository still consistent.
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
async fn tool_container_member_add_then_remove() {
    let fx = make_lifecycle_fixture();
    let client = connect(&fx.base).await;

    // Create a record to manage as a container member.
    let create = call(
        &client,
        "record_create",
        serde_json::json!({
            "type": format!("{NS}/decision"),
            "fieldValues": [
                { "fieldId": fx.base.title_field_id, "value": "Container Member" }
            ]
        }),
    )
    .await;
    assert_eq!(create.is_error, Some(false));
    let instance_id = create.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Add to container.
    let add = call(
        &client,
        "container_member_add",
        serde_json::json!({
            "containerId": fx.container_id,
            "instanceId": instance_id
        }),
    )
    .await;
    assert_eq!(
        add.is_error,
        Some(false),
        "container_member_add failed: {add:?}"
    );
    let members = add.structured_content.as_ref().unwrap()["memberInstanceIds"]
        .as_array()
        .unwrap();
    assert!(
        members
            .iter()
            .any(|m| m.as_str() == Some(instance_id.as_str())),
        "added instance must appear in memberInstanceIds: {members:?}"
    );

    // Idempotent add — no error.
    let add2 = call(
        &client,
        "container_member_add",
        serde_json::json!({
            "containerId": fx.container_id,
            "instanceId": instance_id
        }),
    )
    .await;
    assert_eq!(add2.is_error, Some(false), "idempotent add must succeed");

    // Remove from container.
    let remove = call(
        &client,
        "container_member_remove",
        serde_json::json!({
            "containerId": fx.container_id,
            "instanceId": instance_id
        }),
    )
    .await;
    assert_eq!(
        remove.is_error,
        Some(false),
        "container_member_remove failed: {remove:?}"
    );
    let members_after = remove.structured_content.as_ref().unwrap()["memberInstanceIds"]
        .as_array()
        .unwrap();
    assert!(
        !members_after
            .iter()
            .any(|m| m.as_str() == Some(instance_id.as_str())),
        "removed instance must not appear in memberInstanceIds: {members_after:?}"
    );

    // Repository still consistent.
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
