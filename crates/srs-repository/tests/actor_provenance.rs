//! RFC-046 actor provenance (srs-rust#1171): one test per row of the Change C operation
//! table (session actor, operation, expected `createdBy` or diagnostic), the [R10]-[R13]
//! revision rules, [R12] precedence, the forgery paths, [R6] preservation and the
//! revision 8 -> 9 migration.

use serde_json::{json, Value};
use srs_core::types::actor::Actor;
use srs_core::types::note::{Note, NoteSection};
use srs_core::types::record::{FieldValues, Record};
use srs_core::types::relation::Relation;
use srs_repository::field_type_migration_service::stamp_data_model_revision;
use srs_repository::record_store::{
    self, CreateRecordInput, CreateRecordSuccessorInput, UpdateRecordInput,
};
use srs_repository::relation_service::{self, RebuildPrecedesChainInput};
use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::services::{self, GraduateNoteInput};
use srs_repository::{migration_registry_service, validation, FileStore, RepositoryStore};
use std::collections::BTreeMap;
use std::io::Cursor;
use tempfile::TempDir;

const PURPOSE: &str = "com.semanticops.core/purpose";

fn agent() -> Value {
    json!({"kind": "ai", "id": "agent-1", "name": "Scribe"})
}

fn human() -> Value {
    json!({"kind": "human", "id": "user-7"})
}

fn as_actor(v: &Value) -> Actor {
    Actor::from_value(v).unwrap()
}

/// A fresh revision-9 file repository (its identity purpose record is created by the
/// session that opened it, so stamping of `repo create` is observable too).
fn new_repo(actor: Option<Value>) -> (TempDir, FileStore) {
    let dir = TempDir::new().unwrap();
    let store = FileStore::new(dir.path());
    store.set_session_actor(actor);
    create_repository_with_intent(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "1171aaaa-0000-4000-8000-000000000001".to_string(),
                namespace: "com.test.actor".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "1171bbbb-0000-4000-8000-000000000002".to_string(),
                namespace: "com.test.actor".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    (dir, store)
}

fn purpose_input(statement: &str) -> CreateRecordInput {
    let mut fv = FieldValues::new();
    fv.insert("statement", Value::String(statement.to_string()));
    CreateRecordInput {
        field_values: fv,
        field_meta: None,
        tags: None,
        lifecycle_state: None,
        extra: BTreeMap::new(),
    }
}

fn make_record(store: &dyn RepositoryStore, statement: &str) -> Record {
    record_store::create_record_in_context(
        store,
        PURPOSE,
        None,
        purpose_input(statement),
        None,
        None,
    )
    .unwrap()
    .record
}

fn note(title: &str) -> Note {
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
    }
}

fn make_note(store: &dyn RepositoryStore, title: &str) -> Note {
    services::create_note(store, note(title)).unwrap().note
}

fn relation(src: &str, dst: &str) -> Relation {
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
    }
}

fn code_of<T: std::fmt::Debug>(r: Result<T, srs_repository::error::RepositoryError>) -> String {
    r.unwrap_err().to_string()
}

fn snapshot(store: &dyn RepositoryStore) -> Vec<Value> {
    let mut out: Vec<Value> = store
        .list_instances(&srs_repository::index::InstanceQuery::default())
        .unwrap()
        .iter()
        .map(|i| {
            if i.tier == 0 {
                serde_json::to_value(store.load_note_by_id(&i.instance_id).unwrap()).unwrap()
            } else {
                serde_json::to_value(store.load_record_by_id(&i.instance_id).unwrap()).unwrap()
            }
        })
        .collect();
    out.sort_by_key(|v| v["instanceId"].as_str().unwrap().to_string());
    out
}

fn stored_record(store: &dyn RepositoryStore, id: &str) -> Record {
    store.load_record_by_id(id).unwrap()
}

// ── Row: create a record / note / relation ──────────────────────────────────

#[test]
fn create_record_note_relation_are_stamped_with_the_session_actor() {
    let (_d, store) = new_repo(Some(agent()));
    let r = make_record(&store, "one");
    let n = make_note(&store, "n");
    assert_eq!(r.created_by, Some(as_actor(&agent())));
    assert_eq!(n.created_by, Some(as_actor(&agent())));
    // Persisted, not just returned.
    assert_eq!(
        stored_record(&store, &r.instance_id).created_by,
        Some(as_actor(&agent()))
    );
    assert_eq!(
        store.load_note_by_id(&n.instance_id).unwrap().created_by,
        Some(as_actor(&agent()))
    );
    let rel =
        relation_service::create_relation_auto(&store, relation(&r.instance_id, &n.instance_id))
            .unwrap()
            .relation;
    assert_eq!(rel.created_by, Some(as_actor(&agent())));
    assert_eq!(
        relation_service::get_relation_by_id(&store, &rel.relation_id)
            .map(|g| match g {
                relation_service::GetRelationResult::Found(r) => r.created_by,
                _ => None,
            })
            .unwrap(),
        Some(as_actor(&agent()))
    );
}

#[test]
fn no_session_actor_creates_unattributed_and_never_errors() {
    let (_d, store) = new_repo(None);
    let r = make_record(&store, "one");
    let n = make_note(&store, "n");
    assert!(r.created_by.is_none() && n.created_by.is_none());
    let rel =
        relation_service::create_relation_auto(&store, relation(&r.instance_id, &n.instance_id))
            .unwrap()
            .relation;
    assert!(rel.created_by.is_none());
    // An unattributed session also writes a pre-9 corpus ([R11] never applies).
    stamp_data_model_revision(&store, 8).unwrap();
    make_note(&store, "again");
    // ...and is never refused by [R12] (no actor to be invalid).
    assert!(store.session_actor().is_none());
}

#[test]
fn repo_create_stamps_the_scaffolded_purpose_record() {
    let (_d, store) = new_repo(Some(human()));
    let manifest = store.load_manifest().unwrap();
    let identity = manifest.container.unwrap().identity_instance_id.unwrap();
    assert_eq!(
        stored_record(&store, &identity).created_by,
        Some(as_actor(&human()))
    );
}

// ── Row: successor ──────────────────────────────────────────────────────────

#[test]
fn successor_stamps_new_record_and_relation_not_the_predecessor() {
    let (_d, store) = new_repo(Some(human()));
    let pred = make_record(&store, "v1");
    store.set_session_actor(Some(agent()));
    let result = record_store::create_record_successor(
        &store,
        &pred.instance_id,
        CreateRecordSuccessorInput {
            relation_type: Some("supersedes".to_string()),
            field_values: purpose_input("v2").field_values,
            lifecycle_state: None,
            type_version: None,
            extra: BTreeMap::new(),
        },
    )
    .unwrap();
    assert_eq!(result.record.created_by, Some(as_actor(&agent())));
    assert_eq!(result.relation.created_by, Some(as_actor(&agent())));
    assert_eq!(
        stored_record(&store, &pred.instance_id).created_by,
        Some(as_actor(&human())),
        "the predecessor keeps its own createdBy"
    );
}

// ── Row: graduate ───────────────────────────────────────────────────────────

#[test]
fn graduate_stamps_record_and_derived_from_with_the_graduating_actor() {
    let (_d, store) = new_repo(Some(human()));
    let n = make_note(&store, "idea");
    store.set_session_actor(Some(agent()));
    let result = services::graduate_note(
        &store,
        GraduateNoteInput {
            note_id: n.instance_id.clone(),
            type_ref: PURPOSE.to_string(),
            type_version: None,
            record_input: purpose_input("graduated"),
            container_id: None,
        },
    )
    .unwrap();
    assert_eq!(result.record.created_by, Some(as_actor(&agent())));
    assert_eq!(
        result.note.created_by,
        Some(as_actor(&human())),
        "the note keeps its author; the stamp names the graduating actor"
    );
    let rels = store.list_relations().unwrap();
    let derived: Vec<_> = rels
        .iter()
        .filter(|r| r.relation_type == "derived-from")
        .collect();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].created_by, Some(as_actor(&agent())));
}

// ── Row: any other operation that writes a new relation ─────────────────────

#[test]
fn rebuild_precedes_chain_stamps_the_relations_it_writes() {
    let (_d, store) = new_repo(Some(agent()));
    let a = make_record(&store, "a");
    let b = make_record(&store, "b");
    let created = relation_service::rebuild_precedes_chain(
        &store,
        RebuildPrecedesChainInput {
            instance_ids: vec![a.instance_id.clone(), b.instance_id.clone()],
            clear_ids: vec![],
        },
    )
    .unwrap()
    .created;
    assert_eq!(created.len(), 1);
    let rels = store.list_relations().unwrap();
    let r = rels
        .iter()
        .find(|r| r.relation_id == created[0].relation_id)
        .unwrap();
    assert_eq!(r.created_by, Some(as_actor(&agent())));
}

// ── Row: update (preserve; identical allowed; different/new = actor-changed) ─

fn update_input(statement: &str, created_by: Option<Value>) -> UpdateRecordInput {
    let mut extra = BTreeMap::new();
    if let Some(v) = created_by {
        extra.insert("createdBy".to_string(), v);
    }
    UpdateRecordInput {
        field_values: purpose_input(statement).field_values,
        field_meta: None,
        tags: None,
        type_version: None,
        extra,
    }
}

#[test]
fn update_preserves_created_by_and_allows_identical_whole_object_round_trip() {
    let (_d, store) = new_repo(Some(agent()));
    let r = make_record(&store, "one");
    // A different session actor updating does not restamp.
    store.set_session_actor(Some(human()));
    let updated =
        record_store::update_record(&store, &r.instance_id, update_input("two", None)).unwrap();
    assert_eq!(updated.created_by, Some(as_actor(&agent())));
    // Whole-object round trip: the identical createdBy is accepted and not duplicated.
    let again =
        record_store::update_record(&store, &r.instance_id, update_input("three", Some(agent())))
            .unwrap();
    assert_eq!(again.created_by, Some(as_actor(&agent())));
    assert!(!again.extra.contains_key("createdBy"));
    let wire = serde_json::to_value(stored_record(&store, &r.instance_id)).unwrap();
    assert_eq!(wire["createdBy"], agent());
}

#[test]
fn update_with_a_different_or_new_created_by_is_actor_changed() {
    let (_d, store) = new_repo(Some(agent()));
    let attributed = make_record(&store, "one");
    let forged = record_store::update_record(
        &store,
        &attributed.instance_id,
        update_input("x", Some(human())),
    );
    assert!(code_of(forged).starts_with("actor-changed"));

    store.set_session_actor(None);
    let unattributed = make_record(&store, "two");
    let adds = record_store::update_record(
        &store,
        &unattributed.instance_id,
        update_input("y", Some(agent())),
    );
    assert!(code_of(adds).starts_with("actor-changed"));
    // Nothing written: still unattributed.
    assert!(stored_record(&store, &unattributed.instance_id)
        .created_by
        .is_none());
}

#[test]
fn note_update_preserves_identical_allows_changed_refuses() {
    let (_d, store) = new_repo(Some(agent()));
    let n = make_note(&store, "n");
    // Whole-object update without createdBy keeps the stored one.
    let mut edit = n.clone();
    edit.created_by = None;
    edit.title = Some("renamed".to_string());
    let kept = services::update_note(&store, edit).unwrap().note;
    assert_eq!(kept.created_by, Some(as_actor(&agent())));
    // Identical: allowed.
    let mut same = kept.clone();
    same.title = Some("again".to_string());
    assert!(services::update_note(&store, same).is_ok());
    // Different: refused, nothing written.
    let mut forged = kept.clone();
    forged.created_by = Some(as_actor(&human()));
    forged.title = Some("forged".to_string());
    assert!(code_of(services::update_note(&store, forged)).starts_with("actor-changed"));
    assert_eq!(
        store
            .load_note_by_id(&n.instance_id)
            .unwrap()
            .title
            .as_deref(),
        Some("again")
    );
}

// ── Requests never carry the actor (forgery paths) ──────────────────────────

#[test]
fn create_requests_carrying_created_by_are_actor_supplied_including_extras() {
    let (_d, store) = new_repo(Some(agent()));

    // Record via the flattened extras map.
    let mut input = purpose_input("forged");
    input.extra.insert("createdBy".to_string(), human());
    let err = code_of(record_store::create_record_in_context(
        &store, PURPOSE, None, input, None, None,
    ));
    assert!(err.starts_with("actor-supplied"), "{err}");

    // Successor via extras.
    let pred = make_record(&store, "p");
    let mut extra = BTreeMap::new();
    extra.insert("createdBy".to_string(), human());
    let err = code_of(record_store::create_record_successor(
        &store,
        &pred.instance_id,
        CreateRecordSuccessorInput {
            relation_type: Some("supersedes".to_string()),
            field_values: purpose_input("s").field_values,
            lifecycle_state: None,
            type_version: None,
            extra,
        },
    ));
    assert!(err.starts_with("actor-supplied"), "{err}");

    // Note and relation (typed createdBy).
    let mut n = note("n");
    n.created_by = Some(as_actor(&human()));
    assert!(code_of(services::create_note(&store, n)).starts_with("actor-supplied"));
    let mut rel = relation(&pred.instance_id, &pred.instance_id);
    rel.created_by = Some(as_actor(&human()));
    assert!(
        code_of(relation_service::create_relation_auto(&store, rel)).starts_with("actor-supplied")
    );

    // Supplied is refused even in an unattributed session.
    store.set_session_actor(None);
    let mut input = purpose_input("forged2");
    input.extra.insert("createdBy".to_string(), agent());
    assert!(code_of(record_store::create_record_in_context(
        &store, PURPOSE, None, input, None, None
    ))
    .starts_with("actor-supplied"));
}

#[test]
fn extras_on_update_cannot_smuggle_a_created_by_past_the_envelope_guard() {
    let (_d, store) = new_repo(None);
    let r = make_record(&store, "one");
    let err = code_of(record_store::update_record(
        &store,
        &r.instance_id,
        update_input("two", Some(json!({"kind": "ai", "id": "evil"}))),
    ));
    assert!(err.starts_with("actor-changed"), "{err}");
}

// ── [R12] invalid session actor + precedence ────────────────────────────────

#[test]
fn invalid_session_actor_refuses_every_creation_and_wins_precedence() {
    let (_d, store) = new_repo(None);
    let r = make_record(&store, "one");
    for bad in [
        json!({"kind": "ai", "id": ""}),
        json!({"kind": "robot", "id": "x"}),
        json!({"kind": "ai", "id": "x", "extra": true}),
        json!("someone"),
    ] {
        store.set_session_actor(Some(bad.clone()));
        let err = code_of(record_store::create_record_in_context(
            &store,
            PURPOSE,
            None,
            purpose_input("x"),
            None,
            None,
        ));
        assert!(err.starts_with("actor-invalid"), "{bad}: {err}");
        assert!(code_of(services::create_note(&store, note("n"))).starts_with("actor-invalid"));
        assert!(code_of(relation_service::create_relation_auto(
            &store,
            relation(&r.instance_id, &r.instance_id)
        ))
        .starts_with("actor-invalid"));
        // actor-invalid > actor-supplied.
        let mut input = purpose_input("x");
        input.extra.insert("createdBy".to_string(), agent());
        assert!(code_of(record_store::create_record_in_context(
            &store, PURPOSE, None, input, None, None
        ))
        .starts_with("actor-invalid"));
        // actor-invalid > revision-too-old.
        stamp_data_model_revision(&store, 8).unwrap();
        assert!(code_of(services::create_note(&store, note("n"))).starts_with("actor-invalid"));
        stamp_data_model_revision(&store, 9).unwrap();
    }
}

#[test]
fn actor_supplied_wins_over_revision_too_old() {
    let (_d, store) = new_repo(Some(agent()));
    stamp_data_model_revision(&store, 8).unwrap();
    let mut input = purpose_input("x");
    input.extra.insert("createdBy".to_string(), human());
    assert!(code_of(record_store::create_record_in_context(
        &store, PURPOSE, None, input, None, None
    ))
    .starts_with("actor-supplied"));
}

// ── [R11] revision-too-old; migration 8 -> 9; loads rev 8 and 9 ─────────────

#[test]
fn actor_session_in_a_pre_9_corpus_is_refused_until_migrated() {
    let (_d, store) = new_repo(Some(agent()));
    stamp_data_model_revision(&store, 8).unwrap();
    // The corpus still loads at revision 8 (this build supports 8 and 9).
    assert!(store.load_manifest().is_ok());
    let n_before = snapshot(&store).len();
    for r in [
        code_of(services::create_note(&store, note("n"))),
        code_of(record_store::create_record_in_context(
            &store,
            PURPOSE,
            None,
            purpose_input("x"),
            None,
            None,
        )),
    ] {
        assert!(r.starts_with("revision-too-old"), "{r}");
    }
    assert_eq!(snapshot(&store).len(), n_before, "nothing written");

    // Migration 8 -> 9: a pure re-stamp preserving every instance.
    let before = snapshot(&store);
    let statuses = migration_registry_service::list_migrations(&store).unwrap();
    let m = statuses
        .iter()
        .find(|m| m.id == "rfc046-actor-provenance")
        .unwrap();
    assert_eq!(
        m.status,
        migration_registry_service::MigrationStatus::Needed
    );
    migration_registry_service::apply_migration(&store, "rfc046-actor-provenance").unwrap();
    assert_eq!(
        srs_repository::field_type_migration_service::data_model_revision(&store).unwrap(),
        9
    );
    let after = snapshot(&store);
    assert_eq!(before, after, "[R6] migration preserves every instance");
    assert_eq!(
        make_note(&store, "ok now").created_by,
        Some(as_actor(&agent()))
    );
}

#[test]
fn created_by_in_a_pre_9_corpus_is_a_validation_error() {
    let (_d, store) = new_repo(Some(agent()));
    make_record(&store, "one");
    assert_eq!(
        validation::validate_repository(&store)
            .unwrap()
            .summary
            .errors,
        0
    );
    stamp_data_model_revision(&store, 8).unwrap();
    let report = validation::validate_repository(&store).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("RFC-046 [R10]")),
        "{:?}",
        report.diagnostics
    );
}

// ── [R6] preservation: copy, archive pack/unpack, .srsj export/load ─────────

#[test]
fn copy_archive_and_srsj_preserve_created_by_exactly() {
    let (_d, store) = new_repo(Some(agent()));
    let r = make_record(&store, "one");
    let n = make_note(&store, "n");
    let rel =
        relation_service::create_relation_auto(&store, relation(&r.instance_id, &n.instance_id))
            .unwrap()
            .relation;

    let check = |s: &dyn RepositoryStore, how: &str| {
        let want = Some(as_actor(&agent()));
        assert_eq!(
            s.load_record_by_id(&r.instance_id).unwrap().created_by,
            want,
            "{how}"
        );
        assert_eq!(
            s.load_note_by_id(&n.instance_id).unwrap().created_by,
            want,
            "{how}"
        );
        let rels = s.list_relations().unwrap();
        assert_eq!(
            rels.iter()
                .find(|x| x.relation_id == rel.relation_id)
                .unwrap()
                .created_by,
            want,
            "{how}"
        );
    };

    // copy
    let dir2 = TempDir::new().unwrap();
    let target = FileStore::new(dir2.path());
    srs_repository::repository_portability::copy_repository(&store, &target).unwrap();
    check(&target, "copy");

    // archive pack / unpack
    let mut buf = Cursor::new(Vec::new());
    srs_repository::archive_pack(&store, &mut buf).unwrap();
    let dir3 = TempDir::new().unwrap();
    let unpacked = FileStore::new(dir3.path());
    srs_repository::archive_unpack(Cursor::new(buf.into_inner()), &unpacked).unwrap();
    check(&unpacked, "archive");

    // .srsj export / load
    let srsj = srs_repository::srsj::to_srsj_string(&store).unwrap();
    let loaded = srs_repository::srsj::open_srsj(&srsj).unwrap();
    check(&loaded, ".srsj");
}

// ── Loads revision 8 and 9; migration ladder 8 -> 9 stays idempotent ────────

#[test]
fn migration_is_idempotent_and_refuses_below_revision_8() {
    let (_d, store) = new_repo(None);
    stamp_data_model_revision(&store, 8).unwrap();
    migration_registry_service::apply_migration(&store, "rfc046-actor-provenance").unwrap();
    migration_registry_service::apply_migration(&store, "rfc046-actor-provenance").unwrap();
    assert_eq!(
        srs_repository::field_type_migration_service::data_model_revision(&store).unwrap(),
        9
    );
    stamp_data_model_revision(&store, 7).unwrap();
    assert!(
        migration_registry_service::apply_migration(&store, "rfc046-actor-provenance").is_err()
    );
}

// ── [R13]: transport never writes into an existing corpus ───────────────────

#[test]
fn transport_into_an_existing_pre_9_corpus_is_refused_and_writes_nothing() {
    let (_d, source) = new_repo(Some(agent()));
    let r = make_record(&source, "stamped");
    let n = make_note(&source, "n");
    relation_service::create_relation_auto(&source, relation(&r.instance_id, &n.instance_id))
        .unwrap();

    let (_t, target) = new_repo(None);
    stamp_data_model_revision(&target, 8).unwrap();
    let before = snapshot(&target);
    let rels_before = target.list_relations().unwrap().len();

    // copy (snapshot import) into the existing store
    let e = srs_repository::repository_portability::copy_repository(&source, &target)
        .unwrap_err()
        .to_string();
    assert!(e.to_lowercase().contains("not empty"), "{e}");

    // archive unpack into the existing store
    let mut buf = Cursor::new(Vec::new());
    srs_repository::archive_pack(&source, &mut buf).unwrap();
    assert!(srs_repository::archive_unpack(Cursor::new(buf.into_inner()), &target).is_err());

    // .srsj: copying into an existing store backed by a loaded document is refused too
    let srsj = srs_repository::srsj::to_srsj_string(&target).unwrap();
    let existing = srs_repository::srsj::open_srsj(&srsj).unwrap();
    assert!(srs_repository::repository_portability::copy_repository(&source, &existing).is_err());

    assert_eq!(snapshot(&target), before, "nothing written");
    assert_eq!(target.list_relations().unwrap().len(), rels_before);
    assert!(before.iter().all(|v| v.get("createdBy").is_none()));
}

// ── malformed createdBy on create is actor-supplied, after actor-invalid ────

#[test]
fn malformed_created_by_on_raw_create_input_is_actor_supplied_not_a_parse_error() {
    let (_d, store) = new_repo(Some(agent()));
    let raw = json!({"createdBy": "x"});
    let e =
        srs_repository::actor_service::reject_supplied_created_by(&store, raw.as_object().unwrap())
            .unwrap_err()
            .to_string();
    assert!(e.starts_with("actor-supplied"), "{e}");
    // absent key: no-op
    assert!(srs_repository::actor_service::reject_supplied_created_by(
        &store,
        json!({}).as_object().unwrap()
    )
    .is_ok());
    // invalid session actor wins; too-old comes last
    store.set_session_actor(Some(json!({"kind": "ai", "id": ""})));
    assert!(srs_repository::actor_service::reject_supplied_created_by(
        &store,
        raw.as_object().unwrap()
    )
    .unwrap_err()
    .to_string()
    .starts_with("actor-invalid"));
    store.set_session_actor(Some(agent()));
    stamp_data_model_revision(&store, 8).unwrap();
    assert!(srs_repository::actor_service::reject_supplied_created_by(
        &store,
        raw.as_object().unwrap()
    )
    .unwrap_err()
    .to_string()
    .starts_with("actor-supplied"));
}
