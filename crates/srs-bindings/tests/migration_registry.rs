//! Smoke tests for the WASM `available_migrations` and `apply_migration` bindings (#461).
//!
//! Native Rust tests — run with `cargo test -p srs-bindings`. Exercises the underlying
//! service directly via `JsonStore::from_srsj`, since `to_js()` calls `js_sys::JSON::parse`
//! which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]` exports compile.

use srs_repository::migration_registry_service::{self, MigrationStatus};
use srs_repository::JsonStore;

fn minimal_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-migration-registry",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-migration-test",
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
    .to_string()
}

#[test]
fn available_migrations_lists_two_migrations_with_status() {
    let store = JsonStore::from_srsj(&minimal_srsj()).expect("fixture must load");

    let migrations = migration_registry_service::list_migrations(&store)
        .expect("list_migrations must succeed");

    assert_eq!(migrations.len(), 2, "expected exactly two migrations");
    assert_eq!(migrations[0].id, "migrate-identity");
    assert_eq!(migrations[1].id, "repo-upgrade");

    // No container → migrate-identity is NotApplicable; no instances → repo-upgrade is AlreadyApplied.
    assert_eq!(migrations[0].status, MigrationStatus::NotApplicable);
    assert_eq!(migrations[1].status, MigrationStatus::AlreadyApplied);

    // Result serialises in camelCase for to_js.
    let json = serde_json::to_value(&migrations[0]).expect("must serialise");
    assert!(json["id"].is_string());
    assert!(json["title"].is_string());
    assert!(json["description"].is_string());
    assert!(json["status"].is_string());
}

#[test]
fn apply_migration_repo_upgrade_on_canonical_repo() {
    let store = JsonStore::from_srsj(&minimal_srsj()).expect("fixture must load");

    let result = migration_registry_service::apply_migration(&store, "repo-upgrade")
        .expect("apply_migration must succeed on canonical repo");

    assert_eq!(result.id, "repo-upgrade");
    let payload = &result.payload;
    assert_eq!(payload["totalInstances"], 0);
    assert_eq!(payload["alreadyCanonicalCount"], 0);
    assert!(payload["renames"].as_array().unwrap().is_empty());
}

#[test]
fn apply_migration_unknown_id_returns_error() {
    let store = JsonStore::from_srsj(&minimal_srsj()).expect("fixture must load");

    let err = migration_registry_service::apply_migration(&store, "nonexistent-migration")
        .expect_err("unknown migration ID must return an error");

    let msg = err.to_string();
    assert!(
        msg.contains("nonexistent-migration"),
        "error must name the unknown ID, got: {msg}"
    );
}

#[test]
fn apply_migration_via_registry_migrate_identity() {
    let container_id = "550e8400-e29b-41d4-a716-446655440000";
    let note_id = "bbbb0001-0000-4000-8000-000000000001";

    // Build the fixture as a JsonStore via SRSJ so MemoryStore (test-only) is not needed.
    let srsj = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": container_id,
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [{
                "instanceId": note_id,
                "tier": 0,
                "path": "records/notes/identity.json",
                "title": "Test Repo"
            }],
            "container": {
                "containerId": container_id,
                "title": "Test Repo",
                "identityInstanceId": note_id,
                "memberInstanceIds": [note_id]
            },
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-migration-test",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [], "types": [], "relationTypes": [],
                "views": [], "documentViews": [], "blueprints": []
            },
            format!("containers/{container_id}.json"): {
                "containerId": container_id,
                "title": "Test Repo",
                "identityInstanceId": note_id,
                "memberInstanceIds": [note_id]
            },
            "records/notes/identity.json": {
                "instanceId": note_id,
                "title": "Test Repo",
                "sections": [{"name": "body", "content": "We build SRS."}]
            }
        }
    }).to_string();

    let store = JsonStore::from_srsj(&srsj).expect("fixture must load");

    // Before apply: migrate-identity should be Needed.
    let before = migration_registry_service::list_migrations(&store).unwrap();
    let id_before = before.iter().find(|m| m.id == "migrate-identity").unwrap();
    assert_eq!(id_before.status, MigrationStatus::Needed);

    // Apply via the registry.
    let result = migration_registry_service::apply_migration(&store, "migrate-identity")
        .expect("apply_migration must succeed");
    assert_eq!(result.id, "migrate-identity");
    assert!(result.payload["newIdentityId"].is_string(), "payload must contain newIdentityId");
    assert_eq!(result.payload["statement"], "We build SRS.");

    // After apply: migrate-identity should be AlreadyApplied.
    let after = migration_registry_service::list_migrations(&store).unwrap();
    let id_after = after.iter().find(|m| m.id == "migrate-identity").unwrap();
    assert_eq!(id_after.status, MigrationStatus::AlreadyApplied);
}
