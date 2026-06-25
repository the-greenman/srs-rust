//! Integration test for the WASM `type_schema` binding.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.
//!
//! Gallery `.srsj` Type used: `decision` `1fcad6a2-9f78-5e41-94ba-d82e88b822f3` v1.

use srs_repository::type_schema_service::{self, TypeSchemaInput};
use srs_repository::JsonStore;

const DECISION_TYPE_ID: &str = "1fcad6a2-9f78-5e41-94ba-d82e88b822f3";

fn gallery_store() -> JsonStore {
    let srsj = include_str!("fixtures/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

/// `type_version = None` resolves the latest version and emits a draft-07 object schema —
/// the shape the web client's record editor consumes to render a form for a Type.
#[test]
fn type_schema_resolves_latest_version() {
    let store = gallery_store();
    let result = type_schema_service::type_schema(
        &store,
        TypeSchemaInput {
            type_id: DECISION_TYPE_ID.to_string(),
            type_version: None,
        },
    )
    .expect("type_schema must succeed for a known type");

    assert_eq!(
        result.schema["type"],
        serde_json::json!("object"),
        "schema is a draft-07 object schema"
    );
    assert!(
        result.schema["properties"].is_object(),
        "schema carries field properties"
    );
}

/// A pinned `type_version` resolves the same schema as the latest lookup (gallery has only v1).
#[test]
fn type_schema_resolves_pinned_version() {
    let store = gallery_store();
    let latest = type_schema_service::type_schema(
        &store,
        TypeSchemaInput {
            type_id: DECISION_TYPE_ID.to_string(),
            type_version: None,
        },
    )
    .expect("latest must resolve");
    let pinned = type_schema_service::type_schema(
        &store,
        TypeSchemaInput {
            type_id: DECISION_TYPE_ID.to_string(),
            type_version: Some(1),
        },
    )
    .expect("pinned v1 must resolve");

    assert_eq!(
        pinned.schema["type"],
        serde_json::json!("object"),
        "pinned v1 independently projects a draft-07 object schema"
    );
    assert_eq!(
        latest.schema, pinned.schema,
        "latest and pinned v1 project the same schema"
    );
}

/// An unresolvable Type id is a hard error (not an empty schema).
#[test]
fn type_schema_unknown_id_errors() {
    let store = gallery_store();
    assert!(
        type_schema_service::type_schema(
            &store,
            TypeSchemaInput {
                type_id: "does-not-exist".to_string(),
                type_version: None,
            },
        )
        .is_err(),
        "unknown type id should error"
    );
}
