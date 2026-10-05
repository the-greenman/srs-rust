//! Native coverage for `SrsRepository::{export_package_bundle, install_package_bundle}`
//! (#663, ADR-050). The `#[wasm_bindgen]` methods are deserialize -> one service call ->
//! `to_js` (which panics off-wasm), so these tests exercise the exact services they call on
//! the same MemVfs tree-session store type; the wasm32 build gate covers the methods.

use srs_repository::package_bundle::{export_package_bundle, ExportPackageInput};
use srs_repository::package_install_service::{
    install_package_bundle_bytes, upgrade_package_bundle, UpgradeOptions,
};
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

/// A tree session with the srs-repository `install-package` fixture (all ten kinds,
/// no dangling references) installed at `packages/install-fixture`.
fn fixture_source() -> FileStore {
    let source = blank();
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../srs-repository/tests/fixtures/install-package"
    );
    let input: srs_repository::package_install_service::InstallPackageInput =
        serde_json::from_value(serde_json::json!({"sourceDir": dir})).unwrap();
    srs_repository::package_install_service::install_package(&source, input).unwrap();
    source
}

fn export_json(
    store: &FileStore,
    input_json: &str,
) -> srs_repository::package_bundle::PackageBundleExport {
    let input: ExportPackageInput = serde_json::from_str(input_json).unwrap();
    export_package_bundle(store, input).unwrap()
}

const FIXTURE_INPUT: &str =
    r#"{"selector":"packages/install-fixture","publishedAt":"2026-10-03T00:00:00Z"}"#;

#[test]
fn tree_session_install_advances_write_epoch() {
    let text = export_json(&fixture_source(), FIXTURE_INPUT).text;
    let store = blank();
    let before = store.write_epoch();
    let r = install_package_bundle_bytes(&store, text.as_bytes(), Default::default()).unwrap();
    assert!(r.installed > 0);
    assert!(store.write_epoch() > before);
}

#[test]
fn tree_session_export_does_not_advance_write_epoch() {
    let store = fixture_source();
    let before = store.write_epoch();
    export_json(&store, FIXTURE_INPUT);
    assert_eq!(store.write_epoch(), before);
}

/// The gallery's Types carry three dangling `lifecycleRef`s (LINEAGE): RFC-003 [C1]
/// refuses the export rather than writing a bundle with unresolved references (PD10).
#[test]
fn gallery_export_refuses_dangling_lifecycle_ref() {
    let input: ExportPackageInput =
        serde_json::from_str(r#"{"selector":null,"publishedAt":"2026-10-03T00:00:00Z"}"#).unwrap();
    match export_package_bundle(&gallery(), input).unwrap_err() {
        srs_repository::error::RepositoryError::InvalidPackageBundle { code, message } => {
            assert_eq!(code, "bundle-reference-unresolved");
            assert!(message.contains("/lifecycleRef"), "{message}");
        }
        other => panic!("expected InvalidPackageBundle, got {other:?}"),
    }
}

#[test]
fn tree_session_export_accepts_mode_standalone() {
    let e = export_json(
        &fixture_source(),
        r#"{"selector":"packages/install-fixture","publishedAt":"2026-10-03T00:00:00Z","mode":"standalone"}"#,
    );
    assert_eq!(
        e.summary.mode,
        srs_repository::package_bundle::BundleMode::Standalone
    );
    let b: serde_json::Value = serde_json::from_str(&e.text).unwrap();
    assert_eq!(b["mode"], "standalone");
}

/// The gallery fixture itself carries three V8 dangling-lifecycleRef errors (and no root
/// container), which a faithful bundle carries along, so the zero-error check uses a clean
/// source: the srs-repository `install-package` fixture (all ten kinds) installed into one
/// tree session, exported, and installed into a second.
#[test]
fn tree_session_install_then_validate_has_zero_errors() {
    let text = export_json(&fixture_source(), FIXTURE_INPUT).text;

    let store = blank();
    let r = install_package_bundle_bytes(&store, text.as_bytes(), Default::default()).unwrap();
    assert_eq!(r.installed, 11);
    let report = srs_repository::validation::validate_repository(&store).unwrap();
    assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
}

/// `upgrade_package_bundle`: a dry run writes nothing (epoch unmoved); a real run advances it.
#[test]
fn tree_session_upgrade_advances_write_epoch_only_when_not_dry_run() {
    let text = export_json(&fixture_source(), FIXTURE_INPUT).text;
    let store = blank();
    install_package_bundle_bytes(&store, text.as_bytes(), Default::default()).unwrap();
    let mut bundle: serde_json::Value = serde_json::from_str(&text).unwrap();
    bundle["packageVersion"] = serde_json::json!("99.0.0");
    let newer = serde_json::to_vec(&bundle).unwrap();

    let opts = |dry_run| {
        serde_json::from_value::<UpgradeOptions>(serde_json::json!({"dryRun": dry_run})).unwrap()
    };
    let e0 = store.write_epoch();
    let dry = upgrade_package_bundle(&store, &newer, opts(true)).unwrap();
    assert!(dry.dry_run && dry.upgraded);
    assert_eq!(store.write_epoch(), e0);
    let real = upgrade_package_bundle(&store, &newer, opts(false)).unwrap();
    assert!(real.upgraded && !real.dry_run);
    assert!(store.write_epoch() > e0);
    assert_eq!(real.version, "99.0.0");
}
