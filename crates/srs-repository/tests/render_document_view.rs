//! Service-layer tests for `render_document_view` in `srs-repository`.
//!
//! These tests exercise `render_service::render_document_view` directly via `JsonStore::from_srsj`
//! with inline fixtures, keeping the test close to the service without going through the
//! WASM binding surface. The matching binding-surface test lives in `crates/srs-bindings/tests/`.

use srs_repository::render_service::{render_document_view, RenderDocumentViewOptions};
use srs_repository::JsonStore;

const NOTE_ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const RECORD_ID: &str = "bbbbbbbb-0000-4000-8000-000000000001";
const CONTAINER_ID: &str = "ffffffff-0000-4000-8000-000000000001";
const VIEW_ID: &str = "eeeeeeee-0000-4000-8000-000000000001";
const TYPE_ID: &str = "dddddddd-0000-4000-8000-000000000001";

/// A minimal `.srsj` that has one Tier-0 note and one Tier-2 record in the same container,
/// with a `container-subset` document-view section pointing at that container.
fn tier0_note_container_srsj() -> &'static str {
    r#"{
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
}"#
}

/// A `container-subset` section whose container has a Tier-0 note as a member must:
/// 1. Return `Ok` (not crash with `missing field 'typeId'`).
/// 2. Emit exactly one `[container-subset]` diagnostic for the skipped note.
/// 3. Render the Tier-2 record member in the projection and exclude the note.
#[test]
fn render_document_view_container_subset_skips_tier0_note() {
    let store = JsonStore::from_srsj(tier0_note_container_srsj()).expect("fixture must load");

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
        "expected exactly 1 [container-subset] diagnostic for the skipped note, got: {:?}",
        result.diagnostics
    );
    assert!(
        container_subset_diags[0].contains(NOTE_ID),
        "diagnostic must reference the note instance id; got: {}",
        container_subset_diags[0]
    );
    assert!(
        container_subset_diags[0].contains(CONTAINER_ID),
        "diagnostic must reference the container id; got: {}",
        container_subset_diags[0]
    );

    let proj = result
        .projection
        .expect("json format must produce a projection");

    // The note must NOT appear in the projection.
    let all_record_ids: Vec<&str> = proj
        .sections
        .iter()
        .flat_map(|s| s.records.iter())
        .map(|r| r.instance_id.as_str())
        .collect();
    assert!(
        !all_record_ids.contains(&NOTE_ID),
        "note must be excluded from rendered projection"
    );

    // The Tier-2 record MUST appear in the projection.
    assert!(
        all_record_ids.contains(&RECORD_ID),
        "Tier-2 record must still be rendered alongside the skipped note"
    );
}
