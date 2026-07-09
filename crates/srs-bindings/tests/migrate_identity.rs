//! Integration tests for the `migrate_identity` WASM binding (issue #434).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Exercises the underlying service directly via
//! `JsonStore::from_srsj`, since `to_js()` calls `js_sys::JSON::parse` which panics off-wasm.
//! Verifies that `MigrateIdentityResult` serialises cleanly to the JSON shape the JS caller
//! expects. The wasm-pack build proves the `#[wasm_bindgen]` export compiles.

use srs_repository::migrate_identity_service;
use srs_repository::JsonStore;

const NOTE_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const CONTAINER_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

/// Minimal `.srsj` with a Tier-0 note as the repository identity.
fn srsj_with_note_identity() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-migrate-identity",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": CONTAINER_ID,
                "title": "Test Repo",
                "identityInstanceId": NOTE_ID,
                "memberInstanceIds": [NOTE_ID]
            },
            "instanceIndex": [
                {
                    "instanceId": NOTE_ID,
                    "path": "records/notes/identity.json",
                    "tier": 0
                }
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-migrate-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": []
            },
            "records/notes/identity.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
                "instanceId": NOTE_ID,
                "title": "Test Repo",
                "sections": [
                    {"name": "body", "content": "Building the SRS system."}
                ]
            },
            format!("containers/{CONTAINER_ID}.json"): {
                "containerId": CONTAINER_ID,
                "title": "Test Repo",
                "memberInstanceIds": [NOTE_ID]
            }
        }
    })
    .to_string()
}

/// Minimal `.srsj` with a container but no `identityInstanceId` (None-branch migration).
fn srsj_without_identity() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-migrate-identity-none",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": CONTAINER_ID,
                "title": "No Identity Repo",
                "description": "A repo without an identity instance."
            },
            "instanceIndex": [],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-migrate-002",
                "namespace": "com.test",
                "name": "test-package-none",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": []
            },
            format!("containers/{CONTAINER_ID}.json"): {
                "containerId": CONTAINER_ID,
                "title": "No Identity Repo",
                "description": "A repo without an identity instance."
            }
        }
    })
    .to_string()
}

/// Happy path: Tier-0 note identity migrated to a `com.semanticops.core/purpose` record.
/// Verifies the result serialises to the camelCase JSON shape the JS caller expects.
#[test]
fn migrate_identity_from_note_result_serialises_cleanly() {
    let store = JsonStore::from_srsj(&srsj_with_note_identity()).expect("fixture must load");
    let result =
        migrate_identity_service::migrate_identity(&store).expect("migration must succeed");

    assert_eq!(result.old_identity_id.as_deref(), Some(NOTE_ID));
    assert_eq!(result.old_identity_tier, Some(0));
    assert!(!result.new_identity_id.is_empty());
    assert_eq!(result.statement, "Building the SRS system.");
    assert_eq!(result.title.as_deref(), Some("Test Repo"));

    let json = serde_json::to_value(&result).expect("MigrateIdentityResult must serialise");
    assert_eq!(json["oldIdentityId"].as_str(), Some(NOTE_ID));
    assert_eq!(json["oldIdentityTier"], serde_json::json!(0));
    assert!(json["newIdentityId"].is_string());
    assert!(!json["newIdentityId"].as_str().unwrap().is_empty());
    assert_eq!(json["statement"].as_str(), Some("Building the SRS system."));
    assert_eq!(json["title"].as_str(), Some("Test Repo"));
}

/// None-branch: no prior `identityInstanceId` — derives purpose from container metadata.
#[test]
fn migrate_identity_no_prior_identity_result_serialises_cleanly() {
    let store = JsonStore::from_srsj(&srsj_without_identity()).expect("fixture must load");
    let result =
        migrate_identity_service::migrate_identity(&store).expect("migration must succeed");

    assert!(result.old_identity_id.is_none());
    assert!(result.old_identity_tier.is_none());
    assert!(!result.new_identity_id.is_empty());
    assert_eq!(result.statement, "A repo without an identity instance.");

    let json = serde_json::to_value(&result).expect("MigrateIdentityResult must serialise");
    assert!(json["oldIdentityId"].is_null());
    assert!(json["oldIdentityTier"].is_null());
    assert!(json["newIdentityId"].is_string());
    assert_eq!(
        json["statement"].as_str(),
        Some("A repo without an identity instance.")
    );
}
