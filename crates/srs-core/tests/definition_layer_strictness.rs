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
