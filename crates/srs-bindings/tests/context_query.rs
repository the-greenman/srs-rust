//! Integration tests for the context query bindings (ext:addressability, issue #251).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Exercises the service functions directly via
//! `srs_repository::srsj::open_srsj` rather than through `SrsRepository::context_*()` because
//! `to_js()` calls `js_sys::JSON::parse` which panics off-wasm.
//! The wasm-pack build proves the binding methods compile and are exported.

use srs_repository::context_query_service::{
    get_field_context, get_record_context, FieldContextQuery, RecordContextQuery,
};
use srs_repository::FileStore;

const FIELD_TITLE: &str = "aaaa0001-0000-4000-8000-000000000001";
const TYPE_ID: &str = "bbbb0001-0000-4000-8000-000000000001";
const RECORD_ID: &str = "cccc0001-0000-4000-8000-000000000001";

fn fixture_store() -> FileStore {
    let srsj = serde_json::json!({
        "srsj": "2",
        "manifest": {
            "dataModelRevision": 2,
            "repositoryId": "test-repo-context",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "pkg-context-001",
                "title": "Test Package",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": ["types/decision.json"],
                "relationTypes": [],
                "views": [],
                "compositions": [],
                "blueprints": []
            },
            "package/fields/title.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                "id": FIELD_TITLE,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title field",
                "aiGuidance": {"purpose": "Test guidance"},
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/types/decision.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                "id": TYPE_ID,
                "namespace": "com.test",
                "name": "decision",
                "version": 1,
                "description": "Decision type",
                "fields": [
                    {"fieldId": FIELD_TITLE, "order": 0, "required": true}
                ],
                "createdAt": "2026-01-01T00:00:00Z"
            },
            format!("records/tier-2/{RECORD_ID}.json"): {
                "instanceId": RECORD_ID,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": {"title": "First Decision"}
            }
            // No `.revisions.json` sidecar: rfc-decision-2a1e1590 retired the
            // mechanism, and catalog.rs no longer tolerates one (srs-rust#866)
            // — a repository carrying one now fails to load at all ([R24]).
        }
    })
    .to_string();
    srs_repository::srsj::open_srsj(&srsj).expect("fixture srsj must load")
}

#[test]
fn context_field_returns_current_value() {
    let store = fixture_store();
    let result = get_field_context(
        &store,
        FieldContextQuery {
            record_id: RECORD_ID.to_string(),
            field_id: FIELD_TITLE.to_string(),
        },
    )
    .expect("get_field_context must succeed");

    assert_eq!(result.record_id, RECORD_ID);
    assert_eq!(result.field_id, FIELD_TITLE);
    assert_eq!(result.field_name, Some("title".to_string()));
}

#[test]
fn context_record_returns_type_and_fields() {
    let store = fixture_store();
    let result = get_record_context(
        &store,
        RecordContextQuery {
            record_id: RECORD_ID.to_string(),
        },
    )
    .expect("get_record_context must succeed");

    assert_eq!(result.record_id, RECORD_ID);
    assert_eq!(result.type_id, TYPE_ID);
    assert_eq!(result.type_name, "decision");
    assert_eq!(result.field_values.len(), 1);
    assert_eq!(
        result.field_values.get("title"),
        Some(&serde_json::json!("First Decision"))
    );
}

#[test]
fn context_field_not_found_errors() {
    let store = fixture_store();
    let err = get_field_context(
        &store,
        FieldContextQuery {
            record_id: "00000000-0000-4000-8000-000000000000".to_string(),
            field_id: FIELD_TITLE.to_string(),
        },
    );
    assert!(err.is_err(), "missing record must return an error");
}
