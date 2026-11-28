//! `rfc006-lifecycle-state-identity` migration (srs-rust#1638, RFC-006 Revision 9 [S2]/[S3],
//! srs-programme decision 7324ac6f) — data-model revision 11 -> 12.
//!
//! ## What it does
//!
//! A `LifecycleState` carries the full substrate identity: `id`, `version` and `namespace`
//! ([S1]). For every state of a standalone `Lifecycle` (`<package>/.../lifecycles/*.json`) and
//! of an inline `Type.lifecycle` (`<package>/.../types/**/*.json`) this migration fills **only
//! the absent fields** and leaves a present field as written ([S2]):
//!
//! - `version` = `1`;
//! - `namespace` = the owning container's `namespace` (the `Lifecycle`'s, or for an inline
//!   lifecycle the owning Type's);
//! - `id` = the UUIDv5 whose namespace UUID is the owning container's `id` and whose name is
//!   the state's `key` (UTF-8 bytes, no normalisation, RFC 4122 section 4.3). A state of an
//!   extending `Lifecycle` is keyed to that `Lifecycle`'s own `id`.
//!
//! It takes no value from an agent, a prompt or the clock: the same input always produces the
//! same bytes, so two checkouts migrated apart converge. A lifecycle whose states already carry
//! all three fields is untouched, and a re-run reports `alreadyApplied`. It stamps
//! `dataModelRevision` 12 on the manifest and on every package manifest that carries a stamp.
//!
//! ## Raw tree, all-or-nothing
//!
//! It reads and writes the raw file tree (the revision-12 reader refuses a revision-11 corpus
//! on load, [S3]). Everything is planned in memory first; it refuses before any write when an
//! owner carries no UUID `id` or (for a state that needs it) no `namespace`. Writes go
//! definitions, then package stamps, then the manifest; a failed write undoes every earlier one.
//!
//! The `.srsj` / `.srs` forms are migrated by the registry ladder in `transform_tree`; the
//! `.srspkg` form by [`migrate_package_bundle_value`].

use crate::error::RepositoryError;
use crate::field_type_migration_service::{
    DATA_MODEL_REVISION_KEY, RFC006_LIFECYCLE_STATE_IDENTITY_REVISION,
    RFC051_CONTAINER_ENTRIES_REVISION,
};
use crate::rfc051_container_entries_migration_service::corpus_files;
use crate::rfc052_root_container_migration_service::{
    framed, package_roots, raw_manifest, revision, write_bytes, write_manifest_raw,
};
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const MIGRATION_ID: &str = "rfc006-lifecycle-state-identity";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rfc006Result {
    pub from_revision: u64,
    pub to_revision: u64,
    /// `true` when the corpus was already at revision 12: nothing was written.
    pub already_applied: bool,
    /// Standalone `Lifecycle` files rewritten.
    pub lifecycles_migrated: usize,
    /// `Type` files with an inline lifecycle rewritten.
    pub inline_lifecycles_migrated: usize,
    /// States that gained at least one field.
    pub states_filled: usize,
    pub packages_stamped: usize,
}

fn refuse(code: &'static str, msg: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::MigrationRefused {
        code,
        message: format!("{MIGRATION_ID} migration refused ({code}): {msg}; nothing was written"),
    }
}

/// `Needed` while the stamp is below 12 ([S3]).
pub fn migration_needed(store: &dyn RepositoryStore) -> bool {
    raw_manifest(store).is_ok_and(|m| revision(&m) < RFC006_LIFECYCLE_STATE_IDENTITY_REVISION)
}

/// Fill the absent identity fields of every state in `states` from its owner ([S2]).
/// Returns how many states changed. Refuses (`Err(message)`) before touching a state whose
/// owner cannot supply what is missing.
fn fill_states(
    states: &mut [Value],
    owner_id: Option<&str>,
    owner_ns: Option<&str>,
    at: &str,
) -> Result<usize, String> {
    let mut changed = 0;
    for state in states {
        let Some(obj) = state.as_object_mut() else {
            continue;
        };
        let missing = |k: &str| obj.get(k).is_none_or(Value::is_null);
        let (no_id, no_version, no_ns) = (missing("id"), missing("version"), missing("namespace"));
        if !(no_id || no_version || no_ns) {
            continue;
        }
        // `key` (was `name` in pre-RFC-006 data): the name the UUIDv5 hashes, as written.
        let Some(key) = obj
            .get("key")
            .or_else(|| obj.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let id = if no_id {
            let owner = owner_id
                .and_then(|i| Uuid::parse_str(i).ok())
                .ok_or_else(|| {
                    format!("{at}: the owner carries no UUID `id` to key state '{key}' to")
                })?;
            Some(Value::String(
                Uuid::new_v5(&owner, key.as_bytes())
                    .hyphenated()
                    .to_string(),
            ))
        } else {
            None
        };
        let ns = if no_ns {
            Some(Value::String(
                owner_ns
                    .ok_or_else(|| {
                        format!("{at}: the owner carries no `namespace` for state '{key}'")
                    })?
                    .to_string(),
            ))
        } else {
            None
        };
        let mut out = Map::new();
        for (name, filled) in [
            ("id", id),
            ("version", no_version.then(|| json!(1))),
            ("namespace", ns),
        ] {
            if let Some(v) = filled.or_else(|| obj.get(name).cloned()) {
                out.insert(name.to_string(), v);
            }
        }
        for (k, v) in obj.iter() {
            if !matches!(k.as_str(), "id" | "version" | "namespace") {
                out.insert(k.clone(), v.clone());
            }
        }
        *obj = out;
        changed += 1;
    }
    Ok(changed)
}

/// What one definition value needed: `(standalone, inline, states)` changed counts.
fn migrate_definition(def: &mut Value, at: &str) -> Result<(usize, usize, usize), String> {
    let owner_id = def.get("id").and_then(Value::as_str).map(str::to_string);
    let owner_ns = def
        .get("namespace")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (mut standalone, mut inline, mut states) = (0, 0, 0);
    if let Some(list) = def.get_mut("states").and_then(Value::as_array_mut) {
        let n = fill_states(list, owner_id.as_deref(), owner_ns.as_deref(), at)?;
        standalone = usize::from(n > 0);
        states += n;
    }
    if let Some(list) = def
        .get_mut("lifecycle")
        .and_then(|l| l.get_mut("states"))
        .and_then(Value::as_array_mut)
    {
        let n = fill_states(list, owner_id.as_deref(), owner_ns.as_deref(), at)?;
        inline = usize::from(n > 0);
        states += n;
    }
    Ok((standalone, inline, states))
}

/// Whether a path is a Lifecycle or Type definition inside a package root.
fn is_definition_path(path: &str, roots: &[String]) -> bool {
    path.ends_with(".json")
        && roots.iter().any(|r| path.starts_with(&format!("{r}/")))
        && path
            .split('/')
            .any(|seg| seg == "lifecycles" || seg == "types")
}

pub fn migrate_rfc006_lifecycle_state_identity(
    store: &dyn RepositoryStore,
) -> Result<Rfc006Result, RepositoryError> {
    let manifest_raw = raw_manifest(store)?;
    let from_revision = revision(&manifest_raw);
    let mut result = Rfc006Result {
        from_revision,
        to_revision: RFC006_LIFECYCLE_STATE_IDENTITY_REVISION,
        ..Default::default()
    };
    if from_revision >= RFC006_LIFECYCLE_STATE_IDENTITY_REVISION {
        result.already_applied = true;
        return Ok(result);
    }
    if from_revision < RFC051_CONTAINER_ENTRIES_REVISION {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "{MIGRATION_ID} migration requires data-model revision >= \
                 {RFC051_CONTAINER_ENTRIES_REVISION} (found {from_revision}): run `srs repo \
                 apply-migration --id rfc051-container-entries` first (RFC-051, migration #11)"
            ),
        });
    }

    let files = corpus_files(store)?;
    let roots = package_roots(&manifest_raw);
    let mut steps: Vec<(String, Vec<u8>)> = Vec::new();
    for (path, bytes) in &files {
        if !is_definition_path(path, &roots) {
            continue;
        }
        let Ok(mut def) = serde_json::from_slice::<Value>(bytes) else {
            continue;
        };
        let (standalone, inline, states) = migrate_definition(&mut def, path)
            .map_err(|m| refuse("migration-lifecycle-owner-incomplete", m))?;
        if states == 0 {
            continue;
        }
        result.lifecycles_migrated += standalone;
        result.inline_lifecycles_migrated += inline;
        result.states_filled += states;
        steps.push((path.clone(), framed(Some(bytes), &def)?));
    }

    let mut manifest = manifest_raw.clone();
    manifest[DATA_MODEL_REVISION_KEY] = json!(RFC006_LIFECYCLE_STATE_IDENTITY_REVISION);
    for root in &roots {
        let path = format!("{root}/package.json");
        let Some(mut idx) = files
            .get(&path)
            .and_then(|b| serde_json::from_slice::<Value>(b).ok())
        else {
            continue;
        };
        if idx
            .get(DATA_MODEL_REVISION_KEY)
            .is_some_and(|r| r != &json!(RFC006_LIFECYCLE_STATE_IDENTITY_REVISION))
        {
            idx[DATA_MODEL_REVISION_KEY] = json!(RFC006_LIFECYCLE_STATE_IDENTITY_REVISION);
            steps.push((path.clone(), framed(files.get(&path), &idx)?));
            result.packages_stamped += 1;
        }
    }
    steps.push((
        "manifest.json".to_string(),
        framed(files.get("manifest.json"), &manifest)?,
    ));

    // Apply, undoing every earlier write on a failed one.
    let originals: BTreeMap<&str, &Vec<u8>> = files.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut done: Vec<&str> = Vec::new();
    let applied = (|| -> Result<(), RepositoryError> {
        for (path, bytes) in &steps {
            if path == "manifest.json" {
                write_manifest_raw(store, &manifest, bytes)?;
            } else {
                write_bytes(store, path, bytes)?;
            }
            done.push(path);
        }
        Ok(())
    })();
    if let Err(e) = applied {
        for path in done.into_iter().rev() {
            if let Some(old) = originals.get(path) {
                let _ = if path == "manifest.json" {
                    write_manifest_raw(store, &manifest_raw, old)
                } else {
                    write_bytes(store, path, old)
                };
            }
        }
        return Err(e);
    }
    Ok(result)
}

/// The `.srspkg` bundle form of the same step (RFC-003 [C6]): every bundled `Lifecycle` and
/// every bundled `Type` with an inline lifecycle gets [S2]. The caller sets the stamp.
pub fn migrate_package_bundle_value(bundle: &mut Value) -> Result<Rfc006Result, RepositoryError> {
    let mut result = Rfc006Result {
        from_revision: bundle
            .get(DATA_MODEL_REVISION_KEY)
            .and_then(Value::as_u64)
            .unwrap_or(0),
        to_revision: RFC006_LIFECYCLE_STATE_IDENTITY_REVISION,
        ..Default::default()
    };
    for list in ["lifecycles", "types"] {
        let Some(defs) = bundle.get_mut(list).and_then(Value::as_array_mut) else {
            continue;
        };
        for def in defs {
            let at = format!(
                "bundle {list} entry {}",
                def.get("id").and_then(Value::as_str).unwrap_or("?")
            );
            let (standalone, inline, states) = migrate_definition(def, &at)
                .map_err(|m| refuse("migration-lifecycle-owner-incomplete", m))?;
            result.lifecycles_migrated += standalone;
            result.inline_lifecycles_migrated += inline;
            result.states_filled += states;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use std::path::Path;

    const ROOT: &str = "11111111-0000-4000-8000-000000000001";
    const LC: &str = "0b50b6b5-481c-433b-912e-926f026592a3";
    const TY: &str = "0b50b6b5-481c-433b-912e-926f026592a4";
    const KEPT: &str = "7e093fc7-550b-499b-b08d-f0d0ff80baec";

    fn w(dir: &Path, rel: &str, v: &Value) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(v).unwrap() + "\n").unwrap();
    }

    /// A revision-11 repository: a standalone Lifecycle with one bare state and one that
    /// already carries its identity, and a Type with a bare inline lifecycle.
    fn rev11_repo(dir: &Path, lifecycle_namespace: Option<&str>) {
        w(dir, ".srs/marker", &json!({}));
        w(
            dir,
            "manifest.json",
            &json!({
                "$schema": srs_schema::MANIFEST_SCHEMA_ID, "title": "T",
                "createdAt": "2026-01-01T00:00:00Z",
                "srsVersion": "2.0-draft", "dataModelRevision": 11,
                "repositoryId": "00000000-0000-4000-8000-0000000000f0",
                "namespace": "com.test", "packageRef": {"mode": "local", "path": "package"},
                "rootContainerId": ROOT
            }),
        );
        w(
            dir,
            "package/package.json",
            &json!({"$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                "id": "00000000-0000-4000-8000-00000000bbbb", "namespace": "com.test",
                "name": "p", "version": "1.0.0", "title": "p", "description": "d",
                "status": "draft", "createdAt": "2026-01-01T00:00:00Z", "dataModelRevision": 11,
                "fields": [], "types": ["types/t.json"], "lifecycles": ["lifecycles/l.json"]}),
        );
        let mut lc = json!({
            "id": LC, "version": 1, "name": "flow",
            "states": [
                {"key": "draft", "label": "Draft", "isInitial": true},
                {"id": KEPT, "version": 3, "namespace": "com.kept", "key": "done"}
            ],
            "transitions": [], "initialState": "draft"
        });
        if let Some(ns) = lifecycle_namespace {
            lc["namespace"] = json!(ns);
        }
        w(dir, "package/lifecycles/l.json", &lc);
        w(
            dir,
            "package/types/t.json",
            &json!({"id": TY, "version": 1, "namespace": "com.test", "name": "t",
                "fields": [],
                "lifecycle": {"initialState": "open",
                    "states": [{"name": "open", "isInitial": true}], "transitions": []}}),
        );
        w(
            dir,
            &format!("containers/{ROOT}/container.json"),
            &json!({"containerId": ROOT, "title": "Root"}),
        );
    }

    fn read(dir: &Path, rel: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(rel)).unwrap()).unwrap()
    }

    fn migrated(dir: &Path) -> Result<Rfc006Result, RepositoryError> {
        migrate_rfc006_lifecycle_state_identity(&FileStore::new(dir).with_rfc038_exemption())
    }

    fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
                    out.insert(rel, std::fs::read(&p).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    #[test]
    fn fills_only_absent_fields_per_s2() {
        let tmp = tempfile::tempdir().unwrap();
        rev11_repo(tmp.path(), Some("com.test"));
        let r = migrated(tmp.path()).unwrap();
        assert_eq!(
            (
                r.lifecycles_migrated,
                r.inline_lifecycles_migrated,
                r.states_filled
            ),
            (1, 1, 2)
        );
        let lc = read(tmp.path(), "package/lifecycles/l.json");
        let draft = &lc["states"][0];
        let owner = Uuid::parse_str(LC).unwrap();
        assert_eq!(
            draft["id"],
            json!(Uuid::new_v5(&owner, b"draft").hyphenated().to_string())
        );
        assert_eq!(
            (&draft["version"], &draft["namespace"]),
            (&json!(1), &json!("com.test"))
        );
        // A state that already carries identity is left as written.
        assert_eq!(
            lc["states"][1],
            json!({"id": KEPT, "version": 3, "namespace": "com.kept", "key": "done"})
        );
        // An inline lifecycle is keyed to the owning Type, in the Type's namespace; the
        // legacy `name` is the hashed key and is not renamed.
        let inline = &read(tmp.path(), "package/types/t.json")["lifecycle"]["states"][0];
        let ty = Uuid::parse_str(TY).unwrap();
        assert_eq!(
            inline["id"],
            json!(Uuid::new_v5(&ty, b"open").hyphenated().to_string())
        );
        assert_eq!(inline["namespace"], json!("com.test"));
        assert_eq!(inline["name"], json!("open"));
        // Both stamps move to 12.
        assert_eq!(
            read(tmp.path(), "manifest.json")["dataModelRevision"],
            json!(12)
        );
        assert_eq!(
            read(tmp.path(), "package/package.json")["dataModelRevision"],
            json!(12)
        );
    }

    #[test]
    fn deterministic_and_a_rerun_is_already_applied() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        rev11_repo(a.path(), Some("com.test"));
        rev11_repo(b.path(), Some("com.test"));
        migrated(a.path()).unwrap();
        migrated(b.path()).unwrap();
        assert_eq!(
            snapshot(a.path()),
            snapshot(b.path()),
            "two checkouts converge"
        );
        let before = snapshot(a.path());
        let again = migrated(a.path()).unwrap();
        assert!(again.already_applied);
        assert_eq!(snapshot(a.path()), before, "a re-run writes nothing");
        assert!(!migration_needed(
            &FileStore::new(a.path()).with_rfc038_exemption()
        ));
    }

    #[test]
    fn an_owner_without_a_namespace_is_refused_before_any_write() {
        let tmp = tempfile::tempdir().unwrap();
        rev11_repo(tmp.path(), None);
        let before = snapshot(tmp.path());
        let err = migrated(tmp.path()).unwrap_err();
        assert_eq!(err.code(), "migration-lifecycle-owner-incomplete", "{err}");
        assert_eq!(snapshot(tmp.path()), before, "nothing written");
    }

    #[test]
    fn a_revision_11_corpus_is_refused_by_the_revision_12_reader_naming_the_entry() {
        let tmp = tempfile::tempdir().unwrap();
        rev11_repo(tmp.path(), Some("com.test"));
        let err = FileStore::new(tmp.path()).load_manifest().unwrap_err();
        assert_eq!(err.code(), "container-migration-needed");
        assert!(err.to_string().contains(MIGRATION_ID), "{err}");
    }

    #[test]
    fn validate_rejects_a_bare_state_at_revision_12() {
        let state: srs_core::types::lifecycle::LifecycleState =
            serde_json::from_value(json!({"key": "draft"})).unwrap();
        let msgs = crate::validation::lifecycle_state_identity_messages(&[state], "lifecycle 'x'");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("id, version, namespace"), "{msgs:?}");
        let full: srs_core::types::lifecycle::LifecycleState = serde_json::from_value(
            json!({"id": KEPT, "version": 1, "namespace": "com.t", "key": "draft"}),
        )
        .unwrap();
        assert!(crate::validation::lifecycle_state_identity_messages(&[full], "x").is_empty());
    }

    #[test]
    fn the_bundle_form_matches_the_registry_entry() {
        let tmp = tempfile::tempdir().unwrap();
        rev11_repo(tmp.path(), Some("com.test"));
        let mut bundle = json!({
            "dataModelRevision": 11,
            "lifecycles": [read(tmp.path(), "package/lifecycles/l.json")],
            "types": [read(tmp.path(), "package/types/t.json")]
        });
        let r = migrate_package_bundle_value(&mut bundle).unwrap();
        assert_eq!(r.states_filled, 2);
        migrated(tmp.path()).unwrap();
        assert_eq!(
            bundle["lifecycles"][0],
            read(tmp.path(), "package/lifecycles/l.json")
        );
        assert_eq!(bundle["types"][0], read(tmp.path(), "package/types/t.json"));
        // Idempotent.
        let again = migrate_package_bundle_value(&mut bundle).unwrap();
        assert_eq!(again.states_filled, 0);
    }
}
