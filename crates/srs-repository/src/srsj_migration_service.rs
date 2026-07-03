use crate::error::RepositoryError;

/// Compute the RFC-014 `contentHash` for a package embedded in a `.srsj` bundle.
///
/// Hash = SHA-256 over compact-JSON bytes of: `package/package.json`, then each
/// definition file listed in `package/package.json` (`fields`, `types`, `views`,
/// `documentViews`, `themes`, `relationTypes`, `vocabularies`, `lifecycles`,
/// `blueprints`, `protocols`) in that property order, within each array in
/// declaration order, with no separator bytes.
///
/// `data` is the top-level `data` map from a parsed `.srsj` envelope.
pub fn compute_package_content_hash(data: &serde_json::Value) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let pkg = &data["package/package.json"];
    hasher.update(serde_json::to_string(pkg).unwrap_or_default().as_bytes());
    for array_key in &[
        "fields",
        "types",
        "views",
        "documentViews",
        "themes",
        "relationTypes",
        "vocabularies",
        "lifecycles",
        "blueprints",
        "protocols",
    ] {
        if let Some(files) = pkg[array_key].as_array() {
            for file_ref in files {
                if let Some(rel_path) = file_ref.as_str() {
                    let data_key = format!("package/{rel_path}");
                    hasher.update(
                        serde_json::to_string(&data[&data_key])
                            .unwrap_or_default()
                            .as_bytes(),
                    );
                }
            }
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Apply the RFC-014 manifest migration to a raw `.srsj` JSON string.
///
/// Moves `manifest.meta.upstreamPackage` to the top-level `manifest.upstreamPackage`
/// and adds `contentHash` (SHA-256 over the embedded package files). This must be
/// applied to the raw JSON value before loading into `JsonStore`, because computing
/// `contentHash` requires access to the embedded package files in the `data` map.
///
/// Returns the migrated `.srsj` JSON string. If `meta.upstreamPackage` is absent
/// the input is returned unchanged (idempotent on already-migrated bundles).
pub fn migrate_rfc014(srsj_str: &str) -> Result<String, RepositoryError> {
    let mut seed: serde_json::Value =
        serde_json::from_str(srsj_str).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from("<srsj-input>"),
            source,
        })?;

    if seed["manifest"]["meta"]["upstreamPackage"].is_object() {
        let content_hash = compute_package_content_hash(&seed["data"]);
        let pkg_info = seed["manifest"]["meta"]["upstreamPackage"].clone();
        let mut up = pkg_info.as_object().cloned().unwrap_or_default();
        up.insert(
            "contentHash".to_string(),
            serde_json::Value::String(content_hash),
        );
        seed["manifest"]["upstreamPackage"] = serde_json::Value::Object(up);
        if let Some(meta_obj) = seed["manifest"]["meta"].as_object_mut() {
            meta_obj.remove("upstreamPackage");
        }
    }

    serde_json::to_string(&seed).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from("<srsj-output>"),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn governance_seed_str() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/governance-seed.srsj"
        ))
        .expect("governance-seed.srsj must be present in crates/srs-repository/tests/fixtures/")
    }

    #[test]
    fn migrate_rfc014_moves_upstream_package_and_adds_content_hash() {
        let seed = governance_seed_str();
        let migrated_str = migrate_rfc014(&seed).expect("migration succeeds");
        let migrated: serde_json::Value = serde_json::from_str(&migrated_str).unwrap();

        assert!(
            migrated["manifest"]["upstreamPackage"].is_object(),
            "upstreamPackage must be at top level after migration"
        );
        let content_hash = migrated["manifest"]["upstreamPackage"]["contentHash"]
            .as_str()
            .expect("contentHash present");
        assert!(
            content_hash.starts_with("sha256:"),
            "contentHash must start with sha256:"
        );
        assert!(
            migrated["manifest"]["meta"]["upstreamPackage"].is_null(),
            "meta.upstreamPackage must be absent after migration"
        );
    }

    #[test]
    fn migrate_rfc014_is_idempotent_on_already_migrated_bundle() {
        let seed = governance_seed_str();
        let once = migrate_rfc014(&seed).unwrap();
        let twice = migrate_rfc014(&once).unwrap();
        let once_v: serde_json::Value = serde_json::from_str(&once).unwrap();
        let twice_v: serde_json::Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(
            once_v["manifest"]["upstreamPackage"], twice_v["manifest"]["upstreamPackage"],
            "second migration must not change upstreamPackage"
        );
    }

    #[test]
    fn compute_package_content_hash_returns_sha256_prefix() {
        let seed: serde_json::Value =
            serde_json::from_str(&governance_seed_str()).expect("seed parses");
        let hash = compute_package_content_hash(&seed["data"]);
        assert!(hash.starts_with("sha256:"), "hash: {hash}");
        assert!(hash.len() > 10, "hash non-trivial: {hash}");
    }

    #[test]
    fn migrate_rfc014_rejects_invalid_json() {
        let err = migrate_rfc014("not json").unwrap_err();
        assert!(matches!(err, RepositoryError::Serialize { .. }));
    }
}
