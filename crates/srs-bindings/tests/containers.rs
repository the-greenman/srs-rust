//! Integration test for the WASM `list_containers` / `get_container` bindings.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! exercise the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.
//!
//! Gallery `.srsj` containers used:
//!   - `b30db206…` root `5bbf9209…`, 3 members
//!   - `138e2fac…` root `9054911c…`, 7 members
//!   - `f7562aa3…` root `ad159754…`, 6 members

use srs_repository::container_service::{get_container, list_containers};
use srs_repository::JsonStore;

fn gallery_store() -> JsonStore {
    let srsj = include_str!("../../../../srs/docs/spec/examples/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

/// No filter lists every container.
#[test]
fn list_containers_returns_all() {
    let store = gallery_store();
    let summaries = list_containers(&store, None, None, None).expect("list must succeed");
    assert_eq!(summaries.len(), 3, "gallery has three containers");
}

/// A root filter resolves the single container a guide/root belongs to — the path the
/// web guides editor uses to map a selected guide to its document-view container.
#[test]
fn list_containers_filters_by_root() {
    let store = gallery_store();
    let summaries = list_containers(
        &store,
        None,
        None,
        Some("5bbf9209-1dc9-44b2-b0a3-f2192db5a879"),
    )
    .expect("list must succeed");
    assert_eq!(summaries.len(), 1, "exactly one container has this root");
    assert_eq!(
        summaries[0].container_id,
        "b30db206-e9a7-4588-a9aa-53451aacd243"
    );
}

/// `get_container` returns full membership, used to scope a guide's sections.
#[test]
fn get_container_returns_membership() {
    let store = gallery_store();
    let container =
        get_container(&store, "138e2fac-6a8a-4a06-9511-5aefd99ceae9").expect("container must load");
    let members = container.member_instance_ids.unwrap_or_default();
    assert_eq!(members.len(), 7, "container carries its seven members");
}
