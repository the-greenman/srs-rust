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

const OTHER_ID: &str = "cccc0001-0000-4000-8000-000000000002";

fn fixture_srsj() -> String {
    serde_json::json!({
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
            },
            format!("records/tier-2/{OTHER_ID}.json"): {
                "instanceId": OTHER_ID,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": {"title": "Second Decision"}
            },
            "relations/dddd0001-0000-4000-8000-000000000001.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/relation.json",
                "relationId": "dddd0001-0000-4000-8000-000000000001",
                "relationType": "depends-on",
                "sourceInstanceId": OTHER_ID,
                "targetInstanceId": RECORD_ID,
                "createdAt": "2026-01-01T00:00:00Z"
            }
            // No `.revisions.json` sidecar: rfc-decision-2a1e1590 retired the
            // mechanism, and catalog.rs no longer tolerates one (srs-rust#866)
            // — a repository carrying one now fails to load at all ([R24]).
        }
    })
    .to_string()
}

fn fixture_store() -> FileStore {
    srs_repository::srsj::open_srsj(&fixture_srsj()).expect("fixture srsj must load")
}

/// #1134 acceptance: the native dispatcher and the browser session answer the context
/// resource identically (one shared `SrsMcpApplication`), inbound edge and neighbour included.
#[test]
fn context_resource_native_matches_browser_session() {
    use serde_json::{json, Value};
    let srsj = fixture_srsj();
    let uri = format!("srs://test-repo-context/context/{RECORD_ID}");
    let req = json!({"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri": uri}});

    let mut native = srs_mcp_core::McpDispatcher::new(
        srs_mcp_core::SrsMcpApplication::open(srs_repository::srsj::open_srsj(&srsj).unwrap())
            .unwrap(),
    );
    let init = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}});
    native.dispatch(init.clone()).unwrap();
    let native_out = native.dispatch(req.clone()).unwrap();

    let repo = srs_bindings::SrsRepository::load(&srsj).ok().unwrap();
    let mut session = repo.open_mcp_session().ok().unwrap();
    session.handle(&init.to_string()).unwrap();
    let browser_out: Value =
        serde_json::from_str(&session.handle(&req.to_string()).unwrap()).unwrap();

    assert_eq!(native_out, browser_out);
    let text = native_out["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let ctx: Value = serde_json::from_str(text).unwrap();
    assert_eq!(ctx["relations"][0]["direction"], "in");
    assert_eq!(ctx["relations"][0]["sourceId"], OTHER_ID);
    assert_eq!(ctx["relations"][0]["neighbour"]["kind"], "record");
    assert_eq!(ctx["relations"][0]["neighbour"]["instanceId"], OTHER_ID);
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
            container_id: None,
            exclude_relation_categories: vec![],
            projection: Default::default(),
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

#[test]
fn context_record_query_accepts_exclude_relation_categories_key() {
    // The WASM `context_record` input is this struct deserialized from JSON: pin the key.
    let q: RecordContextQuery = serde_json::from_str(
        r#"{"recordId":"r","excludeRelationCategories":["composition","sequence"]}"#,
    )
    .unwrap();
    assert_eq!(q.exclude_relation_categories.len(), 2);
    let d: RecordContextQuery = serde_json::from_str(r#"{"recordId":"r"}"#).unwrap();
    assert!(d.exclude_relation_categories.is_empty());
    assert!(serde_json::from_str::<RecordContextQuery>(
        r#"{"recordId":"r","excludeRelationCategories":["nope"]}"#
    )
    .is_err());
}

/// srs-rust#1229: the `neighbours` binding is the core service; the fixture's single edge
/// OTHER -> RECORD is an `in` edge of RECORD, paged with the full total.
#[test]
fn neighbours_service_pages_with_total() {
    use srs_repository::context_query_service::{
        list_neighbours, EdgeDirection, NeighboursPage, NeighboursQuery,
    };
    let store = fixture_store();
    let q = |direction| NeighboursQuery {
        instance_id: RECORD_ID.to_string(),
        relation_type: None,
        direction,
    };
    let r = list_neighbours(&store, q(None), NeighboursPage::default()).unwrap();
    assert_eq!(r.total, 1);
    assert_eq!(r.neighbours[0].direction, EdgeDirection::In);
    assert_eq!(r.neighbours[0].neighbour.instance_id, OTHER_ID);
    let page = NeighboursPage {
        limit: Some(0),
        offset: 0,
    };
    let empty = list_neighbours(&store, q(Some(EdgeDirection::In)), page).unwrap();
    assert_eq!((empty.total, empty.neighbours.len()), (1, 0));
    let none = list_neighbours(
        &store,
        q(Some(EdgeDirection::Out)),
        NeighboursPage::default(),
    );
    assert_eq!(none.unwrap().total, 0);
}
