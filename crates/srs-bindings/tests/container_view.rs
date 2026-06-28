//! Integration test for the WASM `resolve_container_view` binding (issue #254).
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.

use srs_repository::container_view_service::{resolve_container_view, ResolveContainerViewInput};
use srs_repository::JsonStore;

const TYPE_ID: &str = "11111111-1111-4111-8111-111111111111";
const CONTAINER_ID: &str = "22222222-2222-4222-8222-222222222222";
const ROOT_ID: &str = "33333333-3333-4333-8333-333333333333";
const MEMBER_ID: &str = "55555555-5555-4555-8555-555555555555";
const VIEW_ID: &str = "44444444-4444-4444-8444-444444444444";
const DV_ID: &str = "66666666-6666-4666-8666-666666666666";
const FIELD_TITLE: &str = "77777777-7777-4777-8777-777777777777";
const FIELD_STATUS: &str = "88888888-8888-4888-8888-888888888888";

/// Minimal `.srsj`: a container (root + one extra member) bound to a DocumentView whose
/// container-subset section renders via a View exposing the title and status fields.
fn fixture_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-container-view",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": ROOT_ID, "path": format!("records/tier-2/{ROOT_ID}.json"), "tier": 2},
                {"instanceId": MEMBER_ID, "path": format!("records/tier-2/{MEMBER_ID}.json"), "tier": 2}
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-container-view-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json", "fields/status.json"],
                "types": [],
                "relationTypes": [],
                "views": ["views/decision-view.json"],
                "documentViews": ["document-views/dv.json"],
                "blueprints": []
            },
            "package/fields/title.json": {
                "id": FIELD_TITLE,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {},
                "valueType": "string",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/fields/status.json": {
                "id": FIELD_STATUS,
                "namespace": "com.test",
                "name": "status",
                "version": 1,
                "description": "Status",
                "aiGuidance": {},
                "valueType": "string",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/views/decision-view.json": {
                "id": VIEW_ID,
                "namespace": "com.test",
                "name": "decision-view",
                "version": 1,
                "description": "List view",
                "fieldViews": [
                    {"fieldId": FIELD_TITLE, "order": 0},
                    {"fieldId": FIELD_STATUS, "order": 1, "displayLabel": "Status"}
                ],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/document-views/dv.json": {
                "id": DV_ID,
                "namespace": "com.test",
                "name": "dv",
                "version": 1,
                "description": "Document view",
                "rootTypeRefs": [{"typeId": TYPE_ID, "typeVersion": 1}],
                "sections": [
                    {
                        "sectionId": "body",
                        "title": "Body",
                        "order": 0,
                        "source": {"type": "container-subset", "containerId": CONTAINER_ID},
                        "renderViewId": VIEW_ID,
                        "emptyBehavior": "hide"
                    }
                ],
                "format": "markdown",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "containers/22222222-2222-4222-8222-222222222222.json": {
                "containerId": CONTAINER_ID,
                "containerType": "document",
                "rootInstanceIds": [ROOT_ID],
                "memberInstanceIds": [ROOT_ID, MEMBER_ID],
                "title": "Bound document",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "records/tier-2/33333333-3333-4333-8333-333333333333.json": {
                "instanceId": ROOT_ID,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": [{"fieldId": FIELD_TITLE, "value": "Root Decision"}]
            },
            "records/tier-2/55555555-5555-4555-8555-555555555555.json": {
                "instanceId": MEMBER_ID,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": [{"fieldId": FIELD_TITLE, "value": "Member Decision"}]
            }
        }
    })
    .to_string()
}

/// The default path the web editor uses: resolve a container into root + ordered members +
/// the DocumentView-driven column spec.
#[test]
fn resolve_container_view_returns_root_members_and_columns() {
    let store = JsonStore::from_srsj(&fixture_srsj()).expect("fixture srsj must load");
    let result = resolve_container_view(
        &store,
        ResolveContainerViewInput {
            container_id: CONTAINER_ID.to_string(),
            view_id: None,
        },
    )
    .expect("resolve must succeed");

    assert_eq!(result.container_id, CONTAINER_ID);
    assert_eq!(result.document_view_id.as_deref(), Some(DV_ID));

    // Columns resolved from the View, ordered, with the displayLabel override.
    assert_eq!(result.columns.len(), 2);
    assert_eq!(result.columns[0].field_id, FIELD_TITLE);
    assert_eq!(result.columns[0].field_name, "title");
    assert_eq!(result.columns[0].display_label, "title");
    assert_eq!(result.columns[1].display_label, "Status");

    // Root + ordered members with core-resolved labels.
    assert_eq!(result.root.as_ref().unwrap().display_label, "Root Decision");
    assert_eq!(result.members.len(), 2);
    assert_eq!(result.members[0].display_label, "Root Decision");
    assert_eq!(result.members[1].display_label, "Member Decision");

    // The result serialises to JSON (the shape the binding hands to JS via to_js).
    let json = serde_json::to_value(&result).expect("serialise");
    assert_eq!(json["containerView"], serde_json::Value::Null); // sanity: not double-wrapped
    assert!(json["columns"].is_array());
}

/// An explicit unknown `view_id` yields empty columns + a diagnostic, but still returns members.
#[test]
fn resolve_container_view_unknown_view_id_is_diagnostic_not_error() {
    let store = JsonStore::from_srsj(&fixture_srsj()).expect("fixture srsj must load");
    let result = resolve_container_view(
        &store,
        ResolveContainerViewInput {
            container_id: CONTAINER_ID.to_string(),
            view_id: Some("no-such-dv".to_string()),
        },
    )
    .expect("resolve must succeed");

    assert!(result.document_view_id.is_none());
    assert!(result.columns.is_empty());
    assert_eq!(result.members.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.contains("documentView no-such-dv not found")));
}
