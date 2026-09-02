//! `composition-cutover` migration (srs-rust#910) — migration #6, data-model
//! revision 5 -> 6.
//!
//! ## Why this exists
//!
//! Three riders land as one shared stamp (RFC-038/039 "composed... in one
//! first-party cutover... neither ships alone" precedent, owner ruling on
//! srs#272's consequence map: "no 6-then-7"):
//!
//! 1. **`DocumentView` -> `Composition`** (`rfc-decision-92d2da05`): the
//!    package-relative `document-views/` directory becomes `compositions/`,
//!    `package.json`'s `documentViews` array becomes `compositions`, each
//!    moved file's `$schema` pointer is repointed from `document-view.json`
//!    to `composition.json`, and `manifest.renderedPresentations[].viewId`
//!    (unconsumed by srs-rust today, srs-rust#567) becomes `compositionId`.
//! 2. **The `semanticObjectType` collapse** (owner ruling on #383,
//!    srs#372/#481/#524, `rfc-decision-c8704763`): every Type's transitional
//!    `semanticObjectType` key is stripped (it was never real Type surface —
//!    `RecordType` carries no successor field, by design); every Composition
//!    section's `type-query` source renames `semanticObjectType` to the
//!    KEYED `typeKey` (same value — the resolution behind it was already
//!    real `namespace/name` Type-keyed selection, only the name was wrong);
//!    every RelationTypeDefinition's `requireSameSemanticObjectType` renames
//!    to `requireSameType`, and `allowedSourceTypes`/`allowedTargetTypes`
//!    are dropped with no successor (srs#372: both were keyed on the same
//!    retired string and could never fire against a schema-conforming
//!    Record/Note in the first place).
//! 3. **`dependencyRefs` -> `packageDependencies`** (srs-rust#873, the parked
//!    srs#487/Train-4a-2 write, folded onto this shared stamp per the map's
//!    accepted, unvetoed proposal on srs#524): each package.json's
//!    `dependencyRefs` array renames to `packageDependencies`.
//!
//! None of these renames carry a serde alias — `RecordType`,
//! `RelationTypeDefinition`, and the `Composition`/`SectionSource` types all
//! `deny_unknown_fields` the old keys outright. An un-migrated rev-5
//! repository therefore fails the checked catalog ([R24]) the moment any one
//! such file is touched, so — like `rfc038-storage`, `graduated-at-cleanup`,
//! and `revisions-sidecar-cleanup` — this migration reads and writes the raw
//! file tree directly, never `store.catalog()`, and is the sole sanctioned
//! reader of the old shapes.
//!
//! ## Scope: first-party corpus only
//!
//! Per the #256 corpus-boundary ruling (also applied by srs#524's own
//! execution plan): this migration transforms whatever package roots the
//! repository declares (the primary `package` plus every local
//! `manifest.packageRefs` sub-package, the same discovery `rfc039_carrier_migration_service`
//! uses) — there is no external-repository compatibility obligation.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::Value;

/// Old package-relative directory name for Compositions (formerly DocumentViews).
const OLD_COMPOSITIONS_DIR: &str = "document-views";
const NEW_COMPOSITIONS_DIR: &str = "compositions";
const OLD_SCHEMA_SUFFIX: &str = "/document-view.json";
const NEW_SCHEMA_SUFFIX: &str = "/composition.json";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositionCutoverResult {
    pub from_revision: u64,
    pub to_revision: u64,
    /// Composition files moved from `document-views/` to `compositions/`,
    /// with their `$schema` pointer and any `type-query` `typeKey` rewritten.
    pub compositions_moved: usize,
    /// Package boundaries whose `package.json` `documentViews` array renamed
    /// to `compositions` (only counted when the key was actually present).
    pub packages_with_compositions_key_renamed: usize,
    /// Type definitions that had a transitional `semanticObjectType` key stripped.
    pub types_stripped_semantic_object_type: usize,
    /// RelationTypeDefinitions re-keyed: `requireSameSemanticObjectType` ->
    /// `requireSameType`, `allowedSourceTypes`/`allowedTargetTypes` dropped.
    pub relation_types_rekeyed: usize,
    /// Package boundaries whose `package.json` `dependencyRefs` array renamed
    /// to `packageDependencies` (srs-rust#873 fold).
    pub packages_with_package_dependencies_renamed: usize,
    /// `manifest.renderedPresentations[].viewId` entries renamed to
    /// `compositionId` (unconsumed by srs-rust today, srs-rust#567; renamed
    /// defensively so a repository carrying the schema-declared key still
    /// converges).
    pub manifest_rendered_presentations_rekeyed: usize,
}

fn abort(path: &str, reason: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::InvalidSnapshotData {
        message: format!("composition-cutover migration aborted at {path}: {reason}"),
    }
}

fn list_of_strings(doc: &Value, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Package roots: the primary `package` plus every local `manifest.packageRefs`
/// sub-package — mirrors `rfc039_carrier_migration_service`'s discovery.
fn package_roots(store: &dyn RepositoryStore) -> Vec<String> {
    let mut roots: Vec<String> = vec!["package".to_string()];
    if let Ok(manifest) = store.load_manifest() {
        for r in manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if r.get("mode").and_then(|m| m.as_str()) == Some("local") {
                if let Some(path) = r.get("path").and_then(|p| p.as_str()) {
                    roots.push(path.to_string());
                }
            }
        }
    }
    roots
}

/// Renames `semanticObjectType` -> `typeKey` on a single `SectionSource` JSON
/// object, only when it is a `type-query` section. Any other section shape
/// (`fixed-instances`, `relation-query`, `container-subset`) is untouched.
fn rekey_type_query_source(source: &mut Value) -> bool {
    let Some(obj) = source.as_object_mut() else {
        return false;
    };
    if obj.get("type").and_then(|v| v.as_str()) != Some("type-query") {
        return false;
    }
    if let Some(v) = obj.remove("semanticObjectType") {
        obj.insert("typeKey".to_string(), v);
        true
    } else {
        false
    }
}

/// Moves one Composition (formerly DocumentView) file: rewrites `$schema` and
/// any `type-query` section's `semanticObjectType` -> `typeKey`, writes the
/// content to `new_path`, then deletes `old_path`. Returns `true` when the
/// file existed and was moved.
fn migrate_one_composition_file(
    store: &dyn RepositoryStore,
    old_path: &str,
    new_path: &str,
) -> Result<bool, RepositoryError> {
    let mut doc = match store.load_instance_json(old_path) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    if let Some(schema) = doc.get("$schema").and_then(|v| v.as_str()) {
        if let Some(prefix) = schema.strip_suffix(OLD_SCHEMA_SUFFIX) {
            doc["$schema"] = Value::String(format!("{prefix}{NEW_SCHEMA_SUFFIX}"));
        }
    }

    if let Some(sections) = doc.get_mut("sections").and_then(|s| s.as_array_mut()) {
        for section in sections.iter_mut() {
            if let Some(source) = section.get_mut("source") {
                rekey_type_query_source(source);
            }
        }
    }

    store.save_instance_json(new_path, &doc)?;
    store.delete_instance_file(old_path)?;
    Ok(true)
}

/// Strips the transitional `semanticObjectType` key from a raw Type document,
/// in place. Returns `true` when the key was present.
fn strip_semantic_object_type(doc: &mut Value) -> bool {
    doc.as_object_mut()
        .map(|obj| obj.remove("semanticObjectType").is_some())
        .unwrap_or(false)
}

/// Re-keys a raw RelationTypeDefinition document: `requireSameSemanticObjectType`
/// -> `requireSameType`, and drops `allowedSourceTypes`/`allowedTargetTypes`.
/// Returns `true` when any of the three keys were present.
fn rekey_relation_type(doc: &mut Value) -> bool {
    let Some(obj) = doc.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    if let Some(v) = obj.remove("requireSameSemanticObjectType") {
        obj.insert("requireSameType".to_string(), v);
        changed = true;
    }
    changed |= obj.remove("allowedSourceTypes").is_some();
    changed |= obj.remove("allowedTargetTypes").is_some();
    changed
}

fn migrate_package_root(
    store: &dyn RepositoryStore,
    root: &str,
    result: &mut CompositionCutoverResult,
) -> Result<(), RepositoryError> {
    let pkg_index_path = format!("{root}/package.json");
    let mut pkg_index = match store.load_instance_json(&pkg_index_path) {
        Ok(v) => v,
        // A missing root (packageless repository, or an undeclared sub-package
        // path) has nothing here to migrate.
        Err(_) => return Ok(()),
    };
    let Some(pkg_obj) = pkg_index.as_object_mut() else {
        return Err(abort(&pkg_index_path, "package.json is not a JSON object"));
    };

    // ── documentViews -> compositions ──────────────────────────────────────
    if let Some(old_paths_val) = pkg_obj.remove("documentViews") {
        let old_paths: Vec<String> = old_paths_val
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();

        let mut new_paths: Vec<String> = Vec::new();
        if !old_paths.is_empty() {
            store.ensure_instance_dir(&format!("{root}/{NEW_COMPOSITIONS_DIR}"))?;
        }
        for rel in &old_paths {
            let old_file = format!("{root}/{rel}");
            let new_rel =
                if let Some(basename) = rel.strip_prefix(&format!("{OLD_COMPOSITIONS_DIR}/")) {
                    format!("{NEW_COMPOSITIONS_DIR}/{basename}")
                } else {
                    // Already lives outside the conventional dir (unusual, but
                    // not this migration's business to relocate further) — keep
                    // the relative path as authored.
                    rel.clone()
                };
            let new_file = format!("{root}/{new_rel}");
            if migrate_one_composition_file(store, &old_file, &new_file)? {
                result.compositions_moved += 1;
            }
            new_paths.push(new_rel);
        }
        pkg_obj.insert(
            "compositions".to_string(),
            Value::Array(new_paths.into_iter().map(Value::String).collect()),
        );
        result.packages_with_compositions_key_renamed += 1;
    }

    // ── dependencyRefs -> packageDependencies (srs-rust#873 fold) ──────────
    if let Some(v) = pkg_obj.remove("dependencyRefs") {
        pkg_obj.insert("packageDependencies".to_string(), v);
        result.packages_with_package_dependencies_renamed += 1;
    }

    // ── Type definitions: strip semanticObjectType ─────────────────────────
    for rel in list_of_strings(&pkg_index, "types") {
        let path = format!("{root}/{rel}");
        let mut doc = store.load_instance_json(&path)?;
        if strip_semantic_object_type(&mut doc) {
            store.save_instance_json(&path, &doc)?;
            result.types_stripped_semantic_object_type += 1;
        }
    }

    // ── RelationTypeDefinitions: re-key E4 constraints ─────────────────────
    for rel in list_of_strings(&pkg_index, "relationTypes") {
        let path = format!("{root}/{rel}");
        let mut doc = store.load_instance_json(&path)?;
        if rekey_relation_type(&mut doc) {
            store.save_instance_json(&path, &doc)?;
            result.relation_types_rekeyed += 1;
        }
    }

    store.save_instance_json(&pkg_index_path, &pkg_index)?;
    Ok(())
}

/// Apply migration #6: the Composition rename, the semanticObjectType
/// collapse, and the packageDependencies fold, then stamp
/// `dataModelRevision: 6`.
///
/// Requires migration #5 (`substrate-properties-to-meta`) to have run first
/// (ladder order). Aborts rather than partially migrates (ADR-021) — a batch
/// store rolls back on any error.
pub fn migrate_composition_cutover(
    store: &dyn RepositoryStore,
) -> Result<CompositionCutoverResult, RepositoryError> {
    let from_revision = crate::field_type_migration_service::data_model_revision(store)?;
    let required = crate::field_type_migration_service::SUBSTRATE_META_REVISION;
    if from_revision < required {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "composition-cutover migration requires data-model revision >= {required} \
                 (found {from_revision}): run `srs repo apply-migration --id \
                 substrate-properties-to-meta` first (migration #5)"
            ),
        });
    }

    store.begin_batch();
    match run_migration(store, from_revision) {
        Ok(result) => {
            store.commit_batch()?;
            Ok(result)
        }
        Err(e) => {
            store.abort_batch();
            Err(e)
        }
    }
}

fn run_migration(
    store: &dyn RepositoryStore,
    from_revision: u64,
) -> Result<CompositionCutoverResult, RepositoryError> {
    let mut result = CompositionCutoverResult {
        from_revision,
        to_revision: crate::field_type_migration_service::COMPOSITION_CUTOVER_REVISION,
        ..Default::default()
    };

    for root in package_roots(store) {
        migrate_package_root(store, &root, &mut result)?;
    }

    // manifest.renderedPresentations[].viewId -> compositionId (unconsumed by
    // srs-rust today, srs-rust#567 — renamed defensively for convergence).
    let mut manifest = store.load_manifest()?;
    if let Some(presentations) = manifest
        .extra
        .get_mut("renderedPresentations")
        .and_then(|v| v.as_array_mut())
    {
        for entry in presentations.iter_mut() {
            if let Some(obj) = entry.as_object_mut() {
                if let Some(v) = obj.remove("viewId") {
                    obj.insert("compositionId".to_string(), v);
                    result.manifest_rendered_presentations_rekeyed += 1;
                }
            }
        }
    }
    manifest.extra.insert(
        crate::field_type_migration_service::DATA_MODEL_REVISION_KEY.to_string(),
        serde_json::json!(result.to_revision),
    );
    store.save_manifest(&manifest)?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;

    /// A store stamped at revision 5 with a primary package carrying one
    /// `type-query` Composition (in `document-views/`), one Type with a
    /// transitional `semanticObjectType`, one RelationTypeDefinition with all
    /// three retired E4 keys, and a `dependencyRefs` array.
    fn rev5_store_with_full_shape() -> MemoryStore {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(5));
        store.save_manifest(&manifest).unwrap();

        store
            .save_instance_json(
                "package/package.json",
                &json!({
                    "id": "00000000-0000-4000-8000-0000000000aa",
                    "namespace": "com.test",
                    "name": "primary",
                    "version": "1.0.0",
                    "fields": [],
                    "types": ["types/decision-type.json"],
                    "relationTypes": ["relation-types/links.json"],
                    "documentViews": ["document-views/decisions.json"],
                    "dependencyRefs": [
                        {"namespace": "com.other", "name": "shared", "version": "1.0.0"}
                    ]
                }),
            )
            .unwrap();

        store
            .save_instance_json(
                "package/types/decision-type.json",
                &json!({
                    "id": "00000000-0000-4000-8000-0000000000bb",
                    "namespace": "com.test",
                    "name": "decision",
                    "version": 1,
                    "description": "A decision",
                    "fields": [],
                    "createdAt": "2026-01-01T00:00:00Z",
                    "semanticObjectType": "com.test/decision"
                }),
            )
            .unwrap();

        store
            .save_instance_json(
                "package/relation-types/links.json",
                &json!({
                    "id": "00000000-0000-4000-8000-0000000000cc",
                    "version": 1,
                    "key": "links",
                    "namespace": "com.test",
                    "label": "Links",
                    "description": "source links to target",
                    "category": "association",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "allowedSourceTypes": ["com.test/decision"],
                    "allowedTargetTypes": ["com.test/decision"],
                    "requireSameSemanticObjectType": true
                }),
            )
            .unwrap();

        store
            .save_instance_json(
                "package/document-views/decisions.json",
                &json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/document-view.json",
                    "id": "00000000-0000-4000-8000-0000000000dd",
                    "namespace": "com.test",
                    "name": "decisions",
                    "version": 1,
                    "description": "Decisions",
                    "sections": [
                        {
                            "sectionId": "body",
                            "order": 0,
                            "source": {
                                "type": "type-query",
                                "semanticObjectType": "com.test/decision"
                            }
                        }
                    ],
                    "createdAt": "2026-01-01T00:00:00Z"
                }),
            )
            .unwrap();

        store
    }

    #[test]
    fn migration_needed_below_revision_6() {
        let store = rev5_store_with_full_shape();
        assert!(
            crate::field_type_migration_service::composition_cutover_migration_needed(&store)
                .unwrap()
        );
    }

    #[test]
    fn migrates_every_shape_and_stamps_revision_6() {
        let store = rev5_store_with_full_shape();
        let result = migrate_composition_cutover(&store).unwrap();

        assert_eq!(result.from_revision, 5);
        assert_eq!(result.to_revision, 6);
        assert_eq!(result.compositions_moved, 1);
        assert_eq!(result.packages_with_compositions_key_renamed, 1);
        assert_eq!(result.types_stripped_semantic_object_type, 1);
        assert_eq!(result.relation_types_rekeyed, 1);
        assert_eq!(result.packages_with_package_dependencies_renamed, 1);

        // package.json: new keys present, old keys gone.
        let pkg = store.load_instance_json("package/package.json").unwrap();
        assert_eq!(pkg["compositions"], json!(["compositions/decisions.json"]));
        assert!(pkg.get("documentViews").is_none());
        assert!(pkg.get("dependencyRefs").is_none());
        assert!(pkg["packageDependencies"].is_array());

        // Composition file moved, $schema repointed, typeKey renamed.
        assert!(store
            .load_instance_json("package/document-views/decisions.json")
            .is_err());
        let moved = store
            .load_instance_json("package/compositions/decisions.json")
            .unwrap();
        assert_eq!(
            moved["$schema"],
            "https://srs.semanticops.com/schema/2.0/composition.json"
        );
        assert_eq!(
            moved["sections"][0]["source"]["typeKey"],
            "com.test/decision"
        );
        assert!(moved["sections"][0]["source"]
            .get("semanticObjectType")
            .is_none());

        // Type: semanticObjectType stripped.
        let ty = store
            .load_instance_json("package/types/decision-type.json")
            .unwrap();
        assert!(ty.get("semanticObjectType").is_none());

        // RelationTypeDefinition: re-keyed.
        let rtd = store
            .load_instance_json("package/relation-types/links.json")
            .unwrap();
        assert_eq!(rtd["requireSameType"], true);
        assert!(rtd.get("requireSameSemanticObjectType").is_none());
        assert!(rtd.get("allowedSourceTypes").is_none());
        assert!(rtd.get("allowedTargetTypes").is_none());

        // Manifest stamped.
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            6
        );
        assert!(
            !crate::field_type_migration_service::composition_cutover_migration_needed(&store)
                .unwrap(),
            "must be idempotent — nothing left to do"
        );
    }

    #[test]
    fn refuses_below_substrate_meta_revision() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(4));
        store.save_manifest(&manifest).unwrap();

        let err = migrate_composition_cutover(&store).unwrap_err();
        assert!(err.to_string().contains("substrate-properties-to-meta"));
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            4,
            "a refused apply must not stamp the manifest"
        );
    }

    #[test]
    fn rendered_presentations_view_id_renamed_to_composition_id() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(5));
        manifest.extra.insert(
            "renderedPresentations".to_string(),
            json!([{"viewId": "00000000-0000-4000-8000-0000000000ee", "isDefault": true}]),
        );
        store.save_manifest(&manifest).unwrap();

        let result = migrate_composition_cutover(&store).unwrap();
        assert_eq!(result.manifest_rendered_presentations_rekeyed, 1);

        let manifest = store.load_manifest().unwrap();
        let presentations = manifest.extra.get("renderedPresentations").unwrap();
        assert_eq!(
            presentations[0]["compositionId"],
            "00000000-0000-4000-8000-0000000000ee"
        );
        assert!(presentations[0].get("viewId").is_none());
    }

    #[test]
    fn no_op_when_nothing_to_migrate() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(5));
        store.save_manifest(&manifest).unwrap();

        let result = migrate_composition_cutover(&store).unwrap();
        assert_eq!(result.compositions_moved, 0);
        assert_eq!(result.types_stripped_semantic_object_type, 0);
        assert_eq!(result.relation_types_rekeyed, 0);
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            6
        );
    }
}
