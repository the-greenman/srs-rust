//! Integration test for the WASM `list_blueprint_structure` binding
//! (srs-rust#1127).
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — follows the same pattern
//! as `blueprints.rs`: exercise the underlying service directly, since
//! `to_js()` calls `js_sys::JSON::parse` which panics off-wasm. The
//! `wasm-pack`/`cargo build --target wasm32-unknown-unknown` build proves the
//! `#[wasm_bindgen]` export itself compiles; this test proves the service
//! call the binding forwards to returns the expected shape.

use srs_repository::blueprint_service;

/// Minimal `.srsj` with one blueprint whose `structure` has two RelationSpec
/// entries, deliberately declared out of the service's documented sort order
/// (`(sourceTypeId, targetTypeId, relationType)` ascending) so the test
/// catches a binding that returns the raw unsorted list.
fn blueprint_srsj() -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "repositoryId": "test-repo-blueprint-structure",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "dataModelRevision": 2,
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "pkg-blueprint-structure-001",
                "title": "Test Package",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": ["types/guide.json", "types/section.json"],
                "relationTypes": [],
                "views": [],
                "compositions": [],
                "blueprints": ["blueprints/guide.json"]
            },
            "package/fields/title.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                "id": "field-title-001",
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "fieldType": {"datatype": "string"},
                "description": "Title field",
                "aiGuidance": {"purpose": "Test guidance"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/types/guide.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                "id": "type-guide-001",
                "namespace": "com.test",
                "name": "guide",
                "version": 1,
                "description": "A guide root",
                "fields": [
                    {"fieldId": "field-title-001", "order": 0, "required": true}
                ],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/types/section.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                "id": "type-section-001",
                "namespace": "com.test",
                "name": "section",
                "version": 1,
                "description": "A guide section",
                "fields": [
                    {"fieldId": "field-title-001", "order": 0, "required": true}
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
                        "relationType": "precedes",
                        "sourceType": {"typeId": "type-section-001"},
                        "targetType": {"typeId": "type-section-001"},
                        "cardinality": "0..1",
                        "required": false
                    },
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

/// `list_blueprint_structure` — the exact service the WASM binding forwards
/// to — resolves type names alongside their ids and sorts deterministically,
/// same shape the CLI's `blueprint structure` command payload exposes as
/// `relationSpecs[]`.
#[test]
fn list_blueprint_structure_returns_sorted_specs_with_resolved_names() {
    let store =
        srs_repository::srsj::open_srsj(&blueprint_srsj()).expect("blueprint srsj must load");
    let specs = blueprint_service::list_blueprint_structure(&store, "bp-guide-001")
        .expect("list_blueprint_structure must succeed");

    assert_eq!(specs.len(), 2, "two RelationSpec entries declared");
    // Sort key is (source_type_id, target_type_id, relation_type) ascending —
    // "type-guide-001" < "type-section-001", so the contains entry (source =
    // guide) must come first despite being declared second.
    assert_eq!(specs[0].relation_type, "contains");
    assert_eq!(specs[0].source_type_id, "type-guide-001");
    assert_eq!(specs[0].source_type_name.as_deref(), Some("com.test/guide"));
    assert_eq!(specs[0].target_type_id, "type-section-001");
    assert_eq!(
        specs[0].target_type_name.as_deref(),
        Some("com.test/section")
    );
    assert_eq!(specs[0].cardinality.as_deref(), Some("1..*"));
    assert_eq!(specs[0].required, Some(true));

    assert_eq!(specs[1].relation_type, "precedes");
    assert_eq!(specs[1].source_type_id, "type-section-001");
    assert_eq!(specs[1].target_type_id, "type-section-001");

    // Confirm the struct actually serializes camelCase, per srs-rust#1127 —
    // this is what `to_js()` hands to the WASM caller.
    let json = serde_json::to_value(&specs[0]).unwrap();
    assert!(
        json.get("relationType").is_some(),
        "must serialize camelCase"
    );
    assert!(json.get("sourceTypeId").is_some());
    assert!(
        json.get("relation_type").is_none(),
        "must not serialize snake_case"
    );
}

/// A blueprint id that doesn't resolve returns the same `BlueprintNotFound`
/// error the CLI's `blueprint structure` command already handles.
#[test]
fn list_blueprint_structure_not_found() {
    let store =
        srs_repository::srsj::open_srsj(&blueprint_srsj()).expect("blueprint srsj must load");
    let err = blueprint_service::list_blueprint_structure(&store, "bp-does-not-exist")
        .expect_err("unknown blueprint id must error");
    assert!(matches!(
        err,
        srs_repository::error::RepositoryError::BlueprintNotFound { .. }
    ));
}
