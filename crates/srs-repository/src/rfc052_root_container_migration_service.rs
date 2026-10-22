//! `rfc052-root-container` migration (srs-rust#1417, RFC-052 Change C / [R7]) — data-model
//! revision 9 -> 10.
//!
//! ## What it does
//!
//! For the manifest it:
//! 1. **refuses before any write** when `containers/<containerId>.json` already exists
//!    (`migration-root-path-exists`), or when a container file already declares the root's
//!    `containerId` (`migration-root-duplicate`);
//! 2. writes the inline `manifest.container` to `containers/<containerId>.json`, removes
//!    `container`, writes `rootContainerId`, and stamps `dataModelRevision` 10 on the manifest
//!    and on every package manifest that carries a stamp.
//!
//! A revision-9 manifest that already names its root by `rootContainerId` (a legacy snapshot
//! import writes that shape) is only re-stamped. One with no root at all cannot satisfy
//! [R1] at revision 10 and is refused (`migration-root-missing`) rather than written into a
//! corpus no command can open.
//!
//! ## Why it reads the raw tree
//!
//! A revision-10 build refuses an embedded root ([R7]), so — like `rfc043-container-entries` —
//! this entry reads `manifest.json` and `containers/` as JSON; it is the sole sanctioned reader
//! of the embedded form.
//!
//! ## All-or-nothing, and the gate
//!
//! No shipped store can roll writes back, so every refusal and the whole gate are decided on
//! an in-memory copy of the migrated tree before the first write. The gate (Change C):
//! - **navigation and identity**: the revision-9 reading, literally (the identity is the
//!   embedded root's `identityInstanceId`; the sections are its entries in order, identity
//!   excluded, RFC-043 [R12]) must equal what the revision-10 navigation service reports on
//!   the migrated tree;
//! - **every Composition**: rendered by this build on the tree before (through the
//!   migration reader) and after must be byte-identical, or fail identically. A Composition
//!   that names the root container cannot be rendered the revision-9 way here — this build
//!   has no reader of the embedded form outside this migration — so it is reported in
//!   `compositionsNotComparable` instead; the corpus-level comparison against the last
//!   revision-9 build covers it. A corpus whose catalog is already fatal for an unrelated
//!   reason must fail the same way after (`gateSkipped`).

use crate::error::RepositoryError;
use crate::field_type_migration_service::{
    DATA_MODEL_REVISION_KEY, RFC046_ACTOR_PROVENANCE_REVISION, RFC052_ROOT_CONTAINER_REVISION,
};
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MIGRATION_ID: &str = "rfc052-root-container";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rfc052Result {
    pub from_revision: u64,
    pub to_revision: u64,
    /// `true` when the corpus was already at revision 10: nothing was written.
    pub already_applied: bool,
    /// The root container `rootContainerId` now names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_container_id: Option<String>,
    /// Where the embedded root was written (absent when nothing moved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_container_path: Option<String>,
    pub packages_stamped: usize,
    /// Compositions rendered before and after and found byte-identical (gate).
    pub compositions_compared: usize,
    /// Compositions that name the root container, which only this migration reads in its
    /// revision-9 place: not compared in-process (the corpus-level comparison against the
    /// last revision-9 build covers them).
    pub compositions_not_comparable: Vec<String>,
    /// Why the render comparison did not run (a catalog already fatal before the migration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_skipped: Option<String>,
}

fn refuse(code: &'static str, msg: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::MigrationRefused {
        code,
        message: format!("{MIGRATION_ID} migration refused ({code}): {msg}; nothing was written"),
    }
}

fn revision(manifest: &Value) -> u64 {
    manifest
        .get(DATA_MODEL_REVISION_KEY)
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn raw_manifest(store: &dyn RepositoryStore) -> Result<Value, RepositoryError> {
    let text = store.load_manifest_raw_text()?;
    serde_json::from_str(&text).map_err(|source| RepositoryError::ManifestParse {
        path: std::path::PathBuf::from("manifest.json"),
        source,
    })
}

/// `Needed` while the stamp is below 10 or the manifest still embeds the root.
pub fn migration_needed(store: &dyn RepositoryStore) -> bool {
    raw_manifest(store).is_ok_and(|m| {
        revision(&m) < RFC052_ROOT_CONTAINER_REVISION || m.get("container").is_some()
    })
}

/// The planned rewrite of one manifest: the new manifest, and the root container file to
/// write (absent when the root is already in the container set).
struct Plan {
    manifest: Value,
    root_id: String,
    root: Option<(String, Value)>,
}

/// Plan the rewrite (Change C step 1 and 2). `containers` are the existing container files
/// as `(path, value)`. Decides every refusal; writes nothing.
fn plan(manifest_raw: &Value, containers: &[(String, Value)]) -> Result<Plan, RepositoryError> {
    let from = revision(manifest_raw);
    if from < RFC046_ACTOR_PROVENANCE_REVISION {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "{MIGRATION_ID} migration requires data-model revision >= \
                 {RFC046_ACTOR_PROVENANCE_REVISION} (found {from}): run `srs repo \
                 apply-migration --id rfc046-actor-provenance` first (RFC-046, migration #9)"
            ),
        });
    }
    let mut manifest = manifest_raw.clone();
    let obj = manifest
        .as_object_mut()
        .ok_or_else(|| refuse("migration-root-missing", "manifest.json is not an object"))?;
    let inline = obj.shift_remove("container");
    let (root_id, root) = match inline {
        Some(container) => {
            let id = container
                .get("containerId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    refuse(
                        "migration-root-missing",
                        "manifest.container has no containerId",
                    )
                })?
                .to_string();
            let path = crate::store::root_container_path(&id);
            if containers.iter().any(|(p, _)| *p == path) {
                return Err(refuse(
                    "migration-root-path-exists",
                    format!("{path} already exists"),
                ));
            }
            if let Some((p, _)) = containers
                .iter()
                .find(|(_, c)| c.get("containerId").and_then(Value::as_str) == Some(&id))
            {
                return Err(refuse(
                    "migration-root-duplicate",
                    format!("{p} already declares the root container's id {id} (RFC-052 [R5])"),
                ));
            }
            (id, Some((path, container)))
        }
        None => {
            let id = obj
                .get("rootContainerId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    refuse(
                        "migration-root-missing",
                        "the manifest declares no root container, and a revision-10 manifest \
                         must name one (RFC-052 [R1]); declare one with `repo set-root-container` \
                         first",
                    )
                })?
                .to_string();
            (id, None)
        }
    };
    obj.insert("rootContainerId".to_string(), json!(root_id));
    obj.insert(
        DATA_MODEL_REVISION_KEY.to_string(),
        json!(RFC052_ROOT_CONTAINER_REVISION),
    );
    Ok(Plan {
        manifest,
        root_id,
        root,
    })
}

/// The rewrite of one manifest value, without the migration's refusals or gate: an
/// embedded root becomes `(containers/<containerId>.json, container)` and the manifest names
/// it by `rootContainerId`; a revision-9 stamp becomes 10. For code that builds a pre-RFC-052
/// manifest in memory (test fixtures, tooling); a repository on disk goes through the
/// registry entry. Returns `None` when there is no embedded root with a `containerId`.
pub fn rewrite_embedded_root(manifest: &mut Value) -> Option<(String, Value)> {
    let obj = manifest.as_object_mut()?;
    let id = obj
        .get("container")?
        .get("containerId")?
        .as_str()?
        .to_string();
    let container = obj.shift_remove("container")?;
    obj.insert("rootContainerId".to_string(), json!(id));
    if revision(manifest) == RFC046_ACTOR_PROVENANCE_REVISION {
        manifest[DATA_MODEL_REVISION_KEY] = json!(RFC052_ROOT_CONTAINER_REVISION);
    }
    Some((crate::store::root_container_path(&id), container))
}

/// Write `manifest` as `<root>/manifest.json`, an embedded root first rewritten by
/// [`rewrite_embedded_root`] and written to its container file. For test fixtures that build
/// a pre-RFC-052 manifest in code.
#[doc(hidden)]
pub fn write_fixture_manifest(root: &std::path::Path, manifest: &Value) -> std::io::Result<()> {
    let mut manifest = manifest.clone();
    if let Some((path, container)) = rewrite_embedded_root(&mut manifest) {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().expect("containers/<id>.json has a parent"))?;
        std::fs::write(file, serde_json::to_string_pretty(&container)?)?;
    }
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )
}

/// [`rewrite_embedded_root`] applied to a `.srsj` document value: the root goes into `data`.
/// For test fixtures that build a pre-RFC-052 `.srsj` in code.
#[doc(hidden)]
pub fn rewrite_srsj_fixture(doc: &mut Value) {
    let Some(manifest) = doc.get_mut("manifest") else {
        return;
    };
    if let Some((path, container)) = rewrite_embedded_root(manifest) {
        if let Some(data) = doc.get_mut("data").and_then(Value::as_object_mut) {
            data.insert(path, container);
        }
    }
}

fn container_files(store: &dyn RepositoryStore) -> Vec<(String, Value)> {
    let mut files: Vec<String> = store
        .list_files_recursive("containers")
        .into_iter()
        .filter(|p| p.ends_with(".json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| store.load_instance_json(&p).ok().map(|v| (p, v)))
        .collect()
}

fn package_roots(manifest: &Value) -> Vec<String> {
    let mut roots = vec!["package".to_string()];
    for r in manifest
        .get("packageRefs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if r.get("mode").and_then(Value::as_str) == Some("local") {
            if let Some(path) = r.get("path").and_then(Value::as_str) {
                roots.push(path.to_string());
            }
        }
    }
    roots
}

fn pretty(value: &Value) -> Result<String, RepositoryError> {
    serde_json::to_string_pretty(value).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from("manifest.json"),
        source,
    })
}

/// Rewrite `files` (a raw tree) to the planned outcome: the manifest, the root file and the
/// package stamps. Returns the number of package manifests stamped.
fn apply_to_tree(
    files: &mut BTreeMap<String, Vec<u8>>,
    plan: &Plan,
) -> Result<usize, RepositoryError> {
    files.insert(
        "manifest.json".to_string(),
        framed(files.get("manifest.json"), &plan.manifest)?,
    );
    if let Some((path, root)) = &plan.root {
        files.insert(path.clone(), format!("{}\n", pretty(root)?).into_bytes());
    }
    let mut stamped = 0;
    for root in package_roots(&plan.manifest) {
        let path = format!("{root}/package.json");
        let Some(mut idx) = files
            .get(&path)
            .and_then(|b| serde_json::from_slice::<Value>(b).ok())
        else {
            continue;
        };
        if idx.get(DATA_MODEL_REVISION_KEY).is_some() {
            idx[DATA_MODEL_REVISION_KEY] = json!(RFC052_ROOT_CONTAINER_REVISION);
            let framed = framed(files.get(&path), &idx)?;
            files.insert(path, framed);
            stamped += 1;
        }
    }
    Ok(stamped)
}

/// 2-space pretty JSON with a trailing newline exactly when the original had one — so the
/// migration's diff is the change, not the framing.
fn framed(original: Option<&Vec<u8>>, value: &Value) -> Result<Vec<u8>, RepositoryError> {
    let mut text = pretty(value)?;
    if original.is_some_and(|b| b.ends_with(b"\n")) {
        text.push('\n');
    }
    Ok(text.into_bytes())
}

/// The gate (Change C), on in-memory trees: `before` is the corpus as it is and `after` the
/// planned tree, both read through the migration reader.
fn gate(
    before_files: BTreeMap<String, Vec<u8>>,
    after_files: BTreeMap<String, Vec<u8>>,
    inline_root: Option<&Value>,
    root_id: &str,
    result: &mut Rfc052Result,
) -> Result<(), RepositoryError> {
    let before = crate::tree_session::open_tree(before_files)?.with_rfc038_exemption();
    // Exempt like `before`: the structural `rfc038-storage` entry runs after this one in the
    // registry, so a corpus mid-ladder may still carry retired manifest properties.
    let after = crate::tree_session::open_tree(after_files)?.with_rfc038_exemption();

    // A corpus that is already unloadable (a fatal catalog diagnostic unrelated to the root)
    // renders nothing either side: the migration must leave it failing the same way.
    if let Err(e) = before.catalog() {
        let after_err = after.catalog().err().map(|e| e.to_string());
        if after_err.as_deref() != Some(e.to_string().as_str()) {
            return Err(refuse(
                "migration-gate-failed",
                format!(
                    "the catalog fails before ({e}) but not the same way after ({after_err:?})"
                ),
            ));
        }
        result.gate_skipped = Some(format!(
            "the catalog is already fatal before the migration: {e}"
        ));
        return Ok(());
    }

    // Navigation and identity: the revision-9 reading of the embedded root, literally.
    if let Some(root) = inline_root {
        let nav = crate::repository_navigation_service::repository_navigation(&after)
            .map_err(|e| refuse("migration-gate-failed", format!("navigation after: {e}")))?;
        let identity = root.get("identityInstanceId").and_then(Value::as_str);
        let cat = after.catalog()?;
        let want: Vec<(String, u64)> = root
            .get("memberInstanceIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let id = e.get("instanceId")?.as_str()?;
                let depth = e.get("depth").and_then(Value::as_u64).unwrap_or(0);
                Some((id.to_string(), depth))
            })
            .filter(|(id, _)| Some(id.as_str()) != identity)
            .filter(|(id, _)| cat.instances.iter().any(|e| &e.id == id))
            .collect();
        let got: Vec<(String, u64)> = nav
            .sections
            .iter()
            .map(|s| (s.instance_id.clone(), u64::from(s.depth)))
            .collect();
        if nav.root_container_id != root_id
            || nav.identity.as_ref().map(|n| n.instance_id.as_str()) != identity
            || got != want
        {
            return Err(refuse(
                "migration-gate-failed",
                format!(
                    "navigation or identity would change (before: identity {identity:?}, \
                     sections {want:?}; after: identity {:?}, sections {got:?})",
                    nav.identity.as_ref().map(|n| &n.instance_id)
                ),
            ));
        }
    }

    // Every Composition, rendered before and after.
    let package = after.load_package()?;
    for composition in &package.compositions {
        let id = composition.id.as_str();
        // This build reads the root container only in its revision-10 place, so a Composition
        // that names it cannot be rendered the revision-9 way here.
        if serde_json::to_string(composition).is_ok_and(|t| t.contains(root_id)) {
            result.compositions_not_comparable.push(id.to_string());
            continue;
        }
        let render = |store: &dyn RepositoryStore| {
            crate::render_service::render_composition(
                crate::render_service::RenderCompositionOptions::new(store, id),
            )
            .map(|r| r.rendered)
        };
        let same = match (render(&before), render(&after)) {
            (Ok(a), Ok(b)) => a == b,
            (Err(a), Err(b)) => a.code() == b.code() && a.to_string() == b.to_string(),
            _ => false,
        };
        if !same {
            return Err(refuse(
                "migration-gate-failed",
                format!("Composition {id} would render differently after the migration"),
            ));
        }
        result.compositions_compared += 1;
    }
    Ok(())
}

pub fn migrate_rfc052_root_container(
    store: &dyn RepositoryStore,
) -> Result<Rfc052Result, RepositoryError> {
    let manifest_raw = raw_manifest(store)?;
    let from_revision = revision(&manifest_raw);
    let mut result = Rfc052Result {
        from_revision,
        to_revision: RFC052_ROOT_CONTAINER_REVISION,
        ..Default::default()
    };
    if from_revision >= RFC052_ROOT_CONTAINER_REVISION && manifest_raw.get("container").is_none() {
        result.already_applied = true;
        result.root_container_id = manifest_raw
            .get("rootContainerId")
            .and_then(Value::as_str)
            .map(str::to_string);
        return Ok(result);
    }

    let plan = plan(&manifest_raw, &container_files(store))?;
    result.root_container_id = Some(plan.root_id.clone());
    result.root_container_path = plan.root.as_ref().map(|(p, _)| p.clone());

    // ---- Gate, on in-memory trees (decided before the first write).
    let before_files = crate::archive::tree_entries(store)?;
    let mut after_files = before_files.clone();
    result.packages_stamped = apply_to_tree(&mut after_files, &plan)?;
    gate(
        before_files,
        after_files.clone(),
        manifest_raw.get("container"),
        &plan.root_id,
        &mut result,
    )?;

    // ---- Apply: the root file first, then the manifest that names it.
    if let Some((path, _)) = &plan.root {
        store.ensure_instance_dir("containers")?;
        write_bytes(store, path, &after_files[path])?;
    }
    for root in package_roots(&plan.manifest) {
        let path = format!("{root}/package.json");
        if let Some(bytes) = after_files.get(&path) {
            if before_has_stamp(store, &path) {
                write_bytes(store, &path, bytes)?;
            }
        }
    }
    write_manifest_raw(store, &plan.manifest, &after_files["manifest.json"])?;
    Ok(result)
}

fn before_has_stamp(store: &dyn RepositoryStore, path: &str) -> bool {
    store
        .load_instance_json(path)
        .is_ok_and(|v| v.get(DATA_MODEL_REVISION_KEY).is_some())
}

fn write_bytes(
    store: &dyn RepositoryStore,
    path: &str,
    bytes: &[u8],
) -> Result<(), RepositoryError> {
    if store.is_file_tree_store() {
        return store.save_text_file(path, &String::from_utf8_lossy(bytes));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from(path),
            source,
        })?;
    store.save_instance_json(path, &value)
}

/// A file tree takes the raw write; `MemoryStore`, whose manifest is typed, takes the typed one.
fn write_manifest_raw(
    store: &dyn RepositoryStore,
    manifest: &Value,
    bytes: &[u8],
) -> Result<(), RepositoryError> {
    if store.is_file_tree_store() {
        return store.save_text_file("manifest.json", &String::from_utf8_lossy(bytes));
    }
    let mut typed: crate::manifest::Manifest =
        serde_json::from_value(manifest.clone()).map_err(|source| {
            RepositoryError::ManifestParse {
                path: std::path::PathBuf::from("manifest.json"),
                source,
            }
        })?;
    typed.root = store.load_manifest()?.root;
    store.save_manifest(&typed)
}

/// `true` when `manifest` (a raw `.srsj`/`.srs` tree manifest) is at revision 9 and still
/// embeds its root: the one shape the pre-load transformer rewrites.
fn tree_needs_transform(manifest: &Value) -> bool {
    revision(manifest) == RFC046_ACTOR_PROVENANCE_REVISION && manifest.get("container").is_some()
}

/// Pre-load transformer for a `.srsj` / `.srs` tree (RFC-052 Change C): a revision-9 tree
/// that embeds its root is rewritten in memory to the registry entry's outcome, with the same
/// refusals, before any store is built — so a published seed (immutable, never rewritten on
/// disk) opens in a revision-10 build. Any other tree is left as it is. Returns whether the
/// tree was rewritten.
pub fn transform_tree(files: &mut BTreeMap<String, Vec<u8>>) -> Result<bool, RepositoryError> {
    let Some(manifest) = files
        .get("manifest.json")
        .and_then(|b| serde_json::from_slice::<Value>(b).ok())
    else {
        return Ok(false);
    };
    if !tree_needs_transform(&manifest) {
        return Ok(false);
    }
    let containers: Vec<(String, Value)> = files
        .iter()
        .filter(|(p, _)| p.starts_with("containers/") && p.ends_with(".json"))
        .filter_map(|(p, b)| Some((p.clone(), serde_json::from_slice(b).ok()?)))
        .collect();
    let plan = plan(&manifest, &containers)?;
    apply_to_tree(files, &plan)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use std::path::Path;

    const ROOT: &str = "11111111-0000-4000-8000-000000000001";
    const IDENT: &str = "22222222-0000-4000-8000-000000000001";
    const SEC_A: &str = "22222222-0000-4000-8000-0000000000aa";
    const SEC_B: &str = "22222222-0000-4000-8000-0000000000bb";
    const PART: &str = "11111111-0000-4000-8000-0000000000c1";

    fn w(dir: &Path, rel: &str, v: &Value) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(v).unwrap() + "\n").unwrap();
    }

    fn note(id: &str) -> Value {
        json!({"$schema": srs_schema::NOTE_SCHEMA_ID, "instanceId": id, "title": id,
            "sections": [{"name": "body", "content": id}], "createdAt": "2026-01-01T00:00:00Z"})
    }

    /// A revision-9 corpus: an embedded root (identity + two sections, B before A), a Part
    /// container anchored on SEC_A, and a Composition over the Part container.
    fn rev9_repo(dir: &Path) {
        w(dir, ".srs/marker", &json!({}));
        w(
            dir,
            "manifest.json",
            &json!({
                "srsVersion": "2.0-draft", "dataModelRevision": 9,
                "repositoryId": "00000000-0000-4000-8000-0000000000f0",
                "namespace": "com.test", "packageRef": {"mode": "local", "path": "package"},
                "container": {
                    "containerId": ROOT, "title": "Root", "identityInstanceId": IDENT,
                    "memberInstanceIds": [
                        {"instanceId": IDENT}, {"instanceId": SEC_B}, {"instanceId": SEC_A}
                    ]
                }
            }),
        );
        w(
            dir,
            "package/package.json",
            &json!({"$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                "id": "00000000-0000-4000-8000-00000000bbbb", "namespace": "com.test",
                "name": "p", "version": "1.0.0", "title": "p", "description": "d",
                "status": "draft", "createdAt": "2026-01-01T00:00:00Z", "dataModelRevision": 9,
                "fields": [], "types": [], "compositions": ["compositions/doc.json"]}),
        );
        w(
            dir,
            "package/compositions/doc.json",
            &json!({"$schema": srs_schema::COMPOSITION_SCHEMA_ID,
                "id": "00000000-0000-4000-8000-00000000cccc", "namespace": "com.test",
                "name": "doc", "version": 1, "description": "d",
                "sections": [
                    {"sectionId": "s1", "order": 0,
                     "source": {"type": "container-subset", "containerId": PART},
                     "ordering": {"source": "arranged"}}
                ], "createdAt": "2026-01-01T00:00:00Z"}),
        );
        for id in [IDENT, SEC_A, SEC_B] {
            w(dir, &format!("records/notes/note-{id}.json"), &note(id));
        }
        w(
            dir,
            "containers/part.json",
            &json!({"containerId": PART, "title": "Part", "anchorInstanceId": SEC_A,
                "memberInstanceIds": [{"instanceId": SEC_A}, {"instanceId": SEC_B}]}),
        );
    }

    fn read(dir: &Path, rel: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(rel)).unwrap()).unwrap()
    }

    #[test]
    fn moves_the_root_stamps_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        assert!(migration_needed(&store));

        let r = migrate_rfc052_root_container(&store).unwrap();
        assert!(!r.already_applied);
        assert_eq!(r.gate_skipped, None);
        assert_eq!(r.compositions_compared, 1);
        assert_eq!(r.packages_stamped, 1);

        let m = read(tmp.path(), "manifest.json");
        assert_eq!(m["dataModelRevision"], 10);
        assert_eq!(m["rootContainerId"], ROOT);
        assert!(m.get("container").is_none());
        let root = read(tmp.path(), &format!("containers/{ROOT}.json"));
        assert_eq!(root["identityInstanceId"], IDENT);
        assert_eq!(
            read(tmp.path(), "package/package.json")["dataModelRevision"],
            10
        );

        // The strict reader loads it, and navigation reads the moved root.
        let strict = FileStore::new(tmp.path());
        let nav = crate::repository_navigation_service::repository_navigation(&strict).unwrap();
        assert_eq!(nav.root_container_id, ROOT);
        let ids: Vec<&str> = nav
            .sections
            .iter()
            .map(|s| s.instance_id.as_str())
            .collect();
        assert_eq!(ids, vec![SEC_B, SEC_A]);

        // alreadyApplied, and a re-run writes nothing.
        assert!(!migration_needed(&store));
        let before = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
        assert!(
            migrate_rfc052_root_container(&store)
                .unwrap()
                .already_applied
        );
        assert_eq!(
            before,
            std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap()
        );
    }

    #[test]
    fn refuses_with_no_write_when_the_root_path_exists() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        w(
            tmp.path(),
            &format!("containers/{ROOT}.json"),
            &json!({"containerId": "33333333-0000-4000-8000-000000000001", "title": "Other"}),
        );
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let before = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
        let err = migrate_rfc052_root_container(&store).unwrap_err();
        assert_eq!(err.code(), "migration-root-path-exists", "{err}");
        assert_eq!(
            before,
            std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap()
        );
    }

    #[test]
    fn refuses_with_no_write_on_a_duplicate_root_id() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        w(
            tmp.path(),
            "containers/twin.json",
            &json!({"containerId": ROOT, "title": "Twin"}),
        );
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let err = migrate_rfc052_root_container(&store).unwrap_err();
        assert_eq!(err.code(), "migration-root-duplicate", "{err}");
        assert!(!tmp.path().join(format!("containers/{ROOT}.json")).exists());
        assert!(read(tmp.path(), "manifest.json").get("container").is_some());
    }

    #[test]
    fn refuses_a_rootless_corpus() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        let mut m = read(tmp.path(), "manifest.json");
        m.as_object_mut().unwrap().remove("container");
        w(tmp.path(), "manifest.json", &m);
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let err = migrate_rfc052_root_container(&store).unwrap_err();
        assert_eq!(err.code(), "migration-root-missing", "{err}");
    }

    /// [R7]: the revision-10 reader refuses the embedded form naming RFC-052 and the
    /// migration, and `repo migrations` still lists the entry as needed.
    #[test]
    fn revision_10_reader_refuses_a_revision_9_corpus_naming_the_migration() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        let err = FileStore::new(tmp.path()).load_manifest().unwrap_err();
        assert_eq!(err.code(), "container-migration-needed");
        let msg = err.to_string();
        assert!(
            msg.contains("RFC-052") && msg.contains(MIGRATION_ID),
            "{msg}"
        );
        let exempt = FileStore::new(tmp.path()).with_rfc038_exemption();
        let listed = crate::migration_registry_service::list_migrations(&exempt).unwrap();
        let m = listed.iter().find(|m| m.id == MIGRATION_ID).unwrap();
        assert_eq!(
            m.status,
            crate::migration_registry_service::MigrationStatus::Needed
        );
    }

    /// [R1]: a `rootContainerId` naming no container in the container set is fatal to
    /// the load (catalog-level), with the [R1] code.
    #[test]
    fn a_root_container_id_naming_no_container_is_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        migrate_rfc052_root_container(&store).unwrap();
        std::fs::remove_file(tmp.path().join(format!("containers/{ROOT}.json"))).unwrap();
        let err = FileStore::new(tmp.path())
            .catalog()
            .unwrap_err()
            .to_string();
        assert!(err.contains("manifest-root-container-missing"), "{err}");
    }

    #[test]
    fn tree_transformer_matches_the_registry_entry() {
        let tmp = tempfile::tempdir().unwrap();
        rev9_repo(tmp.path());
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let mut tree = crate::archive::tree_entries(&store).unwrap();
        assert!(transform_tree(&mut tree).unwrap());
        assert!(!transform_tree(&mut tree).unwrap(), "idempotent");
        migrate_rfc052_root_container(&store).unwrap();
        let migrated = crate::archive::tree_entries(&FileStore::new(tmp.path())).unwrap();
        for key in ["manifest.json", &format!("containers/{ROOT}.json")] {
            let a: Value = serde_json::from_slice(&tree[key]).unwrap();
            let b: Value = serde_json::from_slice(&migrated[key]).unwrap();
            assert_eq!(a, b, "{key}");
        }
    }
}
