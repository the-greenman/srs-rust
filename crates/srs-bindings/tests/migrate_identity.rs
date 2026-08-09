//! Smoke tests for the WASM `migrate_identity` binding (issue #434).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.

use srs_repository::migrate_identity_service;
use srs_repository::repository_navigation_service;
use srs_repository::JsonStore;

const NOTE_ID: &str = "00000000-0000-4000-8000-00000000b001";
const ROOT_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000c000";
const NO_ID_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000d000";

// IDs for sections_survive_migrate_identity fixture
const SECTIONS_ROOT_CTR_ID: &str = "00000000-0000-4000-8000-00000000e000";
const ARTICLES_RECORD_ID: &str = "00000000-0000-4000-8000-00000000e100";
const ARTICLES_CTR_ID: &str = "00000000-0000-4000-8000-00000000e200";
const FIELD_TITLE_ID: &str = "00000000-0000-4000-8000-00000000f001";

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

/// Regression test for #607: repository_navigation() must return non-empty sections after
/// None-branch migrate_identity on a repo that has pre-existing section members.
///
/// Before the fix, the None-branch called `save_container(&manifest.container)` which
/// overwrote the container file with the manifest embed — containing only the new identity
/// member. A subsequent `repository_navigation()` then found only the identity in the
/// container file, skipped it, and returned an empty sections list.
#[test]
fn sections_survive_migrate_identity() {
    let fixture = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-607-sections-survive",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": SECTIONS_ROOT_CTR_ID,
                "title": "My Governance Repo",
                "description": "We govern with SRS."
                // No identityInstanceId — triggers None-branch
            },
            "instanceIndex": [
                {
                    "instanceId": ARTICLES_RECORD_ID,
                    "path": format!("records/tier-2/{ARTICLES_RECORD_ID}.json"),
                    "tier": 2
                }
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-607-test",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [format!("fields/{FIELD_TITLE_ID}.json")],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            format!("package/fields/{FIELD_TITLE_ID}.json"): {
                "id": FIELD_TITLE_ID,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {},
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{ARTICLES_RECORD_ID}.json"): {
                "instanceId": ARTICLES_RECORD_ID,
                "typeId": "type-section-607",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Articles"}
            },
            // Container file with pre-existing section member — must survive migration.
            format!("containers/{SECTIONS_ROOT_CTR_ID}.json"): {
                "containerId": SECTIONS_ROOT_CTR_ID,
                "title": "My Governance Repo",
                "memberInstanceIds": [ARTICLES_RECORD_ID]
            },
            format!("containers/{ARTICLES_CTR_ID}.json"): {
                "containerId": ARTICLES_CTR_ID,
                "containerType": "document",
                "title": "Articles",
                "rootInstanceIds": [ARTICLES_RECORD_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            // "manifest.json" in data exercises the pre-#466 shadow-containerIndex
            // open-time migration in JsonStore::from_srsj: when the top-level manifest
            // has no containerIndex key, from_srsj reads data["manifest.json"] and
            // promotes its containerIndex into the parsed manifest. This entry is
            // intentional — it reflects how legacy .srsj bundles encode the container
            // index, and ensures the migration path does not break navigation.
            "manifest.json": {
                "containerIndex": [
                    {"containerId": ARTICLES_CTR_ID, "title": "Articles"}
                ]
            }
        }
    })
    .to_string();

    let store = JsonStore::from_srsj(&fixture).expect("fixture must load");

    // Run None-branch migration — must succeed.
    let result = migrate_identity_service::migrate_identity(&store)
        .expect("None-branch migration must succeed");
    assert!(result.old_identity_id.is_none(), "should be None-branch");

    // repository_navigation must return the pre-existing Articles section.
    let nav = repository_navigation_service::repository_navigation(&store)
        .expect("repository_navigation must succeed after None-branch migration");

    assert_eq!(
        nav.sections.len(),
        1,
        "Articles section must survive None-branch migration; sections: {:?}",
        nav.sections
    );
    assert_eq!(
        nav.sections[0].display_label, "Articles",
        "section display_label must be Articles"
    );
}
