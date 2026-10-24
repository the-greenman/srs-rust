//! `rfc051-container-entries` migration (srs-rust#1450, RFC-051 [R13]) — data-model revision
//! 10 -> 11.
//!
//! ## What it does
//!
//! Every flat container file `containers/<name>.json` (the root container included) becomes
//! the directory `containers/<containerId>/`: a one-line head (the container without
//! `memberInstanceIds`) and one one-line entry file per member, with each entry's `parent`
//! derived from RFC-043 [R5] and its `key` the next successive append among its siblings
//! (`a0`, `a1`, ...), so the migration is deterministic. It stamps `dataModelRevision` 11 on the
//! manifest and on every package manifest that carries a stamp. A flat file found in a corpus
//! already stamped 11 is converted the same way; a revision-11 corpus with none is
//! `alreadyApplied` and is never written.
//!
//! ## Refusals and the gate, before any write
//!
//! Everything is decided on an in-memory copy of the migrated tree first:
//! - an outline that violates RFC-043 [R2] is refused with its own `arrangement-*` code, and a
//!   container id that cannot name a directory with `container-name-mismatch`;
//! - the gate reads the corpus before (flat) and after (directories) with this build and
//!   requires, byte for byte: every container (head fields and `memberInstanceIds`, an absent
//!   `depth` equal to `depth: 0`), the repository navigation, and every Composition rendered
//!   unscoped and for every container (an error must be the same error). A corpus whose catalog
//!   is already fatal must fail the same way after (`gateSkipped`).
//!
//! ## All-or-nothing
//!
//! Writes go entries, then heads, then the package stamps and the manifest, then the flat
//! files are removed. A failed write undoes every earlier one, so the corpus is left as it was.
//! A run cut short (a crash, not an error) leaves either a revision-10 corpus with some
//! directories beside its flat files, or a revision-11 corpus with flat files left over; a
//! re-run plans from the flat files, rewrites their directories (removing any stray leftovers
//! there) and finishes.

use crate::container_store::{self, Place};
use crate::error::RepositoryError;
use crate::field_type_migration_service::{
    DATA_MODEL_REVISION_KEY, RFC051_CONTAINER_ENTRIES_REVISION, RFC052_ROOT_CONTAINER_REVISION,
};
use crate::rfc052_root_container_migration_service::{
    framed, package_roots, raw_manifest, revision, write_bytes, write_manifest_raw,
};
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::{json, Value};
use srs_core::types::container::ContainerEntry;
use std::collections::BTreeMap;

pub const MIGRATION_ID: &str = "rfc051-container-entries";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rfc051Result {
    pub from_revision: u64,
    pub to_revision: u64,
    /// `true` when the corpus was already at revision 11 with no flat container file: nothing
    /// was written.
    pub already_applied: bool,
    /// Flat container files converted to directories.
    pub containers_converted: usize,
    /// Entry files written.
    pub entries_written: usize,
    pub packages_stamped: usize,
    /// Containers read before and after and found identical (gate).
    pub containers_compared: usize,
    /// Composition renders (unscoped and per container) found byte-identical (gate).
    pub renders_compared: usize,
    /// Why the gate did not compare (a catalog already fatal before the migration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_skipped: Option<String>,
}

fn refuse(code: &'static str, msg: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::MigrationRefused {
        code,
        message: format!("{MIGRATION_ID} migration refused ({code}): {msg}; nothing was written"),
    }
}

/// The flat container files of a tree, sorted.
fn flat_files<'a>(paths: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut out: Vec<String> = paths
        .filter(|p| p.ends_with(".json") && container_store::place(p) == Some(Place::Flat))
        .cloned()
        .collect();
    out.sort();
    out
}

/// `Needed` while the stamp is below 11, or while any flat container file remains ([R13]).
pub fn migration_needed(store: &dyn RepositoryStore) -> bool {
    let below = raw_manifest(store).is_ok_and(|m| revision(&m) < RFC051_CONTAINER_ENTRIES_REVISION);
    below || !flat_files(store.list_files_recursive("containers").iter()).is_empty()
}

/// The directory form of one flat container value: `(path, bytes)` for the head and every
/// entry. Refuses an id that cannot name a directory and an outline that breaks RFC-043 [R2].
fn convert(
    path: &str,
    value: &Value,
    root: Option<&str>,
) -> Result<(String, BTreeMap<String, Vec<u8>>), RepositoryError> {
    let id = value
        .get("containerId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !container_store::is_canonical_id(&id) {
        return Err(refuse(
            container_store::codes::NAME_MISMATCH,
            format!("{path}: containerId '{id}' is not a canonical lowercase UUID, so it cannot name a directory (RFC-051 [R2])"),
        ));
    }
    let container: srs_core::types::container::Container = serde_json::from_value(value.clone())
        .map_err(|e| refuse(container_store::codes::SCHEMA, format!("{path}: {e}")))?;
    let entries: Vec<ContainerEntry> = container.member_instance_ids.clone().unwrap_or_default();
    if let Some(e) = entries
        .iter()
        .find(|e| !container_store::is_canonical_id(&e.instance_id))
    {
        return Err(refuse(
            container_store::codes::NAME_MISMATCH,
            format!(
                "{path}: member '{}' is not a canonical lowercase UUID, so it cannot name an \
                 entry file (RFC-051 [R2])",
                e.instance_id
            ),
        ));
    }
    let identity = (root == Some(id.as_str()))
        .then_some(container.identity_instance_id.as_deref())
        .flatten();
    if let Some(v) = srs_core::arrangement::check_entries(&entries, identity)
        .into_iter()
        .next()
    {
        return Err(refuse(
            v.code,
            format!(
                "{path}: entry {}: {} (RFC-043 [R2])",
                v.instance_id, v.message
            ),
        ));
    }
    let ws = srs_core::container_write::write_set(&[], None, None, |_| Ok(entries.clone()))
        .map_err(|v| refuse(v.code, format!("{path}: {}", v.message)))?;
    let mut files = BTreeMap::new();
    let head = container_store::head_value(&container)?;
    files.insert(
        container_store::head_path(&id),
        container_store::line(&head).into_bytes(),
    );
    for e in &ws.write {
        files.insert(
            container_store::entry_path(&id, &e.instance_id),
            container_store::line(&container_store::entry_value(&id, e)).into_bytes(),
        );
    }
    Ok((id, files))
}

/// Fixture authoring: a container value as a corpus at `revision` stores it — at revision 11
/// its directory (`(path, value)` for the head and every entry, keyed exactly as the migration
/// keys them, without its refusals or gate), below that the flat file at `flat_path`. For tests
/// that author containers in code; a repository on disk goes through the registry entry.
#[doc(hidden)]
pub fn fixture_container_files(
    flat_path: &str,
    container: &Value,
    revision: u64,
) -> Vec<(String, Value)> {
    if revision < RFC051_CONTAINER_ENTRIES_REVISION {
        return vec![(flat_path.to_string(), container.clone())];
    }
    match convert(flat_path, container, None) {
        Ok((_, files)) => files
            .into_iter()
            .map(|(p, b)| {
                (
                    p,
                    serde_json::from_slice(&b).expect("a converted file is JSON"),
                )
            })
            .collect(),
        Err(_) => vec![(flat_path.to_string(), container.clone())],
    }
}

/// The planned outcome: every write (`Some`) and removal (`None`), in apply order.
struct Plan {
    steps: Vec<(String, Option<Vec<u8>>)>,
    manifest: Value,
    converted: Vec<String>,
}

fn parse(files: &BTreeMap<String, Vec<u8>>, path: &str) -> Result<Value, RepositoryError> {
    serde_json::from_slice(&files[path]).map_err(|source| RepositoryError::Serialize {
        path: path.into(),
        source,
    })
}

fn dir_files<'a>(files: &'a BTreeMap<String, Vec<u8>>, id: &str) -> Vec<&'a String> {
    let dir = format!("containers/{id}/");
    files
        .range(dir.clone()..)
        .take_while(|(p, _)| p.starts_with(&dir))
        .map(|(p, _)| p)
        .collect()
}

fn plan(
    files: &BTreeMap<String, Vec<u8>>,
    manifest_raw: &Value,
    result: &mut Rfc051Result,
) -> Result<Plan, RepositoryError> {
    let root = manifest_raw.get("rootContainerId").and_then(Value::as_str);
    let flat = flat_files(files.keys());
    let (mut entries, mut heads, mut strays, mut removals) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut converted = Vec::new();
    let mut seen = BTreeMap::new();
    for path in &flat {
        let (id, out) = convert(path, &parse(files, path)?, root)?;
        if let Some(other) = seen.insert(id.clone(), path.clone()) {
            return Err(refuse(
                "migration-duplicate-container",
                format!("{other} and {path} both declare container {id}"),
            ));
        }
        // A run cut short left files here: anything not planned goes.
        for p in dir_files(files, &id) {
            if !out.contains_key(p) {
                strays.push((p.clone(), None));
            }
        }
        for (p, bytes) in out {
            if p.ends_with("/container.json") {
                heads.push((p, Some(bytes)));
            } else {
                result.entries_written += 1;
                entries.push((p, Some(bytes)));
            }
        }
        removals.push((path.clone(), None));
        converted.push(id);
    }
    let mut manifest = manifest_raw.clone();
    manifest[DATA_MODEL_REVISION_KEY] = json!(RFC051_CONTAINER_ENTRIES_REVISION);
    let mut stamps = Vec::new();
    for root in package_roots(&manifest) {
        let path = format!("{root}/package.json");
        let Some(mut idx) = files
            .get(&path)
            .and_then(|b| serde_json::from_slice::<Value>(b).ok())
        else {
            continue;
        };
        if idx
            .get(DATA_MODEL_REVISION_KEY)
            .is_some_and(|r| r != &json!(RFC051_CONTAINER_ENTRIES_REVISION))
        {
            idx[DATA_MODEL_REVISION_KEY] = json!(RFC051_CONTAINER_ENTRIES_REVISION);
            stamps.push((path.clone(), Some(framed(files.get(&path), &idx)?)));
            result.packages_stamped += 1;
        }
    }
    if revision(manifest_raw) != RFC051_CONTAINER_ENTRIES_REVISION {
        stamps.push((
            "manifest.json".to_string(),
            Some(framed(files.get("manifest.json"), &manifest)?),
        ));
    }
    result.containers_converted = converted.len();
    let steps = strays
        .into_iter()
        .chain(entries)
        .chain(heads)
        .chain(stamps)
        .chain(removals)
        .collect();
    Ok(Plan {
        steps,
        manifest,
        converted,
    })
}

/// The gate, on in-memory trees read by this build: `before` holds the flat files (stamped
/// at most 10, with any directory a cut-short run left for a converted container removed),
/// `after` the planned tree.
fn gate(
    before_files: BTreeMap<String, Vec<u8>>,
    after_files: BTreeMap<String, Vec<u8>>,
    result: &mut Rfc051Result,
) -> Result<(), RepositoryError> {
    let before = crate::tree_session::open_tree(before_files)?.with_rfc038_exemption();
    let after = crate::tree_session::open_tree(after_files)?.with_rfc038_exemption();
    let failed = |what: String| refuse("migration-gate-failed", what);

    if let Err(e) = before.catalog() {
        let after_err = after.catalog().err().map(|e| e.to_string());
        if after_err.as_deref() != Some(e.to_string().as_str()) {
            return Err(failed(format!(
                "the catalog fails before ({e}) but not the same way after ({after_err:?})"
            )));
        }
        result.gate_skipped = Some(format!(
            "the catalog is already fatal before the migration: {e}"
        ));
        return Ok(());
    }
    if let Err(e) = after.catalog() {
        return Err(failed(format!("the migrated corpus does not load: {e}")));
    }

    // Every container: head fields and outline, absent depth equal to depth 0.
    let normal = |c: srs_core::types::container::Container| {
        let mut v = serde_json::to_value(c).unwrap_or_default();
        if v.get("memberInstanceIds") == Some(&json!([])) {
            v.as_object_mut()
                .map(|o| o.shift_remove("memberInstanceIds"));
        }
        if let Some(members) = v.get_mut("memberInstanceIds").and_then(Value::as_array_mut) {
            for m in members.iter_mut() {
                if m.get("depth") == Some(&json!(0)) {
                    m.as_object_mut().map(|o| o.shift_remove("depth"));
                }
            }
        }
        v
    };
    let ids: Vec<String> = before
        .catalog()?
        .containers
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let after_ids: Vec<String> = after
        .catalog()?
        .containers
        .iter()
        .map(|e| e.id.clone())
        .collect();
    if ids != after_ids {
        return Err(failed(format!(
            "the container set would change (before {ids:?}, after {after_ids:?})"
        )));
    }
    for id in &ids {
        let (b, a) = (before.load_container(id)?, after.load_container(id)?);
        if normal(b) != normal(a) {
            return Err(failed(format!("container {id} would read differently")));
        }
        result.containers_compared += 1;
    }

    // Navigation.
    let nav = |s: &dyn RepositoryStore| {
        crate::repository_navigation_service::repository_navigation(s)
            .map(|n| serde_json::to_value(n).unwrap_or_default())
            .map_err(|e| e.to_string())
    };
    if nav(&before) != nav(&after) {
        return Err(failed("the repository navigation would change".to_string()));
    }

    // Every Composition, unscoped and for every container.
    // A corpus with no package file has no Composition to render.
    let compositions = match after.load_package() {
        Ok(p) => p.compositions,
        Err(e) if e.is_not_found() => Vec::new(),
        Err(e) => return Err(e),
    };
    let scopes: Vec<Option<&str>> = std::iter::once(None)
        .chain(ids.iter().map(|i| Some(i.as_str())))
        .collect();
    for composition in &compositions {
        for scope in &scopes {
            let render = |store: &dyn RepositoryStore| {
                let mut o =
                    crate::render_service::RenderCompositionOptions::new(store, &composition.id);
                o.container_id = *scope;
                crate::render_service::render_composition(o).map(|r| r.rendered)
            };
            let same = match (render(&before), render(&after)) {
                (Ok(a), Ok(b)) => a == b,
                (Err(a), Err(b)) => a.code() == b.code() && a.to_string() == b.to_string(),
                _ => false,
            };
            if !same {
                return Err(failed(format!(
                    "Composition {} would render differently{}",
                    composition.id,
                    scope.map_or(String::new(), |c| format!(" for container {c}"))
                )));
            }
            result.renders_compared += 1;
        }
    }
    Ok(())
}

/// The corpus as a file tree: the faithful archive enumeration, or, for a corpus with no
/// package file (which the archive refuses), every file the catalog walks.
fn corpus_files(store: &dyn RepositoryStore) -> Result<BTreeMap<String, Vec<u8>>, RepositoryError> {
    match crate::archive::tree_entries(store) {
        Err(e) if e.is_not_found() => {}
        other => return other,
    }
    let mut files = BTreeMap::new();
    for path in store.list_files_recursive("") {
        if path
            .split('/')
            .any(|seg| crate::catalog::SKIPPED_SEGMENTS.contains(&seg))
        {
            continue;
        }
        let bytes = match store.load_binary_file(&path) {
            Ok(b) => b,
            Err(_) => store.load_text_file(&path)?.into_bytes(),
        };
        files.insert(path, bytes);
    }
    Ok(files)
}

pub fn migrate_rfc051_container_entries(
    store: &dyn RepositoryStore,
) -> Result<Rfc051Result, RepositoryError> {
    let manifest_raw = raw_manifest(store)?;
    let from_revision = revision(&manifest_raw);
    let mut result = Rfc051Result {
        from_revision,
        to_revision: RFC051_CONTAINER_ENTRIES_REVISION,
        ..Default::default()
    };
    if from_revision >= RFC051_CONTAINER_ENTRIES_REVISION
        && flat_files(store.list_files_recursive("containers").iter()).is_empty()
    {
        result.already_applied = true;
        return Ok(result);
    }
    if from_revision < RFC052_ROOT_CONTAINER_REVISION {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "{MIGRATION_ID} migration requires data-model revision >= \
                 {RFC052_ROOT_CONTAINER_REVISION} (found {from_revision}): run `srs repo \
                 apply-migration --id rfc052-root-container` first (RFC-052, migration #10)"
            ),
        });
    }

    let files = corpus_files(store)?;
    let plan = plan(&files, &manifest_raw, &mut result)?;

    // ---- Gate, on in-memory trees (decided before the first write).
    let mut before = files.clone();
    for id in &plan.converted {
        for p in dir_files(&files, id) {
            before.remove(p);
        }
    }
    if from_revision >= RFC051_CONTAINER_ENTRIES_REVISION {
        let mut m = manifest_raw.clone();
        m[DATA_MODEL_REVISION_KEY] = json!(RFC052_ROOT_CONTAINER_REVISION);
        before.insert(
            "manifest.json".into(),
            framed(files.get("manifest.json"), &m)?,
        );
    }
    let mut after = files.clone();
    for (p, bytes) in &plan.steps {
        match bytes {
            Some(b) => after.insert(p.clone(), b.clone()),
            None => after.remove(p),
        };
    }
    gate(before, after, &mut result)?;

    // ---- Apply, undoing every earlier write on a failed one.
    let mut undo: Vec<(&str, Option<&Vec<u8>>)> = Vec::new();
    let applied = (|| -> Result<(), RepositoryError> {
        for (path, bytes) in &plan.steps {
            match bytes {
                Some(b) if path == "manifest.json" => write_manifest_raw(store, &plan.manifest, b)?,
                Some(b) => write_bytes(store, path, b)?,
                None => store.delete_instance_file(path)?,
            }
            undo.push((path, files.get(path)));
        }
        Ok(())
    })();
    if let Err(e) = applied {
        for (path, old) in undo.into_iter().rev() {
            let _ = match old {
                Some(b) if path == "manifest.json" => write_manifest_raw(store, &manifest_raw, b),
                Some(b) => write_bytes(store, path, b),
                None => store.delete_instance_file(path),
            };
        }
        return Err(e);
    }
    Ok(result)
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
    const SEC_C: &str = "22222222-0000-4000-8000-0000000000cc";
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

    /// A revision-10 corpus: the root (identity + sections B, A) and a Part container with a
    /// nested outline (A, then B and C beneath it), and a Composition over the Part container.
    fn rev10_repo(dir: &Path) {
        w(dir, ".srs/marker", &json!({}));
        w(
            dir,
            "manifest.json",
            &json!({
                "$schema": srs_schema::MANIFEST_SCHEMA_ID, "title": "T",
                "createdAt": "2026-01-01T00:00:00Z",
                "srsVersion": "2.0-draft", "dataModelRevision": 10,
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
                "status": "draft", "createdAt": "2026-01-01T00:00:00Z", "dataModelRevision": 10,
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
        for id in [IDENT, SEC_A, SEC_B, SEC_C] {
            w(dir, &format!("records/notes/note-{id}.json"), &note(id));
        }
        w(
            dir,
            &format!("containers/{ROOT}.json"),
            &json!({"containerId": ROOT, "title": "Root", "identityInstanceId": IDENT,
            "memberInstanceIds": [
                {"instanceId": IDENT}, {"instanceId": SEC_B}, {"instanceId": SEC_A}
            ]}),
        );
        w(
            dir,
            "containers/part.json",
            &json!({"containerId": PART, "title": "Part", "anchorInstanceId": SEC_A,
                "memberInstanceIds": [{"instanceId": SEC_A}, {"instanceId": SEC_B, "depth": 1},
                    {"instanceId": SEC_C, "depth": 1}]}),
        );
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

    fn text(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    fn migrated(dir: &Path) -> Rfc051Result {
        let store = FileStore::new(dir).with_rfc038_exemption();
        migrate_rfc051_container_entries(&store).unwrap()
    }

    #[test]
    fn converts_every_container_with_parents_and_append_keys() {
        let tmp = tempfile::tempdir().unwrap();
        rev10_repo(tmp.path());
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        assert!(migration_needed(&store));
        let r = migrated(tmp.path());
        assert_eq!(
            (
                r.containers_converted,
                r.entries_written,
                r.packages_stamped
            ),
            (2, 6, 1)
        );
        assert_eq!(r.containers_compared, 2);
        assert_eq!(
            r.renders_compared, 3,
            "unscoped and for each of two containers"
        );
        let d = tmp.path();
        assert!(!d.join("containers/part.json").exists());
        assert!(!d.join(format!("containers/{ROOT}.json")).exists());
        assert_eq!(
            text(d, &format!("containers/{PART}/container.json")),
            format!(
                "{{\"anchorInstanceId\":\"{SEC_A}\",\"containerId\":\"{PART}\",\"title\":\"Part\"}}\n"
            )
        );
        // B and C beneath A, keyed as successive appends among their siblings.
        for (id, parent, key) in [
            (SEC_A, None, "a0"),
            (SEC_B, Some(SEC_A), "a0"),
            (SEC_C, Some(SEC_A), "a1"),
        ] {
            let mut want = json!({"containerId": PART, "instanceId": id, "key": key});
            if let Some(p) = parent {
                want["parent"] = json!(p);
            }
            assert_eq!(
                text(d, &format!("containers/{PART}/entries/{id}.json")),
                container_store::line(&want)
            );
        }
        let m: Value = serde_json::from_str(&text(d, "manifest.json")).unwrap();
        assert_eq!(m[DATA_MODEL_REVISION_KEY], 11);
        let strict = FileStore::new(d);
        let part = strict.load_container(PART).unwrap();
        assert_eq!(
            part.member_instance_ids.unwrap(),
            vec![
                ContainerEntry::new(SEC_A),
                ContainerEntry::at(SEC_B, 1),
                ContainerEntry::at(SEC_C, 1)
            ]
        );
        let report = crate::validation::validate_repository(&strict).unwrap();
        assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
    }

    #[test]
    fn a_rerun_is_already_applied_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        rev10_repo(tmp.path());
        migrated(tmp.path());
        let before = snapshot(tmp.path());
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        assert!(!migration_needed(&store));
        assert!(migrated(tmp.path()).already_applied);
        assert_eq!(snapshot(tmp.path()), before);
    }

    #[test]
    fn the_result_is_deterministic() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        rev10_repo(a.path());
        rev10_repo(b.path());
        migrated(a.path());
        migrated(b.path());
        assert_eq!(snapshot(a.path()), snapshot(b.path()));
    }

    /// The Appendix vector: the 850th successive append is `bCh`.
    #[test]
    fn the_850th_append_is_keyed_bch() {
        let entries: Vec<Value> = (0..850)
            .map(|i| json!({"instanceId": format!("33333333-0000-4000-8000-{i:012x}")}))
            .collect();
        let c = json!({"containerId": PART, "title": "Big", "memberInstanceIds": entries});
        let files = fixture_container_files("containers/big.json", &c, 11);
        let last = format!(
            "containers/{PART}/entries/33333333-0000-4000-8000-{:012x}.json",
            849
        );
        let (_, v) = files.iter().find(|(p, _)| *p == last).unwrap();
        assert_eq!(v["key"], "bCh");
    }

    /// [R13]: a flat file in a corpus already stamped 11 is converted; the stamp is kept.
    #[test]
    fn a_flat_file_at_revision_11_is_converted() {
        let tmp = tempfile::tempdir().unwrap();
        rev10_repo(tmp.path());
        migrated(tmp.path());
        w(
            tmp.path(),
            "containers/late.json",
            &json!({"containerId": "11111111-0000-4000-8000-0000000000c2", "title": "Late",
                "memberInstanceIds": [{"instanceId": SEC_C}]}),
        );
        let err = FileStore::new(tmp.path())
            .catalog()
            .unwrap_err()
            .to_string();
        assert!(err.contains("container-migration-needed"), "{err}");
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        assert!(migration_needed(&store));
        let r = migrated(tmp.path());
        assert!(!r.already_applied);
        assert_eq!((r.containers_converted, r.packages_stamped), (1, 0));
        assert!(FileStore::new(tmp.path()).catalog().is_ok());
    }

    /// [R13]: an outline that breaks RFC-043 [R2] is refused with its own code, before any
    /// write.
    #[test]
    fn an_invalid_outline_is_refused_before_any_write() {
        let tmp = tempfile::tempdir().unwrap();
        rev10_repo(tmp.path());
        w(
            tmp.path(),
            "containers/part.json",
            &json!({"containerId": PART, "title": "Part",
                "memberInstanceIds": [{"instanceId": SEC_A}, {"instanceId": SEC_B, "depth": 2}]}),
        );
        let before = snapshot(tmp.path());
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        let err = migrate_rfc051_container_entries(&store).unwrap_err();
        assert_eq!(err.code(), srs_core::arrangement::CODE_DEPTH, "{err}");
        assert_eq!(snapshot(tmp.path()), before);
    }

    /// All-or-nothing: a failed manifest write undoes every earlier write, and the re-run
    /// succeeds.
    #[test]
    fn a_failed_write_leaves_the_corpus_untouched_and_the_rerun_succeeds() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        rev10_repo(tmp.path());
        let manifest = tmp.path().join("manifest.json");
        let before = snapshot(tmp.path());
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o444)).unwrap();
        let store = FileStore::new(tmp.path()).with_rfc038_exemption();
        assert!(migrate_rfc051_container_entries(&store).is_err());
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(snapshot(tmp.path()), before, "nothing left behind");
        assert!(!migrated(tmp.path()).already_applied);
    }

    /// A run cut short (a crash, not an error) is finished by a re-run: at revision 10 with
    /// some directories already written beside the flat files, and at revision 11 with flat
    /// files left over. Both end byte-identical to an uninterrupted run.
    #[test]
    fn a_run_cut_short_is_finished_by_a_rerun() {
        let clean = tempfile::tempdir().unwrap();
        rev10_repo(clean.path());
        migrated(clean.path());
        let want = snapshot(clean.path());

        // Cut after the entries of Part were written: a partial directory, no head.
        let a = tempfile::tempdir().unwrap();
        rev10_repo(a.path());
        let entry = format!("containers/{PART}/entries/{SEC_A}.json");
        std::fs::create_dir_all(a.path().join(format!("containers/{PART}/entries"))).unwrap();
        std::fs::write(
            a.path().join(&entry),
            clean
                .path()
                .join(&entry)
                .to_str()
                .map(|p| std::fs::read(p).unwrap())
                .unwrap(),
        )
        .unwrap();
        std::fs::write(
            a.path()
                .join(format!("containers/{PART}/entries/stray.json")),
            "{}",
        )
        .unwrap();
        migrated(a.path());
        assert_eq!(snapshot(a.path()), want);

        // Cut after the manifest was stamped: every directory written, a flat file left.
        let b = tempfile::tempdir().unwrap();
        for (rel, bytes) in &want {
            let p = b.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        let flat = tempfile::tempdir().unwrap();
        rev10_repo(flat.path());
        std::fs::copy(
            flat.path().join("containers/part.json"),
            b.path().join("containers/part.json"),
        )
        .unwrap();
        let r = migrated(b.path());
        assert_eq!(r.containers_converted, 1);
        assert_eq!(snapshot(b.path()), want);
    }

    /// The pre-load transformer brings a revision-10 `.srsj` to the registry entry's outcome.
    #[test]
    fn the_srsj_transformer_matches_the_registry_entry() {
        let tmp = tempfile::tempdir().unwrap();
        rev10_repo(tmp.path());
        let doc = crate::srsj::to_srsj_string(&FileStore::new(tmp.path()).with_rfc038_exemption())
            .unwrap();
        let opened = crate::srsj::open_srsj(&doc).unwrap();
        let notice = opened.migration_notice().expect("migrated in memory");
        assert_eq!(notice.source_revision, 10);
        assert_eq!(notice.migrations, [MIGRATION_ID]);
        migrated(tmp.path());
        let on_disk = FileStore::new(tmp.path());
        for id in [ROOT, PART] {
            assert_eq!(
                opened.load_container(id).unwrap(),
                on_disk.load_container(id).unwrap()
            );
        }
        // Published documents open in memory and are never rewritten: a write is refused.
        let err = crate::services::create_note(
            &opened,
            serde_json::from_value(json!({"title": "t", "sections": []})).unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "migration-required-before-write");
    }
}
