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
use srs_repository::store::memory::{FailPoint, MemoryStore};
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

/// Round-4 review finding: a diagnostic shelved as `Ambiguous` can become
/// resolvable because of a LATER, unrelated repair in the same `--fix`
/// pass. Here the relation that blocks adopt is itself dangling (its other
/// endpoint doesn't resolve), so it gets deleted as a separate
/// `DanglingRelationEndpoint` repair — which removes the very reference
/// that made the duplicate ambiguous. The fix loop must retry and adopt it
/// in the same pass, not leave it shelved until a second `--fix` call, and
/// the final report must show only the `Repaired` verdict, not a stale
/// `Ambiguous` next to it.
#[test]
fn adopt_retries_and_succeeds_after_a_later_repair_removes_its_blocking_reference() {
    let store = MemoryStore::empty();
    plant_duplicate(&store);
    let relation_id = "eeeeeeee-0000-4000-8000-000000000e08";
    store
        .save_relation(&relation(
            relation_id,
            DUP_ID,
            "aaaaaaaa-0000-4000-8000-0000000000ff", // dangling: no such instance
        ))
        .unwrap();

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();

    let adopt_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.class == DoctorClass::DuplicateInstanceId)
        .collect();
    assert_eq!(
        adopt_findings.len(),
        1,
        "exactly one verdict per diagnostic in the final report, not a stale Ambiguous next to \
         the Repaired that superseded it: {:?}",
        report.findings
    );
    assert_eq!(
        adopt_findings[0].outcome,
        DoctorOutcome::Repaired,
        "adopt must retry and succeed once the relation blocking it is deleted in the same \
         --fix pass: {:?}",
        report.findings
    );

    let relation_finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DanglingRelationEndpoint)
        .expect("the dangling relation must be reported");
    assert_eq!(relation_finding.outcome, DoctorOutcome::Repaired);

    let cat = store
        .catalog()
        .expect("repository must load clean after both repairs");
    assert_eq!(cat.instances.len(), 2);
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

/// A relation with BOTH endpoints dangling emits one `DANGLING_REFERENCE`
/// diagnostic per endpoint (`catalog.rs::resolve_references`), so removing
/// it must count as exactly one repair, not two — regression for round-2
/// review finding #3. The rebuild-between-repairs loop in `doctor()` makes
/// this automatic: once the first diagnostic's repair deletes the relation,
/// the second diagnostic (same relation, other endpoint) never reappears in
/// the rebuilt catalog, so it is never even visited.
#[test]
fn a_relation_with_both_endpoints_dangling_is_repaired_exactly_once() {
    let store = MemoryStore::empty();
    let relation_id = "eeeeeeee-0000-4000-8000-000000000e05";
    store
        .save_relation(&relation(
            relation_id,
            "cccccccc-0000-4000-8000-0000000000f1", // dangling
            "cccccccc-0000-4000-8000-0000000000f2", // dangling
        ))
        .unwrap();

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let matching: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.class == DoctorClass::DanglingRelationEndpoint)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "one relation removal must produce exactly one finding, not one per dangling endpoint: {:?}",
        report.findings
    );
    assert_eq!(matching[0].outcome, DoctorOutcome::Repaired);
    assert_eq!(report.repaired, 1);
    assert!(store.load_relation(relation_id).is_err());
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

/// Round-3 review finding: `store.save_relation` overwrites unconditionally
/// by design, so a filename/id mismatch whose rename TARGET already holds
/// its own legitimate content must not be auto-repaired — that would
/// silently destroy the occupant. This is really a relation-id collision
/// (both files declare the same `relationId`), which is the relation
/// duplicate-id case doctor already leaves as a manual step everywhere
/// else; the filename-mismatch repair must reach the same conclusion
/// rather than blindly clobbering.
#[test]
fn relation_filename_mismatch_does_not_clobber_an_existing_relation_at_the_target_name() {
    let store = MemoryStore::empty();
    let a = "dddddddd-0000-4000-8000-000000000d04";
    let b = "dddddddd-0000-4000-8000-000000000d05";
    store.save_note(&note(a)).unwrap();
    store.save_note(&note(b)).unwrap();
    let real_id = "eeeeeeee-0000-4000-8000-000000000e06";
    store.save_relation(&relation(real_id, a, b)).unwrap();
    let real_content = store
        .load_relations_json(&format!("relations/{real_id}.json"))
        .unwrap();

    // A second, wrongly-named file that ALSO declares relationId == real_id
    // — a plausible bad copy: a filename/id mismatch AND a same-id
    // collision with the file `save_relation` would otherwise clobber.
    store
        .save_relations_json("relations/wrong-name.json", &real_content)
        .unwrap();

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let mismatch = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::RelationFilenameMismatch)
        .expect("the filename mismatch must be reported");
    assert_eq!(
        mismatch.outcome,
        DoctorOutcome::Ambiguous,
        "a rename that would overwrite an existing relation must not proceed silently: {:?}",
        report.findings
    );

    let still_there = store
        .load_relations_json(&format!("relations/{real_id}.json"))
        .unwrap();
    assert_eq!(
        still_there, real_content,
        "the real relation's content must survive untouched"
    );
}

/// Round-3 review finding: `instance_id_referenced` (adopt's safety check)
/// must classify a relation the same way `catalog.rs` does — `$schema`
/// stripped if present, never required — or a catalog-valid, schema-less
/// relation naming the duplicate id would be silently skipped, letting
/// adopt proceed on an id that genuinely has an incoming reference.
#[test]
fn adopt_detects_a_reference_from_a_relation_missing_the_schema_property() {
    let store = MemoryStore::empty();
    // Checked writes first — both would fail on an already-bricked catalog.
    let other = "aaaaaaaa-0000-4000-8000-0000000000a3";
    store.save_note(&note(other)).unwrap();
    plant_duplicate(&store);

    let relation_id = "eeeeeeee-0000-4000-8000-000000000e07";
    // Deliberately no "$schema" key — `classify_relations_file` accepts
    // this (strips $schema only if present), `relation_object_from_value`
    // would reject it outright.
    let raw = json!({
        "relationId": relation_id,
        "relationType": "depends-on",
        "sourceInstanceId": other,
        "targetInstanceId": DUP_ID,
    });
    store
        .save_relations_json(&format!("relations/{relation_id}.json"), &raw)
        .unwrap();

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::DuplicateInstanceId)
        .expect("duplicate must still be reported");
    assert_eq!(
        finding.outcome,
        DoctorOutcome::Ambiguous,
        "a schema-less but catalog-valid relation referencing the duplicate id must still \
         block adopt: {:?}",
        report.findings
    );
}

/// Two faults on ONE relation file: a filename/id mismatch *and* a dangling
/// endpoint (a plausible single hand-rename of a relation whose target was
/// later deleted). Regression for round-2 review finding #1: fixing the
/// rename first must not leave the dangling-endpoint repair acting on the
/// pre-rename locator (which `store.delete_relations_json` would silently
/// no-op on, since it is idempotent on a missing path) — `doctor()` rebuilds
/// the catalog between repairs precisely so the second repair sees the
/// relation at its now-correct path.
#[test]
fn a_relation_with_both_a_filename_mismatch_and_a_dangling_endpoint_ends_up_fully_repaired() {
    let store = MemoryStore::empty();
    let survivor = "dddddddd-0000-4000-8000-000000000d03";
    store.save_note(&note(survivor)).unwrap();
    let relation_id = "eeeeeeee-0000-4000-8000-000000000e04";
    store
        .save_relation(&relation(
            relation_id,
            survivor,
            "dddddddd-0000-4000-8000-0000000000ff", // dangling: no such instance
        ))
        .unwrap();

    // Hand rename, as `repairs_a_relation_filename_id_mismatch` does.
    let value = store
        .load_relations_json(&format!("relations/{relation_id}.json"))
        .unwrap();
    store
        .save_relations_json("relations/also-wrong-name.json", &value)
        .unwrap();
    store
        .delete_relations_json(&format!("relations/{relation_id}.json"))
        .unwrap();

    store.catalog().expect_err(
        "a relation with a filename mismatch AND a dangling endpoint must brick the checked load",
    );

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.class == DoctorClass::RelationFilenameMismatch
                && f.outcome == DoctorOutcome::Repaired),
        "{:?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.class == DoctorClass::DanglingRelationEndpoint
                && f.outcome == DoctorOutcome::Repaired),
        "{:?}",
        report.findings
    );

    // The whole point: the repo actually loads clean afterward, and the
    // relation (unresolvable no matter which file held it) is truly gone —
    // not silently left dangling under its now-correct name because the
    // second repair no-op'd against a locator that no longer existed.
    store
        .catalog()
        .expect("repository must load clean — this is the failure mode the test pins");
    assert!(
        store.load_relation(relation_id).is_err(),
        "the relation must actually be gone, not merely renamed and still dangling"
    );
}

// ---------------------------------------------------------------------------
// Class: retired manifest keys (RFC-038 Change K) — file-tree stores only,
// same pre-existing constraint as `repo apply-migration --id rfc038-storage`.
// ---------------------------------------------------------------------------

/// Round-5 review finding: a dry-run preview must match what `--fix` will
/// actually do on every store, including one where `is_file_tree_store()`
/// is `false` (only `MemoryStore` today — every production caller uses
/// `FileStore`, which always answers `true`). Before the fix this branch
/// claimed `WouldRepair` under dry run and then `ManualStep` under `--fix`
/// for the identical store — the same "dry run oversells `--fix`" failure
/// mode round 1/3 fixed elsewhere, missed here.
#[test]
fn retired_manifest_keys_dry_run_matches_fix_on_a_non_file_tree_store() {
    let store = MemoryStore::empty();
    let mut manifest = store.load_manifest().unwrap();
    manifest
        .extra
        .insert("instanceIndex".to_string(), json!([]));
    store.save_manifest(&manifest).unwrap();

    let dry = doctor(&store, DoctorInput { fix: false }).unwrap();
    let fixed = doctor(&store, DoctorInput { fix: true }).unwrap();
    let dry_outcome = dry
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::RetiredManifestKeys)
        .unwrap()
        .outcome;
    let fix_outcome = fixed
        .findings
        .iter()
        .find(|f| f.class == DoctorClass::RetiredManifestKeys)
        .unwrap()
        .outcome;
    assert_eq!(
        dry_outcome, fix_outcome,
        "dry run must preview the same outcome --fix actually produces"
    );
    assert_eq!(fix_outcome, DoctorOutcome::ManualStep);
}

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

/// Round-3 review finding: a store I/O failure on the fix loop's catalog
/// rebuild must not propagate as `Err` and discard everything already
/// recorded in the report — `check_manifest_raw` runs (and records its
/// finding) *before* the loop, at a separate call site the injected fault
/// does not touch, so that finding surviving proves the fault handler
/// returns `Ok(report)` with prior progress intact rather than propagating.
#[test]
fn a_mid_loop_catalog_rebuild_failure_does_not_discard_earlier_findings() {
    let store = MemoryStore::empty();
    let mut manifest = store.load_manifest().unwrap();
    manifest
        .extra
        .insert("instanceIndex".to_string(), json!([]));
    store.save_manifest(&manifest).unwrap();

    // Fires on the fix loop's first `catalog_unchecked()` rebuild, which
    // runs strictly after `check_manifest_raw` has already pushed its
    // finding into the report.
    store.arm_fail_at(FailPoint::CatalogUnchecked);

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();

    assert!(
        report
            .findings
            .iter()
            .any(|f| f.class == DoctorClass::RetiredManifestKeys),
        "the finding recorded before the injected failure must survive: {:?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.outcome == DoctorOutcome::ManualStep
                && f.message.contains("failed to rebuild the catalog")),
        "the injected failure must be named in the report, not silently swallowed or \
         propagated as Err: {:?}",
        report.findings
    );
}

/// A manifest fault `check_manifest_raw`'s two named classes (retired keys,
/// unsupported generation) do not classify — here, an `upstreamPackage`
/// object missing required fields, which is valid JSON but fails the typed
/// `Manifest` deserialize `catalog::build` performs internally. Regression
/// for round-2 review finding #2: doctor must never report a repository
/// clean (zero findings) when it is actually completely unloadable for a
/// reason outside those two classes.
#[test]
fn a_manifest_fault_outside_the_two_named_classes_is_still_reported() {
    let (tmp, store) = file_store();
    let manifest_path = tmp.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    // Missing namespace/name/version/installedAt — UpstreamPackage requires
    // all five; the typed load fails, the raw JSON parse does not.
    manifest["upstreamPackage"] = json!({"packageId": "not-a-real-package"});
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();

    store
        .load_manifest()
        .expect_err("the fixture must actually be unloadable through the typed path");

    let report = doctor(&store, DoctorInput { fix: true }).unwrap();
    assert!(
        !report.findings.is_empty(),
        "doctor must never report a repository clean when catalog::build's internal manifest \
         load failed for any reason, not only the two named classes"
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.class == DoctorClass::Unrepaired
                && f.outcome == DoctorOutcome::ManualStep
                && f.locators.iter().any(|l| l.contains("manifest.json"))),
        "{:?}",
        report.findings
    );
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
