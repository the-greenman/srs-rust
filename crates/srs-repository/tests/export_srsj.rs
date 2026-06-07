/// Roundtrip test: load gallery.srsj → to_srsj_string() → reload → same instance count.
///
/// This lives in srs-repository because the binding under test (`to_srsj_string`) is on
/// `JsonStore` in that crate. The srs-bindings `export_srsj()` WASM wrapper delegates
/// directly to this method — if the method round-trips correctly here, the wrapper is correct.
use srs_repository::{JsonStore, RepositoryStore};

/// Walk up from `start` until a `srs/docs/spec/examples/gallery.srsj` sibling is found.
/// Returns `None` if the filesystem root is reached without finding it.
fn find_gallery_fixture() -> Option<std::path::PathBuf> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dir = manifest_dir.as_path();
    loop {
        let candidate = dir.join("srs/docs/spec/examples/gallery.srsj");
        if candidate.exists() {
            return Some(candidate);
        }
        // Also look for it as a sibling of the current directory.
        if let Some(parent) = dir.parent() {
            let sibling = parent.join("srs/docs/spec/examples/gallery.srsj");
            if sibling.exists() {
                return Some(sibling);
            }
            dir = parent;
        } else {
            return None;
        }
    }
}

#[test]
fn to_srsj_roundtrip_gallery() {
    let gallery_path = match find_gallery_fixture() {
        Some(p) => p,
        None => {
            // The srs/ sibling repo is not present (e.g. in an isolated CI environment).
            // Skip rather than fail — the unit test in json_store.rs already covers the
            // to_srsj_string contract with an inline fixture.
            eprintln!("SKIP: gallery.srsj not found; srs/ sibling repo unavailable");
            return;
        }
    };

    let src = std::fs::read_to_string(&gallery_path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", gallery_path.display()));

    // Load original store and count instances.
    let original_store =
        JsonStore::from_srsj(&src).expect("gallery.srsj must parse as a valid srsj envelope");
    let original_manifest = original_store
        .load_manifest()
        .expect("gallery.srsj must have a valid manifest");
    let original_count = original_manifest.instance_index.len();
    assert!(
        original_count > 0,
        "gallery fixture must contain at least one instance"
    );

    // Serialise back to a .srsj string.
    let exported = original_store
        .to_srsj_string()
        .expect("to_srsj_string must succeed on a valid store");

    // The exported envelope must be valid JSON with srsj == "1".
    let parsed: serde_json::Value =
        serde_json::from_str(&exported).expect("to_srsj_string output must be valid JSON");
    assert_eq!(
        parsed["srsj"].as_str(),
        Some("1"),
        "exported envelope must have srsj == \"1\""
    );

    // Reload from the exported string.
    let reloaded_store =
        JsonStore::from_srsj(&exported).expect("exported srsj string must be re-parseable");
    let reloaded_manifest = reloaded_store
        .load_manifest()
        .expect("reloaded store must have a valid manifest");
    let reloaded_count = reloaded_manifest.instance_index.len();

    // Instance count must be preserved across the roundtrip.
    assert_eq!(
        original_count, reloaded_count,
        "roundtrip must preserve instance count: original={original_count}, reloaded={reloaded_count}"
    );
}
