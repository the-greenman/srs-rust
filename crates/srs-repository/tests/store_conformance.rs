//! RepositoryStore conformance/parity harness — ADR-041 G9.
//!
//! This is the **admission gate** for any new `RepositoryStore` backend. Every new
//! `impl RepositoryStore` must be wired into this file and pass all tests before
//! its PR can be merged.
//!
//! ## How to admit a new backend
//!
//! 1. Add a factory function `fn init_<name>_store() -> (impl RepositoryStore, ...)`.
//! 2. Add `#[test] fn <name>_store_passes_conformance()` that calls `run_conformance_suite`.
//! 3. Add `<name>` rows to the cross-store portability tests where applicable.
//! 4. Run `cargo test -p srs-repository --test store_conformance` — all green is the gate.
//!
//! ## Coverage
//!
//! `run_conformance_suite` exercises 4 behavioral areas via raw `RepositoryStore` trait methods
//! (not service-layer functions, so failures point to the adapter not the service):
//!   1. Manifest round-trip
//!   2. Container CRUD (typed logical-id — the ADR-041 gold standard)
//!   3. Instance persistence (ADR-042 typed methods: save_record/note, load, delete, find, list)
//!   4. Batch write mode — commit path (ADR-021 / ADR-041 G6)
//!
//! Separate standalone tests cover:
//!   - Cross-store portability via `copy_repository` (ADR-008)
//!   - ADR-007 write-ordering invariants via `FailPoint` (MemoryStore only)
//!   - `abort_batch` rollback (JsonStore only — FileStore/MemoryStore use the no-op default)

use srs_core::types::container::Container;
use srs_core::types::note::Note;
use srs_core::types::record::Record;
use srs_repository::index::{InstanceQuery, InstanceRef};
use srs_repository::{
    repository_lifecycle::{
        create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    },
    repository_portability::copy_repository,
    store::memory::{FailPoint, MemoryStore},
    FileStore, JsonStore, RepositoryStore,
};
use std::collections::HashMap;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Store factories
// ---------------------------------------------------------------------------

fn init_input() -> InitializeRepositoryInput {
    InitializeRepositoryInput {
        repository: RepositoryMetadata {
            repository_id: "conf-repo-001".to_string(),
            namespace: "com.test.conf".to_string(),
            srs_version: "2.0-draft".to_string(),
            title: None,
            description: None,
        },
        primary_package: PrimaryPackageMetadata {
            id: "conf-pkg-001".to_string(),
            namespace: "com.test.conf".to_string(),
            name: "primary".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

/// FileStore factory — calls `create_repository` via the G1 service seam, not raw fs.
/// The `TempDir` must be kept alive for the duration of the test.
fn init_file_store() -> (FileStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    create_repository(&store, &init_input())
        .expect("init_file_store: create_repository must succeed");
    (store, tmp)
}

/// JsonStore factory — `JsonStore::create` + `create_repository` via the G1 service seam.
fn init_json_store() -> (JsonStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("repo.srsj");
    let store = JsonStore::create(&path).expect("init_json_store: JsonStore::create must succeed");
    create_repository(&store, &init_input())
        .expect("init_json_store: create_repository must succeed");
    (store, tmp)
}

/// MemoryStore factory — `MemoryStore::empty()` constructs a valid manifest/package in memory;
/// `create_repository` is not required (already has a valid default state).
fn init_memory_store() -> MemoryStore {
    MemoryStore::empty()
}

// ---------------------------------------------------------------------------
// Entity builders
// ---------------------------------------------------------------------------

fn make_container(id: &str, title: &str) -> Container {
    Container {
        container_id: id.to_string(),
        title: title.to_string(),
        namespace: None,
        name: None,
        description: None,
        container_type: None,
        identity_instance_id: None,
        root_instance_ids: None,
        member_instance_ids: None,
        tags: None,
        created_at: None,
        updated_at: None,
        meta: None,
        extra: HashMap::new(),
    }
}

fn make_record(id: &str, type_name: &str, tags: Option<Vec<String>>) -> Record {
    Record {
        instance_id: id.to_string(),
        type_id: "00000000-0000-4000-8000-000000000001".to_string(),
        type_version: 1,
        type_namespace: "com.test.conf".to_string(),
        type_name: type_name.to_string(),
        field_values: vec![],
        group_values: None,
        lifecycle_state: None,
        tags,
        created_at: None,
        updated_at: None,
        extra: HashMap::new(),
    }
}

fn make_note(id: &str, title: &str, tags: Option<Vec<String>>) -> Note {
    Note {
        instance_id: id.to_string(),
        title: Some(title.to_string()),
        tags,
        sections: vec![],
        graduated_at: None,
        source_refs: None,
        created_at: None,
        updated_at: None,
        meta: None,
    }
}

// ---------------------------------------------------------------------------
// Conformance suite — 4 behavioral areas
// ---------------------------------------------------------------------------

/// Runs the shared conformance suite against any `RepositoryStore`.
///
/// Uses raw trait methods (not service-layer functions) so failures point to
/// the adapter under test. ADR-007 FailPoint tests are separate MemoryStore-only
/// functions; `abort_batch` rollback is a separate JsonStore-only test.
fn run_conformance_suite(store: &dyn RepositoryStore) {
    suite_manifest_roundtrip(store);
    suite_container_crud(store);
    suite_instance_persistence(store);
    suite_batch_commit(store);
}

// --- Area 1: Manifest round-trip ---

fn suite_manifest_roundtrip(store: &dyn RepositoryStore) {
    let mut manifest = store
        .load_manifest()
        .expect("load_manifest must succeed on an initialized store");
    let original_namespace = manifest.extra.get("namespace").cloned();
    // Add a sentinel to extra to prove the round-trip is identity.
    manifest.extra.insert(
        "x-conformance-sentinel".to_string(),
        serde_json::json!("harness-v1"),
    );
    store
        .save_manifest(&manifest)
        .expect("save_manifest must succeed");
    let reloaded = store
        .load_manifest()
        .expect("load_manifest after save must succeed");
    assert_eq!(
        reloaded.extra.get("x-conformance-sentinel"),
        Some(&serde_json::json!("harness-v1")),
        "save_manifest → load_manifest must preserve extra fields (manifest round-trip)"
    );
    // Namespace preserved (not silently dropped).
    if let Some(ns) = original_namespace {
        assert_eq!(reloaded.extra.get("namespace"), Some(&ns));
    }
}

// --- Area 2: Container CRUD (typed logical-id — ADR-041 gold standard) ---

fn suite_container_crud(store: &dyn RepositoryStore) {
    let id_a = "cconf-aaaa-0001-4000-8000-000000000001";
    let id_b = "cconf-bbbb-0002-4000-8000-000000000002";
    let ca = make_container(id_a, "Container Alpha");
    let cb = make_container(id_b, "Container Beta");

    // Save and load by id.
    store
        .save_container(&ca)
        .expect("save_container must succeed");
    let loaded = store
        .load_container(id_a)
        .expect("load_container must return saved container");
    assert_eq!(loaded.container_id, id_a, "container_id must round-trip");
    assert_eq!(loaded.title, "Container Alpha", "title must round-trip");

    // Two containers coexist.
    store
        .save_container(&cb)
        .expect("save second container must succeed");
    let summaries = store
        .list_container_summaries()
        .expect("list_container_summaries must succeed");
    let ids: Vec<_> = summaries.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&id_a), "list must include first container");
    assert!(ids.contains(&id_b), "list must include second container");

    // Delete removes from index; subsequent load returns an error.
    store
        .delete_container(id_a)
        .expect("delete_container must succeed");
    let after_delete = store.load_container(id_a);
    assert!(
        after_delete.is_err(),
        "load_container after delete must return an error (got {:?})",
        after_delete
    );
    let remaining = store
        .list_container_summaries()
        .expect("list_container_summaries must succeed after delete");
    let remaining_ids: Vec<_> = remaining.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        !remaining_ids.contains(&id_a),
        "deleted container must not appear in list"
    );
    assert!(
        remaining_ids.contains(&id_b),
        "non-deleted container must still appear in list"
    );
}

// --- Area 3: Instance persistence (ADR-042 typed methods) ---

fn suite_instance_persistence(store: &dyn RepositoryStore) {
    let rec_id = "rconf-0001-0000-4000-8000-aabbccddeeff";
    let note_id = "nconf-0002-0000-4000-8000-aabbccddeeff";
    let rec2_id = "rconf-0003-0000-4000-8000-aabbccddeeff";

    // save_record → load_record_by_id round-trip.
    let rec = make_record(rec_id, "Decision", Some(vec!["alpha".to_string()]));
    store.save_record(&rec).expect("save_record must succeed");
    let loaded_rec = store
        .load_record_by_id(rec_id)
        .expect("load_record_by_id must succeed after save");
    assert_eq!(
        loaded_rec.instance_id, rec_id,
        "record instance_id must round-trip"
    );
    assert_eq!(
        loaded_rec.type_name, "Decision",
        "record type_name must round-trip"
    );
    assert_eq!(
        loaded_rec.tags.as_deref(),
        Some(&["alpha".to_string()][..]),
        "record tags must round-trip"
    );

    // save_note → load_note_by_id round-trip; find_instance reports tier 0.
    let note = make_note(note_id, "My Conformance Note", None);
    store.save_note(&note).expect("save_note must succeed");
    let loaded_note = store
        .load_note_by_id(note_id)
        .expect("load_note_by_id must succeed after save");
    assert_eq!(
        loaded_note.instance_id, note_id,
        "note instance_id must round-trip"
    );
    assert_eq!(
        loaded_note.title.as_deref(),
        Some("My Conformance Note"),
        "note title must round-trip"
    );
    let note_ref = store
        .find_instance(note_id)
        .expect("find_instance must not error for saved note")
        .expect("find_instance must return Some for saved note");
    assert_eq!(note_ref.tier, 0, "saved note must have tier 0");

    // find_instance returns None for unknown id.
    let missing = store
        .find_instance("does-not-exist-0000-4000-8000-000000000000")
        .expect("find_instance must not error for unknown id");
    assert!(
        missing.is_none(),
        "find_instance must return None for unknown id"
    );

    // delete_instance → find_instance returns None; load_record_by_id errors.
    store
        .delete_instance(rec_id)
        .expect("delete_instance must succeed");
    let after_delete = store
        .find_instance(rec_id)
        .expect("find_instance must not error after delete");
    assert!(
        after_delete.is_none(),
        "find_instance must return None after delete"
    );
    assert!(
        store.load_record_by_id(rec_id).is_err(),
        "load_record_by_id must error after delete"
    );

    // list_instances by tier: a second record + saved note → tier-2 query returns only records.
    let rec2 = make_record(rec2_id, "Action", Some(vec!["beta".to_string()]));
    store
        .save_record(&rec2)
        .expect("save second record must succeed");
    let tier2_refs = store
        .list_instances(&InstanceQuery {
            tier: Some(2),
            tag: None,
        })
        .expect("list_instances by tier must succeed");
    let tier2_ids: Vec<_> = tier2_refs
        .iter()
        .map(|r: &InstanceRef| r.instance_id.as_str())
        .collect();
    assert!(
        tier2_ids.contains(&rec2_id),
        "list_instances tier=2 must include saved record"
    );
    assert!(
        !tier2_ids.contains(&note_id),
        "list_instances tier=2 must not include note (tier 0)"
    );

    // Existing-id update: second save_record on same id updates content and index tags.
    let mut updated_rec2 = rec2.clone();
    updated_rec2.type_name = "UpdatedAction".to_string();
    updated_rec2.tags = Some(vec!["beta".to_string(), "updated".to_string()]);
    store
        .save_record(&updated_rec2)
        .expect("save_record existing id must succeed");
    let after_update = store
        .load_record_by_id(rec2_id)
        .expect("load_record_by_id after update must succeed");
    assert_eq!(
        after_update.type_name, "UpdatedAction",
        "existing-id save must update content"
    );
    let updated_ref = store
        .find_instance(rec2_id)
        .expect("find_instance must succeed after update")
        .expect("find_instance must return Some after update");
    assert!(
        updated_ref.tags.contains(&"updated".to_string()),
        "index tags must be refreshed from entity on update"
    );
}

// --- Area 4: Batch write mode — commit path (ADR-021 / ADR-041 G6) ---

fn suite_batch_commit(store: &dyn RepositoryStore) {
    let note_id = "nbatch-commit-0000-4000-8000-aabbccddee01";
    let note = make_note(note_id, "Batch Commit Note", None);

    store.begin_batch();
    store
        .save_note(&note)
        .expect("save_note during batch must succeed");
    store.commit_batch().expect("commit_batch must succeed");

    let loaded = store
        .load_note_by_id(note_id)
        .expect("load_note_by_id must succeed after commit_batch");
    assert_eq!(
        loaded.title.as_deref(),
        Some("Batch Commit Note"),
        "data saved in batch must be visible after commit_batch"
    );
}

// ---------------------------------------------------------------------------
// Admission-gate tests (one per backend)
// ---------------------------------------------------------------------------

#[test]
fn file_store_passes_conformance() {
    let (store, _tmp) = init_file_store();
    run_conformance_suite(&store);
}

#[test]
fn json_store_passes_conformance() {
    let (store, _tmp) = init_json_store();
    run_conformance_suite(&store);
}

#[test]
fn memory_store_passes_conformance() {
    let store = init_memory_store();
    run_conformance_suite(&store);
}

// ---------------------------------------------------------------------------
// Cross-store portability tests (ADR-008)
// ---------------------------------------------------------------------------

#[test]
fn copy_repository_memory_to_file_preserves_instances() {
    let src = init_memory_store();
    let rec_id = "rport-m2f-0001-4000-8000-aabbccddeeff";
    let note_id = "nport-m2f-0002-4000-8000-aabbccddeeff";
    src.save_record(&make_record(rec_id, "Portability", None))
        .unwrap();
    src.save_note(&make_note(note_id, "Port Note", None))
        .unwrap();

    let tmp = TempDir::new().unwrap();
    let file = FileStore::new(tmp.path());
    copy_repository(&src, &file).expect("copy_repository memory→file must succeed");

    let loaded_rec = file
        .load_record_by_id(rec_id)
        .expect("load_record_by_id after memory→file copy must succeed");
    assert_eq!(loaded_rec.instance_id, rec_id);
    assert_eq!(loaded_rec.type_name, "Portability");

    let loaded_note = file
        .load_note_by_id(note_id)
        .expect("load_note_by_id after memory→file copy must succeed");
    assert_eq!(loaded_note.title.as_deref(), Some("Port Note"));
}

#[test]
fn copy_repository_memory_to_json_preserves_instances() {
    let src = init_memory_store();
    let rec_id = "rport-m2j-0001-4000-8000-aabbccddeeff";
    let note_id = "nport-m2j-0002-4000-8000-aabbccddeeff";
    src.save_record(&make_record(rec_id, "JsonPort", None))
        .unwrap();
    src.save_note(&make_note(note_id, "Json Port Note", None))
        .unwrap();

    let tmp = TempDir::new().unwrap();
    let json = JsonStore::create(tmp.path().join("repo.srsj")).unwrap();
    copy_repository(&src, &json).expect("copy_repository memory→json must succeed");

    let loaded_rec = json
        .load_record_by_id(rec_id)
        .expect("load_record_by_id after memory→json copy must succeed");
    assert_eq!(loaded_rec.type_name, "JsonPort");

    let loaded_note = json
        .load_note_by_id(note_id)
        .expect("load_note_by_id after memory→json copy must succeed");
    assert_eq!(loaded_note.title.as_deref(), Some("Json Port Note"));
}

#[test]
fn copy_repository_full_chain_memory_json_file_memory() {
    // memory → json → file → fresh memory; verify instance ids and count preserved.
    let src_mem = init_memory_store();
    let ids = [
        ("rchain-0001-4000-8000-aabbccddeeff", "ChainRec1"),
        ("rchain-0002-4000-8000-aabbccddeeff", "ChainRec2"),
    ];
    for (id, name) in &ids {
        src_mem.save_record(&make_record(id, name, None)).unwrap();
    }

    // hop 1: memory → json
    let tmp1 = TempDir::new().unwrap();
    let json = JsonStore::create(tmp1.path().join("chain.srsj")).unwrap();
    copy_repository(&src_mem, &json).expect("memory→json hop must succeed");

    // hop 2: json → file
    let tmp2 = TempDir::new().unwrap();
    let file = FileStore::new(tmp2.path());
    copy_repository(&json, &file).expect("json→file hop must succeed");

    // hop 3: file → fresh memory (must start uninitialized — copy_repository initializes it)
    let dst_mem = MemoryStore::uninitialized();
    copy_repository(&file, &dst_mem).expect("file→memory hop must succeed");

    // Verify all ids survive the full chain.
    for (id, name) in &ids {
        let r = dst_mem
            .load_record_by_id(id)
            .unwrap_or_else(|e| panic!("load_record_by_id({id}) after full chain: {e}"));
        assert_eq!(&r.type_name, name, "type_name must survive full chain");
    }

    // Instance count must match — use list_instances (ADR-042 query surface) not the raw index.
    let dst_refs = dst_mem
        .list_instances(&InstanceQuery {
            tier: None,
            tag: None,
        })
        .expect("list_instances on copy destination must succeed");
    let src_refs = json
        .list_instances(&InstanceQuery {
            tier: None,
            tag: None,
        })
        .expect("list_instances on copy source must succeed");
    assert_eq!(
        dst_refs.len(),
        src_refs.len(),
        "instance count must be preserved across the full portability chain"
    );
}

// ---------------------------------------------------------------------------
// ADR-007 write-ordering tests — MemoryStore only (via FailPoint)
// ---------------------------------------------------------------------------

#[test]
fn adr007_index_ordering_instance_save() {
    // Entity-before-index on create: a failed index update after a successful data write
    // must leave data in the store (orphaned, invisible to readers) but no dangling index entry.
    let store = MemoryStore::empty();
    let rec = make_record("rorder-0001-4000-8000-aabbccddeeff", "Thing", None);

    store.arm_fail_at(FailPoint::SaveInstanceIndex);
    let result = store.save_record(&rec);
    assert!(
        result.is_err(),
        "save_record must return an error when SaveInstanceIndex is armed"
    );

    // No dangling index entry — the index is internally consistent.
    let idx_entry = store
        .find_instance("rorder-0001-4000-8000-aabbccddeeff")
        .expect("find_instance must not error");
    assert!(
        idx_entry.is_none(),
        "instance index must have no entry after a failed index update (ADR-007)"
    );
}

#[test]
fn adr007_index_ordering_container_save() {
    // Container variant of the ADR-007 invariant.
    let store = MemoryStore::empty();
    let container = make_container("corder-0001-4000-8000-aabbccddeeff", "Order Test");

    store.arm_fail_at(FailPoint::SaveContainerIndex);
    let result = store.save_container(&container);
    assert!(
        result.is_err(),
        "save_container must return an error when SaveContainerIndex is armed"
    );

    let summaries = store
        .list_container_summaries()
        .expect("list_container_summaries must not error");
    assert!(
        summaries.is_empty(),
        "container index must have no entry after a failed index update (ADR-007)"
    );
}

// ---------------------------------------------------------------------------
// JsonStore-only: abort_batch rollback (ADR-021 / ADR-041 G6)
// ---------------------------------------------------------------------------

#[test]
fn json_store_batch_abort_rolls_back() {
    // abort_batch must roll back in-memory state; data saved in the batch must not be
    // accessible after abort. Only JsonStore has a real rollback — FileStore and MemoryStore
    // use the no-op default (begin_batch/abort_batch are pass-through), so this test is
    // JsonStore-specific.
    let (store, _tmp) = init_json_store();
    let note_id = "nabort-0001-4000-8000-aabbccddeeff";
    let note = make_note(note_id, "Aborted Note", None);

    store.begin_batch();
    store
        .save_note(&note)
        .expect("save_note during batch must succeed");
    store.abort_batch();

    let result = store
        .find_instance(note_id)
        .expect("find_instance must not error after abort");
    assert!(
        result.is_none(),
        "find_instance must return None after abort_batch — aborted writes must not be visible (ADR-021)"
    );
}
