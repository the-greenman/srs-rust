//! Integration test for the WASM `render_document_view` binding.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! test the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.
//!
//! Gallery `.srsj` document views used:
//!   - `5a3ce87e` — decision-deliberation  (markdown format declared)
//!   - `b5c8d124` — decision-log           (markdown format declared)
//!   - `78b11038` — articles-and-roles     (markdown format declared)

use srs_repository::render_service::{render_document_view, RenderDocumentViewOptions};
use srs_repository::JsonStore;

fn gallery_store() -> JsonStore {
    let srsj = include_str!("fixtures/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

/// `format = "json"` populates `projection` and leaves `rendered` as the serialised JSON string.
#[test]
fn render_document_view_json_format_returns_projection() {
    let store = gallery_store();
    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: "5a3ce87e-8340-4d91-a140-ab56b57f704f", // decision-deliberation
        format: Some("json"),
        theme_variant: None,
        container_id: None,
        instance_id_filter: None,
    })
    .expect("render must succeed");

    assert!(
        result.projection.is_some(),
        "json format must populate projection"
    );
    let proj = result.projection.unwrap();
    assert_eq!(
        proj.document_view_id, "5a3ce87e-8340-4d91-a140-ab56b57f704f",
        "projection carries view id"
    );
    assert!(
        !proj.sections.is_empty(),
        "projection must have at least one section"
    );
}

/// `format = "markdown"` renders markdown text and leaves `projection` as `None`.
#[test]
fn render_document_view_markdown_format_no_projection() {
    let store = gallery_store();
    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: "b5c8d124-2084-4a6b-a231-425e800e1e55", // decision-log
        format: Some("markdown"),
        theme_variant: None,
        container_id: None,
        instance_id_filter: None,
    })
    .expect("render must succeed");

    assert!(
        result.projection.is_none(),
        "markdown format must not set projection"
    );
    assert!(
        !result.rendered.is_empty(),
        "markdown format must produce rendered output"
    );
}

/// A `container-subset` section whose container includes a Tier-0 note as a member must not
/// crash with `missing field 'typeId'`. The note is skipped with a `[container-subset]`
/// diagnostic; the Tier-2 record sibling is still rendered.
#[test]
fn render_document_view_container_subset_skips_tier0_note() {
    const NOTE_ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const RECORD_ID: &str = "bbbbbbbb-0000-4000-8000-000000000001";
    const CONTAINER_ID: &str = "ffffffff-0000-4000-8000-000000000001";
    const VIEW_ID: &str = "eeeeeeee-0000-4000-8000-000000000001";

    let fixture = r#"{
  "srsj": "1",
  "manifest": {
    "instanceIndex": [
      {
        "instanceId": "aaaaaaaa-0000-4000-8000-000000000001",
        "path": "notes/aaaaaaaa-0000-4000-8000-000000000001.json",
        "tier": 0,
        "title": "Identity Note",
        "tags": []
      },
      {
        "instanceId": "bbbbbbbb-0000-4000-8000-000000000001",
        "path": "records/tier-2/bbbbbbbb-0000-4000-8000-000000000001.json",
        "tier": 2
      }
    ],
    "namespace": "com.test.example",
    "repositoryId": "00000000-0000-4000-8000-000000000001",
    "srsVersion": "2.0"
  },
  "data": {
    "package/package.json": {
      "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
      "id": "cccccccc-0000-4000-8000-000000000001",
      "name": "test-package",
      "namespace": "com.test",
      "version": "1.0.0",
      "title": "Test Package",
      "description": "",
      "status": "active",
      "createdAt": "2026-01-01T00:00:00Z",
      "blueprints": [],
      "fields": [],
      "types": ["types/test-type.json"],
      "relationTypes": [],
      "views": [],
      "documentViews": ["document-views/test-dv.json"]
    },
    "package/types/test-type.json": {
      "id": "dddddddd-0000-4000-8000-000000000001",
      "name": "test-record",
      "namespace": "com.test",
      "version": 1,
      "fields": [],
      "createdAt": "2026-01-01T00:00:00Z"
    },
    "package/document-views/test-dv.json": {
      "id": "eeeeeeee-0000-4000-8000-000000000001",
      "name": "test-dv",
      "namespace": "com.test",
      "version": 1,
      "format": "json",
      "description": "Test document view",
      "createdAt": "2026-01-01T00:00:00Z",
      "sections": [
        {
          "sectionId": "test-section",
          "title": "Members",
          "order": 0,
          "source": {
            "containerId": "ffffffff-0000-4000-8000-000000000001",
            "type": "container-subset"
          }
        }
      ]
    },
    "notes/aaaaaaaa-0000-4000-8000-000000000001.json": {
      "instanceId": "aaaaaaaa-0000-4000-8000-000000000001",
      "title": "Identity Note",
      "sections": []
    },
    "records/tier-2/bbbbbbbb-0000-4000-8000-000000000001.json": {
      "instanceId": "bbbbbbbb-0000-4000-8000-000000000001",
      "typeId": "dddddddd-0000-4000-8000-000000000001",
      "typeName": "test-record",
      "typeNamespace": "com.test",
      "typeVersion": 1,
      "fieldValues": [],
      "createdAt": "2026-01-01T00:00:00Z"
    },
    "containers/ffffffff-0000-4000-8000-000000000001.json": {
      "containerId": "ffffffff-0000-4000-8000-000000000001",
      "containerType": "document",
      "title": "Test Container",
      "createdAt": "2026-01-01T00:00:00Z",
      "memberInstanceIds": [
        "aaaaaaaa-0000-4000-8000-000000000001",
        "bbbbbbbb-0000-4000-8000-000000000001"
      ]
    }
  }
}"#;

    let store = JsonStore::from_srsj(fixture).expect("fixture must load");

    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: VIEW_ID,
        format: Some("json"),
        theme_variant: None,
        container_id: None,
        instance_id_filter: None,
    })
    .expect("render must succeed even when container has a Tier-0 note member");

    let container_subset_diags: Vec<&String> = result
        .diagnostics
        .iter()
        .filter(|d| d.starts_with("[container-subset]"))
        .collect();
    assert_eq!(
        container_subset_diags.len(),
        1,
        "expected exactly 1 [container-subset] diagnostic, got: {:?}",
        result.diagnostics
    );
    assert!(
        container_subset_diags[0].contains(NOTE_ID),
        "diagnostic must reference the note id"
    );
    assert!(
        container_subset_diags[0].contains(CONTAINER_ID),
        "diagnostic must reference the container id"
    );

    let proj = result.projection.expect("json format must produce projection");
    let all_ids: Vec<&str> = proj
        .sections
        .iter()
        .flat_map(|s| s.records.iter())
        .map(|r| r.instance_id.as_str())
        .collect();
    assert!(!all_ids.contains(&NOTE_ID), "note must not appear in projection");
    assert!(all_ids.contains(&RECORD_ID), "Tier-2 record must still render");
}

/// Unknown view ID returns an error.
#[test]
fn render_document_view_unknown_view_errors() {
    let store = gallery_store();
    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: "00000000-0000-0000-0000-000000000000",
        format: Some("json"),
        theme_variant: None,
        container_id: None,
        instance_id_filter: None,
    });
    assert!(result.is_err(), "unknown view id must return Err");
}
