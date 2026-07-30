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
pub const CURRENT_DATA_MODEL_REVISION: u64 = 1;

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
    Ok(data_model_revision(store)? < CURRENT_DATA_MODEL_REVISION)
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

    stamp_data_model_revision(store, CURRENT_DATA_MODEL_REVISION)?;

    Ok(FieldTypeMigrationResult {
        fields_migrated,
        fields_skipped_not_owned,
        from_revision,
        to_revision: CURRENT_DATA_MODEL_REVISION,
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
