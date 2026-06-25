//! Integration test for the WASM `document_views_for_container` binding.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.
//!
//! The service joins a container's root-instance Type to a DocumentView's `rootTypeRefs`
//! (RFC-009). The gallery's views carry no `rootTypeRefs`, so the populated case uses a small
//! inline `.srsj` fixture; the gallery exercises the empty (unbound) path.

use srs_repository::view_service;
use srs_repository::JsonStore;

const TYPE_ID: &str = "11111111-1111-4111-8111-111111111111";
const CONTAINER_ID: &str = "22222222-2222-4222-8222-222222222222";
const ROOT_ID: &str = "33333333-3333-4333-8333-333333333333";
const VIEW_ID: &str = "44444444-4444-4444-8444-444444444444";

/// Minimal `.srsj`: a container whose root record binds `TYPE_ID` v1, and a DocumentView whose
/// `rootTypeRefs` anchors that exact type binding.
fn bound_view_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-docviews",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": ROOT_ID, "path": format!("records/tier-2/{ROOT_ID}.json"), "tier": 2}
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-docviews-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": ["document-views/bound.json"],
                "blueprints": []
            },
            "package/document-views/bound.json": {
                "id": VIEW_ID,
                "namespace": "com.test",
                "name": "bound-view",
                "version": 1,
                "description": "A document view anchored to TYPE_ID v1",
                "rootTypeRefs": [{"typeId": TYPE_ID, "typeVersion": 1}],
                "sections": [
                    {
                        "sectionId": "body",
                        "title": "Body",
                        "order": 0,
                        "source": {"type": "container-subset", "containerId": CONTAINER_ID},
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
                "memberInstanceIds": [ROOT_ID],
                "title": "Bound document",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "records/tier-2/33333333-3333-4333-8333-333333333333.json": {
                "instanceId": ROOT_ID,
                "typeId": TYPE_ID,
                "typeVersion": 1
            }
        }
    })
    .to_string()
}

fn gallery_store() -> JsonStore {
    let srsj = include_str!("fixtures/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

/// A container whose root Type matches a view's `rootTypeRefs` returns that view — the join the
/// web client uses to pick which document view renders a selected container.
#[test]
fn document_views_for_container_returns_matching() {
    let store = JsonStore::from_srsj(&bound_view_srsj()).expect("bound-view srsj must load");
    let views = view_service::document_views_for_container(&store, CONTAINER_ID)
        .expect("lookup must succeed");
    assert_eq!(
        views.len(),
        1,
        "exactly one view binds this container's root type"
    );
    assert_eq!(views[0].id, VIEW_ID);
}

/// Gallery containers have typed roots but no view carries `rootTypeRefs`, so the join yields
/// an empty list (not an error).
#[test]
fn document_views_for_container_empty_when_unbound() {
    let store = gallery_store();
    let views =
        view_service::document_views_for_container(&store, "138e2fac-6a8a-4a06-9511-5aefd99ceae9")
            .expect("lookup must succeed");
    assert!(
        views.is_empty(),
        "gallery views declare no rootTypeRefs, so nothing matches"
    );
}
