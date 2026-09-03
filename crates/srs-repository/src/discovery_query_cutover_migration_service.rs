//! `discovery-query-cutover` migration (srs-rust#924) — migration #7, data-model
//! revision 6 -> 7.
//!
//! ## Why this exists
//!
//! Two riders land as one shared stamp (the `composition-cutover` precedent,
//! owner ruling on srs#525's two parks):
//!
//! 1. **`SectionSource.type-query` -> `discovery-query`** (`rfc-decision-cce3c00e`):
//!    a section's selection predicate moves onto the one structured query
//!    mechanism (`DiscoveryQuery`, ext:discovery) — `typeKey` (KEYED
//!    `namespace/name`) splits into independent `query.typeNamespace`/
//!    `query.typeName`; `lifecycleState`/`lifecycleStates`/
//!    `excludeLifecycleStates` move onto `query`. `containerIds`/
//!    `containerScope` stay on `SectionSource` itself (arrangement, not
//!    selection — unchanged by this collapse).
//! 2. **`Composition`/`View` ExportConfig unification** (`rfc-decision-9ee14517`):
//!    a Composition's own top-level `format`/`preamble` retire in favor of the
//!    same `exportConfig: { format, preamble, omitEmptyFields }` shape `View`
//!    already carries.
//!
//! Neither rename carries a serde alias — `SectionSource`/`Composition`
//! `deny_unknown_fields` the old keys outright. An un-migrated rev-6
//! repository therefore fails the checked catalog ([R24]) the moment any one
//! such file is touched, so — like `composition-cutover` before it — this
//! migration reads and writes the raw file tree directly, never
//! `store.catalog()`, and is the sole sanctioned reader of the old shapes.
//!
//! ## Scope: first-party corpus only
//!
//! Per the #256 corpus-boundary ruling (also applied by `composition-cutover`):
//! this migration transforms whatever package roots the repository declares
//! (the primary `package` plus every local `manifest.packageRefs`
//! sub-package) — there is no external-repository compatibility obligation.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryQueryCutoverResult {
    pub from_revision: u64,
    pub to_revision: u64,
    /// `type-query` sections rewritten to `discovery-query`.
    pub sections_migrated_to_discovery_query: usize,
    /// Composition files whose top-level `format`/`preamble` moved onto `exportConfig`.
    pub compositions_export_config_migrated: usize,
}

fn abort(path: &str, reason: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::InvalidSnapshotData {
        message: format!("discovery-query-cutover migration aborted at {path}: {reason}"),
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
/// sub-package — mirrors `composition_cutover_migration_service`'s discovery.
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

/// Rewrites one raw `SectionSource` JSON object in place: `type-query` ->
/// `discovery-query`, `typeKey`/`lifecycleState`/`lifecycleStates`/
/// `excludeLifecycleStates` folded into a nested `query` object. Any other
/// section shape (`container-subset`, and the already-dead `fixed-instances`/
/// `relation-query`) is untouched. Returns `true` when rewritten.
fn rekey_type_query_source(source: &mut Value) -> bool {
    let Some(obj) = source.as_object_mut() else {
        return false;
    };
    if obj.get("type").and_then(|v| v.as_str()) != Some("type-query") {
        return false;
    }

    let mut query = serde_json::Map::new();
    if let Some(Value::String(type_key)) = obj.remove("typeKey") {
        match type_key.split_once('/') {
            Some((ns, name)) => {
                query.insert("typeNamespace".to_string(), Value::String(ns.to_string()));
                query.insert("typeName".to_string(), Value::String(name.to_string()));
            }
            // Malformed KEYED string (no namespace separator) — carry it
            // forward as typeName alone; nothing upstream can split it further.
            None => {
                query.insert("typeName".to_string(), Value::String(type_key));
            }
        }
    }
    for key in [
        "lifecycleState",
        "lifecycleStates",
        "excludeLifecycleStates",
    ] {
        if let Some(v) = obj.remove(key) {
            query.insert(key.to_string(), v);
        }
    }

    obj.insert(
        "type".to_string(),
        Value::String("discovery-query".to_string()),
    );
    obj.insert("query".to_string(), Value::Object(query));
    true
}

/// Moves a Composition's top-level `format`/`preamble` onto `exportConfig`, in
/// place. Returns `true` when either key was present.
fn migrate_export_config(doc: &mut Value) -> bool {
    let Some(obj) = doc.as_object_mut() else {
        return false;
    };
    let format = obj.remove("format");
    let preamble = obj.remove("preamble");
    if format.is_none() && preamble.is_none() {
        return false;
    }
    let mut export_config = match obj.remove("exportConfig") {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    if let Some(v) = format {
        export_config.insert("format".to_string(), v);
    }
    if let Some(v) = preamble {
        export_config.insert("preamble".to_string(), v);
    }
    obj.insert("exportConfig".to_string(), Value::Object(export_config));
    true
}

/// Migrates one Composition file: every `type-query` section -> `discovery-query`,
/// top-level `format`/`preamble` -> `exportConfig`. Returns
/// `(any_section_migrated, export_config_migrated)`.
fn migrate_one_composition_file(
    store: &dyn RepositoryStore,
    path: &str,
) -> Result<(usize, bool), RepositoryError> {
    let mut doc = store.load_instance_json(path)?;

    let mut sections_migrated = 0usize;
    if let Some(sections) = doc.get_mut("sections").and_then(|s| s.as_array_mut()) {
        for section in sections.iter_mut() {
            if let Some(source) = section.get_mut("source") {
                if rekey_type_query_source(source) {
                    sections_migrated += 1;
                }
            }
        }
    }
    let export_config_migrated = migrate_export_config(&mut doc);

    if sections_migrated > 0 || export_config_migrated {
        store.save_instance_json(path, &doc)?;
    }
    Ok((sections_migrated, export_config_migrated))
}

fn migrate_package_root(
    store: &dyn RepositoryStore,
    root: &str,
    result: &mut DiscoveryQueryCutoverResult,
) -> Result<(), RepositoryError> {
    let pkg_index_path = format!("{root}/package.json");
    let pkg_index = match store.load_instance_json(&pkg_index_path) {
        Ok(v) => v,
        // A missing root (packageless repository, or an undeclared sub-package
        // path) has nothing here to migrate.
        Err(_) => return Ok(()),
    };
    if !pkg_index.is_object() {
        return Err(abort(&pkg_index_path, "package.json is not a JSON object"));
    }

    for rel in list_of_strings(&pkg_index, "compositions") {
        let path = format!("{root}/{rel}");
        let (sections_migrated, export_config_migrated) =
            migrate_one_composition_file(store, &path)?;
        result.sections_migrated_to_discovery_query += sections_migrated;
        if export_config_migrated {
            result.compositions_export_config_migrated += 1;
        }
    }

    Ok(())
}

/// Apply migration #7: the SectionSource -> DiscoveryQuery collapse and the
/// ExportConfig unification, then stamp `dataModelRevision: 7`.
///
/// Requires migration #6 (`composition-cutover`) to have run first (ladder
/// order). Aborts rather than partially migrates (ADR-021) — a batch store
/// rolls back on any error.
pub fn migrate_discovery_query_cutover(
    store: &dyn RepositoryStore,
) -> Result<DiscoveryQueryCutoverResult, RepositoryError> {
    let from_revision = crate::field_type_migration_service::data_model_revision(store)?;
    let required = crate::field_type_migration_service::COMPOSITION_CUTOVER_REVISION;
    if from_revision < required {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "discovery-query-cutover migration requires data-model revision >= {required} \
                 (found {from_revision}): run `srs repo apply-migration --id \
                 composition-cutover` first (migration #6)"
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
) -> Result<DiscoveryQueryCutoverResult, RepositoryError> {
    let mut result = DiscoveryQueryCutoverResult {
        from_revision,
        to_revision: crate::field_type_migration_service::DISCOVERY_QUERY_CUTOVER_REVISION,
        ..Default::default()
    };

    for root in package_roots(store) {
        migrate_package_root(store, &root, &mut result)?;
    }

    let mut manifest = store.load_manifest()?;
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

    /// A store stamped at revision 6 with a primary package carrying one
    /// `type-query` Composition (with lifecycle axes) and a `container-subset`
    /// Composition that must be left untouched.
    fn rev6_store_with_full_shape() -> MemoryStore {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(6));
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
                    "compositions": [
                        "compositions/decisions.json",
                        "compositions/articles.json"
                    ]
                }),
            )
            .unwrap();

        store
            .save_instance_json(
                "package/compositions/decisions.json",
                &json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/composition.json",
                    "id": "00000000-0000-4000-8000-0000000000dd",
                    "namespace": "com.test",
                    "name": "decisions",
                    "version": 1,
                    "description": "Decisions",
                    "format": "markdown",
                    "preamble": "# {{container-title}}",
                    "sections": [
                        {
                            "sectionId": "body",
                            "order": 0,
                            "source": {
                                "type": "type-query",
                                "typeKey": "com.test/decision",
                                "lifecycleStates": ["active", "draft"],
                                "excludeLifecycleStates": ["superseded"],
                                "containerIds": ["11111111-1111-4111-8111-111111111111"],
                                "containerScope": "explicit"
                            }
                        }
                    ],
                    "createdAt": "2026-01-01T00:00:00Z"
                }),
            )
            .unwrap();

        store
            .save_instance_json(
                "package/compositions/articles.json",
                &json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/composition.json",
                    "id": "00000000-0000-4000-8000-0000000000ee",
                    "namespace": "com.test",
                    "name": "articles",
                    "version": 1,
                    "description": "Articles",
                    "format": "markdown",
                    "sections": [
                        {
                            "sectionId": "body",
                            "order": 0,
                            "source": {
                                "type": "container-subset",
                                "containerId": "22222222-2222-4222-8222-222222222222"
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
    fn migration_needed_below_revision_7() {
        let store = rev6_store_with_full_shape();
        assert!(
            crate::field_type_migration_service::discovery_query_cutover_migration_needed(&store)
                .unwrap()
        );
    }

    #[test]
    fn migrates_every_shape_and_stamps_revision_7() {
        let store = rev6_store_with_full_shape();
        let result = migrate_discovery_query_cutover(&store).unwrap();

        assert_eq!(result.from_revision, 6);
        assert_eq!(result.to_revision, 7);
        assert_eq!(result.sections_migrated_to_discovery_query, 1);
        assert_eq!(result.compositions_export_config_migrated, 2);

        let decisions = store
            .load_instance_json("package/compositions/decisions.json")
            .unwrap();
        assert!(decisions.get("format").is_none());
        assert!(decisions.get("preamble").is_none());
        assert_eq!(decisions["exportConfig"]["format"], "markdown");
        assert_eq!(
            decisions["exportConfig"]["preamble"],
            "# {{container-title}}"
        );

        let source = &decisions["sections"][0]["source"];
        assert_eq!(source["type"], "discovery-query");
        assert!(source.get("typeKey").is_none());
        assert_eq!(source["query"]["typeNamespace"], "com.test");
        assert_eq!(source["query"]["typeName"], "decision");
        assert_eq!(
            source["query"]["lifecycleStates"],
            json!(["active", "draft"])
        );
        assert_eq!(
            source["query"]["excludeLifecycleStates"],
            json!(["superseded"])
        );
        // Arrangement fields stay on the section, not the query.
        assert_eq!(
            source["containerIds"],
            json!(["11111111-1111-4111-8111-111111111111"])
        );
        assert_eq!(source["containerScope"], "explicit");

        let articles = store
            .load_instance_json("package/compositions/articles.json")
            .unwrap();
        assert!(articles.get("format").is_none());
        assert_eq!(articles["exportConfig"]["format"], "markdown");
        // container-subset section untouched.
        assert_eq!(
            articles["sections"][0]["source"]["type"],
            "container-subset"
        );

        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            7
        );
        assert!(
            !crate::field_type_migration_service::discovery_query_cutover_migration_needed(&store)
                .unwrap(),
            "must be idempotent — nothing left to do"
        );
    }

    #[test]
    fn refuses_below_composition_cutover_revision() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(5));
        store.save_manifest(&manifest).unwrap();

        let err = migrate_discovery_query_cutover(&store).unwrap_err();
        assert!(err.to_string().contains("composition-cutover"));
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            5,
            "a refused apply must not stamp the manifest"
        );
    }

    #[test]
    fn no_op_when_nothing_to_migrate() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("dataModelRevision".to_string(), json!(6));
        store.save_manifest(&manifest).unwrap();

        let result = migrate_discovery_query_cutover(&store).unwrap();
        assert_eq!(result.sections_migrated_to_discovery_query, 0);
        assert_eq!(result.compositions_export_config_migrated, 0);
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            7
        );
    }
}
