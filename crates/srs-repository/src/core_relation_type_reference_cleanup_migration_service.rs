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
//! their publisher (RFC-048 [R14], MUST): unlike
//! `relation_type_shadow_diagnostics` (a read-only warning, correct to scan
//! every package root so a shadow inside an installed dependency is still
//! surfaced), this migration **writes**, so it only ever touches the
//! repository's own primary package — always rooted at the fixed path
//! `"package"` (`store.rs`'s `load_package_from_dir(vfs, "package", ...)` for
//! the primary vs. a sub-package's own boundary path for everything else).
//! A byte-identical vendored copy inside an installed dependency (e.g.
//! `com.mudemocracy.governance`'s own `derived-from`/`evidences`) is left
//! alone even though it would pass the "identical to core" test — deciding
//! to drop it is the publisher's call, not this repository's.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use std::collections::BTreeSet;

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

/// The repository's primary package root, as `catalog::build`'s
/// `package_roots` spells it — always this fixed path (`store.rs`'s
/// `load_package_from_dir(vfs, "package", ...)` call for the primary
/// package; every other entry in `package_roots` is a sub-package boundary).
const PRIMARY_PACKAGE_ROOT: &str = "package";

/// The primary package's own relation-type definitions that are identical
/// reference copies of a core definition: `(declared entry as package.json
/// spells it, definition's own repo-relative path, definition)`. Never a
/// dependency sub-package's (RFC-048 [R14]) — see the module doc.
fn find_identical_reference_copies(
    store: &dyn RepositoryStore,
) -> Vec<(String, String, RelationTypeDefinition)> {
    let cp = crate::core_package::core_package();
    let mut out = Vec::new();
    let manifest_rel = format!("{PRIMARY_PACKAGE_ROOT}/package.json");
    let Ok(manifest) = store.load_instance_json(&manifest_rel) else {
        return out;
    };
    let Some(rt_paths) = manifest.get("relationTypes").and_then(|v| v.as_array()) else {
        return out;
    };
    for p in rt_paths.iter().filter_map(|v| v.as_str()) {
        let rel = format!("{PRIMARY_PACKAGE_ROOT}/{p}");
        let Ok(def_json) = store.load_instance_json(&rel) else {
            continue;
        };
        let Ok(def) = serde_json::from_value::<RelationTypeDefinition>(def_json) else {
            continue;
        };
        if cp.relation_types.iter().any(|c| c == &def) {
            out.push((p.to_string(), rel, def));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Whether the repository's primary package has any local relation-type
/// definition that is an identical reference copy of a core definition.
pub fn migration_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    Ok(!find_identical_reference_copies(store).is_empty())
}

/// Apply the `core-relation-type-reference-cleanup` migration: delete every
/// identical reference copy in the repository's **primary** package and drop
/// its entry from `package/package.json`'s `relationTypes[]` (never an
/// installed dependency's own package — RFC-048 [R14]). Idempotent — a
/// repository with no such copies returns an empty result.
pub fn migrate(store: &dyn RepositoryStore) -> Result<CoreRelationTypeCleanupResult, RepositoryError> {
    let copies = find_identical_reference_copies(store);
    let mut result = CoreRelationTypeCleanupResult::default();
    if copies.is_empty() {
        return Ok(result);
    }

    let manifest_rel = format!("{PRIMARY_PACKAGE_ROOT}/package.json");
    let mut manifest = store.load_instance_json(&manifest_rel)?;
    let declared_to_remove: BTreeSet<&str> =
        copies.iter().map(|(d, _, _)| d.as_str()).collect();
    if let Some(arr) = manifest.get_mut("relationTypes").and_then(|v| v.as_array_mut()) {
        arr.retain(|v| {
            v.as_str()
                .map(|s| !declared_to_remove.contains(s))
                .unwrap_or(true)
        });
    }
    store.save_instance_json(&manifest_rel, &manifest)?;

    for (_, def_rel_path, def) in copies {
        store.delete_instance_file(&def_rel_path)?;
        result.removed.push(RemovedReferenceCopy {
            key: def.key,
            path: def_rel_path,
            id: def.id,
            version: def.version,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_service::{create_package, create_relation_type, CreatePackageInput};
    use crate::store::memory::MemoryStore;

    #[test]
    fn migrate_removes_identical_reference_copy_from_primary_package() {
        let store = MemoryStore::default();
        let identical = crate::core_package::core_package().relation_types[0].clone();
        let key = identical.key.clone();
        create_relation_type(&store, identical, None).unwrap();

        assert!(migration_needed(&store).unwrap());
        let result = migrate(&store).unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].key, key);

        // Idempotent: nothing left to clean up.
        assert!(!migration_needed(&store).unwrap());
        let second = migrate(&store).unwrap();
        assert!(second.removed.is_empty());

        // The merged package still resolves the key via core — only the
        // redundant local file is gone, not the key itself.
        let package = store.load_package().unwrap();
        assert!(package.relation_type_definitions.iter().any(|rt| rt.key == key));
    }

    #[test]
    fn migrate_never_touches_an_installed_dependency_s_identical_copy() {
        // RFC-048 [R14] MUST: a copy inside an installed dependency package is
        // left to its publisher, even when it is byte-identical to core.
        let store = MemoryStore::default();
        create_package(
            &store,
            CreatePackageInput {
                id: "dep-pkg-001".to_string(),
                namespace: "com.dep".to_string(),
                name: "dep".to_string(),
                version: "1.0.0".to_string(),
                boundary_path: Some("packages/dep".to_string()),
            },
        )
        .unwrap();
        let identical = crate::core_package::core_package().relation_types[0].clone();
        create_relation_type(&store, identical, Some("packages/dep".to_string())).unwrap();

        assert!(
            !migration_needed(&store).unwrap(),
            "an installed dependency's own identical copy must never be reported as needing cleanup"
        );
        let result = migrate(&store).unwrap();
        assert!(
            result.removed.is_empty(),
            "an installed dependency's own identical copy must never be removed by this repository's migration"
        );

        // The file must still exist, untouched.
        let dep_manifest = store
            .load_instance_json("packages/dep/package.json")
            .unwrap();
        assert_eq!(
            dep_manifest["relationTypes"].as_array().unwrap().len(),
            1,
            "the dependency's own relationTypes entry must survive"
        );
    }
}
