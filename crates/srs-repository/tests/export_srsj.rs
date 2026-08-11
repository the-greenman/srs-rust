//! `.srsj` codec round-trip: a final-format document opens through the tree
//! path, and its catalog identity sets survive a decode → encode → decode
//! cycle unchanged (RFC-038 [R17]/[R19]/[R20], srs-rust#783 Phase 4).
//!
//! This is the same codec the WASM `SrsRepository::{load, export_srsj}`
//! bindings and the CLI's `--repo <file>.srsj` session call — there is only one
//! `.srsj` mechanism, so proving it here proves it for every consumer.

use srs_repository::catalog;
use srs_repository::srsj::{open_srsj, to_srsj_string, SRSJ_VERSION};
use srs_repository::RepositoryStore;

const GALLERY: &str = include_str!("fixtures/gallery.srsj");

/// Every id in the six authoritative sets, as `kind:id`, sorted.
fn identity_sets(store: &dyn RepositoryStore) -> Vec<String> {
    let cat = catalog::build(store).expect("catalog must build");
    let mut ids: Vec<String> = cat
        .instances
        .iter()
        .chain(&cat.relations)
        .chain(&cat.containers)
        .chain(&cat.source_documents)
        .chain(&cat.definitions)
        .chain(&cat.extensions)
        .map(|e| format!("{}:{}", e.kind.as_str(), e.id))
        .collect();
    ids.sort();
    ids
}

#[test]
fn final_format_document_opens_through_the_tree_path() {
    let store = open_srsj(GALLERY).expect("a final-format .srsj must open");
    let manifest = store.load_manifest().expect("manifest must parse");
    assert_eq!(
        manifest.extra.get("namespace").and_then(|v| v.as_str()),
        Some("com.limoma")
    );

    let ids = identity_sets(&store);
    assert!(
        !ids.is_empty(),
        "the fixture must enumerate through the catalog, not an index"
    );
}

#[test]
fn catalog_identity_sets_survive_a_srsj_round_trip() {
    let original = open_srsj(GALLERY).expect("a final-format .srsj must open");
    let before = identity_sets(&original);

    let exported = to_srsj_string(&original).expect("projection must succeed");
    let parsed: serde_json::Value =
        serde_json::from_str(&exported).expect("the projection must be valid JSON");
    assert_eq!(parsed["srsj"].as_str(), Some(SRSJ_VERSION));

    let reloaded = open_srsj(&exported).expect("the projection must reopen");
    assert_eq!(
        before,
        identity_sets(&reloaded),
        "a `.srsj` round-trip must preserve every authoritative set"
    );
}

#[test]
fn projection_is_idempotent() {
    let store = open_srsj(GALLERY).expect("a final-format .srsj must open");
    let once = to_srsj_string(&store).expect("projection must succeed");
    let twice = to_srsj_string(&open_srsj(&once).unwrap()).expect("projection must succeed");
    assert_eq!(once, twice, "decode → encode must be byte-stable");
}

/// RFC-038 acceptance test 9: a reader given a document at an unrecognised
/// `srsj` version refuses it rather than reporting an empty repository.
#[test]
fn r20_refuses_a_pre_cutover_document_rather_than_reading_it_empty() {
    let mut doc: serde_json::Value = serde_json::from_str(GALLERY).unwrap();
    doc["srsj"] = serde_json::json!("1");
    let err = open_srsj(&doc.to_string()).expect_err("srsj '1' must be refused");
    let message = err.to_string();
    assert!(
        message.contains("unsupported srsj version '1'"),
        "got: {message}"
    );
    assert!(
        message.contains("[R20]"),
        "must cite the governing rule: {message}"
    );
}

/// Cross-store copy into a `.srsj` session must carry the definition families
/// that live only in `package.json`'s side arrays — vocabularies and lifecycles
/// were dropped by an earlier codec.
#[test]
fn copy_into_a_srsj_session_preserves_vocabularies_and_lifecycles() {
    use srs_repository::repository_lifecycle::{
        create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    };
    use srs_repository::repository_portability::copy_repository;
    use srs_repository::srsj::SrsjSession;
    use srs_repository::FileStore;

    let src_tmp = tempfile::TempDir::new().unwrap();
    let src = FileStore::new(src_tmp.path());
    create_repository(
        &src,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "3b3d5b0c-0000-4000-8000-00000000cafe".to_string(),
                namespace: "com.semanticops.json".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "3b3d5b0c-0000-4000-8000-00000000beef".to_string(),
                namespace: "com.semanticops.json".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();

    src.save_text_file(
        "package/vocabularies/test-vocab.json",
        &serde_json::json!({
            "id": "3b3d5b0c-0000-4000-8000-0000000v0c01",
            "version": 1,
            "namespace": "com.semanticops.json",
            "name": "test-vocab",
            "mode": "open",
            "terms": [],
            "createdAt": "2026-01-01T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    src.save_text_file(
        "package/lifecycles/test-lifecycle.json",
        &serde_json::json!({
            "id": "3b3d5b0c-0000-4000-8000-00000000lc01",
            "version": 1,
            "namespace": "com.semanticops.json",
            "name": "test-lifecycle",
            "states": [
                {"id": "s1", "key": "draft", "isInitial": true},
                {"id": "s2", "key": "active", "isFinal": true}
            ],
            "transitions": [{"name": "publish", "from": "draft", "to": "active"}],
            "initialState": "draft",
            "createdAt": "2026-01-01T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    let mut pkg = src.load_package_json().unwrap();
    pkg["vocabularies"] = serde_json::json!(["vocabularies/test-vocab.json"]);
    pkg["lifecycles"] = serde_json::json!(["lifecycles/test-lifecycle.json"]);
    src.save_package_json(&pkg).unwrap();

    let dst_tmp = tempfile::TempDir::new().unwrap();
    let dst_path = dst_tmp.path().join("copy.srsj");
    let mut session = SrsjSession::create(&dst_path).unwrap();
    copy_repository(&src, session.store()).unwrap();
    session.flush().unwrap();
    drop(session);

    let reopened = SrsjSession::open(&dst_path).unwrap();
    let package = reopened.store().load_package().unwrap();
    assert_eq!(package.vocabularies.len(), 1, "vocabulary must survive");
    assert_eq!(package.vocabularies[0].name, "test-vocab");
    assert_eq!(package.lifecycles.len(), 1, "lifecycle must survive");
    assert_eq!(package.lifecycles[0].name, "test-lifecycle");
}
