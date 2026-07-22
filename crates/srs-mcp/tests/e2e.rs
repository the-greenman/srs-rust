//! Phase 5: one full scripted client session — initialize, orient, write,
//! validate, discover — against a real repo over an in-process transport.

use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams};
use rmcp::ServiceExt;
use srs_mcp::SrsMcpServer;
use srs_repository::package_service::{create_field_normalized, create_type_normalized};
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::store::FileStore;

const NS: &str = "com.example.e2e";

#[tokio::test]
async fn e2e_full_session_read_write_validate() {
    // ── Fixture: repo + decision type ────────────────────────────────────────
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: uuid::Uuid::new_v4().to_string(),
                namespace: NS.into(),
                srs_version: "2.0-draft".into(),
                title: Some("E2E Repo".into()),
                description: Some("End-to-end session fixture".into()),
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
    create_field_normalized(
        &store,
        serde_json::json!({
            "id": title_field_id, "namespace": NS, "name": "title",
            "version": 1, "valueType": "string"
        }),
        None,
    )
    .unwrap();
    create_type_normalized(
        &store,
        serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "namespace": NS, "name": "decision",
            "version": 1,
            "fields": [{ "fieldId": title_field_id, "order": 1, "required": true }]
        }),
        None,
    )
    .unwrap();

    // ── Connect ──────────────────────────────────────────────────────────────
    let (client_stream, server_stream) = tokio::io::duplex(1 << 20);
    let server = SrsMcpServer::new(dir.path().to_path_buf()).unwrap();
    let repo_id = server.repository_id().to_string();
    tokio::spawn(async move {
        if let Ok(service) = server.serve(server_stream).await {
            let _ = service.waiting().await;
        }
    });
    let client = ().serve(client_stream).await.unwrap();

    // Handshake carries identity + both capabilities.
    let info = client.peer_info().expect("handshake info");
    assert_eq!(info.server_info.name, "srs-mcp");
    assert!(info.capabilities.resources.is_some());
    assert!(info.capabilities.tools.is_some());
    assert!(info.instructions.is_some());

    // ── Orient: list, read map ───────────────────────────────────────────────
    let resources = client.list_resources(None).await.unwrap().resources;
    assert!(resources.iter().any(|r| r.uri.ends_with("/map")));
    let map = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "srs://{repo_id}/map"
        )))
        .await
        .unwrap();
    assert_eq!(map.contents.len(), 1);

    // ── Write: record_create through the validated contract ──────────────────
    let created = client
        .call_tool(
            CallToolRequestParams::new("record_create").with_arguments(
                serde_json::json!({
                    "type": format!("{NS}/decision"),
                    "fieldValues": [
                        { "fieldId": title_field_id, "value": "Adopt MCP" }
                    ]
                })
                .as_object()
                .cloned()
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(created.is_error, Some(false));
    let record_id = created.structured_content.as_ref().unwrap()["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // ── Read back the record resource ────────────────────────────────────────
    let record = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "srs://{repo_id}/record/{record_id}"
        )))
        .await
        .unwrap();
    assert_eq!(record.contents.len(), 1);

    // ── Validate: zero diagnostics ───────────────────────────────────────────
    let validate = client
        .call_tool(CallToolRequestParams::new("repo_validate"))
        .await
        .unwrap();
    assert_eq!(validate.is_error, Some(false));
    assert_eq!(
        validate.structured_content.as_ref().unwrap()["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // ── Discover: find returns the created record ────────────────────────────
    let found = client
        .call_tool(
            CallToolRequestParams::new("find").with_arguments(
                serde_json::json!({ "contentMatch": "Adopt MCP" })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    let hits = found.structured_content.as_ref().unwrap()["hits"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        hits.iter()
            .any(|h| h["instanceId"].as_str() == Some(record_id.as_str())),
        "find should return the record created in this session: {hits:?}"
    );

    client.cancel().await.unwrap();
}
