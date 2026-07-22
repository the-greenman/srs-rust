//! Pre-load bundle-format migrations for `.srsj` JSON strings.
//!
//! These functions operate on raw `.srsj` bytes **before** a `RepositoryStore` is constructed.
//! They are intentionally outside the `MIGRATIONS` static registry in
//! `migration_registry_service.rs` (see ADR-032): by the time any store exists the
//! RFC-014 transformation has already been applied, so a registry `status_fn` would always
//! return `AlreadyApplied` — noise, not signal. Pre-load migrations remain standalone entry
//! points called directly at bundle-load time — `load_from_srsj` (this module) is the
//! public entry point; `JsonStore::from_srsj` is its lower-level delegate and does **not**
//! apply the migration.

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

/// Project any repository as a `.srsj` string (ADR-037: `.srsj` is a boundary
/// codec, not session state).
///
/// Snapshot export → in-memory `JsonStore` → `to_srsj_string`. The projection
/// re-canonicalizes instance/definition paths — acceptable for an interchange
/// format; the operational tree keeps real paths.
pub fn export_srsj_string(
    source: &dyn crate::store::RepositoryStore,
) -> Result<String, RepositoryError> {
    let snapshot = crate::repository_portability::export_repository_snapshot_with_options(
        source,
        crate::repository_portability::ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )?;
    let codec = crate::JsonStore::new_in_memory();
    crate::repository_portability::import_repository_snapshot(&codec, &snapshot)?;
    codec.to_srsj_string()
}

/// Apply the RFC-014 manifest migration to a raw `.srsj` JSON string.
///
/// - Moves `manifest.meta.upstreamPackage` to `manifest.upstreamPackage` if it is
///   still nested under `meta`.
/// - Strips `contentHash` from `manifest.upstreamPackage` unconditionally (the field
///   was removed from the spec schema in RFC-014 Rev 4; old migration code added it).
///
/// Idempotent: running the migration twice produces the same result.
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

    // Strip contentHash from the top-level field unconditionally: bundles migrated by
    // old code that added contentHash during promotion are not re-entered by the guard
    // above and must have contentHash removed here.
    // get_mut, NOT `seed["manifest"]["upstreamPackage"]`: serde_json's IndexMut inserts
    // Null for missing keys, which added `"upstreamPackage": null` to every manifest
    // without provenance and failed schema validation on load (#487).
    if let Some(up) = seed
        .get_mut("manifest")
        .and_then(|m| m.get_mut("upstreamPackage"))
        .and_then(|v| v.as_object_mut())
    {
        up.remove("contentHash");
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
    fn migrate_rfc014_does_not_add_upstream_package_when_absent() {
        // Regression #487: `seed["manifest"]["upstreamPackage"].as_object_mut()` used
        // IndexMut, which inserts Null for missing keys — every manifest without
        // provenance gained `"upstreamPackage": null` and failed schema validation.
        let input = serde_json::json!({
            "manifest": {
                "srsVersion": "2.0-draft",
                "repositoryId": "00000000-0000-4000-8000-000000000000",
                "instanceIndex": []
            },
            "data": {}
        });
        let migrated_str = migrate_rfc014(&serde_json::to_string(&input).unwrap()).unwrap();
        let migrated: serde_json::Value = serde_json::from_str(&migrated_str).unwrap();
        assert!(
            !migrated["manifest"]
                .as_object()
                .unwrap()
                .contains_key("upstreamPackage"),
            "migration must not add an upstreamPackage key to a manifest without provenance"
        );
        assert!(
            !migrated["manifest"]
                .as_object()
                .unwrap()
                .contains_key("meta"),
            "migration must not add a meta key either"
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
    fn migrate_rfc014_strips_content_hash_from_already_promoted_bundle() {
        // Regression #428: bundles migrated by old code have upstreamPackage already
        // at the top level but may still carry contentHash (old migration added it).
        let input = serde_json::json!({
            "manifest": {
                "upstreamPackage": {
                    "packageId": "com.example.pkg",
                    "contentHash": "sha256:abc123"
                },
                "meta": {}
            },
            "data": {}
        });
        let result = migrate_rfc014(&serde_json::to_string(&input).unwrap())
            .expect("migration succeeds on already-promoted bundle");
        let migrated: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            migrated["manifest"]["upstreamPackage"]["contentHash"].is_null(),
            "contentHash must be absent from already-promoted upstreamPackage (regression #428)"
        );
    }

    #[test]
    fn migrate_rfc014_rejects_invalid_json() {
        let err = migrate_rfc014("not json").unwrap_err();
        assert!(matches!(err, RepositoryError::Serialize { .. }));
    }
}
