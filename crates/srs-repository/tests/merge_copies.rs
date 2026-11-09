//! Bringing new items in from another copy of a repository (srs-rust#1580). The RFC-046 rows
//! follow the Change C fixture shape: a session actor, an operation, and the expected
//! `createdBy` of what it writes, or the expected code.

use serde_json::{json, Value};
use srs_core::types::actor::Actor;
use srs_core::types::container::{entries, Container};
use srs_core::types::note::{Note, NoteSection};
use srs_core::types::record::{FieldValues, Record};
use srs_core::types::relation::Relation;
use srs_repository::attachment_service::{
    add_attachment, link_attachment, AddAttachmentInput, LinkAttachmentInput,
};
use srs_repository::container_service;
use srs_repository::field_type_migration_service::stamp_data_model_revision;
use srs_repository::merge_service::{
    merge_apply, merge_preview, MergeEntryId, MergeReport, MergeSelection,
};
use srs_repository::record_store::{self, CreateRecordInput};
use srs_repository::relation_service;
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::services;
use srs_repository::validation::validate_repository;
use srs_repository::{archive_to_tree, archive_to_vec, FileStore, RepositoryStore};
use std::collections::BTreeMap;
use std::io::Cursor;
use tempfile::TempDir;

const PURPOSE: &str = "com.semanticops.core/purpose";

fn agent() -> Value {
    json!({"kind": "ai", "id": "agent-1", "name": "Scribe"})
}

fn merger() -> Value {
    json!({"kind": "human", "id": "user-7"})
}

/// A new repository whose work is done by `agent()`.
fn origin(repository_id: &str) -> (TempDir, FileStore) {
    let dir = TempDir::new().unwrap();
    let store = FileStore::new(dir.path());
    store.set_session_actor(Some(agent()));
    create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: repository_id.to_string(),
                namespace: "com.test.merge".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "15800000-0000-4000-8000-000000000002".to_string(),
                namespace: "com.test.merge".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    (dir, store)
}

const REPO: &str = "15800000-0000-4000-8000-000000000001";

fn record(store: &dyn RepositoryStore, statement: &str) -> Record {
    let mut field_values = FieldValues::new();
    field_values.insert("statement", Value::String(statement.to_string()));
    record_store::create_record_in_context(
        store,
        PURPOSE,
        None,
        CreateRecordInput {
            field_values,
            field_meta: None,
            tags: Some(vec!["kept".to_string()]),
            lifecycle_state: None,
            extra: BTreeMap::new(),
        },
        None,
        None,
    )
    .unwrap()
    .record
}

fn note(store: &dyn RepositoryStore, title: &str) -> Note {
    services::create_note(
        store,
        Note {
            instance_id: String::new(),
            title: Some(title.to_string()),
            tags: None,
            sections: vec![NoteSection {
                name: "body".to_string(),
                label: None,
                content: "text".to_string(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            created_by: None,
            meta: None,
        },
    )
    .unwrap()
    .note
}

fn relate(store: &dyn RepositoryStore, src: &str, dst: &str) -> Relation {
    relation_service::create_relation_auto(
        store,
        Relation {
            relation_id: String::new(),
            relation_type: "evidences".to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: dst.to_string(),
            created_at: None,
            created_by: None,
            notes: None,
            source_refs: None,
            meta: None,
        },
    )
    .unwrap()
    .relation
}

fn container(store: &dyn RepositoryStore, title: &str, members: &[&str]) -> Container {
    let value = json!({"title": title});
    let mut c: Container = serde_json::from_value(value).unwrap();
    c.member_instance_ids = Some(entries(members.iter().copied()));
    c.anchor_instance_id = members.first().map(|s| s.to_string());
    container_service::create_container(store, c).unwrap()
}

fn pack(store: &dyn RepositoryStore) -> Cursor<Vec<u8>> {
    Cursor::new(archive_to_vec(store).unwrap())
}

/// A copy of `store` taken now, in memory, opened by `merger()`.
fn copy_in_memory(store: &dyn RepositoryStore) -> FileStore {
    let copy = archive_to_tree(pack(store)).unwrap();
    copy.set_session_actor(Some(merger()));
    copy
}

fn select_all(report: &MergeReport) -> MergeSelection {
    MergeSelection {
        instance_ids: report
            .bring
            .instances
            .iter()
            .map(|i| i.instance_id.clone())
            .collect(),
        relation_ids: report
            .bring
            .relations
            .iter()
            .map(|r| r.relation_id.clone())
            .collect(),
        container_ids: report
            .bring
            .containers
            .iter()
            .map(|c| c.container_id.clone())
            .collect(),
        container_entries: report
            .bring
            .container_entries
            .iter()
            .map(|e| MergeEntryId {
                container_id: e.container_id.clone(),
                instance_id: e.instance_id.clone(),
            })
            .collect(),
        created_by: None,
    }
}

fn diagnostics(store: &dyn RepositoryStore) -> Vec<String> {
    let mut all: Vec<String> = validate_repository(store)
        .unwrap()
        .diagnostics
        .iter()
        .map(|d| format!("{d:?}"))
        .collect();
    all.sort();
    all
}

fn code_of<T: std::fmt::Debug>(r: Result<T, srs_repository::error::RepositoryError>) -> String {
    r.unwrap_err().code().to_string()
}

/// Every file in the tree, so "nothing was written" is checked and not assumed.
fn tree(store: &dyn RepositoryStore) -> BTreeMap<String, Vec<u8>> {
    let mut paths = store.list_files_recursive("");
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let bytes = store.load_binary_file(&p).unwrap();
            (p, bytes)
        })
        .collect()
}

fn reasons(report: &MergeReport, kind: &str, id: &str) -> Vec<String> {
    report
        .held_back
        .iter()
        .filter(|h| h.item.kind == kind && h.item.id == id)
        .map(|h| h.reason.clone())
        .collect()
}

/// The whole first pass, against each kind of target the adapters open.
fn merges_added_items_unchanged(target: &dyn RepositoryStore, source_archive: Vec<u8>, w: &Work) {
    let diagnostics_before = diagnostics(target);
    let own = record(target, "the target moved on");

    let preview = merge_preview(target, Cursor::new(source_archive.clone()), None).unwrap();
    assert!(!preview.applied);
    assert_eq!(preview.repository_id, REPO);
    let ids: Vec<&str> = preview
        .bring
        .instances
        .iter()
        .map(|i| i.instance_id.as_str())
        .collect();
    assert!(ids.contains(&w.record.instance_id.as_str()));
    assert!(ids.contains(&w.note.instance_id.as_str()));
    // srs-rust#1597: a typed record is named by its display label, a note by its title.
    let title_of = |id: &str| {
        preview
            .bring
            .instances
            .iter()
            .find(|i| i.instance_id == id)
            .and_then(|i| i.title.clone())
    };
    let source_copy = archive_to_tree(Cursor::new(source_archive.clone())).unwrap();
    let want = record_store::get_record_summary_by_id(&source_copy, &w.record.instance_id)
        .unwrap()
        .unwrap()
        .display_label;
    assert_eq!(title_of(&w.record.instance_id), Some(want));
    assert_eq!(title_of(&w.note.instance_id), w.note.title);
    assert!(
        !ids.contains(&own.instance_id.as_str()),
        "only what the source alone holds"
    );
    assert_eq!(preview.bring.relations.len(), 1);
    assert_eq!(preview.bring.containers.len(), 1);
    assert_eq!(preview.bring.attachments.len(), 1);

    // No store position leaks into a payload (RFC-051 [R5]).
    let text = serde_json::to_string(&preview).unwrap();
    assert!(!text.contains("\"parent\"") && !text.contains("\"key\""));

    let before = tree(target);
    let selection = select_all(&preview);
    assert_eq!(tree(target), before, "a preview writes nothing");
    let applied = merge_apply(target, Cursor::new(source_archive.clone()), &selection).unwrap();
    assert!(applied.applied);
    assert!(applied.unavailable.is_empty(), "{:?}", applied.unavailable);

    // Unchanged: identity, createdBy, timestamps, lifecycle and tags, not the merger's stamp.
    let source = archive_to_tree(Cursor::new(source_archive.clone())).unwrap();
    let want_actor = Some(Actor::from_value(&agent()).unwrap());
    let got = target.load_record_by_id(&w.record.instance_id).unwrap();
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        serde_json::to_value(source.load_record_by_id(&w.record.instance_id).unwrap()).unwrap()
    );
    assert_eq!(got.created_by, want_actor);
    let got = target.load_note_by_id(&w.note.instance_id).unwrap();
    assert_eq!(got, source.load_note_by_id(&w.note.instance_id).unwrap());
    assert_eq!(got.created_by, want_actor);
    let rels = target.list_relations().unwrap();
    let got = rels
        .iter()
        .find(|r| r.relation_id == w.relation.relation_id)
        .unwrap();
    assert_eq!(*got, w.relation);
    assert_eq!(got.created_by, want_actor);
    assert_eq!(
        rels.len(),
        source.list_relations().unwrap().len(),
        "the operation writes no relation of its own ([R3])"
    );

    let brought = target.load_container(&w.container.container_id).unwrap();
    assert_eq!(brought.title, w.container.title);
    assert_eq!(brought.member_ids().len(), 2);

    // The target's own work and the attachment bytes are both there.
    assert!(target.load_record_by_id(&own.instance_id).is_ok());
    let doc = &preview.bring.attachments[0].document_id;
    let docs = srs_repository::repository_portability::export_repository_snapshot_with_options(
        target,
        srs_repository::repository_portability::ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )
    .unwrap();
    assert!(docs
        .source_documents
        .iter()
        .any(|d| d.document_id == *doc && d.content_base64.is_some()));

    assert_eq!(
        diagnostics(target),
        diagnostics_before,
        "no new diagnostics"
    );

    // Nothing is left to bring: the merge is repeatable.
    let again = merge_preview(target, Cursor::new(source_archive), None).unwrap();
    assert!(again.bring.instances.is_empty());
    assert!(again.bring.relations.is_empty());
    assert!(again.bring.containers.is_empty());
    assert!(again.bring.container_entries.is_empty());
}

struct Work {
    record: Record,
    note: Note,
    relation: Relation,
    container: Container,
}

/// Adds one of everything to `store`.
fn add_work(store: &FileStore) -> Work {
    let record_ = record(store, "added in the source");
    let note_ = note(store, "added note");
    let relation = relate(store, &record_.instance_id, &note_.instance_id);
    let doc = add_attachment(
        store,
        AddAttachmentInput {
            file_name: "brief.txt".to_string(),
            content: b"bytes".to_vec(),
            subdir: None,
            title: Some("Brief".to_string()),
            content_type: None,
            enforce_policy: false,
        },
    )
    .unwrap();
    link_attachment(
        store,
        LinkAttachmentInput {
            instance_id: record_.instance_id.clone(),
            document_id: doc.document_id.clone(),
        },
    )
    .unwrap();
    let record_ = store.load_record_by_id(&record_.instance_id).unwrap();
    let container = container(store, "Added", &[&record_.instance_id, &note_.instance_id]);
    Work {
        record: record_,
        note: note_,
        relation,
        container,
    }
}

#[test]
fn merges_into_a_disk_target_that_moved_on() {
    let (_d, store) = origin(REPO);
    let dir = TempDir::new().unwrap();
    let target = FileStore::new(dir.path());
    srs_repository::archive_unpack(pack(&store), &target).unwrap();
    target.set_session_actor(Some(merger()));
    let w = add_work(&store);
    merges_added_items_unchanged(&target, archive_to_vec(&store).unwrap(), &w);
    // Reopened from disk it is the same repository.
    let reopened = FileStore::new(dir.path());
    assert!(reopened.load_record_by_id(&w.record.instance_id).is_ok());
}

#[test]
fn merges_into_an_in_memory_target_that_moved_on() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let w = add_work(&store);
    merges_added_items_unchanged(&target, archive_to_vec(&store).unwrap(), &w);
    // It saves and reopens: the browser path.
    let reopened = archive_to_tree(pack(&target)).unwrap();
    assert!(reopened.load_record_by_id(&w.record.instance_id).is_ok());
}

#[test]
fn a_record_edited_on_both_sides_is_reported_and_left() {
    let (_d, store) = origin(REPO);
    let shared = record(&store, "both");
    let target = copy_in_memory(&store);
    let held = target.load_record_by_id(&shared.instance_id).unwrap();

    let mut edited = store.load_record_by_id(&shared.instance_id).unwrap();
    edited.tags = Some(vec!["changed-in-source".to_string()]);
    store.save_record(&edited).unwrap();

    let report = merge_preview(&target, pack(&store), None).unwrap();
    assert!(report
        .differ
        .iter()
        .any(|d| d.kind == "instance" && d.id == shared.instance_id));
    assert!(report.bring.instances.is_empty());
    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    assert_eq!(
        serde_json::to_value(target.load_record_by_id(&shared.instance_id).unwrap()).unwrap(),
        serde_json::to_value(held).unwrap()
    );
}

#[test]
fn a_different_repository_is_refused_and_nothing_is_written() {
    let (_a, store) = origin(REPO);
    let (_b, other) = origin("15800000-0000-4000-8000-0000000000ff");
    let before = tree(&store);
    assert_eq!(
        code_of(merge_preview(&store, pack(&other), None)),
        "merge-repository-mismatch"
    );
    assert_eq!(
        code_of(merge_apply(
            &store,
            pack(&other),
            &MergeSelection::default()
        )),
        "merge-repository-mismatch"
    );
    assert_eq!(tree(&store), before);
}

#[test]
fn a_source_newer_than_this_build_is_refused() {
    let (_d, store) = origin(REPO);
    let newer_dir = TempDir::new().unwrap();
    let newer = FileStore::new(newer_dir.path());
    srs_repository::archive_unpack(pack(&store), &newer).unwrap();
    stamp_data_model_revision(&newer, 99).unwrap();
    let before = tree(&store);
    let code = code_of(merge_preview(&store, pack(&newer), None));
    assert_eq!(code, "merge-revision-unsupported");
    assert_eq!(tree(&store), before);
}

// ── RFC-046 rows ─────────────────────────────────────────────────────────────

/// Session: none. A move stamps nothing, so it needs no actor (owner's answer, srs#1008).
#[test]
fn r6_no_session_actor_carries_created_by_unchanged() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    target.set_session_actor(None);
    let r = record(&store, "carried");
    let report = merge_preview(&target, pack(&store), None).unwrap();
    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    assert_eq!(
        target.load_record_by_id(&r.instance_id).unwrap().created_by,
        Some(Actor::from_value(&agent()).unwrap())
    );
}

/// Session: an actor that does not even validate. Expected: `createdBy` unchanged.
#[test]
fn r6_an_invalid_session_actor_does_not_stop_a_move() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    target.set_session_actor(Some(json!({"kind": "nonsense"})));
    let r = record(&store, "carried");
    let report = merge_preview(&target, pack(&store), None).unwrap();
    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    assert_eq!(
        target.load_record_by_id(&r.instance_id).unwrap().created_by,
        Some(Actor::from_value(&agent()).unwrap())
    );
}

/// Request supplies `createdBy`. Expected: `actor-supplied`, even as `null`.
#[test]
fn r6_a_request_supplying_created_by_is_refused() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let r = record(&store, "carried");
    let before = tree(&target);
    for supplied in [merger(), Value::Null] {
        let selection: MergeSelection = serde_json::from_value(json!({
            "instanceIds": [r.instance_id],
            "createdBy": supplied,
        }))
        .unwrap();
        assert_eq!(
            code_of(merge_apply(&target, pack(&store), &selection)),
            "actor-supplied"
        );
    }
    assert_eq!(tree(&target), before);
}

/// Target below revision 9, an instance carrying `createdBy`. Expected: `revision-too-old`.
#[test]
fn r13_created_by_into_a_pre_9_corpus_is_refused() {
    let (_d, store) = origin(REPO);
    let dir = TempDir::new().unwrap();
    let target = FileStore::new(dir.path());
    srs_repository::archive_unpack(pack(&store), &target).unwrap();
    stamp_data_model_revision(&target, 8).unwrap();
    let r = record(&store, "carried");
    let selection = MergeSelection {
        instance_ids: vec![r.instance_id.clone()],
        ..Default::default()
    };
    let before = tree(&target);
    assert_eq!(
        code_of(merge_apply(&target, pack(&store), &selection)),
        "revision-too-old"
    );
    assert_eq!(tree(&target), before);
    // With nothing carrying createdBy it is only a revision mismatch.
    assert_eq!(
        code_of(merge_apply(
            &target,
            pack(&store),
            &MergeSelection::default()
        )),
        "merge-revision-mismatch"
    );
}

// ── The chosen set ───────────────────────────────────────────────────────────

#[test]
fn a_relation_whose_endpoint_is_not_chosen_is_held_and_apply_refuses_whole() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let a = record(&store, "a");
    let b = note(&store, "b");
    let rel = relate(&store, &a.instance_id, &b.instance_id);
    let selection = MergeSelection {
        instance_ids: vec![a.instance_id.clone()],
        relation_ids: vec![rel.relation_id.clone()],
        ..Default::default()
    };

    // The preview answers for the choice, naming what was left out.
    let report = merge_preview(&target, pack(&store), Some(&selection)).unwrap();
    assert_eq!(
        reasons(&report, "relation", &rel.relation_id),
        ["relation-endpoint-missing"]
    );
    assert!(report.unavailable.iter().any(|u| u.id == rel.relation_id));
    assert!(report
        .not_chosen
        .iter()
        .any(|n| n.kind == "instance" && n.id == b.instance_id));

    let before = tree(&target);
    assert_eq!(
        code_of(merge_apply(&target, pack(&store), &selection)),
        "merge-selection-invalid"
    );
    assert_eq!(tree(&target), before, "nothing was written");
}

#[test]
fn a_chosen_id_the_source_does_not_offer_is_refused() {
    let (_d, store) = origin(REPO);
    let shared = record(&store, "already in both");
    let target = copy_in_memory(&store);
    let selection = MergeSelection {
        instance_ids: vec![
            shared.instance_id.clone(),
            "15800000-0000-4000-8000-0000000000aa".to_string(),
        ],
        ..Default::default()
    };
    assert_eq!(
        code_of(merge_apply(&target, pack(&store), &selection)),
        "merge-selection-invalid"
    );
}

#[test]
fn an_unknown_selection_field_is_a_parse_error() {
    let parsed: Result<MergeSelection, _> =
        serde_json::from_value(json!({"instanceIds": [], "records": []}));
    assert!(parsed.is_err());
}

#[test]
fn a_new_container_needs_its_anchor_entry_chosen() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let a = record(&store, "anchor");
    let b = note(&store, "other");
    let c = container(&store, "New", &[&a.instance_id, &b.instance_id]);
    let selection = MergeSelection {
        instance_ids: vec![a.instance_id.clone(), b.instance_id.clone()],
        container_ids: vec![c.container_id.clone()],
        container_entries: vec![MergeEntryId {
            container_id: c.container_id.clone(),
            instance_id: b.instance_id.clone(),
        }],
        ..Default::default()
    };
    let report = merge_preview(&target, pack(&store), Some(&selection)).unwrap();
    assert_eq!(
        reasons(&report, "container", &c.container_id),
        ["container-anchor-missing"]
    );
    assert_eq!(
        reasons(&report, "container-entry", &b.instance_id),
        ["container-held-back"]
    );
    assert!(report.bring.containers.is_empty());
    assert!(report.bring.container_entries.is_empty());
}

#[test]
fn a_new_entry_in_an_existing_container_comes_in_and_a_moved_one_is_reported() {
    let (_d, store) = origin(REPO);
    let a = record(&store, "a");
    let b = record(&store, "b");
    let c = container(&store, "Shared", &[&a.instance_id, &b.instance_id]);
    let target = copy_in_memory(&store);

    let n = note(&store, "new member");
    container_service::add_member(&store, &c.container_id, &n.instance_id, None, None).unwrap();
    // The source reorders an entry both hold.
    container_service::move_member(&store, &c.container_id, &b.instance_id, Some(0), None).unwrap();

    let report = merge_preview(&target, pack(&store), None).unwrap();
    assert!(report
        .differ
        .iter()
        .any(|d| d.kind == "container-entry" && d.id == b.instance_id));
    assert!(report
        .bring
        .container_entries
        .iter()
        .any(|e| e.instance_id == n.instance_id));

    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    let got = target.load_container(&c.container_id).unwrap();
    let ids = got.member_ids();
    assert!(ids.contains(&n.instance_id));
    assert_eq!(ids.len(), 3);
    // The target's order of what it already held is untouched.
    let pos = |id: &str| ids.iter().position(|i| *i == id).unwrap();
    assert!(pos(&a.instance_id) < pos(&b.instance_id));
}

#[test]
fn a_new_entry_in_the_root_container_comes_in_and_the_head_stays() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let manifest_before = target.load_manifest_raw_text().unwrap();
    let root_id = store
        .load_manifest()
        .unwrap()
        .root_container_id()
        .unwrap()
        .to_string();
    let n = note(&store, "section");
    container_service::add_member(&store, &root_id, &n.instance_id, None, None).unwrap();
    let report = merge_preview(&target, pack(&store), None).unwrap();
    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    assert!(target
        .load_container(&root_id)
        .unwrap()
        .member_ids()
        .contains(&n.instance_id));
    assert_eq!(target.load_manifest_raw_text().unwrap(), manifest_before);
}

#[test]
fn a_record_whose_type_does_not_resolve_is_held_with_its_relations() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let a = record(&store, "a");
    let b = note(&store, "b");
    let rel = relate(&store, &a.instance_id, &b.instance_id);
    // A source archive whose record names a type the target lacks.
    let mut value: Value = store
        .list_files_recursive("records")
        .into_iter()
        .find(|p| p.contains(&a.instance_id[..8]))
        .map(|p| store.load_instance_json(&p).unwrap())
        .unwrap();
    value["typeVersion"] = json!(999);
    let path = store
        .list_files_recursive("records")
        .into_iter()
        .find(|p| p.contains(&a.instance_id[..8]))
        .unwrap();
    store.save_instance_json(&path, &value).unwrap();

    let report = merge_preview(&target, pack(&store), None).unwrap();
    assert_eq!(
        reasons(&report, "instance", &a.instance_id),
        ["type-not-resolved"]
    );
    assert_eq!(
        reasons(&report, "relation", &rel.relation_id),
        ["relation-endpoint-missing"]
    );

    // Applying what is on offer leaves both out; asking for them refuses.
    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    assert!(target.find_instance(&a.instance_id).unwrap().is_none());
    assert!(target.find_instance(&b.instance_id).unwrap().is_some());
    assert!(target.list_relations().unwrap().is_empty());
    let asking = MergeSelection {
        instance_ids: vec![a.instance_id.clone()],
        ..Default::default()
    };
    assert_eq!(
        code_of(merge_apply(&target, pack(&store), &asking)),
        "merge-selection-invalid"
    );
}

/// The target already holds another document at that filename. The attachment cannot come in,
/// and neither can the record that attaches it, or validation would report a new error.
#[test]
fn a_record_whose_attachment_cannot_come_in_is_held_back() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let attach = |s: &dyn RepositoryStore, bytes: &[u8]| {
        add_attachment(
            s,
            AddAttachmentInput {
                file_name: "brief.txt".to_string(),
                content: bytes.to_vec(),
                subdir: None,
                title: None,
                content_type: None,
                enforce_policy: false,
            },
        )
        .unwrap()
        .document_id
    };
    attach(&target, b"the target's own");
    let doc = attach(&store, b"the source's");
    let r = record(&store, "attaches");
    link_attachment(
        &store,
        LinkAttachmentInput {
            instance_id: r.instance_id.clone(),
            document_id: doc.clone(),
        },
    )
    .unwrap();
    let n = note(&store, "linked to it");
    let rel = relate(&store, &r.instance_id, &n.instance_id);

    let before = diagnostics(&target);
    let report = merge_preview(&target, pack(&store), None).unwrap();
    assert_eq!(
        reasons(&report, "instance", &r.instance_id),
        ["attachment-unavailable"]
    );
    assert_eq!(
        reasons(&report, "relation", &rel.relation_id),
        ["relation-endpoint-missing"]
    );
    merge_apply(&target, pack(&store), &select_all(&report)).unwrap();
    assert!(target.find_instance(&r.instance_id).unwrap().is_none());
    assert_eq!(diagnostics(&target), before);
    let asking = MergeSelection {
        instance_ids: vec![r.instance_id.clone()],
        ..Default::default()
    };
    assert_eq!(
        code_of(merge_apply(&target, pack(&store), &asking)),
        "merge-selection-invalid"
    );
}

/// An id that is not canonical cannot name a file, so it is held in the preview and not found
/// out partway through a write.
#[test]
fn a_non_canonical_id_is_held_back_before_any_write() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store);
    let r = record(&store, "shouting");
    let path = store
        .list_files_recursive("records")
        .into_iter()
        .find(|p| p.contains(&r.instance_id[..8]))
        .unwrap();
    let mut value = store.load_instance_json(&path).unwrap();
    let upper = r.instance_id.to_uppercase();
    value["instanceId"] = json!(upper);
    store.save_instance_json(&path, &value).unwrap();

    let report = merge_preview(&target, pack(&store), None).unwrap();
    assert_eq!(reasons(&report, "instance", &upper), ["id-not-canonical"]);
    let before = tree(&target);
    let asking = MergeSelection {
        instance_ids: vec![upper],
        ..Default::default()
    };
    assert_eq!(
        code_of(merge_apply(&target, pack(&store), &asking)),
        "merge-selection-invalid"
    );
    assert_eq!(tree(&target), before);
}

/// A target that refuses writes says so at preview, not on the first write.
#[test]
fn a_target_opened_through_a_migration_is_refused_up_front() {
    let (_d, store) = origin(REPO);
    let target = copy_in_memory(&store).with_migration_notice(Some(
        srs_repository::store::MigrationNotice {
            source_revision: 9,
            migrations: vec!["rfc052-root-container".to_string()],
        },
    ));
    assert_eq!(
        code_of(merge_preview(&target, pack(&store), None)),
        "merge-target-migration-needed"
    );
}
