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
/// The revision this build reads and writes — Tier 1 (TypedRecord) retirement
/// (migration #4: srs#448/srs-rust#882, srs PR #505). RFC-040's metamodel
/// v1.1.0 engine sync is revision 3; RFC-039's carrier model is revision 2;
/// RFC-032's fieldType model is revision 1.
pub const CURRENT_DATA_MODEL_REVISION: u64 = 4;
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
