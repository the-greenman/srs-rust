//! RFC-049: one server over the repositories of a registry (srs-rust#683). Reads and
//! routed writes by `repositoryId`, the default repository, and every [R5]/[R17]
//! refusal in order, over an in-process transport.

use std::path::Path;

use rmcp::model::{CallToolRequestParams, CallToolResult, ReadResourceRequestParams};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use srs_mcp::SrsMcpServer;
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::store::FileStore;

const UNKNOWN: &str = "00000000-0000-4000-8000-000000000000";

/// A fresh repository under `root/name`; returns its `repositoryId`.
fn repo(root: &Path, name: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    create_repository_with_intent(
        &FileStore::new(root.join(name)),
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: id.clone(),
                namespace: format!("com.example.{name}"),
                srs_version: "2.0-draft".into(),
                title: Some(name.into()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                namespace: format!("com.example.{name}"),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
    id
}

fn set_manifest(root: &Path, name: &str, key: &str, value: Value) {
    let path = root.join(name).join("manifest.json");
    let mut m: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    m[key] = value;
    std::fs::write(&path, serde_json::to_string_pretty(&m).unwrap()).unwrap();
}

fn note_count(root: &Path, name: &str) -> usize {
    std::fs::read_dir(root.join(name).join("records"))
        .map(|d| {
            d.flatten()
                .flat_map(|e| std::fs::read_dir(e.path()).into_iter().flatten().flatten())
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

async fn note_create(
    client: &RunningService<RoleClient, ()>,
    repository_id: Option<&str>,
) -> CallToolResult {
    let mut args = json!({ "sections": [{ "name": "body", "content": "x" }] });
    if let Some(id) = repository_id {
        args["repositoryId"] = json!(id);
    }
    client
        .call_tool(
            CallToolRequestParams::new("note_create")
                .with_arguments(args.as_object().cloned().unwrap()),
        )
        .await
        .unwrap()
}

async fn connect(server: SrsMcpServer) -> RunningService<RoleClient, ()> {
    let (client_stream, server_stream) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(service) = server.serve(server_stream).await {
            let _ = service.waiting().await;
        }
    });
    ().serve(client_stream).await.unwrap()
}

async fn map_text(client: &RunningService<RoleClient, ()>, id: &str) -> String {
    let map = client
        .read_resource(ReadResourceRequestParams::new(format!("srs://{id}/map")))
        .await
        .unwrap();
    serde_json::to_string(&map.contents).unwrap()
}

fn code(result: &CallToolResult) -> &str {
    assert_eq!(
        result.is_error,
        Some(true),
        "expected a refusal: {result:?}"
    );
    result.structured_content.as_ref().unwrap()["code"]
        .as_str()
        .unwrap()
}

#[tokio::test]
async fn one_server_serves_a_registry_of_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = repo(root, "main");
    let reference = repo(root, "reference");
    let other = repo(root, "other");
    let stale = repo(root, "stale");
    // `other`'s entry names a different id than its manifest reports; `stale` is too new.
    let wrong = uuid::Uuid::new_v4().to_string();
    set_manifest(root, "stale", "dataModelRevision", json!(99));
    let remote_ro = uuid::Uuid::new_v4().to_string();
    let remote = uuid::Uuid::new_v4().to_string();
    let registry = json!({
        "registryRevision": 1,
        "entries": [
            { "kind": "repository", "id": main, "label": "main", "locator": { "mode": "local", "path": "main" } },
            { "kind": "repository", "id": reference, "locator": { "mode": "local", "path": "reference" }, "readOnly": true },
            { "kind": "repository", "id": wrong, "locator": { "mode": "local", "path": "other" } },
            { "kind": "repository", "id": stale, "locator": { "mode": "local", "path": "stale" } },
            { "kind": "repository", "id": remote, "locator": { "mode": "remote", "url": "https://example.org/srs" } },
            { "kind": "repository", "id": remote_ro, "locator": { "mode": "remote", "url": "https://example.org/srs" }, "readOnly": true },
        ]
    });
    let registry_path = root.join("registry.json");
    std::fs::write(&registry_path, registry.to_string()).unwrap();
    let _ = other;

    let server = SrsMcpServer::from_registry(&registry_path, None).unwrap();
    assert_eq!(
        server.repository_id(),
        main,
        "the first repository entry is the default"
    );
    let (client_stream, server_stream) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(service) = server.serve(server_stream).await {
            let _ = service.waiting().await;
        }
    });
    let client = ().serve(client_stream).await.unwrap();

    // [R13]: the instructions name the default repository; every tool takes `repositoryId`.
    let instructions = client.peer_info().unwrap().instructions.clone().unwrap();
    assert!(
        instructions.contains(&format!("default repository is {main}")),
        "{instructions}"
    );
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().all(|t| t
        .input_schema
        .get("properties")
        .and_then(|p| p.get("repositoryId"))
        .is_some()));

    // Reads: another repository by its uri, read-only entries included ([R10]).
    let map = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "srs://{reference}/map"
        )))
        .await
        .unwrap();
    assert!(!map.contents.is_empty());
    let read = client
        .call_tool(
            CallToolRequestParams::new("read").with_arguments(
                json!({ "uri": format!("srs://{reference}/map") })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(read.is_error, Some(false), "{read:?}");
    let err = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "srs://{UNKNOWN}/map"
        )))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("not-in-boundary") || err.contains("boundary"),
        "{err}"
    );

    // Routed writes ([R16], [R17]): the default, and the default named by its own id.
    let before = note_count(root, "main");
    assert_eq!(note_create(&client, None).await.is_error, Some(false));
    assert_eq!(
        note_create(&client, Some(&main.to_uppercase()))
            .await
            .is_error,
        Some(false)
    );
    assert_eq!(note_count(root, "main"), before + 2);

    // Refusals, each writing nothing.
    let ref_before = note_count(root, "reference");
    let (other_before, stale_before) = (note_count(root, "other"), note_count(root, "stale"));
    assert_eq!(
        code(&note_create(&client, Some(UNKNOWN)).await),
        "not-in-boundary"
    );
    assert_eq!(
        code(&note_create(&client, Some(&reference)).await),
        "read-only"
    );
    assert_eq!(
        code(&note_create(&client, Some(&remote)).await),
        "locator-unusable"
    );
    assert_eq!(
        code(&note_create(&client, Some(&wrong)).await),
        "id-mismatch"
    );
    assert_eq!(
        code(&note_create(&client, Some(&stale)).await),
        "revision-unsupported"
    );
    // [R17] order: `read-only` is decided from the entry, before the locator is tried.
    assert_eq!(
        code(&note_create(&client, Some(&remote_ro)).await),
        "read-only"
    );
    assert_eq!(note_count(root, "reference"), ref_before);
    assert_eq!(note_count(root, "other"), other_before);
    assert_eq!(note_count(root, "stale"), stale_before);

    // An unknown tool is unknown, not a boundary refusal; a malformed or conflicting
    // `repositoryId` is invalid params, never routed to the default.
    let unknown = client
        .call_tool(
            CallToolRequestParams::new("no_such_tool").with_arguments(
                json!({ "repositoryId": reference })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("unknown tool"), "{unknown}");
    let bad = client
        .call_tool(
            CallToolRequestParams::new("note_create").with_arguments(
                json!({ "repositoryId": 7, "sections": [{ "name": "b", "content": "x" }] })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(bad.contains("repositoryId must be a string"), "{bad}");
    let conflict = client
        .call_tool(
            CallToolRequestParams::new("read").with_arguments(
                json!({ "uri": format!("srs://{reference}/map"), "repositoryId": main })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(conflict.contains("conflicts"), "{conflict}");

    // [R10]: a read through the registry equals the same read against the repository directly.
    let routed = map_text(&client, &reference).await;
    let direct = connect(SrsMcpServer::new(root.join("reference")).unwrap()).await;
    assert_eq!(routed, map_text(&direct, &reference).await);
    direct.cancel().await.unwrap();

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_default_that_did_not_load_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = repo(root, "main");
    let registry_path = root.join("registry.json");
    std::fs::write(
        &registry_path,
        json!({ "entries": [{ "kind": "repository", "id": main, "locator": { "mode": "local", "path": "main" } }] })
            .to_string(),
    )
    .unwrap();
    // [R13]: a default named but absent from the registry refuses default requests.
    let client = connect(SrsMcpServer::from_registry(&registry_path, Some(UNKNOWN)).unwrap()).await;
    // Listings open no repository, so they work with an unusable default; the
    // instructions report it ([R13]).
    assert!(!client.list_tools(None).await.unwrap().tools.is_empty());
    let instructions = client.peer_info().unwrap().instructions.clone().unwrap();
    assert!(instructions.contains("did not load"), "{instructions}");
    assert_eq!(code(&note_create(&client, None).await), "not-in-boundary");
    assert_eq!(
        note_create(&client, Some(&main)).await.is_error,
        Some(false)
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_single_repository_server_does_not_route_writes() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "main");
    let client = connect(SrsMcpServer::new(dir.path().join("main")).unwrap()).await;
    // Core [R16]: a write naming another repository is refused, never applied here.
    assert_eq!(
        code(&note_create(&client, Some(UNKNOWN)).await),
        "write-not-routed"
    );
    // Its own id is the same as no id ([R13]).
    assert_eq!(
        note_create(&client, Some(&main)).await.is_error,
        Some(false)
    );
    // No `repositoryId` is advertised without a registry.
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().all(|t| t
        .input_schema
        .get("properties")
        .and_then(|p| p.get("repositoryId"))
        .is_none()));
    client.cancel().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_write_the_target_refuses_is_write_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = repo(root, "main");
    let registry_path = root.join("registry.json");
    std::fs::write(
        &registry_path,
        json!({ "entries": [{ "kind": "repository", "id": main, "locator": { "mode": "local", "path": "main" } }] })
            .to_string(),
    )
    .unwrap();
    let client = connect(SrsMcpServer::from_registry(&registry_path, None).unwrap()).await;
    let records = root.join("main").join("records");
    let lock = |mode| {
        for d in std::fs::read_dir(&records).unwrap().flatten() {
            std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        }
        std::fs::set_permissions(&records, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    lock(0o555);
    let result = note_create(&client, Some(&main)).await;
    lock(0o755);
    assert_eq!(code(&result), "write-refused");
    client.cancel().await.unwrap();
}
