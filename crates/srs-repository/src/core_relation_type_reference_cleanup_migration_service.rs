//! `core-relation-type-reference-cleanup` migration (srs-rust#1341) — removes
//! repo-local relation-type definitions that are byte-for-byte identical
//! reference copies of an embedded core relation type.
//!
//! RFC-048 ruling 8 / [R11]/[R14]: core governs every relation-type key it
//! defines. A local copy identical to the governing core definition is
//! harmless (`core_package::merge_core_into_package` already treats it as a
//! no-op reference copy, never consulted for resolution or validation) but
//! also redundant — core already provides it implicitly (ADR-025) in every
//! repository with zero configuration. Removing it is safe because relations
//! reference relation types by bare `key` only, never by id, so no
//! relation's resolution changes.
//!
//! This migration removes **only** a definition that is identical to core
//! (full value equality, the same test `merge_core_into_package` and
//! `core_package::relation_type_shadow_diagnostics` use). A local copy that
//! *differs* from core (RFC-048's `relation-type-core-key-shadow`) is left
//! untouched — it needs a human decision (fix the copy to match core, or
//! accept the shadow warning), never an automated deletion of content that
//! doesn't match core.
//!
//! Structural, not revision-keyed, like `graduated-at-cleanup` and
//! `rfc038-storage`: the defect this cleans up (a now-redundant reference
//! copy) is not gated by `dataModelRevision`, so this migration stamps
//! nothing. Copies inside an *installed dependency* package are left to
//! their publisher (RFC-048 [R14]) — this only ever touches a package root's
//! own `relationTypes[]`, the same set `relation_type_shadow_diagnostics`
//! reads.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedReferenceCopy {
    pub key: String,
    pub path: String,
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreRelationTypeCleanupResult {
    pub removed: Vec<RemovedReferenceCopy>,
}

fn manifest_rel_for(root: &str) -> String {
    if root.is_empty() {
        "package.json".to_string()
    } else {
        format!("{}/package.json", root.trim_end_matches('/'))
    }
}

/// Every package root's own relation-type definitions that are identical
/// reference copies of a core definition: `(manifest path, declared entry as
/// package.json spells it, definition's own repo-relative path, definition)`.
fn find_identical_reference_copies(
    store: &dyn RepositoryStore,
    package_roots: &[String],
) -> Vec<(String, String, String, RelationTypeDefinition)> {
    let cp = crate::core_package::core_package();
    let mut out = Vec::new();
    for root in package_roots {
        let manifest_rel = manifest_rel_for(root);
        let Ok(manifest) = store.load_instance_json(&manifest_rel) else {
            continue;
        };
        let Some(rt_paths) = manifest.get("relationTypes").and_then(|v| v.as_array()) else {
            continue;
        };
        for p in rt_paths.iter().filter_map(|v| v.as_str()) {
            let rel = if root.is_empty() {
                p.to_string()
            } else {
                format!("{root}/{p}")
            };
            let Ok(def_json) = store.load_instance_json(&rel) else {
                continue;
            };
            let Ok(def) = serde_json::from_value::<RelationTypeDefinition>(def_json) else {
                continue;
            };
            if cp.relation_types.iter().any(|c| c == &def) {
                out.push((manifest_rel.clone(), p.to_string(), rel, def));
            }
        }
    }
    out.sort_by(|a, b| a.2.cmp(&b.2));
    out
}

/// Whether this repository has any local relation-type definition that is an
/// identical reference copy of a core definition.
pub fn migration_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    let cat = crate::catalog::build(store)?;
    Ok(!find_identical_reference_copies(store, &cat.package_roots).is_empty())
}

/// Apply the `core-relation-type-reference-cleanup` migration: delete every
/// identical reference copy and drop its entry from its package's
/// `relationTypes[]`. Idempotent — a repository with no such copies returns
/// an empty result.
pub fn migrate(store: &dyn RepositoryStore) -> Result<CoreRelationTypeCleanupResult, RepositoryError> {
    let cat = crate::catalog::build(store)?;
    let copies = find_identical_reference_copies(store, &cat.package_roots);
    let mut result = CoreRelationTypeCleanupResult::default();

    // Group by manifest so each package.json is read-modified-written once,
    // not once per removed entry.
    let mut by_manifest: BTreeMap<String, Vec<(String, String, RelationTypeDefinition)>> =
        BTreeMap::new();
    for (manifest_rel, declared_entry, def_rel_path, def) in copies {
        by_manifest
            .entry(manifest_rel)
            .or_default()
            .push((declared_entry, def_rel_path, def));
    }

    for (manifest_rel, entries) in by_manifest {
        let mut manifest = store.load_instance_json(&manifest_rel)?;
        let declared_to_remove: BTreeSet<&str> =
            entries.iter().map(|(d, _, _)| d.as_str()).collect();
        if let Some(arr) = manifest.get_mut("relationTypes").and_then(|v| v.as_array_mut()) {
            arr.retain(|v| {
                v.as_str()
                    .map(|s| !declared_to_remove.contains(s))
                    .unwrap_or(true)
            });
        }
        store.save_instance_json(&manifest_rel, &manifest)?;

        for (_, def_rel_path, def) in entries {
            store.delete_instance_file(&def_rel_path)?;
            result.removed.push(RemovedReferenceCopy {
                key: def.key,
                path: def_rel_path,
                id: def.id,
                version: def.version,
            });
        }
    }

    Ok(result)
}
