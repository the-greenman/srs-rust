//! Integration tests for the context query bindings (ext:addressability, issue #251).
//!
//! Native Rust tests (not `#[wasm_bindgen_test]`) — run with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Exercises the service functions directly via
//! `JsonStore::from_srsj` rather than through `SrsRepository::context_*()` because
//! `to_js()` calls `js_sys::JSON::parse` which panics off-wasm.
//! The wasm-pack build proves the binding methods compile and are exported.

use srs_repository::context_query_service::{
    get_field_context, get_record_context, get_revision_trace, FieldContextQuery,
    RecordContextQuery, RevisionTraceQuery,
};
use srs_repository::JsonStore;

const FIELD_TITLE: &str = "aaaa0001-0000-4000-8000-000000000001";
const TYPE_ID: &str = "bbbb0001-0000-4000-8000-000000000001";
const RECORD_ID: &str = "cccc0001-0000-4000-8000-000000000001";
const REVISION_ID: &str = "dddd0001-0000-4000-8000-000000000001";

fn fixture_store() -> JsonStore {
    let srsj = serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-context",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": RECORD_ID, "path": format!("records/tier-2/{RECORD_ID}.json"), "tier": 2}
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-context-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json"],
                "types": ["types/decision.json"],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            },
            "package/fields/title.json": {
                "id": FIELD_TITLE,
                "namespace": "com.test",
                "name": "title",
                "version": 1,
                "description": "Title field",
                "aiGuidance": null,
                "valueType": "string",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/types/decision.json": {
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
                "fieldValues": [
                    {"fieldId": FIELD_TITLE, "value": "First Decision"}
                ]
            },
            format!("records/tier-2/{RECORD_ID}.revisions.json"): {
                "recordId": RECORD_ID,
                "revisions": [
                    {
                        "revisionId": REVISION_ID,
                        "recordId": RECORD_ID,
                        "fieldId": FIELD_TITLE,
                        "value": "First Decision",
                        "agent": {"type": "Human"},
                        "createdAt": "2026-01-01T00:00:00Z"
                    }
                ]
            }
        }
    })
    .to_string();
    JsonStore::from_srsj(&srsj).expect("fixture srsj must load")
}

#[test]
fn context_field_returns_value_and_revision() {
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
    assert_eq!(result.revisions.len(), 1);
    assert_eq!(result.revisions[0].revision_id, REVISION_ID);
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
    assert_eq!(result.field_values[0].field_id, FIELD_TITLE);
}

#[test]
fn context_revision_traces_single_revision() {
    let store = fixture_store();
    let result = get_revision_trace(
        &store,
        RevisionTraceQuery {
            record_id: RECORD_ID.to_string(),
            field_id: FIELD_TITLE.to_string(),
            revision_id: REVISION_ID.to_string(),
        },
    )
    .expect("get_revision_trace must succeed");

    assert_eq!(result.revision.revision_id, REVISION_ID);
    assert!(
        result.prior_chain.is_empty(),
        "no prior revisions for the root"
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
