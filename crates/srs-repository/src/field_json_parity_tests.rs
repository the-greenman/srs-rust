//! Cross-store FieldJson deserialization parity tests (#450).
//!
//! Proves that the shared [`crate::field_json::FieldJson`] mapper deserializes
//! a Field identically through both the [`FileStore`] and [`JsonStore`] adapters.
//! `MemoryStore` is excluded intentionally: it stores typed `Field` structs in
//! memory and never exercises `FieldJson` parsing, so it cannot diverge on JSON
//! deserialization.

use crate::json_store::JsonStore;
use crate::package_service::create_field;
use crate::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use crate::store::{FileStore, RepositoryStore};
use serde_json::json;
use srs_core::types::field::{Field, ValueType};
use std::collections::HashMap;
use tempfile::TempDir;

fn write_minimal_file_repo(temp: &TempDir) {
    let root = temp.path();
    std::fs::create_dir_all(root.join("package")).unwrap();

    let manifest = json!({
        "instanceIndex": [],
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
        id: "00000000-0000-0000-0000-aabbccddee10".to_string(),
        namespace: "com.test".to_string(),
        name: "parity-field".to_string(),
        version: 1,
        value_type: ValueType::String,
        description: "Parity test field".to_string(),
        instructions: Some("Cross-store parity check.".to_string()),
        ai_guidance: json!(null),
        allowed_values: None,
        vocabulary_ref: None,
        default_value: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        extra: HashMap::from([("x-future-hint".to_string(), json!("preserved"))]),
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

    // --- JsonStore path ---
    let js_tmp = TempDir::new().unwrap();
    let js_path = js_tmp.path().join("repo.srsj");
    let json_store = JsonStore::create(&js_path).unwrap();
    create_repository(
        &json_store,
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
    create_field(&json_store, field.clone()).unwrap();
    drop(json_store);
    let js_store = JsonStore::open(&js_path).unwrap();
    let js_package = js_store.load_package().unwrap();
    let js_field = js_package
        .fields
        .iter()
        .find(|f| f.id == field.id)
        .expect("field must be present in JsonStore after round-trip");

    // --- Assert parity ---
    assert_eq!(fs_field.id, js_field.id);
    assert_eq!(fs_field.namespace, js_field.namespace);
    assert_eq!(fs_field.name, js_field.name);
    assert_eq!(fs_field.version, js_field.version);
    assert_eq!(fs_field.value_type, js_field.value_type);
    assert_eq!(fs_field.description, js_field.description);
    assert_eq!(
        fs_field.instructions, js_field.instructions,
        "instructions must round-trip identically through both adapters"
    );
    assert_eq!(
        fs_field.extra.get("x-future-hint"),
        Some(&json!("preserved")),
        "unknown extra fields must survive the round-trip through FieldJson::into_field"
    );
    assert_eq!(
        js_field.extra.get("x-future-hint"),
        Some(&json!("preserved")),
        "unknown extra fields must survive the round-trip through FieldJson::into_field"
    );
}
