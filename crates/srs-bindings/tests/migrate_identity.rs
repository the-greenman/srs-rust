//! Smoke tests for the WASM `migrate_identity` binding (issue #434).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.

use srs_repository::migrate_identity_service;
use srs_repository::JsonStore;

const NOTE_ID: &str = "00000000-0000-4000-8000-00000000b001";
const ROOT_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000c000";
const NO_ID_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000d000";

/// Minimal `.srsj` with a Tier-0 note as the `identityInstanceId`.
fn tier0_fixture_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-migrate-identity",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": ROOT_CONTAINER_ID,
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
                "id": "pkg-migrate-test",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            "records/notes/identity.json": {
                "instanceId": NOTE_ID,
                "title": "Test Repo",
                "sections": [
                    {
                        "name": "body",
                        "content": "We govern with SRS."
                    }
                ]
            },
            format!("containers/{ROOT_CONTAINER_ID}.json"): {
                "containerId": ROOT_CONTAINER_ID,
                "title": "Test Repo",
                "identityInstanceId": NOTE_ID,
                "memberInstanceIds": [NOTE_ID]
            }
        }
    })
    .to_string()
}

/// Happy path: Tier-0 note identity → migrate → typed purpose record.
///
/// Checks that the result is structurally correct and that the serialised JSON
/// keys are camelCase (matching `serde(rename_all = "camelCase")` on `MigrateIdentityResult`).
#[test]
fn migrate_identity_tier0_to_purpose_record_succeeds() {
    let store = JsonStore::from_srsj(&tier0_fixture_srsj()).expect("fixture srsj must load");

    let result =
        migrate_identity_service::migrate_identity(&store).expect("migration must succeed");

    assert_eq!(result.old_identity_id.as_deref(), Some(NOTE_ID));
    assert_eq!(result.old_identity_tier, Some(0));
    assert!(
        !result.new_identity_id.is_empty(),
        "newIdentityId must not be empty"
    );
    assert_eq!(result.statement, "We govern with SRS.");
    assert_eq!(result.title.as_deref(), Some("Test Repo"));

    // Result must serialise cleanly in camelCase — this is the shape `to_js` passes to JS.
    let json = serde_json::to_value(&result).expect("result must serialise");
    assert!(json["oldIdentityId"].is_string());
    assert_eq!(json["oldIdentityTier"].as_u64(), Some(0));
    assert!(json["newIdentityId"].is_string());
    assert_eq!(json["statement"].as_str(), Some("We govern with SRS."));
    assert_eq!(json["title"].as_str(), Some("Test Repo"));
}

/// None-branch: container has no `identityInstanceId` → purpose record derived from title/description.
///
/// Verifies the camelCase serialisation shape for this path: `oldIdentityId` and `oldIdentityTier`
/// serialise as JSON `null` (not omitted), because `MigrateIdentityResult` has no
/// `skip_serializing_if` on those fields.
#[test]
fn migrate_identity_no_prior_identity_succeeds() {
    let fixture = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-migrate-no-identity",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": NO_ID_CONTAINER_ID,
                "title": "No Identity Repo",
                "description": "Bootstrap from container metadata."
            },
            "instanceIndex": [],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-no-id-test",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            }
        }
    })
    .to_string();

    let store = JsonStore::from_srsj(&fixture).expect("fixture srsj must load");

    let result = migrate_identity_service::migrate_identity(&store)
        .expect("none-branch migration must succeed");

    assert!(result.old_identity_id.is_none());
    assert!(result.old_identity_tier.is_none());
    assert!(
        !result.new_identity_id.is_empty(),
        "newIdentityId must not be empty"
    );
    assert_eq!(result.statement, "Bootstrap from container metadata.");
    assert_eq!(result.title.as_deref(), Some("No Identity Repo"));

    // oldIdentityId and oldIdentityTier serialise as null (not omitted) because
    // MigrateIdentityResult has no skip_serializing_if on those fields.
    let json = serde_json::to_value(&result).expect("result must serialise");
    assert!(
        json["oldIdentityId"].is_null(),
        "oldIdentityId must be null"
    );
    assert!(
        json["oldIdentityTier"].is_null(),
        "oldIdentityTier must be null"
    );
    assert!(json["newIdentityId"].is_string());
    assert_eq!(
        json["statement"].as_str(),
        Some("Bootstrap from container metadata.")
    );
    assert_eq!(json["title"].as_str(), Some("No Identity Repo"));
}

/// Negative case: calling `migrate_identity` a second time on an already-migrated store
/// must return an error whose message contains "already".
#[test]
fn migrate_identity_already_migrated_returns_error() {
    let store = JsonStore::from_srsj(&tier0_fixture_srsj()).expect("fixture srsj must load");

    migrate_identity_service::migrate_identity(&store).expect("first migration must succeed");

    let err = migrate_identity_service::migrate_identity(&store)
        .expect_err("second migration on already-migrated store must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("already"),
        "error message must contain 'already', got: {msg}"
    );
}
