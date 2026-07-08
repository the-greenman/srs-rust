use crate::error::RepositoryError;

/// Migrate a raw `.srsj` string (RFC-014) and load it into a `JsonStore` in one call.
///
/// Equivalent to `JsonStore::from_srsj(&migrate_rfc014(srsj_str)?)`, but presented
/// as a single entry point so callers (e.g. WASM bindings) satisfy the one-service-call
/// rule without embedding migration logic themselves.
pub fn load_from_srsj(srsj_str: &str) -> Result<crate::JsonStore, RepositoryError> {
    let migrated = migrate_rfc014(srsj_str)?;
    crate::JsonStore::from_srsj(&migrated)
}

/// Apply the RFC-014 manifest migration to a raw `.srsj` JSON string.
///
/// Moves `manifest.meta.upstreamPackage` to the top-level `manifest.upstreamPackage`
/// and strips `contentHash` if present (the field was removed from the spec schema).
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
        let pkg_info = seed["manifest"]["meta"]["upstreamPackage"].clone();
        let mut up = pkg_info.as_object().cloned().unwrap_or_default();
        up.remove("contentHash");
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
    fn migrate_rfc014_moves_upstream_package_to_top_level() {
        let seed = governance_seed_str();
        let migrated_str = migrate_rfc014(&seed).expect("migration succeeds");
        let migrated: serde_json::Value = serde_json::from_str(&migrated_str).unwrap();

        assert!(
            migrated["manifest"]["upstreamPackage"].is_object(),
            "upstreamPackage must be at top level after migration"
        );
        assert!(
            migrated["manifest"]["upstreamPackage"]["contentHash"].is_null(),
            "contentHash must be absent after migration (removed from spec schema)"
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
    fn migrate_rfc014_succeeds_when_package_json_absent_in_data() {
        // Migration no longer computes contentHash, so missing package/package.json is fine.
        let input = serde_json::json!({
            "manifest": {
                "meta": {
                    "upstreamPackage": { "packageId": "com.example.pkg" }
                }
            },
            "data": {}
        });
        let result = migrate_rfc014(&serde_json::to_string(&input).unwrap());
        assert!(
            result.is_ok(),
            "migration must succeed without package data: {result:?}"
        );
    }

    #[test]
    fn migrate_rfc014_rejects_invalid_json() {
        let err = migrate_rfc014("not json").unwrap_err();
        assert!(matches!(err, RepositoryError::Serialize { .. }));
    }
}
