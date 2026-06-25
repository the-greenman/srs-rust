//! Integration test for the WASM `list_blueprints` binding.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.
//!
//! The gallery fixture carries `blueprints: []`, so the populated case uses a small inline
//! `.srsj` fixture (same approach as `blueprint_schema.rs`); the gallery exercises the
//! empty-envelope path.

use srs_repository::blueprint_service;
use srs_repository::JsonStore;

/// Minimal `.srsj` with one blueprint registered in the package boundary.
fn blueprint_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-blueprint-list",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-blueprint-list-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": ["types/guide.json", "types/section.json"],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": ["blueprints/guide.json"]
            },
            "package/fields/title.json": {
                "id": "field-title-001",
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "valueType": "string",
                "description": "Title field",
                "aiGuidance": null,
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/types/guide.json": {
                "id": "type-guide-001",
                "namespace": "com.test",
                "name": "guide",
                "version": 1,
                "description": "A guide root",
                "fields": [
                    {"fieldId": "field-title-001", "order": 0, "required": true, "repeatable": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/types/section.json": {
                "id": "type-section-001",
                "namespace": "com.test",
                "name": "section",
                "version": 1,
                "description": "A guide section",
                "fields": [
                    {"fieldId": "field-title-001", "order": 0, "required": true, "repeatable": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/blueprints/guide.json": {
                "id": "bp-guide-001",
                "namespace": "com.test",
                "name": "guide-blueprint",
                "version": 1,
                "description": "A guide containing an ordered sequence of sections",
                "rootTypes": [{"typeId": "type-guide-001", "typeVersion": 1}],
                "structure": [
                    {
                        "relationType": "contains",
                        "sourceType": {"typeId": "type-guide-001"},
                        "targetType": {"typeId": "type-section-001"},
                        "cardinality": "1..*",
                        "required": true
                    }
                ],
                "requiredTypes": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }
        }
    })
    .to_string()
}

fn gallery_store() -> JsonStore {
    let srsj = include_str!("fixtures/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

/// A repo with one registered blueprint returns a single summary carrying its identity and
/// root-type count — the shape the web client lists in a blueprint picker.
#[test]
fn list_blueprints_returns_summaries() {
    let store = JsonStore::from_srsj(&blueprint_srsj()).expect("blueprint srsj must load");
    let result =
        blueprint_service::list_blueprints_summary(&store).expect("list_blueprints must succeed");

    assert_eq!(result.summaries.len(), 1, "one blueprint registered");
    let bp = &result.summaries[0];
    assert_eq!(bp.id, "bp-guide-001");
    assert_eq!(bp.namespace, "com.test");
    assert_eq!(bp.name, "guide-blueprint");
    assert_eq!(bp.root_type_count, 1, "one root type");
    assert!(
        result.diagnostics.is_empty(),
        "expected no provenance diagnostics: {:?}",
        result.diagnostics
    );
}

/// The gallery has no blueprints — the binding returns an empty `{ summaries: [], diagnostics: [] }`
/// envelope rather than erroring.
#[test]
fn list_blueprints_empty_on_gallery() {
    let store = gallery_store();
    let result =
        blueprint_service::list_blueprints_summary(&store).expect("list_blueprints must succeed");
    assert!(
        result.summaries.is_empty(),
        "gallery declares no blueprints"
    );
    assert!(
        result.diagnostics.is_empty(),
        "no provenance diagnostics expected: {:?}",
        result.diagnostics
    );
}
