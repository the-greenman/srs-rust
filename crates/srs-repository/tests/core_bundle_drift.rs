/// Drift check: the embedded `core-bundle.srsj` must match the committed SHA256 hash.
///
/// This test catches the case where the bundle was updated without updating the hash file,
/// or vice versa. It runs in every CI environment without requiring the `srs/` spec repo
/// to be present as a sibling checkout.
///
/// To refresh after updating the bundle:
///   sha256sum crates/srs-repository/assets/core-bundle.srsj | awk '{print $1}' \
///     > crates/srs-repository/assets/core-bundle.sha256
///
/// Additionally, when the `srs/` spec repo is present as a sibling checkout, this test
/// also verifies that the embedded bundle matches the canonical source artifact:
///   srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj
#[test]
fn core_bundle_matches_committed_sha256() {
    let embedded = include_bytes!("../assets/core-bundle.srsj");
    let committed_hash = include_str!("../assets/core-bundle.sha256")
        .trim()
        .to_string();

    // Compute SHA256 of the embedded bytes using the sha2 crate is not available here,
    // so use a process call — this is an integration test so spawning sha256sum is fine.
    let output = std::process::Command::new("sha256sum")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/core-bundle.srsj"))
        .output()
        .expect("sha256sum must be available");

    let stdout = String::from_utf8(output.stdout).expect("sha256sum output is UTF-8");
    let actual_hash = stdout.split_whitespace().next().unwrap_or("").to_string();

    assert_eq!(
        actual_hash, committed_hash,
        "Embedded core-bundle.srsj hash mismatch. \
         Run: sha256sum crates/srs-repository/assets/core-bundle.srsj | awk '{{print $1}}' \
         > crates/srs-repository/assets/core-bundle.sha256"
    );

    // Also check against the canonical srs/ spec repo if present.
    let canonical = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj");
    if !canonical.exists() {
        return;
    }
    let canonical_content = std::fs::read_to_string(&canonical).unwrap();
    let embedded_str = std::str::from_utf8(embedded).expect("core-bundle.srsj is valid UTF-8");

    // RFC-038 [R21] / acceptance test 17: the reader gate requires the
    // generation stamp. srs#378 stamped the *published* bundle, so the
    // transitional compare-modulo-the-stamp-line allowance has expired and
    // this is a plain byte comparison again.
    assert!(
        embedded_str.contains("\"dataModelRevision\": 2"),
        "the vendored bundle must carry the generation stamp ([R21])"
    );
    assert_eq!(
        embedded_str.trim(),
        canonical_content.trim(),
        "Embedded core-bundle.srsj has drifted from the canonical srs repo. \
         Copy packages/com.semanticops.core/1.0.0/core-bundle.srsj to \
         crates/srs-repository/assets/core-bundle.srsj and update \
         core-bundle.sha256."
    );
}
