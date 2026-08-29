//! RFC-032 migration #1 — `valueType` → `fieldType` (data-model revision 0 → 1).
//!
//! RFC-033 [R6] registers RFC-032's transform as **migration #1**
//! (`fromRevision: 0`, `toRevision: 1`) and requires it to stamp
//! `dataModelRevision: 1` on apply. This module is that migration.
//!
//! The transform itself lives in `srs-core`
//! ([`FieldType::from_legacy`](srs_core::types::field::FieldType::from_legacy))
//! and is applied on **load** by `field_json`, so a revision-0 repository is
//! already readable. Applying the migration is therefore a **persistence** step:
//! re-serialize every Field in the current model and stamp the manifest, so the
//! files on disk say what the engine already believes.
//!
//! Idempotent: re-running on a stamped repository is a no-op.

use crate::error::RepositoryError;
use crate::package_service::update_field;
use crate::store::RepositoryStore;
use serde::Serialize;

/// The data-model generation this build writes.
/// The revision this build reads and writes — the substrate escape-bag
/// rename `properties` -> `meta` (migration #5: srs#433/srs-rust#894, srs PR
/// #510). Tier 1 (TypedRecord) retirement is revision 4; RFC-040's metamodel
/// v1.1.0 engine sync is revision 3; RFC-039's carrier model is revision 2;
/// RFC-032's fieldType model is revision 1.
pub const CURRENT_DATA_MODEL_REVISION: u64 = 5;
/// The revision the RFC-032 `field-type` migration produces.
pub const FIELD_TYPE_REVISION: u64 = 1;
/// The revision RFC-040's metamodel v1.1.0 engine sync produces. This is
/// migration #3 (revision 2 -> 3). Unlike #1/#2 it stamps only — the RFC-040
/// train's construct retirals (`Field.defaultValue`/`deprecatedAt`,
/// `SourceReference.relationType`, `SectionSource.fixed-instances`/
/// `relation-query`, ...) are enforced unconditionally by the loader/schema
/// regardless of the stamped revision (srs decision 4f1e12e5 family), so a
/// repository that still loads successfully already carries none of them —
/// there is no content left to rewrite, only the generation number to record.
pub const METAMODEL_V1_1_0_REVISION: u64 = 3;
/// The revision Tier 1 (TypedRecord) retirement produces. This is migration
/// #4 (revision 3 -> 4), per srs#448 (rfc-decision-53635966) / srs PR #505.
/// Unlike #3, Tier-1 content is not rejected unconditionally by the loader —
/// a `typed-record.json` instance still loads fine today — so this migration
/// cannot be a pure re-stamp: it verifies the repository carries zero Tier-1
/// instances (the same corpus attestation #505 made for the spec repo) and
/// aborts, rather than silently stamping a false claim, if any remain.
pub const TIER1_REMOVAL_REVISION: u64 = 4;
/// The revision the substrate `properties` -> `meta` rename produces. This is
/// migration #5 (revision 4 -> 5), per srs#433 (rfc-decision-6fc7e142,
/// rfc-decision-628cf6c4) / srs PR #510. Unlike #3/#4 this migration is a
/// real content transform, not a re-stamp or a content-count guard: the
/// `srs-core` structs read either key (serde `alias`) but always write
/// `meta`, so applying this migration means re-persisting every
/// repository-owned `Term`, `RelationTypeDefinition`, standalone `Lifecycle`
/// (its `LifecycleState`/`LifecycleTransition` entries), and `Type` carrying
/// an inline `lifecycle` — which rewrites any lingering `properties` key to
/// `meta` on disk. Idempotent: a definition already using `meta` reproduces
/// byte for byte.
pub const SUBSTRATE_META_REVISION: u64 = 5;

/// The manifest property carrying the generation stamp (RFC-033 [R6] / #265).
pub const DATA_MODEL_REVISION_KEY: &str = "dataModelRevision";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldTypeMigrationResult {
    /// Repository-owned Fields rewritten in the current model.
    pub fields_migrated: usize,
    /// Fields skipped because this repository does not own them — the embedded
    /// core package is merged in at load (ADR-025) and is not ours to rewrite.
    pub fields_skipped_not_owned: usize,
    /// The revision the repository carried before this run (absent ⇒ 0).
    pub from_revision: u64,
    /// The revision stamped on the manifest.
    pub to_revision: u64,
}

/// Read a repository's declared data-model generation. Absent ⇒ 0 (RFC-033 [R6]).
pub fn data_model_revision(store: &dyn RepositoryStore) -> Result<u64, RepositoryError> {
    let manifest = store.load_manifest()?;
    Ok(manifest
        .extra
        .get(DATA_MODEL_REVISION_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0))
}

/// Whether this repository still needs migration #1.
pub fn migration_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    Ok(data_model_revision(store)? < FIELD_TYPE_REVISION)
}

/// Apply migration #1: persist every Field in the `fieldType` model and stamp
/// `dataModelRevision: 1`.
///
/// Fields are read through the loader, which has already upgraded any
/// revision-0 document in memory; writing them back is what makes the upgrade
/// durable. Rewriting an already-current Field reproduces its content byte for
/// byte, so the migration is idempotent whether or not the stamp is present.
pub fn migrate_field_types(
    store: &dyn RepositoryStore,
) -> Result<FieldTypeMigrationResult, RepositoryError> {
    let from_revision = data_model_revision(store)?;
    let package = store.load_package()?;

    let mut fields_migrated = 0usize;
    let mut fields_skipped_not_owned = 0usize;

    for field in &package.fields {
        // Core-package fields are merged in from the embedded bundle (ADR-025)
        // and are not owned by this repository — migrating them here would
        // write a copy the repo never declared.
        if crate::package_service::find_field_path(store, &field.id)?.is_none() {
            fields_skipped_not_owned += 1;
            continue;
        }
        update_field(store, field.clone())?;
        fields_migrated += 1;
    }

    stamp_data_model_revision(store, FIELD_TYPE_REVISION)?;

    Ok(FieldTypeMigrationResult {
        fields_migrated,
        fields_skipped_not_owned,
        from_revision,
        to_revision: FIELD_TYPE_REVISION,
    })
}

/// Write `dataModelRevision: <revision>` onto the manifest, preserving every
/// other property.
pub fn stamp_data_model_revision(
    store: &dyn RepositoryStore,
    revision: u64,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    manifest.extra.insert(
        DATA_MODEL_REVISION_KEY.to_string(),
        serde_json::json!(revision),
    );
    store.save_manifest(&manifest)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetamodelV1_1_0MigrationResult {
    /// The revision the repository carried before this run.
    pub from_revision: u64,
    /// The revision stamped on the manifest (`METAMODEL_V1_1_0_REVISION`).
    pub to_revision: u64,
}

/// Whether this repository still needs migration #3.
pub fn metamodel_v1_1_0_migration_needed(
    store: &dyn RepositoryStore,
) -> Result<bool, RepositoryError> {
    Ok(data_model_revision(store)? < METAMODEL_V1_1_0_REVISION)
}

/// Apply migration #3: stamp `dataModelRevision: 3`.
///
/// A pure re-stamp — see `METAMODEL_V1_1_0_REVISION`'s doc comment for why no
/// content rewrite is needed. Requires the RFC-039 carrier migration (#2) to
/// have run first, so the ladder is climbed in order like #1/#2.
pub fn migrate_metamodel_v1_1_0(
    store: &dyn RepositoryStore,
) -> Result<MetamodelV1_1_0MigrationResult, RepositoryError> {
    let from_revision = data_model_revision(store)?;
    let required = crate::rfc039_carrier_migration_service::CARRIER_REVISION;
    if from_revision < required {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "metamodel-v1-1-0 migration requires data-model revision >= {required} \
                 (found {from_revision}): run `srs repo apply-migration --id rfc039-carrier` first \
                 (RFC-039, migration #2)"
            ),
        });
    }
    stamp_data_model_revision(store, METAMODEL_V1_1_0_REVISION)?;
    Ok(MetamodelV1_1_0MigrationResult {
        from_revision,
        to_revision: METAMODEL_V1_1_0_REVISION,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tier1RemovalMigrationResult {
    /// The revision the repository carried before this run.
    pub from_revision: u64,
    /// The revision stamped on the manifest (`TIER1_REMOVAL_REVISION`).
    pub to_revision: u64,
}

/// Whether this repository still needs migration #4.
pub fn tier1_removal_migration_needed(
    store: &dyn RepositoryStore,
) -> Result<bool, RepositoryError> {
    Ok(data_model_revision(store)? < TIER1_REMOVAL_REVISION)
}

/// Apply migration #4: retire Tier 1 (TypedRecord) and stamp
/// `dataModelRevision: 4`.
///
/// Requires migration #3 to have run first (ladder order, as #1/#2/#3).
/// Then verifies the repository carries zero Tier-1 instances — every
/// real corpus checked for srs PR #505 had none (Tier 1 was specified but
/// never instantiated outside test fixtures) — and aborts rather than
/// stamp a false claim if any are found. A repository with real Tier-1
/// content must graduate each one to a Tier-2 Record (`note graduate`'s
/// Tier-2 counterpart, or an equivalent authored Record) before this
/// migration can apply; this migration does not do that conversion for
/// you, since it would be a content decision (a Type + field mapping),
/// not a mechanical re-stamp.
pub fn migrate_tier1_removal(
    store: &dyn RepositoryStore,
) -> Result<Tier1RemovalMigrationResult, RepositoryError> {
    let from_revision = data_model_revision(store)?;
    let required = METAMODEL_V1_1_0_REVISION;
    if from_revision < required {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "tier1-removal migration requires data-model revision >= {required} \
                 (found {from_revision}): run `srs repo apply-migration --id metamodel-v1-1-0` \
                 first (migration #3)"
            ),
        });
    }

    let catalog = store.catalog()?;
    let tier1_count = catalog
        .instances
        .iter()
        .filter(|entry| entry.tier == Some(1))
        .count();
    if tier1_count > 0 {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "tier1-removal migration cannot apply: {tier1_count} Tier-1 (TypedRecord) \
                 instance(s) remain. Tier 1 is retired (srs#448, rfc-decision-53635966) — \
                 graduate each one to a Tier-2 Record before running this migration."
            ),
        });
    }

    stamp_data_model_revision(store, TIER1_REMOVAL_REVISION)?;
    Ok(Tier1RemovalMigrationResult {
        from_revision,
        to_revision: TIER1_REMOVAL_REVISION,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubstratePropertiesToMetaMigrationResult {
    /// Repository-owned RelationTypeDefinitions rewritten.
    pub relation_types_migrated: usize,
    /// RelationTypeDefinitions skipped because this repository does not own
    /// them (embedded core package, ADR-025).
    pub relation_types_skipped_not_owned: usize,
    /// Repository-owned Vocabularies rewritten (each carries zero or more Terms).
    pub vocabularies_migrated: usize,
    /// Vocabularies skipped because this repository does not own them.
    pub vocabularies_skipped_not_owned: usize,
    /// Repository-owned standalone Lifecycles rewritten.
    pub lifecycles_migrated: usize,
    /// Standalone Lifecycles skipped because this repository does not own them.
    pub lifecycles_skipped_not_owned: usize,
    /// Repository-owned Types carrying an inline `lifecycle` facet rewritten.
    pub types_with_inline_lifecycle_migrated: usize,
    /// Types with an inline `lifecycle` facet skipped because not owned.
    pub types_with_inline_lifecycle_skipped_not_owned: usize,
    /// The revision the repository carried before this run.
    pub from_revision: u64,
    /// The revision stamped on the manifest (`SUBSTRATE_META_REVISION`).
    pub to_revision: u64,
}

/// Whether this repository still needs migration #5.
pub fn substrate_properties_to_meta_migration_needed(
    store: &dyn RepositoryStore,
) -> Result<bool, RepositoryError> {
    Ok(data_model_revision(store)? < SUBSTRATE_META_REVISION)
}

/// Apply migration #5: rename the substrate escape bag `properties` -> `meta`
/// on every repository-owned `Term`, `RelationTypeDefinition`, standalone
/// `Lifecycle`, and `Type` with an inline `lifecycle` facet, then stamp
/// `dataModelRevision: 5`.
///
/// The typed model already reads either key (serde `alias`) and always
/// serializes `meta` — so, like migration #1, applying this migration is a
/// **persistence** step: read every owned definition through the loader
/// (which has already upgraded any lingering `properties` key in memory) and
/// write it back, making the rename durable on disk. A definition with no
/// escape bag at all, or one already keyed `meta`, reproduces byte for byte.
///
/// Requires migration #4 (tier1-removal) to have run first (ladder order).
pub fn migrate_substrate_properties_to_meta(
    store: &dyn RepositoryStore,
) -> Result<SubstratePropertiesToMetaMigrationResult, RepositoryError> {
    let from_revision = data_model_revision(store)?;
    let required = TIER1_REMOVAL_REVISION;
    if from_revision < required {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "substrate-properties-to-meta migration requires data-model revision >= \
                 {required} (found {from_revision}): run `srs repo apply-migration --id \
                 tier1-removal` first (migration #4)"
            ),
        });
    }

    let package = store.load_package()?;

    let mut relation_types_migrated = 0usize;
    let mut relation_types_skipped_not_owned = 0usize;
    for rtd in &package.relation_type_definitions {
        if crate::package_service::find_relation_type_path(store, &rtd.id)?.is_none() {
            relation_types_skipped_not_owned += 1;
            continue;
        }
        crate::package_service::update_relation_type(store, rtd.clone())?;
        relation_types_migrated += 1;
    }

    let mut vocabularies_migrated = 0usize;
    let mut vocabularies_skipped_not_owned = 0usize;
    for vocabulary in &package.vocabularies {
        match crate::vocabulary_service::find_vocabulary_file_path(store, &vocabulary.id) {
            Ok(path) => {
                store.save_vocabulary(&path, vocabulary)?;
                vocabularies_migrated += 1;
            }
            Err(RepositoryError::NotFound { .. }) => {
                vocabularies_skipped_not_owned += 1;
            }
            Err(e) => return Err(e),
        }
    }

    let mut lifecycles_migrated = 0usize;
    let mut lifecycles_skipped_not_owned = 0usize;
    for lifecycle in &package.lifecycles {
        match crate::lifecycle_service::find_lifecycle_path(store, &lifecycle.id)? {
            Some((path, _owner)) => {
                store.save_lifecycle(&path, lifecycle)?;
                lifecycles_migrated += 1;
            }
            None => {
                lifecycles_skipped_not_owned += 1;
            }
        }
    }

    let mut types_with_inline_lifecycle_migrated = 0usize;
    let mut types_with_inline_lifecycle_skipped_not_owned = 0usize;
    for record_type in &package.record_types {
        if record_type.lifecycle.is_none() {
            // No inline `Type.lifecycle` facet — nothing on this Type carries
            // the substrate escape bag.
            continue;
        }
        if crate::package_service::find_type_path(store, &record_type.id)?.is_none() {
            types_with_inline_lifecycle_skipped_not_owned += 1;
            continue;
        }
        crate::package_service::update_type(store, record_type.clone())?;
        types_with_inline_lifecycle_migrated += 1;
    }

    stamp_data_model_revision(store, SUBSTRATE_META_REVISION)?;

    Ok(SubstratePropertiesToMetaMigrationResult {
        relation_types_migrated,
        relation_types_skipped_not_owned,
        vocabularies_migrated,
        vocabularies_skipped_not_owned,
        lifecycles_migrated,
        lifecycles_skipped_not_owned,
        types_with_inline_lifecycle_migrated,
        types_with_inline_lifecycle_skipped_not_owned,
        from_revision,
        to_revision: SUBSTRATE_META_REVISION,
    })
}

#[cfg(test)]
mod substrate_properties_to_meta_tests {
    use super::*;
    use crate::store::FileStore;

    /// Real file-tree fixture: a rev-4 repository whose single owned
    /// `RelationTypeDefinition` still carries the pre-rename `properties` key
    /// on disk, verbatim (not round-tripped through the typed model first —
    /// this is what a genuinely pre-#510 repository looks like).
    fn rev4_repo_with_legacy_properties_relation_type() -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".srs")).unwrap();
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::json!({
                "repositoryId": "00000000-0000-4000-8000-000000000001",
                "dataModelRevision": 4
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("package/relation-types")).unwrap();
        std::fs::write(
            temp.path().join("package/package.json"),
            serde_json::json!({
                "id": "pkg",
                "namespace": "com.test",
                "name": "test",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": ["relation-types/probe.json"]
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            temp.path().join("package/relation-types/probe.json"),
            serde_json::json!({
                "id": "a1000001-0000-4000-b000-000000000099",
                "version": 1,
                "relationType": "com.test/probe",
                "namespace": "com.test",
                "label": "Probe",
                "description": "A probe relation type.",
                "category": "association",
                "createdAt": "2026-01-01T00:00:00Z",
                "properties": {"color": "blue"}
            })
            .to_string(),
        )
        .unwrap();
        temp
    }

    #[test]
    fn substrate_properties_to_meta_requires_tier1_removal_first() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".srs")).unwrap();
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::json!({
                "repositoryId": "00000000-0000-4000-8000-000000000002",
                "dataModelRevision": 3
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("package")).unwrap();
        std::fs::write(
            temp.path().join("package/package.json"),
            serde_json::json!({
                "id": "pkg", "namespace": "com.test", "name": "test", "version": "1.0.0",
                "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();
        let store = FileStore::new(temp.path());
        let err = migrate_substrate_properties_to_meta(&store).unwrap_err();
        assert!(
            err.to_string().contains("tier1-removal"),
            "must name the missing prerequisite migration, got: {err}"
        );
        assert_eq!(data_model_revision(&store).unwrap(), 3);
    }

    /// Red-then-green proof for srs-rust#894: before the migration, the raw
    /// file on disk carries the legacy `properties` key and the manifest is
    /// stamped `dataModelRevision: 4`. Reverting `SUBSTRATE_META_REVISION`'s
    /// registration (or this function) reproduces that pre-fix state; after
    /// applying the migration, the on-disk file says `meta` (never
    /// `properties`) and the manifest is stamped `5`.
    #[test]
    fn substrate_properties_to_meta_rewrites_legacy_key_and_stamps_revision_5() {
        let temp = rev4_repo_with_legacy_properties_relation_type();
        let raw_before =
            std::fs::read_to_string(temp.path().join("package/relation-types/probe.json")).unwrap();
        assert!(
            raw_before.contains("\"properties\""),
            "fixture must start with the legacy key"
        );

        let store = FileStore::new(temp.path());
        assert_eq!(data_model_revision(&store).unwrap(), 4);

        let result = migrate_substrate_properties_to_meta(&store).unwrap();
        assert_eq!(result.from_revision, 4);
        assert_eq!(result.to_revision, 5);
        assert_eq!(result.relation_types_migrated, 1);
        // The embedded core package's canonical relation types (contains,
        // depends-on, supersedes, ...) are merged in at load (ADR-025) but
        // not owned by this repository — must be skipped, not rewritten.
        assert!(
            result.relation_types_skipped_not_owned > 0,
            "core-package relation types must be counted as skipped, not migrated"
        );

        assert_eq!(data_model_revision(&store).unwrap(), 5);

        let raw_after =
            std::fs::read_to_string(temp.path().join("package/relation-types/probe.json")).unwrap();
        assert!(
            raw_after.contains("\"meta\""),
            "the on-disk file must be rewritten to use `meta`: {raw_after}"
        );
        assert!(
            !raw_after.contains("\"properties\""),
            "the on-disk file must not retain the retired `properties` key: {raw_after}"
        );
    }

    #[test]
    fn substrate_properties_to_meta_is_idempotent() {
        let temp = rev4_repo_with_legacy_properties_relation_type();
        let store = FileStore::new(temp.path());
        migrate_substrate_properties_to_meta(&store).unwrap();

        // Re-running after the stamp is already at 5 is a no-op per the
        // registry's `status_fn` (AlreadyApplied), but calling the migration
        // function directly a second time must still leave the content
        // stable — same guarantee migration #1 (field-type) documents. (A
        // second `save_relation_type_definition` call reorders the
        // already-unrelated `$schema` key per its own injection helper, so
        // this asserts content stability, not full byte identity.)
        stamp_data_model_revision(&store, TIER1_REMOVAL_REVISION).unwrap();
        migrate_substrate_properties_to_meta(&store).unwrap();
        let raw_twice =
            std::fs::read_to_string(temp.path().join("package/relation-types/probe.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw_twice).unwrap();
        assert_eq!(parsed["meta"]["color"], "blue");
        assert!(!raw_twice.contains("\"properties\""));
    }
}
