//! Chained migration-ladder test (srs-rust#934, srs#256/#242 standing check).
//!
//! Every existing migration test (this crate's own `migration_registry_service`
//! tests, `srs-bindings/tests/migration_registry.rs`) applies exactly *one*
//! registered migration at a time against a fixture built for that migration.
//! Nothing walks the full chain from the oldest supported revision to
//! current — the exact gap the srs#256 scoping comment (2026-09-04) found by
//! grep, not assumption. This test closes it: start a fixture at
//! `MIN_SUPPORTED_DATA_MODEL_REVISION` (2) with every structural migration
//! still `Needed`, drive `list_migrations`/`apply_migration` — the real
//! registry, walked in order, never a hardcoded id list — until nothing is
//! `Needed`, and assert the result lands on `CURRENT_DATA_MODEL_REVISION`
//! with a clean `repo validate`.

use srs_repository::migration_registry_service::{
    apply_migration, list_migrations, MigrationStatus,
};
use srs_repository::srsj::open_srsj;
use srs_repository::store::RepositoryStore;
use srs_repository::validation::validate_repository;

/// A minimal repository at `dataModelRevision: 2` (RFC-039 carrier — the
/// oldest revision `MIN_SUPPORTED_DATA_MODEL_REVISION` still loads) that
/// also carries the pre-RFC-038 `instanceIndex` shape, so every structural
/// migration in the registry (not just the revision-keyed ones) has real
/// work to do. Mirrors `migration_registry_service`'s own `indexed_srsj_store`
/// fixture (used there to test `rfc038-storage` in isolation) — same shape,
/// driven here through the whole registry instead of one entry.
fn rev2_fixture() -> srs_repository::store::FileStore {
    open_srsj(
        &serde_json::json!({
            "srsj": "2",
            "manifest": {
                "$schema": srs_schema::MANIFEST_SCHEMA_ID,
                "srsVersion": "2.0-draft",
                "dataModelRevision": 2,
                "repositoryId": "00000000-0000-4000-8000-00000000aaaa",
                "namespace": "com.semanticops.ladder",
                "title": "Migration Ladder Fixture",
                "createdAt": "2026-01-01T00:00:00Z",
                "instanceIndex": [],
                "packageRef": { "mode": "local", "path": "package" },
                "container": {
                    "containerId": "00000000-0000-4000-8000-00000000cccc",
                    "title": "Migration Ladder Fixture",
                },
            },
            "data": {
                "package/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": "00000000-0000-4000-8000-00000000bbbb",
                    "namespace": "com.semanticops.ladder",
                    "name": "primary",
                    "version": "1.0.0",
                    "title": "Ladder Fixture Package",
                    "description": "Migration-ladder fixture package.",
                    "status": "draft",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": [],
                    "types": [],
                },
            },
        })
        .to_string(),
    )
    .unwrap()
    .with_rfc038_exemption()
}

#[test]
fn a_rev2_fixture_climbs_the_full_registry_to_current() {
    let store = rev2_fixture();

    // One ordered pass applying every migration the registry reports
    // `Needed` for, in the registry's own order (which is also its
    // documented dependency order — see migration_registry_service.rs).
    let mut applied = Vec::new();
    for m in list_migrations(&store).expect("list_migrations must not error") {
        if m.status == MigrationStatus::Needed {
            apply_migration(&store, &m.id)
                .unwrap_or_else(|e| panic!("migration '{}' must apply cleanly: {e}", m.id));
            applied.push(m.id);
        }
    }
    assert!(
        applied.contains(&"discovery-query-cutover".to_string())
            && applied.contains(&"rfc038-storage".to_string()),
        "expected the full revision climb (…, discovery-query-cutover) plus the \
         structural rfc038-storage migration to run; got: {applied:?}"
    );

    // Fixed point: a second pass over the registry must report nothing left
    // `Needed` — the exact "walks the registry... until none report Needed"
    // shape the srs#256 scoping comment proposed.
    let remaining: Vec<String> = list_migrations(&store)
        .expect("list_migrations must not error post-migration")
        .into_iter()
        .filter(|m| m.status == MigrationStatus::Needed)
        .map(|m| m.id)
        .collect();
    assert!(
        remaining.is_empty(),
        "migrations still Needed after one full registry pass: {remaining:?}"
    );

    // Revision-independent by construction: read the constant, never a
    // hardcoded literal, so this assertion does not go stale the next time
    // a data-model migration lands.
    let manifest = store
        .load_manifest()
        .expect("manifest must load post-ladder");
    let revision = manifest
        .extra
        .get("dataModelRevision")
        .and_then(|v| v.as_u64())
        .expect("manifest must carry a stamped dataModelRevision");
    assert_eq!(
        revision,
        srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION,
        "the fully-migrated fixture must land on the binary's current data-model revision"
    );

    // And the landing state is a genuinely valid repository, not just a
    // revision stamp with broken content underneath.
    let report = validate_repository(&store).expect("validate_repository must not error");
    assert!(
        report.is_ok(),
        "post-ladder repository must validate cleanly: {:?}",
        report.diagnostics
    );
}
