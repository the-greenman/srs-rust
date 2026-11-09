//! Integration test for the WASM `getTypeSubtypes` binding's service, and its
//! FileStore vs MemoryStore agreement (cross-store rule, CLAUDE.md).
//!
//! Native test, as the other binding tests: the service is called directly because
//! `to_js()` panics off-wasm. The gallery `.srsj` has no inheritance, so every Type's
//! subtype set is empty and `include_self` yields just itself.

use srs_repository::store::memory::MemoryStore;
use srs_repository::type_subtypes_service::{type_subtypes, TypeSubtypesInput};
use srs_repository::{FileStore, RepositoryStore};

const DECISION_TYPE_ID: &str = "1fcad6a2-9f78-5e41-94ba-d82e88b822f3";

fn gallery_store() -> FileStore {
    let srsj = include_str!("../../srs-repository/tests/fixtures/gallery.srsj");
    srs_repository::srsj::open_srsj(srsj).expect("gallery srsj must load")
}

fn input(id: &str, include_self: bool) -> TypeSubtypesInput {
    TypeSubtypesInput {
        type_id: id.to_string(),
        include_self,
    }
}

#[test]
fn subtypes_with_and_without_self() {
    let store = gallery_store();
    let plain = type_subtypes(&store, input(DECISION_TYPE_ID, false)).unwrap();
    assert_eq!(plain.type_id, DECISION_TYPE_ID);
    assert!(plain.subtypes.is_empty());
    let with_self = type_subtypes(&store, input(DECISION_TYPE_ID, true)).unwrap();
    assert_eq!(with_self.subtypes.len(), 1);
    assert_eq!(with_self.subtypes[0].id, DECISION_TYPE_ID);
}

#[test]
fn unknown_type_is_definition_not_found() {
    let err = type_subtypes(&gallery_store(), input("nope", false)).unwrap_err();
    assert_eq!(err.code(), "definition-not-found");
}

/// FileStore and a MemoryStore over the same manifest + package agree for every Type.
#[test]
fn file_store_and_memory_store_agree() {
    let file = gallery_store();
    let package = file.load_package().unwrap();
    let mem = MemoryStore::new(file.load_manifest().unwrap(), package.clone());
    for rt in package.record_types() {
        for include_self in [false, true] {
            assert_eq!(
                type_subtypes(&file, input(&rt.id, include_self)).unwrap(),
                type_subtypes(&mem, input(&rt.id, include_self)).unwrap(),
            );
        }
    }
}
