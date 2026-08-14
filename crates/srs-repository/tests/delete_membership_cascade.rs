//! srs-rust#834 — an instance delete must not leave the repository unloadable.
//!
//! Before the fix, `record delete` removed the instance and its incident
//! relations but left its id in Container `memberInstanceIds` / `rootInstanceIds`.
//! The command reported success and the next open failed:
//! `SRS038-R13-DANGLING-REFERENCE`, which RFC-038 [R24] makes fatal.
//!
//! RFC-038 Change F classifies removing a deleted member from a container as an
//! *explicit container-membership operation*, so it may write `manifest.json`
//! where [R22] forbids routine writes from doing so.
//!
//! The reproduction runs against both `MemoryStore` and `FileStore`: they share
//! the service path but enumerate instances differently, and CLAUDE.md requires a
//! cross-store test for a new service behaviour. The ordering test is
//! `MemoryStore`-only — fault injection (`FailPoint`) exists there alone.

use srs_core::types::container::Container;
use srs_core::types::note::Note;
use srs_core::types::record::{FieldValues, Record};
use srs_repository::{
    container_service,
    error::RepositoryError,
    record_store,
    repository_lifecycle::{
        create_repository, create_repository_with_intent, InitializeRepositoryInput,
        PrimaryPackageMetadata, RepositoryMetadata,
    },
    repository_navigation_service, services,
    store::memory::{FailPoint, MemoryStore},
    validation, FileStore, RepositoryStore,
};
use std::collections::BTreeMap;
use tempfile::TempDir;

const RECORD_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const NOTE_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const KEEPER_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const ROOT_CONTAINER_ID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const SECTION_CONTAINER_ID: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
/// Holds the doomed record and nothing else — pins the emptied-array collapse.
const SOLE_CONTAINER_ID: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";

fn init_input() -> InitializeRepositoryInput {
    InitializeRepositoryInput {
        repository: RepositoryMetadata {
            repository_id: "0834aaaa-0000-4000-8000-000000000001".to_string(),
            namespace: "com.test.cascade".to_string(),
            srs_version: "2.0-draft".to_string(),
            title: None,
            description: None,
        },
        primary_package: PrimaryPackageMetadata {
            id: "0834bbbb-0000-4000-8000-000000000002".to_string(),
            namespace: "com.test.cascade".to_string(),
            name: "primary".to_string(),
            version: "1.0.0".to_string(),
        },
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

/// A repository whose root container ([R1], inline in the manifest) and one
/// file-backed container both name the doomed instances.
fn populate(store: &dyn RepositoryStore) {
    store
        .save_record(&Record {
            field_meta: None,
            instance_id: RECORD_ID.to_string(),
            type_id: "00000000-0000-4000-8000-000000000001".to_string(),
            type_version: 1,
            type_namespace: "com.test.cascade".to_string(),
            type_name: "thing".to_string(),
            field_values: FieldValues::new(),
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: BTreeMap::new(),
        })
        .unwrap();
    store.save_note(&note(NOTE_ID)).unwrap();
    store.save_note(&note(KEEPER_ID)).unwrap();

    let mut root = container(ROOT_CONTAINER_ID, "Root");
    root.identity_instance_id = Some(KEEPER_ID.to_string());
    root.root_instance_ids = Some(vec![NOTE_ID.to_string(), KEEPER_ID.to_string()]);
    root.member_instance_ids = Some(vec![
        KEEPER_ID.to_string(),
        NOTE_ID.to_string(),
        RECORD_ID.to_string(),
    ]);
    let mut manifest = store.load_manifest().unwrap();
    manifest.container = Some(root);
    store.save_manifest(&manifest).unwrap();

    let mut section = container(SECTION_CONTAINER_ID, "Section");
    section.member_instance_ids = Some(vec![RECORD_ID.to_string(), KEEPER_ID.to_string()]);
    // Roots on a *file-backed* container, not just on the inline root — and the
    // record is its only root, so this also pins the collapse to absent.
    section.root_instance_ids = Some(vec![RECORD_ID.to_string()]);
    store.save_container(&section).unwrap();

    let mut sole = container(SOLE_CONTAINER_ID, "Sole");
    sole.member_instance_ids = Some(vec![RECORD_ID.to_string()]);
    store.save_container(&sole).unwrap();
}

/// The whole point: after the delete the repository must still *load*. A [R13]
/// dangling reference is fatal under [R24] and `catalog()` is `build_checked`,
/// so a successful build *is* the "repo can be reopened" assertion.
fn assert_loads_clean(store: &dyn RepositoryStore, when: &str) {
    store
        .catalog()
        .unwrap_or_else(|e| panic!("repository must still load {when}: {e}"));
}

fn run_cascade_suite(store: &dyn RepositoryStore) {
    populate(store);
    assert_loads_clean(store, "before the delete");

    // Tier 2 — the reproduction filed on #834.
    record_store::delete_record(store, RECORD_ID).unwrap();
    assert_loads_clean(store, "after deleting a container member (record)");

    // Tier 0 — the same cascade, via the other instance-delete path.
    services::delete_note(store, NOTE_ID).unwrap();
    assert_loads_clean(store, "after deleting a container member (note)");

    let root = store.load_manifest().unwrap().container.unwrap();
    assert_eq!(
        root.member_instance_ids,
        Some(vec![KEEPER_ID.to_string()]),
        "inline root container membership must have been cleaned"
    );
    assert_eq!(
        root.root_instance_ids,
        Some(vec![KEEPER_ID.to_string()]),
        "rootInstanceIds is cleaned too, not just memberInstanceIds"
    );
    assert_eq!(
        container_service::get_container(store, SECTION_CONTAINER_ID)
            .unwrap()
            .member_instance_ids,
        Some(vec![KEEPER_ID.to_string()]),
        "file-backed container membership must have been cleaned"
    );
    assert!(
        container_service::get_container(store, SECTION_CONTAINER_ID)
            .unwrap()
            .root_instance_ids
            .is_none(),
        "rootInstanceIds is cleaned on a file-backed container too, and an \
         emptied array collapses to absent"
    );
    assert!(
        container_service::get_container(store, SOLE_CONTAINER_ID)
            .unwrap()
            .member_instance_ids
            .is_none(),
        "an emptied membership array is cleared, not left as []"
    );

    // The cascade removes one id, not a set: the other member survives, as an
    // instance and as a membership.
    assert!(
        store.find_instance(KEEPER_ID).unwrap().is_some(),
        "the surviving instance must not have been deleted"
    );
    assert_eq!(
        root.identity_instance_id,
        Some(KEEPER_ID.to_string()),
        "an identity naming a surviving instance is left alone"
    );
}

/// The ordering claim, made testable: the cascade runs *before* the instance is
/// removed, so an interruption can never produce the unloadable repository #834
/// is about. `FailPoint::SaveManifest` fires on the inline root container's write
/// — the first thing the cascade does here.
///
/// Cascade-first (this code): the delete aborts with the record still on disk and
/// the repository loads. Cascade-last (the regression this guards): the record is
/// gone, the root still names it, and the next open is fatal.
#[test]
fn a_failed_membership_write_never_leaves_the_repository_unloadable() {
    let store = MemoryStore::empty();
    populate(&store);
    assert_loads_clean(&store, "before the delete");

    store.arm_fail_at(FailPoint::SaveManifest);
    let err = record_store::delete_record(&store, RECORD_ID)
        .expect_err("the injected manifest-write fault must surface, not be swallowed");
    assert!(
        matches!(err, RepositoryError::Io { .. }),
        "expected the injected Io fault, got {err:?}"
    );

    assert!(
        store.find_instance(RECORD_ID).unwrap().is_some(),
        "the record must survive a cascade that failed before it was deleted"
    );
    assert_loads_clean(&store, "after a failed delete");
}

/// Deleting the record a container names as its `identityInstanceId` must leave
/// a repository that is *valid*, not merely loadable: a dangling identity is
/// I-81 at **error** severity. RFC-029 says a root container with no identity is
/// valid, so the cascade clears it.
///
/// Run against the shape `repo create` actually produces — the scaffolded Tier-2
/// purpose record, identity and sole member of the inline root container —
/// rather than a synthetic one. In that shape the repository is left with no
/// instances at all, and the last assertion pins what that costs: navigation has
/// nothing to resolve and says so. `srs container update` re-points an identity
/// once a successor exists.
#[test]
fn deleting_a_container_identity_record_leaves_a_valid_repository() {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    let created = create_repository_with_intent(&store, &init_input()).unwrap();
    let identity = created
        .identity_instance_id
        .expect("repo create scaffolds a purpose record");

    let root = store.load_manifest().unwrap().container.unwrap();
    assert_eq!(
        root.identity_instance_id.as_deref(),
        Some(identity.as_str()),
        "premise: the scaffolded record is the root container's identity"
    );

    record_store::delete_record(&store, &identity).unwrap();

    assert_loads_clean(&store, "after deleting the identity record");
    let root = store.load_manifest().unwrap().container.unwrap();
    assert!(
        root.identity_instance_id.is_none(),
        "a dangling identityInstanceId is I-81 at error severity — it must be cleared"
    );
    assert!(
        !root
            .member_instance_ids
            .unwrap_or_default()
            .contains(&identity),
        "and the membership goes with it"
    );

    let report = validation::validate_repository(&store).unwrap();
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == validation::DiagnosticSeverity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "repository must still validate: {errors:?}"
    );

    // #834 pinned the cost here as a hard error: navigation could not resolve an
    // identity, so it failed outright. #838/ADR-044 removes that cost — an
    // identity-less root container is valid under RFC-029, so navigation now
    // succeeds and reports the absence instead of failing. What must still hold
    // is that the absence is stated, never inferred or papered over.
    let nav = repository_navigation_service::repository_navigation(&store)
        .expect("an identity-less container still navigates (RFC-029 permits it)");
    assert!(
        nav.identity.is_none(),
        "identity must be absent, not inferred from a root: {nav:?}"
    );
    assert!(
        nav.diagnostics
            .iter()
            .any(|d| d.contains("identityInstanceId")),
        "the diagnostic must name what is missing, got {:?}",
        nav.diagnostics
    );
}

#[test]
fn memory_store_delete_cascades_container_membership() {
    // `MemoryStore::empty()` already carries a valid manifest/package — it does
    // not take `create_repository` (see the store conformance harness).
    run_cascade_suite(&MemoryStore::empty());
}

#[test]
fn file_store_delete_cascades_container_membership() {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    create_repository(&store, &init_input()).unwrap();
    run_cascade_suite(&store);
}
