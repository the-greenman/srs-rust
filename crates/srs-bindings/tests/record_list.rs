//! Integration test for the WASM `list_records` binding (issue #293).
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests
//! (see `container_view.rs`): exercise the underlying service directly via
//! `JsonStore::from_srsj`, since the binding's `to_js()` calls `js_sys::JSON::parse` which
//! panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]` export compiles.

use srs_repository::record_store::{list_record_summaries, RecordListFilter};
use srs_repository::JsonStore;

const TYPE_ID: &str = "11111111-1111-4111-8111-111111111111";
const REC_TITLED: &str = "33333333-3333-4333-8333-333333333333";
const REC_PLAIN: &str = "55555555-5555-4555-8555-555555555555";
const FIELD_TITLE: &str = "77777777-7777-4777-8777-777777777777";
const FIELD_SUMMARY: &str = "88888888-8888-4888-8888-888888888888";

/// Minimal `.srsj`: two Tier-2 records of the same type — one with a `title` field set,
/// one with only a non-label `summary` field (exercising the `type_name` fallback).
fn fixture_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-record-list",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": REC_TITLED, "path": format!("records/tier-2/{REC_TITLED}.json"), "tier": 2},
                {"instanceId": REC_PLAIN, "path": format!("records/tier-2/{REC_PLAIN}.json"), "tier": 2}
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-record-list-001",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": ["fields/title.json", "fields/summary.json"],
                "types": [],
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
                "description": "Title",
                "aiGuidance": {},
                "valueType": "string",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "package/fields/summary.json": {
                "id": FIELD_SUMMARY,
                "namespace": "com.test",
                "name": "summary",
                "version": 1,
                "description": "Summary",
                "aiGuidance": {},
                "valueType": "string",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "records/tier-2/33333333-3333-4333-8333-333333333333.json": {
                "instanceId": REC_TITLED,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": [{"fieldId": FIELD_TITLE, "value": "Building Authority"}]
            },
            "records/tier-2/55555555-5555-4555-8555-555555555555.json": {
                "instanceId": REC_PLAIN,
                "typeId": TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": [{"fieldId": FIELD_SUMMARY, "value": "no title here"}]
            }
        }
    })
    .to_string()
}

/// The path `list_records` exposes: each record carries a core-resolved `displayLabel`
/// (title when present, else the type_name fallback), with the full `Record` nested.
#[test]
fn list_record_summaries_carries_core_display_label() {
    let store = JsonStore::from_srsj(&fixture_srsj()).expect("fixture srsj must load");
    let summaries =
        list_record_summaries(&store, RecordListFilter::default()).expect("list must succeed");

    assert_eq!(summaries.len(), 2);

    let titled = summaries
        .iter()
        .find(|s| s.instance_id == REC_TITLED)
        .expect("titled record present");
    assert_eq!(titled.display_label, "Building Authority");
    assert_eq!(titled.record.instance_id, REC_TITLED);

    let plain = summaries
        .iter()
        .find(|s| s.instance_id == REC_PLAIN)
        .expect("plain record present");
    // No title/name/label field → falls back to the record's type_name.
    assert_eq!(plain.display_label, "decision");

    // Serialises to the { instanceId, displayLabel, record } shape the binding hands to JS.
    let json = serde_json::to_value(&summaries).expect("serialise");
    let first = &json.as_array().unwrap()[0];
    assert!(first["instanceId"].is_string());
    assert!(first["displayLabel"].is_string());
    assert!(first["record"]["typeName"].is_string());
}
