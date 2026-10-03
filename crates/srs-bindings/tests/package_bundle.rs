//! Native coverage for `SrsRepository::{export_package_bundle, install_package_bundle}`
//! (#663, ADR-050). The `#[wasm_bindgen]` methods are deserialize -> one service call ->
//! `to_js` (which panics off-wasm), so these tests exercise the exact services they call on
//! the same MemVfs tree-session store type; the wasm32 build gate covers the methods.

use srs_repository::package_bundle::{export_package_bundle, ExportPackageInput};
use srs_repository::package_install_service::install_package_bundle_bytes;
use srs_repository::repository_lifecycle::{create_blank_repository, CreateBlankRepositoryInput};
use srs_repository::FileStore;

fn gallery() -> FileStore {
    let srsj = include_str!("../../srs-repository/tests/fixtures/gallery.srsj");
    srs_repository::srsj::open_srsj(srsj).expect("gallery srsj must load")
}

/// A blank tree session, as `SrsRepository::create` builds it.
fn blank() -> FileStore {
    let store = srs_repository::new_tree_session();
    let input: CreateBlankRepositoryInput =
        serde_json::from_str(r#"{"namespace":"com.test.blank"}"#).unwrap();
    create_blank_repository(&store, input).unwrap();
    store
}

fn gallery_bundle() -> String {
    let input: ExportPackageInput =
        serde_json::from_str(r#"{"selector":null,"publishedAt":"2026-10-03T00:00:00Z"}"#).unwrap();
    export_package_bundle(&gallery(), input).unwrap().text
}

#[test]
fn tree_session_install_advances_write_epoch() {
    let text = gallery_bundle();
    let store = blank();
    let before = store.write_epoch();
    let r = install_package_bundle_bytes(&store, text.as_bytes(), Default::default()).unwrap();
    assert!(r.installed > 0);
    assert!(store.write_epoch() > before);
}

#[test]
fn tree_session_export_does_not_advance_write_epoch() {
    let store = gallery();
    let before = store.write_epoch();
    export_package_bundle(&store, ExportPackageInput::default()).unwrap();
    assert_eq!(store.write_epoch(), before);
}

/// The gallery fixture itself carries three V8 dangling-lifecycleRef errors (and no root
/// container), which a faithful bundle carries along, so the zero-error check uses a clean
/// source: the srs-repository `install-package` fixture (all ten kinds) installed into one
/// tree session, exported, and installed into a second.
#[test]
fn tree_session_install_then_validate_has_zero_errors() {
    let source = blank();
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../srs-repository/tests/fixtures/install-package"
    );
    let input: srs_repository::package_install_service::InstallPackageInput =
        serde_json::from_value(serde_json::json!({"sourceDir": dir})).unwrap();
    srs_repository::package_install_service::install_package(&source, input).unwrap();
    let input: ExportPackageInput = serde_json::from_str(
        r#"{"selector":"packages/install-fixture","publishedAt":"2026-10-03T00:00:00Z"}"#,
    )
    .unwrap();
    let text = export_package_bundle(&source, input).unwrap().text;

    let store = blank();
    let r = install_package_bundle_bytes(&store, text.as_bytes(), Default::default()).unwrap();
    assert_eq!(r.installed, 11);
    let report = srs_repository::validation::validate_repository(&store).unwrap();
    assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
}
