//! Integration test for the WASM `find` binding (issue #218).
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Exercises `discovery_service::find` directly
//! via `JsonStore::from_srsj`, not the `SrsRepository::find()` binding method,
//! because `to_js()` calls `js_sys::JSON::parse` which panics off-wasm.
//! The wasm-pack build proves the binding itself compiles and is exported.

use srs_repository::discovery_service::{find, DiscoveryQuery};
use srs_repository::JsonStore;

const FIELD_TITLE: &str = "11111111-1111-4111-8111-111111111111";
const FIELD_DESC: &str = "22222222-2222-4222-8222-222222222222";
const TYPE_ID: &str = "33333333-3333-4333-8333-333333333333";
const REC_AUTHORITY: &str = "44444444-4444-4444-8444-444444444444";
const REC_SECURITY: &str = "55555555-5555-4555-8555-555555555555";

fn fixture_store() -> JsonStore {
    let srsj = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-find",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": REC_AUTHORITY, "path": format!("records/tier-2/{REC_AUTHORITY}.json"), "tier": 2},
                {"instanceId": REC_SECURITY,  "path": format!("records/tier-2/{REC_SECURITY}.json"),  "tier": 2}
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-find-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json", "fields/description.json"],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            "package/fields/title.json": {
                "id": FIELD_TITLE,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {},
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/fields/description.json": {
                "id": FIELD_DESC,
                "namespace": "com.test",
                "name": "description",
                "version": 1,
                "description": "Description",
                "aiGuidance": {},
                "fieldType": {"datatype": "string", "format": "plain"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{REC_AUTHORITY}.json"): {
                "instanceId": REC_AUTHORITY,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": {
                    "title": "Building Authority",
                    "description": "Foundation of authority"
                }
            },
            format!("records/tier-2/{REC_SECURITY}.json"): {
                "instanceId": REC_SECURITY,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": {
                    "title": "Security Posture",
                    "description": "Annual compliance review"
                }
            }
        }
    })
    .to_string();
    JsonStore::from_srsj(&srsj).expect("fixture srsj must load")
}

/// Empty query returns all Tier-2 records.
#[test]
fn find_empty_query_returns_all() {
    let store = fixture_store();
    let result = find(&store, DiscoveryQuery::default()).expect("find must succeed");
    assert_eq!(result.total, 2, "both records returned for empty query");
    assert_eq!(result.hits.len(), 2);
}

/// `content_match` filters to records whose text projection contains the substring.
#[test]
fn find_content_match_filters_hits() {
    let store = fixture_store();
    let result = find(
        &store,
        DiscoveryQuery {
            content_match: Some("authority".to_string()),
            ..Default::default()
        },
    )
    .expect("find must succeed");
    assert_eq!(result.hits.len(), 1, "only 'Building Authority' matches");
    assert_eq!(result.hits[0].instance_id, REC_AUTHORITY);
}

/// Malformed JSON is rejected before reaching the service.
/// Exercises the `serde_json::from_str::<DiscoveryQuery>` call the binding wraps at its first
/// line — the error path that `to_js()` prevents from being tested via `SrsRepository::find()`.
#[test]
fn find_rejects_malformed_query_json() {
    let result = serde_json::from_str::<DiscoveryQuery>("{invalid json}");
    assert!(
        result.is_err(),
        "malformed JSON must be rejected by DiscoveryQuery deserializer"
    );
}

/// Structured `type_name` filter with no match returns an empty result set.
#[test]
fn find_type_name_filter_is_exact() {
    let store = fixture_store();
    let result = find(
        &store,
        DiscoveryQuery {
            type_name: Some("nonexistent".to_string()),
            ..Default::default()
        },
    )
    .expect("find must succeed");
    assert_eq!(result.hits.len(), 0, "unknown type_name yields no hits");
    assert_eq!(result.total, 0);
}
