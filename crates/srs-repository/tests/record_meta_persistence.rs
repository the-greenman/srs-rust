//! srs-rust#1041 (follow-up to #1034/#1031): `update_record`'s `extra`/`meta`
//! merge is proved only against `MemoryStore` in `record_store.rs`'s unit
//! tests. The bug it fixed (`UpdateRecordInput` had no `extra` field to
//! flatten into, so `meta` was silently dropped by serde) is exactly the
//! class of serialization interaction — key collision, actual JSON-on-disk
//! round trip — a `MemoryStore`-only test cannot catch. CLAUDE.md's Storage
//! Boundary Rules require a cross-store roundtrip test for new service
//! behaviour; this is the `FileStore` half.
//!
//! The store is dropped and re-opened from the same directory before the
//! final read, so the assertion exercises an actual disk round trip rather
//! than a value the store instance merely still holds in memory.

use srs_core::types::field::{AiGuidance, Field, FieldType};
use srs_core::types::record::FieldValues;
use srs_core::types::record_type::{FieldAssignment, RecordType};
use srs_repository::{
    package_service,
    record_store::{self, UpdateRecordInput},
    repository_lifecycle::{
        create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    },
    FileStore,
};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn init_input() -> InitializeRepositoryInput {
    InitializeRepositoryInput {
        repository: RepositoryMetadata {
            repository_id: "1041aaaa-0000-4000-8000-000000000001".to_string(),
            namespace: "com.test.metapersist".to_string(),
            srs_version: "2.0-draft".to_string(),
            title: None,
            description: None,
        },
        primary_package: PrimaryPackageMetadata {
            id: "1041bbbb-0000-4000-8000-000000000002".to_string(),
            namespace: "com.test.metapersist".to_string(),
            name: "primary".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn name_field() -> Field {
    Field {
        schema: None,
        id: "1041field-name-0001".to_string(),
        namespace: "com.test.metapersist".to_string(),
        name: "test-name".to_string(),
        version: 1,
        field_type: FieldType::string(),
        description: "Name field".to_string(),
        instructions: None,
        ai_guidance: Some(AiGuidance {
            purpose: "Test guidance".to_string(),
            ..Default::default()
        }),
        editor_hint: None,
        tags: None,
        lineage: None,
        provenance: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn test_type() -> RecordType {
    RecordType {
        schema: None,
        ai_guidance: None,
        tags: None,
        id: "1041type-test-0001".to_string(),
        namespace: "com.test.metapersist".to_string(),
        name: "test-type".to_string(),
        version: 1,
        description: "Test type".to_string(),
        fields: vec![FieldAssignment {
            field_id: "1041field-name-0001".to_string(),
            order: 0,
            required: true,
            display_label: Some("Name".to_string()),
            description: None,
        }],
        extends_type_id: None,
        extends_type_version: None,
        field_order: None,
        field_assignment_overrides: None,
        identity_field_id: None,
        lifecycle: None,
        lifecycle_ref: None,
        validation_rules: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        lineage: None,
        provenance: None,
    }
}

/// `record update`'s `extra`/`meta` merge (srs-rust#1031/#1034) must survive
/// an actual `FileStore` round trip, not just a `MemoryStore` one.
#[test]
fn file_store_update_record_persists_meta_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    create_repository(&store, &init_input()).unwrap();
    package_service::create_field(&store, name_field()).unwrap();
    package_service::create_type(&store, test_type()).unwrap();

    let mut fv = FieldValues::new();
    fv.insert("test-name", serde_json::json!("Initial"));
    let record = record_store::create_record(&store, "1041type-test-0001", 1, fv, None, None)
        .expect("create");
    let id = record.instance_id.clone();

    let mut extra = BTreeMap::new();
    extra.insert(
        "meta".to_string(),
        serde_json::json!({"derivedFrom": "predecessor-id"}),
    );
    let mut fv = FieldValues::new();
    fv.insert("test-name", serde_json::json!("Initial"));
    record_store::update_record(
        &store,
        &id,
        UpdateRecordInput {
            field_values: fv,
            field_meta: None,
            tags: None,
            type_version: None,
            extra,
        },
    )
    .expect("update");

    // Drop and re-open the store from the same directory: the assertion below
    // must read what actually landed on disk, not a value the original
    // `FileStore` instance still holds some other way.
    drop(store);
    let reopened = FileStore::new(tmp.path());
    let loaded = record_store::get_record_by_id(&reopened, &id)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.extra.get("meta"),
        Some(&serde_json::json!({"derivedFrom": "predecessor-id"})),
        "meta must survive an update_record round trip through actual JSON-on-disk storage"
    );
}
