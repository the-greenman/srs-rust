//! Native coverage for the tree-session paths the WASM bindings expose
//! (#684, ADR-037/038).
//!
//! `SrsRepository::{load_tree, export_tree, load, load_archive, export_srsj}`
//! route through js-sys types that cannot be constructed natively (the wasm32
//! build gate covers their compilation — ADR-013); these tests exercise the
//! exact `srs-repository` calls each binding makes, in the same order.

use srs_repository::record_store::{
    list_record_summaries, update_record, RecordListFilter, UpdateRecordInput,
};
use srs_repository::srsj_migration_service::{export_srsj_string, load_from_srsj};
use srs_repository::{
    archive_to_tree, archive_to_vec, export_tree, materialize_tree, open_tree, RepositoryStore,
};
use std::collections::BTreeMap;
use std::path::Path;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../srs-repository/tests/fixtures/exploded-basic"
);
const RECORD_1: &str = "0ce8cbdd-5a77-4740-a34a-83b3afa63a3e";
const FIELD_TITLE: &str = "22222222-2222-4222-8222-222222222221";
const FIELD_APPROVED: &str = "22222222-2222-4222-8222-222222222222";

fn fixture_map() -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, map: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, map);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                map.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let root = Path::new(FIXTURE);
    let mut map = BTreeMap::new();
    walk(root, root, &mut map);
    map
}

/// The load_tree → edit → export_tree binding flow: the byte-diff is exactly
/// the edit.
#[test]
fn tree_binding_flow_single_edit_single_diff() {
    let base = fixture_map();
    let store = open_tree(base.clone()).expect("open_tree");
    update_record(
        &store,
        RECORD_1,
        UpdateRecordInput {
            field_values: vec![
                srs_core::types::record::FieldValue {
                    field_id: FIELD_TITLE.to_string(),
                    value: serde_json::json!("Adopt the tree model"),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                srs_core::types::record::FieldValue {
                    field_id: FIELD_APPROVED.to_string(),
                    value: serde_json::json!(false),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            group_values: None,
            tags: None,
            type_version: None,
        },
    )
    .expect("update_record");

    let exported = export_tree(&store).expect("export_tree");
    let changed: Vec<&String> = exported
        .iter()
        .filter(|(k, v)| base.get(*k) != Some(*v))
        .map(|(k, _)| k)
        .collect();
    assert_eq!(
        changed,
        vec!["records/tier-2/decision-0ce8cbdd.json"],
        "binding-level diff must be exactly the edited record"
    );
}

/// The load(srsj) → export_srsj binding flow round-trips with parity.
#[test]
fn srsj_codec_flow_roundtrip_parity() {
    // Build an .srsj the way a JsonStore session would carry it.
    let base = fixture_map();
    let mut data = serde_json::Map::new();
    let mut manifest = serde_json::Value::Null;
    for (path, bytes) in &base {
        if path == "manifest.json" {
            manifest = serde_json::from_slice(bytes).unwrap();
        } else if path.ends_with(".json") && !path.starts_with(".github/") {
            data.insert(path.clone(), serde_json::from_slice(bytes).unwrap());
        }
    }
    let envelope =
        serde_json::json!({ "srsj": "1", "manifest": manifest, "data": data }).to_string();

    // load(): codec → materialize.
    let codec = load_from_srsj(&envelope).expect("load_from_srsj");
    let session = materialize_tree(&codec).expect("materialize_tree");

    // export_srsj() → load() again: inventory parity.
    let exported = export_srsj_string(&session).expect("export_srsj_string");
    let codec2 = load_from_srsj(&exported).expect("reload exported srsj");
    let session2 = materialize_tree(&codec2).expect("re-materialize");

    let ids = |s: &dyn RepositoryStore| -> Vec<String> {
        let mut v: Vec<String> = list_record_summaries(s, RecordListFilter::default())
            .unwrap()
            .into_iter()
            .map(|r| r.instance_id)
            .collect();
        v.sort();
        v
    };
    assert_eq!(ids(&session), ids(&session2), "srsj round-trip parity");
}

/// The export_archive → load_archive binding flow preserves the tree
/// byte-for-byte (new-format archives, ADR-038).
#[test]
fn archive_binding_flow_byte_faithful() {
    let base = fixture_map();
    let store = open_tree(base.clone()).expect("open_tree");
    let bytes = archive_to_vec(&store).expect("archive_to_vec");
    let reloaded = archive_to_tree(std::io::Cursor::new(bytes)).expect("archive_to_tree");
    let exported = export_tree(&reloaded).expect("export_tree");
    assert_eq!(
        base, exported,
        "tree → .srs → tree must be byte-identical (decoys included)"
    );
}
