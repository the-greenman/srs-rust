//! The store's read memo (srs-rust#1649) serves parsed relations, containers and the label
//! index between writes. It must never serve them across one: a browser MCP session and the UI
//! repository handle are two clones of one store over one VFS (srs-rust#1057), so a write
//! through either clone must be visible to the next read through the other.

use srs_core::types::container::{entries, Container};
use srs_core::types::note::Note;
use srs_core::types::relation::Relation;
use srs_repository::{
    container_service,
    relation_service::{self, ListRelationsFilter},
    repository_lifecycle::{
        create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    },
    vfs::MemVfs,
    FileStore, RepositoryStore,
};
use std::collections::BTreeMap;
use std::rc::Rc;

const A: &str = "16490000-0000-4000-8000-00000000000a";
const B: &str = "16490000-0000-4000-8000-00000000000b";
const C: &str = "16490000-0000-4000-8000-00000000000c";
const BOX: &str = "16490000-0000-4000-8000-0000000000b0";

fn store() -> FileStore {
    let store = FileStore::from_vfs(Rc::new(MemVfs::new()));
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "16490000-0000-4000-8000-000000000001".to_string(),
                namespace: "com.test.memo".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "16490000-0000-4000-8000-000000000002".to_string(),
                namespace: "com.test.memo".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        },
    )
    .unwrap();
    for id in [A, B, C] {
        store
            .save_note(&Note {
                created_by: None,
                instance_id: id.to_string(),
                title: Some(id.to_string()),
                tags: None,
                sections: vec![],
                graduated_at: None,
                source_refs: None,
                created_at: None,
                updated_at: None,
                meta: None,
            })
            .unwrap();
    }
    store
}

fn relation(id: &str, source: &str, target: &str) -> Relation {
    serde_json::from_value(serde_json::json!({
        "relationId": id, "relationType": "refines", "sourceInstanceId": source, "targetInstanceId": target
    }))
    .unwrap()
}

fn targets_of(store: &dyn RepositoryStore, source: &str) -> Vec<String> {
    relation_service::list_relations(
        store,
        ListRelationsFilter {
            source: Some(source.to_string()),
            ..Default::default()
        },
    )
    .unwrap()
    .into_iter()
    .map(|r| r.target_id)
    .collect()
}

#[test]
fn a_relation_written_through_another_clone_is_read_next() {
    let ui = store();
    let agent = ui.clone();
    ui.save_relation(&relation("16490000-0000-4000-8000-0000000000e1", A, B))
        .unwrap();
    assert_eq!(targets_of(&ui, A), vec![B]); // warms the memo
    assert_eq!(targets_of(&ui, A), vec![B]); // served from it
    agent
        .save_relation(&relation("16490000-0000-4000-8000-0000000000e2", A, C))
        .unwrap();
    let mut seen = targets_of(&ui, A);
    seen.sort();
    assert_eq!(
        seen,
        vec![B, C],
        "the memo served relations from before another clone's write"
    );
}

#[test]
fn a_membership_written_through_another_clone_is_read_next() {
    let ui = store();
    let agent = ui.clone();
    let mut boxed = Container {
        container_id: BOX.to_string(),
        title: "Box".to_string(),
        namespace: None,
        name: None,
        description: None,
        container_type: None,
        identity_instance_id: None,
        anchor_instance_id: None,
        member_instance_ids: Some(entries(vec![A.to_string()])),
        child_container_ids: None,
        tags: None,
        created_at: None,
        updated_at: None,
        meta: None,
        extra: BTreeMap::new(),
    };
    ui.save_container(&boxed).unwrap();
    let holds = |s: &dyn RepositoryStore, id: &str| {
        container_service::containers_for_instance(s, id)
            .unwrap()
            .iter()
            .any(|c| c.container_id == BOX)
    };
    assert!(holds(&ui, A)); // warms the memo
    assert!(!holds(&ui, B));
    container_service::add_member(&agent, BOX, B, None, None).unwrap();
    assert!(
        holds(&ui, B),
        "the memo served a container from before another clone's write"
    );
    boxed.member_instance_ids = Some(entries(vec![C.to_string()]));
    agent.save_container(&boxed).unwrap();
    assert!(
        holds(&ui, C) && !holds(&ui, A),
        "the memo served a container from before another clone's write"
    );
}
