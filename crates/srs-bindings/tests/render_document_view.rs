//! Integration test for the WASM `render_document_view` binding.
//!
//! Native Rust test (not `#[wasm_bindgen_test]`) — runs with `cargo test -p srs-bindings`
//! without a browser or wasm-pack build. Follows the same pattern as the other binding tests:
//! test the underlying service directly via `JsonStore::from_srsj`, since `to_js()` calls
//! `js_sys::JSON::parse` which panics off-wasm. The wasm-pack build proves the `#[wasm_bindgen]`
//! export compiles.
//!
//! Gallery `.srsj` document views used:
//!   - `5a3ce87e` — decision-deliberation  (markdown format declared)
//!   - `b5c8d124` — decision-log           (markdown format declared)
//!   - `78b11038` — articles-and-roles     (markdown format declared)

use srs_repository::render_service::{render_document_view, RenderDocumentViewOptions};
use srs_repository::JsonStore;

fn gallery_store() -> JsonStore {
    let srsj = include_str!("../../../../srs/docs/spec/examples/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

/// `format = "json"` populates `projection` and leaves `rendered` as the serialised JSON string.
#[test]
fn render_document_view_json_format_returns_projection() {
    let store = gallery_store();
    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: "5a3ce87e-8340-4d91-a140-ab56b57f704f", // decision-deliberation
        format: Some("json"),
        theme_variant: None,
        container_id: None,
    })
    .expect("render must succeed");

    assert!(
        result.projection.is_some(),
        "json format must populate projection"
    );
    let proj = result.projection.unwrap();
    assert_eq!(
        proj.document_view_id, "5a3ce87e-8340-4d91-a140-ab56b57f704f",
        "projection carries view id"
    );
    assert!(
        !proj.sections.is_empty(),
        "projection must have at least one section"
    );
}

/// `format = "markdown"` renders markdown text and leaves `projection` as `None`.
#[test]
fn render_document_view_markdown_format_no_projection() {
    let store = gallery_store();
    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: "b5c8d124-2084-4a6b-a231-425e800e1e55", // decision-log
        format: Some("markdown"),
        theme_variant: None,
        container_id: None,
    })
    .expect("render must succeed");

    assert!(
        result.projection.is_none(),
        "markdown format must not set projection"
    );
    assert!(
        !result.rendered.is_empty(),
        "markdown format must produce rendered output"
    );
}

/// Unknown view ID returns an error.
#[test]
fn render_document_view_unknown_view_errors() {
    let store = gallery_store();
    let result = render_document_view(RenderDocumentViewOptions {
        store: &store,
        view_id: "00000000-0000-0000-0000-000000000000",
        format: Some("json"),
        theme_variant: None,
        container_id: None,
    });
    assert!(result.is_err(), "unknown view id must return Err");
}
