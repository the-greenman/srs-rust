//! `srs repo doctor` (srs-rust#857) — the repair inventory, red-then-green
//! per class, dry-run-changes-nothing, and cross-store agreement.
//!
//! Fixtures are built with raw writes (`save_instance_json`/`save_relations_json`/
//! `save_container_unchecked`) precisely because doctor's whole job is to repair
//! damage that never went through a validating service call — the "copy a
//! record", "hand-edit a container", "rename a relation file" acts a `srs-usage.md`
//! forbids but that happen anyway (srs-rust#857, srs#397).

use serde_json::json;
use srs_core::types::container::Container;
use srs_core::types::note::Note;
use srs_core::types::relation::Relation;
use srs_repository::doctor_service::{doctor, DoctorClass, DoctorInput, DoctorOutcome};
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::memory::MemoryStore;
use srs_repository::{container_service, FileStore, RepositoryStore};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;

fn init_input() -> InitializeRepositoryInput {
    InitializeRepositoryInput {
        repository: RepositoryMetadata {
            repository_id: "0857aaaa-0000-4000-8000-000000000001".to_string(),
            namespace: "com.test.doctor".to_string(),
            srs_version: "2.0-draft".to_string(),
            title: None,
            description: None,
        },
        primary_package: PrimaryPackageMetadata {
            id: "0857bbbb-0000-4000-8000-000000000002".to_string(),
            namespace: "com.test.doctor".to_string(),
            name: "primary".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn file_store() -> (TempDir, FileStore) {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    create_repository(&store, &init_input()).unwrap();
    (tmp, store)
}

fn note_value(id: &str) -> serde_json::Value {
    json!({"instanceId": id, "sections": [{"name": "body", "content": "x"}]})
}

fn note(id: &str) -> Note {
    Note {
        instance_id: id.to_string(),
        title: Some("Note".to_string()),
        tags: None,
        sections: vec![],
        graduated_at: None,
        source_refs: None,
        created_at: None,
        updated_at: None,
        meta: None,
    }
}

fn container(id: &str, title: &str) -> Container {
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
        extra: BTreeMap::new(),
    }
}

fn relation(id: &str, source: &str, target: &str) -> Relation {
    Relation {
        relation_id: id.to_string(),
        relation_type: "depends-on".to_string(),
        source_instance_id: source.to_string(),
        target_instance_id: target.to_string(),
        asserted_by: None,
        confidence: None,
        created_at: None,
        created_by: None,
        status: None,
        valid_from: None,
        valid_until: None,
        notes: None,
        source_refs: None,
        source_repository_id: None,
        target_repository_id: None,
        meta: None,
    }
}

/// Walk the whole tree into (relative path, bytes) pairs — the dry-run
/// byte-compare oracle.
fn snapshot_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Class: duplicate instance id ("adopt")
// ---------------------------------------------------------------------------

const DUP_ID: &str = "aaaaaaaa-0000-4000-8000-0000000000a1";

fn plant_duplicate(store: &dyn RepositoryStore) {
    store.ensure_instance_dir("records").unwrap();
    store
        .save_instance_json("records/a.json", &note_value(DUP_ID))
        .unwrap();
    store
        .save_instance_json("records/b.json", &note_value(DUP_ID))
        .unwrap();
}

fn run_adopt_suite(store: &dyn RepositoryStore) {
    plant_duplicate(store);
    store
        .catalog()
        .expect_err("a duplicate instanceId must brick the checked load ([R12]/[R24])");

    // Dry run: reports, changes nothing.
    let report = doctor(store, DoctorInput { fix: false }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DuplicateInstanceId)
        .expect("duplicate instance id must be reported");
    assert_eq!(finding.outcome, DoctorOutcome::WouldRepair);
    assert_eq!(report.repaired, 0);
    store
        .catalog()
        .expect_err("dry run must not have repaired anything");

    // --fix: adopts the duplicate.
    let report = doctor(store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DuplicateInstanceId)
        .unwrap();
    assert_eq!(finding.outcome, DoctorOutcome::Repaired);
    assert_eq!(report.repaired, 1);

    let cat = store
        .catalog()
        .expect("repository must load clean after adopt");
    assert_eq!(
        cat.instances.len(),
        2,
        "both records must survive, under distinct ids"
    );

    // "a.json" is the deterministic keeper (locators sort first); "b.json"
    // was reminted — content preserved, id changed.
    let a = store.load_instance_json("records/a.json").unwrap();
    assert_eq!(a["instanceId"], DUP_ID);
    let b = store.load_instance_json("records/b.json").unwrap();
    let new_id = b["instanceId"].as_str().unwrap();
    assert_ne!(new_id, DUP_ID, "the duplicate must have a fresh id");
    assert_eq!(
        b["sections"], a["sections"],
        "adopt preserves content, only the id changes"
    );
}

#[test]
fn memory_store_adopts_a_verbatim_duplicate() {
    run_adopt_suite(&MemoryStore::empty());
}

#[test]
fn file_store_adopts_a_verbatim_duplicate() {
    let (_tmp, store) = file_store();
    run_adopt_suite(&store);
}

/// Stop condition (srs-rust#857): a duplicate id with an incoming relation
/// reference is genuinely undecidable — which physical file did the
/// relation mean? Doctor must not guess; it reports `Ambiguous` and leaves
/// both copies untouched even under `--fix`.
#[test]
fn adopt_is_ambiguous_when_the_duplicate_id_has_an_incoming_relation() {
    let store = MemoryStore::empty();
    // Written through the checked, typed API *before* the duplicate lands —
    // both would fail on an already-bricked catalog, and that ordering is
    // exactly right: the relation has to predate the damage to be the thing
    // doctor cannot disambiguate.
    let other = "aaaaaaaa-0000-4000-8000-0000000000a2";
    store.save_note(&note(other)).unwrap();
    store
        .save_relation(&relation(
            "eeeeeeee-0000-4000-8000-000000000e01",
            other,
            DUP_ID,
        ))
        .unwrap();
    plant_duplicate(&store);

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DuplicateInstanceId)
        .expect("duplicate must still be reported");
    assert_eq!(finding.outcome, DoctorOutcome::Ambiguous);
    assert_eq!(report.repaired, 0);

    // Untouched: both copies still declare the same shared id.
    let a = store.load_instance_json("records/a.json").unwrap();
    let b = store.load_instance_json("records/b.json").unwrap();
    assert_eq!(a["instanceId"], DUP_ID);
    assert_eq!(b["instanceId"], DUP_ID);
}

// ---------------------------------------------------------------------------
// Class: dangling container membership (the srs-rust#834 shape)
// ---------------------------------------------------------------------------

const GHOST_ID: &str = "bbbbbbbb-0000-4000-8000-000000000b01";
const SECTION_ID: &str = "bbbbbbbb-0000-4000-8000-000000000b02";
const SURVIVOR_ID: &str = "bbbbbbbb-0000-4000-8000-000000000b03";

fn run_dangling_membership_suite(store: &dyn RepositoryStore) {
    store.save_note(&note(SURVIVOR_ID)).unwrap();
    let mut section = container(SECTION_ID, "Section");
    section.member_instance_ids = Some(vec![GHOST_ID.to_string(), SURVIVOR_ID.to_string()]);
    // ADR-045: constructing an already-incoherent container is the sanctioned
    // way to build this fixture — the repair seam standing in for external
    // damage, not a widening of it.
    store.save_container_unchecked(&section).unwrap();

    store
        .catalog()
        .expect_err("a dangling container reference must brick the checked load ([R13]/[R24])");

    let report = doctor(store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DanglingContainerMembership)
        .expect("dangling membership must be reported");
    assert_eq!(finding.outcome, DoctorOutcome::Repaired);

    store
        .catalog()
        .expect("repository must load clean after the membership repair");
    let members = container_service::get_container(store, SECTION_ID)
        .unwrap()
        .member_instance_ids
        .unwrap_or_default();
    assert_eq!(members, vec![SURVIVOR_ID.to_string()]);
}

#[test]
fn memory_store_repairs_dangling_container_membership() {
    run_dangling_membership_suite(&MemoryStore::empty());
}

#[test]
fn file_store_repairs_dangling_container_membership() {
    let (_tmp, store) = file_store();
    run_dangling_membership_suite(&store);
}

// ---------------------------------------------------------------------------
// Class: dangling relation endpoint
// ---------------------------------------------------------------------------

#[test]
fn repairs_a_dangling_relation_endpoint_by_deleting_the_relation() {
    let store = MemoryStore::empty();
    let survivor = "cccccccc-0000-4000-8000-000000000c01";
    store.save_note(&note(survivor)).unwrap();
    store
        .save_relation(&relation(
            "eeeeeeee-0000-4000-8000-000000000e02",
            survivor,
            "cccccccc-0000-4000-8000-0000000000ff",
        ))
        .unwrap();

    store
        .catalog()
        .expect_err("a dangling relation endpoint must brick the checked load ([R13]/[R24])");

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DanglingRelationEndpoint)
        .expect("dangling relation endpoint must be reported");
    assert_eq!(finding.outcome, DoctorOutcome::Repaired);

    store
        .catalog()
        .expect("repository must load clean after the relation is removed");
    assert!(store
        .load_relation("eeeeeeee-0000-4000-8000-000000000e02")
        .is_err());
}

// ---------------------------------------------------------------------------
// Class: relation filename <-> id mismatch ([R11])
// ---------------------------------------------------------------------------

#[test]
fn repairs_a_relation_filename_id_mismatch() {
    let store = MemoryStore::empty();
    let a = "dddddddd-0000-4000-8000-000000000d01";
    let b = "dddddddd-0000-4000-8000-000000000d02";
    store.save_note(&note(a)).unwrap();
    store.save_note(&note(b)).unwrap();
    store
        .save_relation(&relation("eeeeeeee-0000-4000-8000-000000000e03", a, b))
        .unwrap();

    // Simulate a hand rename: move the correctly-named object to a
    // wrong-named file, as a manual `mv` would.
    let value = store
        .load_relations_json("relations/eeeeeeee-0000-4000-8000-000000000e03.json")
        .unwrap();
    store
        .save_relations_json("relations/wrong-name.json", &value)
        .unwrap();
    store
        .delete_relations_json("relations/eeeeeeee-0000-4000-8000-000000000e03.json")
        .unwrap();

    store
        .catalog()
        .expect_err("a relation filename/id mismatch must brick the checked load ([R11]/[R24])");

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::RelationFilenameMismatch)
        .expect("filename mismatch must be reported");
    assert_eq!(finding.outcome, DoctorOutcome::Repaired);

    let cat = store
        .catalog()
        .expect("repository must load clean after the rename");
    assert_eq!(cat.relations.len(), 1);
    assert!(store
        .load_relation("eeeeeeee-0000-4000-8000-000000000e03")
        .is_ok());
    assert!(store
        .load_relations_json("relations/wrong-name.json")
        .is_err());
}

// ---------------------------------------------------------------------------
// Class: retired manifest keys (RFC-038 Change K) — file-tree stores only,
// same pre-existing constraint as `repo apply-migration --id rfc038-storage`.
// ---------------------------------------------------------------------------

#[test]
fn repairs_retired_manifest_keys_via_the_rfc038_storage_transform() {
    let (_tmp, store) = file_store();
    let mut manifest = store.load_manifest().unwrap();
    // load_manifest returns the typed struct; go through save_manifest with
    // a raw injected key the typed shape has no field for, mirroring a
    // pre-Phase-6 corpus untouched by the storage migration.
    manifest
        .extra
        .insert("instanceIndex".to_string(), json!([]));
    store.save_manifest(&manifest).unwrap();

    let report = doctor(&store, DoctorInput { fix: false }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::RetiredManifestKeys)
        .expect("retired manifest key must be reported");
    assert_eq!(finding.outcome, DoctorOutcome::WouldRepair);

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::RetiredManifestKeys)
        .unwrap();
    assert_eq!(finding.outcome, DoctorOutcome::Repaired);

    let raw = store.load_manifest_raw_text().unwrap();
    let raw: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        raw.get("instanceIndex").is_none(),
        "retired key must be stripped: {raw}"
    );
    // The checked load now succeeds too.
    store
        .load_manifest()
        .expect("manifest must load cleanly through the checked path after the strip");
}

/// A retired manifest key makes `catalog::build`'s own internal manifest
/// read fail, so every catalog-derived finding (here: dangling container
/// membership) is invisible to a dry run — the dry-run report says so
/// explicitly rather than silently undercounting. A single `--fix` pass,
/// though, strips the manifest key *and* sees (and repairs) the
/// now-visible catalog-derived fault in the same call, because the raw
/// manifest repair runs before `catalog_unchecked()` is built.
#[test]
fn a_pending_manifest_fix_hides_catalog_findings_from_dry_run_but_fix_clears_both_in_one_pass() {
    let (_tmp, store) = file_store();

    // Plant the container fault first — `save_container_unchecked` still
    // resolves the manifest through the checked path internally, so it must
    // land before the manifest itself is bricked.
    let mut section = container(SECTION_ID, "Section");
    section.member_instance_ids = Some(vec![GHOST_ID.to_string()]);
    store.save_container_unchecked(&section).unwrap();

    let mut manifest = store.load_manifest().unwrap();
    manifest
        .extra
        .insert("instanceIndex".to_string(), json!([]));
    store.save_manifest(&manifest).unwrap();

    // Dry run: only the manifest-level finding is visible, and it says so.
    let report = doctor(&store, DoctorInput { fix: false }).unwrap();
    assert_eq!(
        report.findings.len(),
        1,
        "the dangling-membership finding must be hidden behind the manifest fault: {:?}",
        report.findings
    );
    let manifest_finding = &report.findings[0];
    assert_eq!(manifest_finding.class, DoctorClass::RetiredManifestKeys);
    assert!(
        manifest_finding.detail.contains("invisible")
            || manifest_finding.detail.contains("cannot show"),
        "the dry-run detail must warn that more findings may be hidden: {}",
        manifest_finding.detail
    );

    // --fix: both are cleared in the same pass.
    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    assert_eq!(
        report.repaired, 2,
        "one --fix pass must clear both the manifest and the now-visible container fault: {:?}",
        report.findings
    );
    store
        .catalog()
        .expect("repository must load clean after a single --fix pass");
}

// ---------------------------------------------------------------------------
// Class: malformed candidate — report only, never guessed, never touched.
// ---------------------------------------------------------------------------

#[test]
fn malformed_candidate_is_reported_but_never_touched() {
    let (tmp, store) = file_store();
    let bad_path = tmp.path().join("records/bad.json");
    std::fs::create_dir_all(bad_path.parent().unwrap()).unwrap();
    std::fs::write(&bad_path, b"{not valid json").unwrap();
    let before = std::fs::read(&bad_path).unwrap();

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.locators.iter().any(|l| l.contains("bad.json")))
        .expect("the malformed candidate must be reported");
    assert_eq!(finding.class, DoctorClass::Unrepaired);
    assert_eq!(finding.outcome, DoctorOutcome::ManualStep);

    let after = std::fs::read(&bad_path).unwrap();
    assert_eq!(
        before, after,
        "doctor must never guess or touch malformed content"
    );
}

// ---------------------------------------------------------------------------
// Dry-run changes nothing — byte-compare the whole tree.
// ---------------------------------------------------------------------------

#[test]
fn dry_run_leaves_the_tree_byte_identical() {
    let (tmp, store) = file_store();
    plant_duplicate(&store);
    let mut section = container(SECTION_ID, "Section");
    section.member_instance_ids = Some(vec![GHOST_ID.to_string()]);
    store.save_container_unchecked(&section).unwrap();

    let before = snapshot_tree(tmp.path());
    let report = doctor(&store, DoctorInput { fix: false }).unwrap();
    assert!(
        !report.findings.is_empty(),
        "the fixture must actually have something to report"
    );
    assert_eq!(report.repaired, 0, "dry run must never repair");
    let after = snapshot_tree(tmp.path());
    assert_eq!(
        before, after,
        "dry run must not change a single byte on disk"
    );
}

#[test]
fn full_cycle_dry_run_then_fix_is_idempotent() {
    let (_tmp, store) = file_store();
    plant_duplicate(&store);
    let report1 = doctor(&store, DoctorInput { fix: true }).unwrap();
    assert_eq!(report1.repaired, 1);
    let report2 = doctor(&store, DoctorInput { fix: true }).unwrap();
    assert_eq!(
        report2.findings.len(),
        0,
        "a clean repository must have nothing left to report: {:?}",
        report2.findings
    );
}
