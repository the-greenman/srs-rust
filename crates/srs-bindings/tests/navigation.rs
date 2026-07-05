//! Integration test for the WASM `repository_navigation` binding (issue #268).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.

use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::JsonStore;

const IDENTITY_ID: &str = "00000000-0000-4000-8000-00000000a100";
const ARTICLES_ID: &str = "00000000-0000-4000-8000-00000000a200";
const DECISIONS_ID: &str = "00000000-0000-4000-8000-00000000a300";
const ROOT_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000a000";
const ARTICLES_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000b000";
const DECISIONS_CONTAINER_ID: &str = "00000000-0000-4000-8000-00000000c000";
const FIELD_TITLE_ID: &str = "00000000-0000-4000-8000-00000000f100";

/// Minimal `.srsj` with `manifest.container`, an identity record, two section roots with a
/// `precedes` relation (articles precedes decisions), and two section containers.
fn nav_fixture_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
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
            "instanceIndex": [
                {"instanceId": IDENTITY_ID, "path": format!("records/tier-2/{IDENTITY_ID}.json"), "tier": 2},
                {"instanceId": ARTICLES_ID, "path": format!("records/tier-2/{ARTICLES_ID}.json"), "tier": 2},
                {"instanceId": DECISIONS_ID, "path": format!("records/tier-2/{DECISIONS_ID}.json"), "tier": 2}
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-nav-001",
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
                "id": FIELD_TITLE_ID,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title",
                "aiGuidance": {},
                "valueType": "string",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{IDENTITY_ID}.json"): {
                "instanceId": IDENTITY_ID,
                "typeId": "type-identity",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "governance-repo",
                "fieldValues": [{"fieldId": FIELD_TITLE_ID, "value": "Governance Repo"}]
            },
            format!("records/tier-2/{ARTICLES_ID}.json"): {
                "instanceId": ARTICLES_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": [{"fieldId": FIELD_TITLE_ID, "value": "Articles"}]
            },
            format!("records/tier-2/{DECISIONS_ID}.json"): {
                "instanceId": DECISIONS_ID,
                "typeId": "type-section",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "section",
                "fieldValues": [{"fieldId": FIELD_TITLE_ID, "value": "Decision Log"}]
            },
            format!("containers/{ROOT_CONTAINER_ID}.json"): {
                "containerId": ROOT_CONTAINER_ID,
                "containerType": "root",
                "title": "Governance Repo",
                "rootInstanceIds": [IDENTITY_ID],
                "memberInstanceIds": [IDENTITY_ID, ARTICLES_ID, DECISIONS_ID],
                "createdAt": "2026-01-01T00:00:00Z"
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
            // list_container_summaries reads containerIndex from data["manifest.json"],
            // not from the typed manifest struct. Include only the section containers here;
            // the root container is embedded in manifest.container, not enumerated separately.
            "manifest.json": {
                "containerIndex": [
                    {"containerId": ARTICLES_CONTAINER_ID, "title": "Articles"},
                    {"containerId": DECISIONS_CONTAINER_ID, "title": "Decision Log"}
                ]
            }
        }
    })
    .to_string()
}

/// Happy path: identity, two precedes-ordered sections, sectionContainerId resolved.
#[test]
fn repository_navigation_returns_identity_and_sections() {
    let store = JsonStore::from_srsj(&nav_fixture_srsj()).expect("fixture srsj must load");
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

/// Missing manifest.container (pre-RFC-013 repo): sections empty, one diagnostic entry.
/// Uses the gallery fixture, which has no manifest.container field.
#[test]
fn repository_navigation_without_manifest_container_returns_diagnostic() {
    let srsj = include_str!("fixtures/gallery.srsj");
    let store = JsonStore::from_srsj(srsj).expect("gallery fixture must load");
    let nav = repository_navigation(&store).expect("navigation must return ok (not error)");

    assert_eq!(nav.root_container_id, "");
    assert!(nav.sections.is_empty());
    assert_eq!(nav.diagnostics.len(), 1);
    assert!(nav.diagnostics[0].contains("manifest.container is absent"));
}
