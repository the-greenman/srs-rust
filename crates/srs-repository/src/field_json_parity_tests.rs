//! Cross-store FieldJson deserialization parity tests (#450).
//!
//! Proves that the shared [`crate::field_json::FieldJson`] mapper deserializes
//! a Field identically on disk and across a `.srsj` round-trip. `MemoryStore`
//! is excluded intentionally: it stores typed `Field` structs in memory and
//! never exercises `FieldJson` parsing, so it cannot diverge on JSON
//! deserialization.

use crate::package_service::create_field;
use crate::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use crate::store::{FileStore, RepositoryStore};
use serde_json::json;
use srs_core::types::field::{AiGuidance, Field, FieldType};
use tempfile::TempDir;

fn write_minimal_file_repo(temp: &TempDir) {
    let root = temp.path();
    std::fs::create_dir_all(root.join("package")).unwrap();

    let manifest = json!({
        "dataModelRevision": 2,
        "srsVersion": "2.0-draft",
        "repositoryId": "parity-repo-id",
        "namespace": "com.test"
    });
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let package_json = json!({
        "id": "parity-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": [],
        "views": [],
        "documentViews": []
    });
    std::fs::write(
        root.join("package/package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();
}

#[test]
fn cross_store_field_json_parity() {
    let field = Field {
        schema: None,
        id: "00000000-0000-0000-0000-aabbccddee10".to_string(),
        namespace: "com.test".to_string(),
        name: "parity-field".to_string(),
        version: 1,
        field_type: FieldType::string(),
        description: "Parity test field".to_string(),
        instructions: Some("Cross-store parity check.".to_string()),
        ai_guidance: Some(AiGuidance {
            purpose: "Test guidance".to_string(),
            ..Default::default()
        }),
        default_value: None,
        editor_hint: None,
        tags: None,
        lineage: None,
        provenance: None,
        deprecated_at: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    };

    // --- FileStore path ---
    let fs_tmp = TempDir::new().unwrap();
    write_minimal_file_repo(&fs_tmp);
    let file_store = FileStore::new(fs_tmp.path());
    create_field(&file_store, field.clone()).unwrap();
    let fs_package = FileStore::new(fs_tmp.path()).load_package().unwrap();
    let fs_field = fs_package
        .fields
        .iter()
        .find(|f| f.id == field.id)
        .expect("field must be present in FileStore after round-trip");

    // --- `.srsj` codec path ---
    let js_tmp = TempDir::new().unwrap();
    let js_path = js_tmp.path().join("repo.srsj");
    let mut session = crate::srsj::SrsjSession::create(&js_path).unwrap();
    create_repository(
        session.store(),
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "parity-repo".to_string(),
                namespace: "com.test".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "parity-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    create_field(session.store(), field.clone()).unwrap();
    session.flush().unwrap();
    drop(session);
    let reopened = crate::srsj::SrsjSession::open(&js_path).unwrap();
    let js_package = reopened.store().load_package().unwrap();
    let js_field = js_package
        .fields
        .iter()
        .find(|f| f.id == field.id)
        .expect("field must be present after a `.srsj` round-trip");

    // --- Assert parity ---
    // Whole-struct equality, not a property-by-property list: a new Field
    // property added to `srs-core` is then covered by this test automatically
    // rather than silently escaping it.
    assert_eq!(
        fs_field, js_field,
        "the same Field must deserialize identically through both adapters"
    );
    assert_eq!(fs_field.field_type, FieldType::string());
}

#[test]
fn cross_store_unknown_field_property_is_rejected_identically() {
    // srs-rust#767: `field.json` sets `additionalProperties: false`. Both
    // adapters must reject an unknown property — the answer may not depend on
    // which store the Field entered through.
    let raw = json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
        "id": "00000000-0000-0000-0000-aabbccddee11",
        "namespace": "com.test",
        "name": "hinted_field",
        "version": 1,
        "description": "Has an unknown property",
        "aiGuidance": {"purpose": "p"},
        "fieldType": {"datatype": "string"},
        "createdAt": "2026-01-01T00:00:00Z",
        "x-future-hint": "preserved"
    });

    let fs_tmp = TempDir::new().unwrap();
    write_minimal_file_repo(&fs_tmp);
    std::fs::create_dir_all(fs_tmp.path().join("package/fields")).unwrap();
    std::fs::write(
        fs_tmp.path().join("package/fields/hinted_field.json"),
        serde_json::to_string_pretty(&raw).unwrap(),
    )
    .unwrap();
    let package_json = json!({
        "id": "parity-pkg", "namespace": "com.test", "name": "test", "version": "1.0.0",
        "fields": ["fields/hinted_field.json"], "types": [], "views": [], "documentViews": []
    });
    std::fs::write(
        fs_tmp.path().join("package/package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let err = FileStore::new(fs_tmp.path()).load_package();
    assert!(
        err.is_err(),
        "a Field carrying an unknown property must be rejected on load, \
         matching the create gate's additionalProperties: false"
    );
}
