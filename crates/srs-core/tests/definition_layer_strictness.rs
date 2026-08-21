//! srs-rust#863 / decision `rfc-decision-2e0cd70a`: the definition layer is the
//! trust boundary and **rejects** unknown keys, matching every definition
//! schema's `additionalProperties: false`. Instance-layer tolerance is the
//! other half of the same ruling and is deliberately not exercised here.
//!
//! One case per definition kind, each asserting the rejection *names the key* —
//! a rejection nobody can act on is not much better than a silent one.

use serde::de::DeserializeOwned;
use srs_core::types::{
    blueprint::Blueprint, lifecycle::Lifecycle, protocol::Protocol,
    relation_type_definition::RelationTypeDefinition, theme::Theme, view::DocumentView, view::View,
};

fn rejects_unknown_key<T: DeserializeOwned>(kind: &str, mut doc: serde_json::Value) {
    // Sanity: the minimal document must itself load, or the test proves nothing.
    serde_json::from_value::<T>(doc.clone())
        .unwrap_or_else(|e| panic!("{kind}: minimal definition must load, got {e}"));

    doc.as_object_mut()
        .unwrap()
        .insert("xUnknown".to_string(), serde_json::json!("nope"));
    let err = serde_json::from_value::<T>(doc)
        .err()
        .unwrap_or_else(|| panic!("{kind}: unknown key must be rejected"));
    assert!(
        err.to_string().contains("xUnknown"),
        "{kind}: rejection must name the key, got: {err}"
    );
}

#[test]
fn definition_types_reject_unknown_keys() {
    rejects_unknown_key::<View>(
        "View",
        serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000001",
            "namespace": "com.test", "name": "v", "version": 1,
            "description": "d", "fieldViews": [],
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    rejects_unknown_key::<DocumentView>(
        "DocumentView",
        serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000002",
            "namespace": "com.test", "name": "dv", "version": 1,
            "description": "d", "sections": [],
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    rejects_unknown_key::<Theme>(
        "Theme",
        serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000003",
            "namespace": "com.test", "name": "t", "version": 1,
            "description": "d", "targets": ["markdown"],
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    rejects_unknown_key::<Lifecycle>(
        "Lifecycle",
        serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000004",
            "namespace": "com.test", "name": "lc", "version": 1,
            "states": [], "transitions": [], "initialState": "draft",
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    rejects_unknown_key::<RelationTypeDefinition>(
        "RelationTypeDefinition",
        serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000005",
            "version": 1, "key": "contains", "namespace": "com.test",
            "label": "Contains", "description": "d", "category": "composition",
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    rejects_unknown_key::<Blueprint>(
        "Blueprint",
        serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000006",
            "namespace": "com.test", "name": "bp", "version": 1,
            "description": "d", "rootTypes": [],
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    rejects_unknown_key::<Protocol>(
        "Protocol",
        serde_json::json!({
            "protocolId": "00000000-0000-4000-8000-000000000007",
            "protocolNamespace": "com.test", "protocolName": "p",
            "protocolVersion": 1, "protocolTargetType": "com.test/x",
            "protocolStages": [],
            "protocolCreatedAt": "2026-01-01T00:00:00Z"
        }),
    );
}

/// Strictness must not reject a key the schema declares but the engine does
/// not yet act on. `DocumentSection.ordering.memberOrder` (RFC-015 [N+29]) is
/// the case that nearly slipped through: no first-party corpus uses it, so a
/// corpus-only safety gate would have stayed green while a schema-valid
/// DocumentView became unloadable.
#[test]
fn document_view_accepts_schema_declared_but_unconsumed_keys() {
    let dv: DocumentView = serde_json::from_value(serde_json::json!({
        "id": "00000000-0000-4000-8000-000000000002",
        "namespace": "com.test", "name": "dv", "version": 1,
        "description": "d",
        "sections": [{
            "sectionId": "s1",
            "order": 0,
            "source": {"type": "container-subset", "containerId": "c1"},
            "ordering": {"memberOrder": [
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222"
            ]}
        }],
        "createdAt": "2026-01-01T00:00:00Z"
    }))
    .expect("memberOrder is declared by document-view.json");
    let ordering = dv.sections[0].ordering.as_ref().expect("ordering");
    assert_eq!(ordering.member_order.as_ref().map(Vec::len), Some(2));
    // Carried back out — an unconsumed key must not be a silently dropped one.
    let back = serde_json::to_value(&dv).unwrap();
    assert_eq!(
        back["sections"][0]["ordering"]["memberOrder"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

/// The `$schema` pointer every definition schema declares is a *known* key —
/// strictness must not turn the pointer the files actually carry into a
/// rejection.
#[test]
fn definition_types_accept_the_schema_pointer() {
    let view: View = serde_json::from_value(serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/view.json",
        "id": "00000000-0000-4000-8000-000000000001",
        "namespace": "com.test", "name": "v", "version": 1,
        "description": "d", "fieldViews": [],
        "createdAt": "2026-01-01T00:00:00Z"
    }))
    .expect("$schema is declared by view.json");
    assert_eq!(
        view.schema.as_deref(),
        Some("https://srs.semanticops.com/schema/2.0/view.json")
    );
    // …and it survives back out, so a load → save round trip is lossless.
    let back = serde_json::to_value(&view).unwrap();
    assert_eq!(
        back["$schema"],
        "https://srs.semanticops.com/schema/2.0/view.json"
    );
}

/// Every newly modelled key is carried *back out*. A key that loads and then
/// vanishes on write is the silent loss `rfc-decision-2e0cd70a` forbids, and it
/// is indistinguishable from strictness working until someone diffs a file.
#[test]
fn newly_modelled_keys_survive_a_round_trip() {
    let view_json = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/view.json",
        "id": "00000000-0000-4000-8000-000000000001",
        "namespace": "com.test", "name": "v", "version": 1,
        "description": "d",
        "aiGuidance": {"purpose": "p"},
        "lineage": {"derivedFrom": "x"},
        "provenance": {"author": "a"},
        "updatedAt": "2026-02-02T00:00:00Z",
        "fieldViews": [{
            "fieldId": "00000000-0000-4000-8000-0000000000f1",
            "order": 0,
            "displayHint": "block",
            "editorHintOverride": {"kind": "textarea"}
        }],
        "createdAt": "2026-01-01T00:00:00Z"
    });
    let view: View = serde_json::from_value(view_json.clone()).expect("loads");
    let back = serde_json::to_value(&view).unwrap();
    for key in [
        "$schema",
        "aiGuidance",
        "lineage",
        "provenance",
        "updatedAt",
    ] {
        assert_eq!(back[key], view_json[key], "{key} must survive");
    }
    for key in ["displayHint", "editorHintOverride"] {
        assert_eq!(
            back["fieldViews"][0][key], view_json["fieldViews"][0][key],
            "fieldViews[0].{key} must survive"
        );
    }
}
