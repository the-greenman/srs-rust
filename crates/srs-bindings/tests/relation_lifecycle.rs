// Native Rust integration tests for the relation and lifecycle bindings.
//
// These tests run under the native target (not wasm32). Because wasm-bindgen's
// `JsValue` requires the WASM runtime — `JsValue::from_str` and `js_sys::JSON::parse`
// both abort outside of a browser/WASM context — we test the repository service layer
// directly via `JsonStore` and the service functions that the WASM wrapper delegates to.
// This exercises exactly the same code paths as the WASM methods.

use srs_repository::record_store::{self, TransitionLifecycleInput};
use srs_repository::relation_service::{self, ListRelationsFilter};
use srs_repository::JsonStore;

const GALLERY_SRSJ: &str = include_str!("../../../../srs/docs/spec/examples/gallery.srsj");

// Two tier-2 instance IDs present in gallery.srsj (no existing "evidences" relation between them)
const GALLERY_SRC: &str = "ad159754-2edd-4bf8-a70f-a29a617e5809";
const GALLERY_TGT: &str = "31291422-cd8b-4840-b884-d55023d938cb";
// Relation type declared in gallery's package
const GALLERY_REL_TYPE: &str = "evidences";

// ---------------------------------------------------------------------------
// Helper: build a minimal srsj fixture that has a lifecycle-enabled type and
// one record in the "draft" initial state.
// ---------------------------------------------------------------------------
fn lifecycle_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-lc-repo",
            "srsVersion": "2.0-draft",
            "namespace": "com.test.lc",
            "instanceIndex": [
                {
                    "instanceId": "rec-lc-001",
                    "tier": 2,
                    "path": "records/tier-2/rec-lc-001.json"
                }
            ],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-lc-001",
                "namespace": "com.test.lc",
                "name": "lc-package",
                "version": "1.0.0",
                "fields": ["fields/title-lc.json"],
                "types": ["types/proposal.json"],
                "relationTypes": [],
                "views": [],
                "documentViews": []
            },
            "package/fields/title-lc.json": {
                "id": "field-title-lc",
                "namespace": "com.test.lc",
                "name": "title",
                "version": 1,
                "valueType": "string"
            },
            "package/types/proposal.json": {
                "id": "type-proposal-001",
                "namespace": "com.test.lc",
                "name": "proposal",
                "version": 1,
                "description": "A proposal with lifecycle",
                "fields": [
                    {
                        "fieldId": "field-title-lc",
                        "order": 1,
                        "required": true
                    }
                ],
                "lifecycle": {
                    "initialState": "draft",
                    "states": [
                        { "key": "draft" },
                        { "key": "active" },
                        { "key": "archived", "isFinal": true }
                    ],
                    "transitions": [
                        { "name": "promote", "from": "draft", "to": "active" },
                        { "name": "archive", "from": "active", "to": "archived" }
                    ]
                },
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "records/tier-2/rec-lc-001.json": {
                "instanceId": "rec-lc-001",
                "typeId": "type-proposal-001",
                "typeName": "proposal",
                "typeNamespace": "com.test.lc",
                "typeVersion": 1,
                "lifecycleState": "draft",
                "fieldValues": [
                    {
                        "fieldId": "field-title-lc",
                        "value": "My Proposal"
                    }
                ],
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }
        }
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// 1. list_relations with empty filter — assert it does not error and returns
//    the expected relations from gallery.
// ---------------------------------------------------------------------------
#[test]
fn list_relations_empty_filter_succeeds() {
    let store = JsonStore::from_srsj(GALLERY_SRSJ).expect("gallery must load");
    let summaries = relation_service::list_relations(&store, ListRelationsFilter::default())
        .expect("list_relations with empty filter must not error");
    // gallery.srsj has 15 relations
    assert_eq!(summaries.len(), 15, "gallery should have 15 relations");
    // Verify relation types include the expected types
    assert!(
        summaries.iter().any(|r| r.relation_type == "precedes"),
        "gallery must contain precedes relations"
    );
}

// ---------------------------------------------------------------------------
// 2. create_relation between two real gallery instance IDs, then list by
//    source — confirm the new relation appears.
// ---------------------------------------------------------------------------
#[test]
fn create_relation_appears_in_list_by_source() {
    use srs_core::types::relation::Relation;

    let store = JsonStore::from_srsj(GALLERY_SRSJ).expect("gallery must load");

    let relation = Relation {
        relation_id: "test-rel-bindings-001".to_string(),
        relation_type: GALLERY_REL_TYPE.to_string(),
        source_instance_id: GALLERY_SRC.to_string(),
        target_instance_id: GALLERY_TGT.to_string(),
        asserted_by: None,
        confidence: None,
        created_at: Some("2026-06-07T00:00:00Z".to_string()),
        created_by: None,
        status: None,
        valid_from: None,
        valid_until: None,
        notes: None,
        source_refs: None,
        meta: None,
        source_repository_id: None,
        target_repository_id: None,
    };

    let result = relation_service::create_relation_auto(&store, relation)
        .expect("create_relation should succeed");
    assert_eq!(result.relation.relation_id, "test-rel-bindings-001");

    // List by source — new relation must appear.
    let filter = ListRelationsFilter {
        source: Some(GALLERY_SRC.to_string()),
        target: None,
        relation_type: None,
        container_id: None,
    };
    let summaries = relation_service::list_relations(&store, filter)
        .expect("list_relations with source filter should succeed");
    assert!(
        summaries
            .iter()
            .any(|r| r.relation_id == "test-rel-bindings-001"),
        "newly created relation must appear in filtered list"
    );
}

// ---------------------------------------------------------------------------
// 3. delete_relation — delete the one just created, confirm gone.
// ---------------------------------------------------------------------------
#[test]
fn delete_relation_removes_it() {
    use srs_core::types::relation::Relation;

    let store = JsonStore::from_srsj(GALLERY_SRSJ).expect("gallery must load");

    // Create a relation to delete.
    let relation = Relation {
        relation_id: "test-rel-delete-002".to_string(),
        relation_type: GALLERY_REL_TYPE.to_string(),
        source_instance_id: GALLERY_SRC.to_string(),
        target_instance_id: GALLERY_TGT.to_string(),
        asserted_by: None,
        confidence: None,
        created_at: Some("2026-06-07T00:00:00Z".to_string()),
        created_by: None,
        status: None,
        valid_from: None,
        valid_until: None,
        notes: None,
        source_refs: None,
        meta: None,
        source_repository_id: None,
        target_repository_id: None,
    };
    relation_service::create_relation_auto(&store, relation)
        .expect("create_relation should succeed before delete");

    // Delete it.
    let del_result = relation_service::delete_relation(&store, "test-rel-delete-002")
        .expect("delete_relation should succeed");
    assert_eq!(del_result.relation_id, "test-rel-delete-002");

    // Listing all relations must not contain this id.
    let summaries = relation_service::list_relations(&store, ListRelationsFilter::default())
        .expect("list_relations after delete should succeed");
    assert!(
        !summaries
            .iter()
            .any(|r| r.relation_id == "test-rel-delete-002"),
        "deleted relation must not appear in list"
    );
}

// ---------------------------------------------------------------------------
// 4. set_lifecycle_state transitions a record through its lifecycle.
// ---------------------------------------------------------------------------
#[test]
fn set_lifecycle_state_transitions_record() {
    let store = JsonStore::from_srsj(&lifecycle_srsj()).expect("lifecycle fixture must load");

    // draft → active
    let result = record_store::transition_record_lifecycle(
        &store,
        "rec-lc-001",
        TransitionLifecycleInput {
            to: Some("active".to_string()),
            by_transition: None,
        },
    )
    .expect("draft→active should succeed");

    assert_eq!(
        result.record.lifecycle_state.as_deref(),
        Some("active"),
        "record must be in 'active' state after transition"
    );
    assert!(
        result.warnings.is_empty(),
        "no warnings expected for non-final transition"
    );
}

// ---------------------------------------------------------------------------
// 4b. set_lifecycle_state: full chain draft → active → archived (final state).
// ---------------------------------------------------------------------------
#[test]
fn set_lifecycle_state_full_chain_to_final() {
    let store = JsonStore::from_srsj(&lifecycle_srsj()).expect("lifecycle fixture must load");

    // draft → active
    record_store::transition_record_lifecycle(
        &store,
        "rec-lc-001",
        TransitionLifecycleInput {
            to: Some("active".to_string()),
            by_transition: None,
        },
    )
    .expect("draft→active should succeed");

    // active → archived (final state — should succeed with a warning)
    let result = record_store::transition_record_lifecycle(
        &store,
        "rec-lc-001",
        TransitionLifecycleInput {
            to: Some("archived".to_string()),
            by_transition: None,
        },
    )
    .expect("active→archived should succeed");

    assert_eq!(
        result.record.lifecycle_state.as_deref(),
        Some("archived"),
        "record must be in 'archived' state after final transition"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("LIFECYCLE_FINAL_STATE")),
        "final-state transition must emit LIFECYCLE_FINAL_STATE warning"
    );
}
