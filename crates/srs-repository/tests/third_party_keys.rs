//! srs-rust#1635 (srs-programme decision ba478783, RFC-038 Revision 18 [R7]/[R24]): an
//! unrecognised top-level key on a Record, Note or Relation file is a non-fatal warning,
//! the object loads, and every write carries the key. A missing required property or a
//! wrong value of a known key stays an error.

use srs_repository::catalog::{self, codes};
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::RepositoryStore;
use srs_repository::validation::DiagnosticSeverity;
use srs_repository::FileStore;
use tempfile::TempDir;

const NOTE_ID: &str = "16350000-0000-4000-8000-000000000001";
const REL_ID: &str = "16350000-0000-4000-8000-0000000000a1";
const OTHER_ID: &str = "16350000-0000-4000-8000-000000000002";

fn init() -> (TempDir, FileStore) {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "16350000-0000-4000-8000-0000000000f0".to_string(),
                namespace: "com.test.thirdparty".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "16350000-0000-4000-8000-0000000000f1".to_string(),
                namespace: "com.test.thirdparty".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    (tmp, store)
}

fn note_json(id: &str, extra: bool) -> serde_json::Value {
    let mut v = serde_json::json!({
        "$schema": srs_schema::NOTE_SCHEMA_ID,
        "instanceId": id,
        "sections": [{"name": "body", "content": "hello"}]
    });
    if extra {
        v["x-thirdParty"] = serde_json::json!(1);
    }
    v
}

fn write(tmp: &TempDir, rel: &str, v: &serde_json::Value) {
    let p = tmp.path().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_string_pretty(v).unwrap()).unwrap();
}

fn warnings(store: &FileStore) -> Vec<catalog::CatalogDiagnostic> {
    catalog::build(store)
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|d| d.code == codes::UNKNOWN_INSTANCE_KEY)
        .collect()
}

#[test]
fn note_with_unknown_key_loads_with_one_named_warning_and_round_trips() {
    let (tmp, store) = init();
    write(
        &tmp,
        &format!("records/notes/{NOTE_ID}.json"),
        &note_json(NOTE_ID, true),
    );
    let cat = catalog::build_checked(&store).expect("an unknown key must not be fatal");
    assert!(cat.instances.iter().any(|e| e.id == NOTE_ID));
    let w = warnings(&store);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].severity, DiagnosticSeverity::Warning);
    assert!(w[0].message.contains("x-thirdParty"), "{}", w[0].message);

    let mut note = store.load_note_by_id(NOTE_ID).unwrap();
    assert_eq!(note.extra.get("x-thirdParty"), Some(&serde_json::json!(1)));
    note.title = Some("renamed".into());
    store.save_note(&note).unwrap();
    let again = store.load_note_by_id(NOTE_ID).unwrap();
    assert_eq!(again.extra.get("x-thirdParty"), Some(&serde_json::json!(1)));
}

#[test]
fn note_with_unknown_key_and_missing_required_property_is_still_an_error() {
    let (tmp, store) = init();
    let mut v = note_json(NOTE_ID, true);
    v.as_object_mut().unwrap().remove("sections");
    write(&tmp, &format!("records/notes/{NOTE_ID}.json"), &v);
    assert!(catalog::build_checked(&store).is_err());
}

#[test]
fn relation_with_unknown_key_loads_with_a_warning_and_is_carried() {
    let (tmp, store) = init();
    write(
        &tmp,
        &format!("records/notes/{NOTE_ID}.json"),
        &note_json(NOTE_ID, false),
    );
    write(
        &tmp,
        &format!("records/notes/{OTHER_ID}.json"),
        &note_json(OTHER_ID, false),
    );
    write(
        &tmp,
        &format!("relations/{REL_ID}.json"),
        &serde_json::json!({
            "$schema": srs_schema::RELATION_SCHEMA_ID,
            "relationId": REL_ID,
            "relationType": "precedes",
            "sourceInstanceId": NOTE_ID,
            "targetInstanceId": OTHER_ID,
            "x-thirdParty": {"a": 1}
        }),
    );
    catalog::build_checked(&store).expect("an unknown key must not be fatal");
    let w = warnings(&store);
    assert_eq!(w.len(), 1);
    assert!(w[0].message.contains("x-thirdParty"));
    let rel = store.load_relation(REL_ID).unwrap();
    assert_eq!(
        rel.extra.get("x-thirdParty"),
        Some(&serde_json::json!({"a": 1}))
    );
    store.save_relation(&rel).unwrap();
    let text =
        std::fs::read_to_string(tmp.path().join(format!("relations/{REL_ID}.json"))).unwrap();
    assert!(text.contains("x-thirdParty"), "{text}");
}

#[test]
fn clean_repository_has_no_unknown_key_warning() {
    let (tmp, store) = init();
    write(
        &tmp,
        &format!("records/notes/{NOTE_ID}.json"),
        &note_json(NOTE_ID, false),
    );
    assert!(warnings(&store).is_empty());
}
