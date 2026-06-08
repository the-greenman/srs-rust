//! Integration test for the WASM `blueprint_schema` binding.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) so it runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Following the pattern of the other binding tests in
//! this crate: in native tests the binding's `JsValue` return path is not usable (it calls
//! into JS via `JSON::parse`), so we exercise the exact same service the binding wraps against
//! the same `JsonStore::from_srsj` fixture. The wasm-pack build proves the binding itself
//! compiles and is `#[wasm_bindgen]`-exported.

use srs_repository::blueprint_schema_service::{self, BlueprintSchemaInput};
use srs_repository::JsonStore;

/// Minimal `.srsj` with two types and a blueprint (root `guide`, `contains` → `section`).
fn blueprint_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-blueprint",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-blueprint-001",
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

#[test]
fn blueprint_schema_returns_root_and_child_definitions() {
    let store = JsonStore::from_srsj(&blueprint_srsj()).expect("blueprint srsj must load");
    let result = blueprint_schema_service::blueprint_schema(
        &store,
        BlueprintSchemaInput {
            blueprint_id: "bp-guide-001".to_string(),
        },
    )
    .expect("blueprint_schema must succeed");

    let schema = &result.schema;
    assert_eq!(
        schema["properties"]["root"]["$ref"],
        serde_json::json!("#/definitions/type-guide-001"),
        "root should $ref the guide definition"
    );
    // `contains` becomes a child-array property keyed by lowerCamelCase relation type.
    assert_eq!(
        schema["properties"]["contains"]["type"],
        serde_json::json!("array"),
        "contains should be a child-array property"
    );
    assert_eq!(
        schema["properties"]["contains"]["items"]["$ref"],
        serde_json::json!("#/definitions/type-section-001"),
        "contains items should $ref the section definition"
    );
    let defs = schema["definitions"]
        .as_object()
        .expect("definitions object");
    assert!(
        defs.contains_key("type-guide-001"),
        "guide definition present"
    );
    assert!(
        defs.contains_key("type-section-001"),
        "section definition present"
    );
    assert!(
        result.diagnostics.is_empty(),
        "expected no diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn blueprint_schema_unknown_id_errors() {
    let store = JsonStore::from_srsj(&blueprint_srsj()).expect("blueprint srsj must load");
    assert!(
        blueprint_schema_service::blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: "does-not-exist".to_string(),
            },
        )
        .is_err(),
        "unknown blueprint id should error"
    );
}
