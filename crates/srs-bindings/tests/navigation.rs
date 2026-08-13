//! Integration test for the WASM `repository_navigation` binding (issue #268).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `srs_repository::srsj::open_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.

use srs_repository::repository_navigation_service::repository_navigation;

const IDENTITY_ID: &str = "00000000-0000-4000-8000-00000000a100";
const ARTICLES_ID: &str = "00000000-0000-4000-8000-00000000a200";
const DECISIONS_ID: &str = "00000000-0000-4000-8000-00000000a300";
const ROOT_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000a000";
const ARTICLES_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000b000";
const DECISIONS_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000c000";
const FIELD_TITLE_ID: &str = "00000000-0000-4000-8000-00000000f100";
const NOTE_INSTANCE_ID: &str = "00000000-0000-4000-8000-00000000d100";

/// Minimal `.srsj` with `manifest.container`, an identity record, two section roots with a
/// `precedes` relation (articles precedes decisions), and two section containers.
fn nav_fixture_srsj() -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "dataModelRevision": 2,
            "repositoryId": "test-repo-navigation",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": ROOT_CONTAINER_ID,
                "title": "Governance Repo",
                "identityInstanceId": IDENTITY_ID,
                "memberInstanceIds": [IDENTITY_ID, ARTICLES_ID, DECISIONS_ID],
                "rootInstanceIds": [IDENTITY_ID]
            },
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "pkg-nav-001",
                "title": "Test Package",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "namespace": "com.test",
                "name": "test-nav-package",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            "package/fields/title.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                "id": FIELD_TITLE_ID,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {"purpose": "Test guidance"},
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{IDENTITY_ID}.json"): {
                "instanceId": IDENTITY_ID,
                "typeId": "type-identity",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "governance-repo",
                "fieldValues": {"title": "Governance Repo"}
            },
            format!("records/tier-2/{ARTICLES_ID}.json"): {
                "instanceId": ARTICLES_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Articles"}
            },
            format!("records/tier-2/{DECISIONS_ID}.json"): {
                "instanceId": DECISIONS_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Decision Log"}
            },
            format!("containers/{ARTICLES_CONTAINER_ID}.json"): {
                "containerId": ARTICLES_CONTAINER_ID,
                "containerType": "document",
                "title": "Articles",
                "rootInstanceIds": [ARTICLES_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("containers/{DECISIONS_CONTAINER_ID}.json"): {
                "containerId": DECISIONS_CONTAINER_ID,
                "containerType": "document",
                "title": "Decision Log",
                "rootInstanceIds": [DECISIONS_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "relations/relations.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                "relations": [{
                    "relationId": "rel-articles-precedes-decisions",
                    "relationType": "precedes",
                    "sourceInstanceId": ARTICLES_ID,
                    "targetInstanceId": DECISIONS_ID,
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            },
        }
    })
    .to_string()
}

/// Happy path: identity, two precedes-ordered sections, sectionContainerId resolved.
#[test]
fn repository_navigation_returns_identity_and_sections() {
    let store =
        srs_repository::srsj::open_srsj(&nav_fixture_srsj()).expect("fixture srsj must load");
    let nav = repository_navigation(&store).expect("navigation must succeed");

    assert_eq!(nav.root_container_id, ROOT_CONTAINER_ID);
    assert_eq!(nav.identity.instance_id, IDENTITY_ID);
    assert_eq!(nav.identity.display_label, "Governance Repo");

    assert_eq!(nav.sections.len(), 2);
    // precedes relation: articles (a200) precedes decisions (a300)
    assert_eq!(nav.sections[0].display_label, "Articles");
    assert_eq!(
        nav.sections[0].section_container_id.as_deref(),
        Some(ARTICLES_CONTAINER_ID)
    );
    assert_eq!(nav.sections[1].display_label, "Decision Log");
    assert_eq!(
        nav.sections[1].section_container_id.as_deref(),
        Some(DECISIONS_CONTAINER_ID)
    );

    assert!(nav.diagnostics.is_empty());

    // Result must serialise cleanly — this is the shape `to_js` passes to JS.
    let json = serde_json::to_value(&nav).expect("must serialise");
    assert!(json["rootContainerId"].is_string());
    assert!(json["sections"].is_array());
    assert_eq!(json["sections"].as_array().unwrap().len(), 2);
    assert!(json["diagnostics"].as_array().unwrap().is_empty());
}

/// Fixture for un-migrated repos where identityInstanceId points to a Tier-0 note.
/// `note_title` controls whether the note body carries a title (RFC-038: titles
/// are body-derived — the manifest index is retired and inert).
fn tier0_nav_fixture_srsj(note_title: Option<&str>) -> String {
    let mut note_body = serde_json::json!({
        "instanceId": NOTE_INSTANCE_ID,
        "sections": []
    });
    if let Some(t) = note_title {
        note_body["title"] = serde_json::Value::String(t.to_string());
    }

    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "dataModelRevision": 2,
            "repositoryId": "test-repo-tier0",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": ROOT_CONTAINER_ID,
                "title": "Governance Repo",
                "identityInstanceId": NOTE_INSTANCE_ID,
                "memberInstanceIds": [NOTE_INSTANCE_ID, ARTICLES_ID, DECISIONS_ID]
            },
            "dataModelRevision": 2,
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "records/notes/intent.json": note_body,
            "package/package.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "pkg-nav-tier0",
                "title": "Test Package",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "namespace": "com.test",
                "name": "test-nav-tier0-package",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            "package/fields/title.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                "id": FIELD_TITLE_ID,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {"purpose": "Test guidance"},
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{ARTICLES_ID}.json"): {
                "instanceId": ARTICLES_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Articles"}
            },
            format!("records/tier-2/{DECISIONS_ID}.json"): {
                "instanceId": DECISIONS_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Decision Log"}
            },
            format!("containers/{ARTICLES_CONTAINER_ID}.json"): {
                "containerId": ARTICLES_CONTAINER_ID,
                "containerType": "document",
                "title": "Articles",
                "rootInstanceIds": [ARTICLES_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("containers/{DECISIONS_CONTAINER_ID}.json"): {
                "containerId": DECISIONS_CONTAINER_ID,
                "containerType": "document",
                "title": "Decision Log",
                "rootInstanceIds": [DECISIONS_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "relations/relations.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                "relations": [{
                    "relationId": "rel-articles-precedes-decisions",
                    "relationType": "precedes",
                    "sourceInstanceId": ARTICLES_ID,
                    "targetInstanceId": DECISIONS_ID,
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            }
        }
    })
    .to_string()
}

/// Tier-0 note as identityInstanceId: navigation returns Ok with a diagnostic and uses the
/// note title from the instance index as the identity display label.
#[test]
fn repository_navigation_tier0_note_identity_returns_diagnostic() {
    let store =
        srs_repository::srsj::open_srsj(&tier0_nav_fixture_srsj(Some("Example Governance")))
            .expect("fixture must load");
    let nav = repository_navigation(&store).expect("navigation must return Ok, not Err");

    assert_eq!(nav.identity.instance_id, NOTE_INSTANCE_ID);
    assert_eq!(nav.identity.display_label, "Example Governance");
    assert_eq!(nav.diagnostics.len(), 1);
    assert!(nav.diagnostics[0].contains("Tier-0"));

    assert_eq!(nav.sections.len(), 2);
    assert_eq!(nav.sections[0].display_label, "Articles");
    assert_eq!(
        nav.sections[0].section_container_id.as_deref(),
        Some(ARTICLES_CONTAINER_ID)
    );
    assert_eq!(nav.sections[1].display_label, "Decision Log");
    assert_eq!(
        nav.sections[1].section_container_id.as_deref(),
        Some(DECISIONS_CONTAINER_ID)
    );
}

/// Tier-0 note with no title in the index: display label falls back to the instance ID.
#[test]
fn repository_navigation_tier0_note_identity_no_title_uses_id_as_label() {
    let store =
        srs_repository::srsj::open_srsj(&tier0_nav_fixture_srsj(None)).expect("fixture must load");
    let nav = repository_navigation(&store).expect("navigation must return Ok, not Err");

    assert_eq!(nav.identity.instance_id, NOTE_INSTANCE_ID);
    assert_eq!(nav.identity.display_label, NOTE_INSTANCE_ID);
    assert_eq!(nav.diagnostics.len(), 1);
}

/// Regression (#460): sub-containers where the root record also appears in memberInstanceIds
/// must still resolve sectionContainerId — the old root_is_member guard silently dropped them.
#[test]
fn repository_navigation_root_is_member_of_its_own_sub_container() {
    let srsj = serde_json::json!({
        "srsj": "2",
        "manifest": {
            "dataModelRevision": 2,
            "repositoryId": "test-repo-root-is-member",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "container": {
                "containerId": ROOT_CONTAINER_ID,
                "title": "Governance Repo",
                "identityInstanceId": IDENTITY_ID,
                "memberInstanceIds": [IDENTITY_ID, ARTICLES_ID, DECISIONS_ID],
                "rootInstanceIds": [IDENTITY_ID]
            },
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "pkg-nav-rim",
                "title": "Test Package",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "namespace": "com.test",
                "name": "test-nav-root-is-member",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            "package/fields/title.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                "id": FIELD_TITLE_ID,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {"purpose": "Test guidance"},
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{IDENTITY_ID}.json"): {
                "instanceId": IDENTITY_ID,
                "typeId": "type-identity",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "governance-repo",
                "fieldValues": {"title": "Governance Repo"}
            },
            format!("records/tier-2/{ARTICLES_ID}.json"): {
                "instanceId": ARTICLES_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Articles"}
            },
            format!("records/tier-2/{DECISIONS_ID}.json"): {
                "instanceId": DECISIONS_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": {"title": "Decision Log"}
            },
            // Both sub-containers have their root record also in memberInstanceIds —
            // this is the "root is also a member" shape that triggered the bug.
            format!("containers/{ARTICLES_CONTAINER_ID}.json"): {
                "containerId": ARTICLES_CONTAINER_ID,
                "containerType": "document",
                "title": "Articles",
                "rootInstanceIds": [ARTICLES_ID],
                "memberInstanceIds": [ARTICLES_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("containers/{DECISIONS_CONTAINER_ID}.json"): {
                "containerId": DECISIONS_CONTAINER_ID,
                "containerType": "document",
                "title": "Decision Log",
                "rootInstanceIds": [DECISIONS_ID],
                "memberInstanceIds": [DECISIONS_ID],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "relations/relations.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                "relations": [{
                    "relationId": "rel-articles-precedes-decisions",
                    "relationType": "precedes",
                    "sourceInstanceId": ARTICLES_ID,
                    "targetInstanceId": DECISIONS_ID,
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            }
        }
    })
    .to_string();

    let store = srs_repository::srsj::open_srsj(&srsj).expect("fixture srsj must load");
    let nav = repository_navigation(&store).expect("navigation must succeed");

    assert_eq!(nav.sections.len(), 2);
    assert_eq!(nav.sections[0].display_label, "Articles");
    assert_eq!(
        nav.sections[0].section_container_id.as_deref(),
        Some(ARTICLES_CONTAINER_ID),
        "articles section_container_id must resolve even when root is also a member"
    );
    assert_eq!(nav.sections[1].display_label, "Decision Log");
    assert_eq!(
        nav.sections[1].section_container_id.as_deref(),
        Some(DECISIONS_CONTAINER_ID),
        "decisions section_container_id must resolve even when root is also a member"
    );
    assert!(nav.diagnostics.is_empty());
}

/// Missing manifest.container (pre-RFC-013 repo): sections empty, one diagnostic entry.
/// Uses the gallery fixture, which has no manifest.container field.
#[test]
fn repository_navigation_without_manifest_container_returns_diagnostic() {
    let srsj = include_str!("../../srs-repository/tests/fixtures/gallery.srsj");
    let store = srs_repository::srsj::open_srsj(srsj).expect("gallery fixture must load");
    let nav = repository_navigation(&store).expect("navigation must return ok (not error)");

    assert_eq!(nav.root_container_id, "");
    assert_eq!(nav.identity.instance_id, ""); // NavigationNode::default(), not null
    assert!(nav.sections.is_empty());
    assert_eq!(nav.diagnostics.len(), 1);
    assert!(nav.diagnostics[0].contains("manifest.container is absent"));
}
