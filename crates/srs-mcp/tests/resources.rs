//! Phase 2 integration tests: the read surface, exercised through a real
//! rmcp client over an in-process duplex transport.
//!
//! Fixture repos are created inline per test with `srs-repository` writer
//! services in a tempdir — one fixture per test for isolation.

use rmcp::model::ReadResourceRequestParams;
use rmcp::ServiceExt;
use srs_mcp::SrsMcpServer;
use srs_repository::analysis::build_repo_map;
use srs_repository::container_view_service::{resolve_container_view, ResolveContainerViewInput};
use srs_repository::record_store::get_record_by_id;
use srs_repository::render_service::{render_document_view, RenderDocumentViewOptions};
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::store::FileStore;
use srs_repository::view_service::create_document_view_normalized;

struct Fixture {
    #[allow(dead_code)]
    dir: tempfile::TempDir,
    repo_id: String,
    identity_id: String,
    view_id: String,
    container_id: String,
}

fn make_fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    let result = create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: uuid::Uuid::new_v4().to_string(),
                namespace: "com.example.mcptest".into(),
                srs_version: "2.0-draft".into(),
                title: Some("MCP Test Repo".into()),
                description: Some("Fixture repository for srs-mcp tests".into()),
            },
            primary_package: PrimaryPackageMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                namespace: "com.example.mcptest".into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();

    let view_id = uuid::Uuid::new_v4().to_string();
    create_document_view_normalized(
        &store,
        serde_json::json!({
            "id": view_id,
            "namespace": "com.example.mcptest",
            "name": "test-view",
            "version": 1,
            "sections": [{
                "sectionId": "s1",
                "title": "Identity",
                "order": 1,
                "source": {
                    "type": "fixed-instances",
                    "instanceIds": [result.identity_instance_id.clone().unwrap()]
                }
            }]
        }),
        None,
    )
    .unwrap();

    let manifest = srs_repository::store::RepositoryStore::load_manifest(&store).unwrap();
    let container_id = manifest.container.as_ref().unwrap().container_id.clone();

    Fixture {
        dir,
        repo_id: result.repository_id,
        identity_id: result.identity_instance_id.expect("identity scaffolded"),
        view_id,
        container_id,
    }
}

/// Start the server over one end of a duplex pipe; return a connected client.
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

fn store_for(fixture: &Fixture) -> FileStore {
    FileStore::new(fixture.dir.path())
}

#[tokio::test]
async fn list_resources_enumerates_containers_and_views() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let listed = client.list_resources(None).await.unwrap().resources;
    let uris: Vec<&str> = listed.iter().map(|r| r.uri.as_str()).collect();

    assert!(uris.contains(&format!("srs://{}/map", fx.repo_id).as_str()));
    assert!(uris.contains(&format!("srs://{}/navigation", fx.repo_id).as_str()));
    assert!(uris.contains(&format!("srs://{}/container/{}", fx.repo_id, fx.container_id).as_str()));
    assert!(uris.contains(&format!("srs://{}/view/{}", fx.repo_id, fx.view_id).as_str()));

    // Container resource is named by its title; view by namespace-qualified name.
    let container = listed
        .iter()
        .find(|r| r.uri.ends_with(&fx.container_id))
        .unwrap();
    assert_eq!(container.name, "MCP Test Repo");
    let view = listed
        .iter()
        .find(|r| r.uri.ends_with(&fx.view_id))
        .unwrap();
    assert_eq!(view.name, "com.example.mcptest/test-view");

    // Records are exposed via template, not enumerated.
    let templates = client
        .list_resource_templates(None)
        .await
        .unwrap()
        .resource_templates;
    assert_eq!(templates.len(), 1);
    assert_eq!(
        templates[0].uri_template,
        format!("srs://{}/record/{{instanceId}}", fx.repo_id)
    );

    client.cancel().await.unwrap();
}

async fn read_text(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    uri: String,
) -> (Option<String>, String) {
    let result = client
        .read_resource(ReadResourceRequestParams::new(uri.clone()))
        .await
        .unwrap();
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents {
            uri: u,
            mime_type,
            text,
            ..
        } => {
            assert_eq!(u, &uri);
            (mime_type.clone(), text.clone())
        }
        other => panic!("expected text contents, got {other:?}"),
    }
}

#[tokio::test]
async fn read_record_matches_service_output() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let (mime, text) = read_text(
        &client,
        format!("srs://{}/record/{}", fx.repo_id, fx.identity_id),
    )
    .await;
    assert_eq!(mime.as_deref(), Some("application/json"));

    let record = get_record_by_id(&store_for(&fx), &fx.identity_id)
        .unwrap()
        .expect("identity record exists");
    assert_eq!(text, serde_json::to_string_pretty(&record).unwrap());

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn read_map_navigation_container_match_service_output() {
    let fx = make_fixture();
    let client = connect(&fx).await;
    let store = store_for(&fx);

    let (mime, text) = read_text(&client, format!("srs://{}/map", fx.repo_id)).await;
    assert_eq!(mime.as_deref(), Some("application/json"));
    assert_eq!(
        text,
        serde_json::to_string_pretty(&build_repo_map(&store).unwrap()).unwrap()
    );

    let (_, nav_text) = read_text(&client, format!("srs://{}/navigation", fx.repo_id)).await;
    assert_eq!(
        nav_text,
        serde_json::to_string_pretty(&repository_navigation(&store).unwrap()).unwrap()
    );

    let (_, container_text) = read_text(
        &client,
        format!("srs://{}/container/{}", fx.repo_id, fx.container_id),
    )
    .await;
    let expected = resolve_container_view(
        &store,
        ResolveContainerViewInput {
            container_id: fx.container_id.clone(),
            view_id: None,
        },
    )
    .unwrap();
    assert_eq!(
        container_text,
        serde_json::to_string_pretty(&expected).unwrap()
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn read_view_renders_markdown() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let (mime, text) =
        read_text(&client, format!("srs://{}/view/{}", fx.repo_id, fx.view_id)).await;
    assert_eq!(mime.as_deref(), Some("text/markdown"));

    let expected = render_document_view(RenderDocumentViewOptions {
        store: &store_for(&fx),
        view_id: &fx.view_id,
        format: Some("markdown"),
        theme_variant: None,
        container_id: None,
        instance_id_filter: None,
    })
    .unwrap();
    assert_eq!(text, expected.rendered);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn read_unknown_record_is_mcp_error() {
    let fx = make_fixture();
    let client = connect(&fx).await;

    let missing = uuid::Uuid::new_v4().to_string();
    let err = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "srs://{}/record/{}",
            fx.repo_id, missing
        )))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("resource not found"), "got: {msg}");

    // Malformed URI → error, not a crash; server still answers afterwards.
    let err = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "srs://{}/bogus/kind/extra",
            fx.repo_id
        )))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("invalid srs:// uri"), "got: {err}");

    let after = client.list_resources(None).await.unwrap();
    assert!(!after.resources.is_empty());

    client.cancel().await.unwrap();
}
