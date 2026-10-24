//! RFC-051 per-entry container storage (`dataModelRevision` 11).
//!
//! A Container is the directory `containers/<containerId>/`: a one-line head
//! `container.json` (every Container property except `memberInstanceIds`) and one one-line
//! entry file per member, `entries/<instanceId>.json` (`containerId`, `instanceId`,
//! `parent?`, `key`) ([R1]-[R3]). This module is the one place that reads and writes that
//! layout. Every reader gets the logical Container, its outline decoded by
//! [`srs_core::container_decode::decode`] ([R5]); every writer writes exactly the entries
//! [`srs_core::container_write`] names ([R8], [R9], [R11]) and never an unchanged head.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde_json::{json, Value};
use srs_core::container_decode::{decode, StoredEntry};
use srs_core::container_write;
use srs_core::types::container::Container;
use std::collections::HashSet;

/// [R14] diagnostic codes owned by the storage layer.
pub mod codes {
    pub const SCHEMA: &str = "container-schema";
    pub const NAME_MISMATCH: &str = "container-name-mismatch";
    pub const ENTRY_MISPLACED: &str = "container-entry-misplaced";
    pub const STRAY_FILE: &str = "container-stray-file";
    pub const HEAD_MISSING: &str = "container-head-missing";
    pub const MIGRATION_NEEDED: &str = "container-migration-needed";
    /// `identityInstanceId` or `anchorInstanceId` names no entry (RFC-043 [R3], which
    /// assigned no code): an error, not fatal, fixed by a head edit.
    pub const POINTER_UNRESOLVED: &str = "container-pointer-unresolved";
}

/// Where a file under `containers/` sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place<'a> {
    /// `containers/<name>` — a revision-10-or-lower container file.
    Flat,
    /// `containers/<dir>/container.json`.
    Head(&'a str),
    /// `containers/<dir>/entries/<file>.json`, `file` without its extension.
    Entry { dir: &'a str, file: &'a str },
    /// Any other file in `containers/<dir>/` ([R1]: `container-stray-file`).
    Stray(&'a str),
}

pub fn place(path: &str) -> Option<Place<'_>> {
    let rest = path.strip_prefix("containers/")?;
    let (dir, inner) = match rest.split_once('/') {
        None => return Some(Place::Flat),
        Some(split) => split,
    };
    Some(match inner.split('/').collect::<Vec<_>>().as_slice() {
        ["container.json"] => Place::Head(dir),
        ["entries", f] => match f.strip_suffix(".json") {
            Some(file) => Place::Entry { dir, file },
            None => Place::Stray(dir),
        },
        _ => Place::Stray(dir),
    })
}

pub fn head_path(container_id: &str) -> String {
    format!("containers/{container_id}/container.json")
}

pub fn entry_path(container_id: &str, instance_id: &str) -> String {
    format!("containers/{container_id}/entries/{instance_id}.json")
}

/// The locator names a stored head (directory form), not a flat container file.
pub fn is_head(locator: &str) -> bool {
    matches!(place(locator), Some(Place::Head(_)))
}

/// [R2]: the canonical lowercase hyphenated UUID form.
pub fn is_canonical_id(id: &str) -> bool {
    uuid::Uuid::try_parse(id).is_ok_and(|u| u.as_hyphenated().to_string() == id)
}

/// [R3]: UTF-8 JSON on one line, properties in byte-wise key order, no insignificant
/// whitespace, a trailing LF.
pub fn line(value: &Value) -> String {
    fn sorted(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|k| (k.clone(), sorted(&m[k])))
                        .collect(),
                )
            }
            Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let mut s = serde_json::to_string(&sorted(value)).expect("a JSON value serialises");
    s.push('\n');
    s
}

/// The head of `container`: every property except `memberInstanceIds` ([R1]).
pub fn head_value(container: &Container) -> Result<Value, RepositoryError> {
    let mut v = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
        path: head_path(&container.container_id).into(),
        source,
    })?;
    if let Some(o) = v.as_object_mut() {
        o.shift_remove("memberInstanceIds");
    }
    Ok(v)
}

pub fn entry_value(container_id: &str, e: &StoredEntry) -> Value {
    let mut v = json!({"containerId": container_id, "instanceId": e.instance_id, "key": e.key});
    if let Some(p) = &e.parent {
        v["parent"] = json!(p);
    }
    v
}

/// [R1], [R2] and [R4] for one entry file found at `containers/<dir>/entries/<file>.json`:
/// the stored entry, or the `(code, message)` of the first rule it breaks.
pub fn parse_entry(
    value: &Value,
    dir: &str,
    file: &str,
) -> Result<StoredEntry, (&'static str, String)> {
    srs_schema::SchemaRegistry::global()
        .validate_by_id(srs_schema::STORED_CONTAINER_ENTRY_SCHEMA_ID, value)
        .map_err(|e| {
            (
                codes::SCHEMA,
                format!("entry fails stored-container-entry.json: {e}"),
            )
        })?;
    let s = |k: &str| value.get(k).and_then(Value::as_str).unwrap_or_default();
    let instance_id = s("instanceId");
    if file != instance_id || !is_canonical_id(file) {
        return Err((
            codes::NAME_MISMATCH,
            format!(
                "entry file name '{file}' must be its instanceId '{instance_id}' in canonical \
                 lowercase UUID form (RFC-051 [R2])"
            ),
        ));
    }
    if s("containerId") != dir {
        return Err((
            codes::ENTRY_MISPLACED,
            format!(
                "entry declares containerId '{}' but sits in containers/{dir}/ (RFC-051 [R2])",
                s("containerId")
            ),
        ));
    }
    let key = s("key");
    srs_core::arrangement_key::validate_key(key).map_err(|e| {
        (
            srs_core::arrangement_key::CODE_KEY,
            format!("key '{key}': {}", e.0),
        )
    })?;
    Ok(StoredEntry {
        instance_id: instance_id.to_string(),
        parent: value
            .get("parent")
            .and_then(Value::as_str)
            .map(str::to_string),
        key: key.to_string(),
    })
}

fn storage_error(code: &'static str, message: String) -> RepositoryError {
    RepositoryError::ContainerStorage { code, message }
}

/// The stored entries of `containers/<container_id>/`, sorted by `instanceId` so the decode's
/// findings do not depend on directory listing order. Every entry is checked against [R1],
/// [R2] and [R4] first, so `decode` never sees a duplicate or misnamed id (one file per
/// member, named by its `instanceId`). Strays are left to the catalog.
pub fn read_entries<S: RepositoryStore + ?Sized>(
    store: &S,
    container_id: &str,
) -> Result<Vec<StoredEntry>, RepositoryError> {
    let mut out = Vec::new();
    for path in store.list_files_recursive(&format!("containers/{container_id}/entries")) {
        let Some(Place::Entry { dir, file }) = place(&path) else {
            continue;
        };
        let value = store.load_instance_json(&path)?;
        let entry = parse_entry(&value, dir, file)
            .map_err(|(code, m)| storage_error(code, format!("{path}: {m}")))?;
        out.push(entry);
    }
    out.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    Ok(out)
}

/// The logical Container stored at the head `locator` ([R5]).
pub fn load<S: RepositoryStore + ?Sized>(
    store: &S,
    locator: &str,
) -> Result<Container, RepositoryError> {
    serde_json::from_value(logical_value(store, locator)?).map_err(|source| {
        RepositoryError::ManifestParse {
            path: locator.into(),
            source,
        }
    })
}

/// The logical Container JSON at a container locator: a head with its decoded
/// `memberInstanceIds`, or a flat container file as it is.
pub fn logical_value<S: RepositoryStore + ?Sized>(
    store: &S,
    locator: &str,
) -> Result<Value, RepositoryError> {
    let mut value = store.load_instance_json(locator)?;
    let Some(Place::Head(dir)) = place(locator) else {
        return Ok(value);
    };
    let decoded = decode(&read_entries(store, dir)?, None);
    if let Some(o) = value.as_object_mut() {
        o.shift_remove("memberInstanceIds");
        if !decoded.entries.is_empty() {
            o.insert("memberInstanceIds".into(), json!(decoded.entries));
        }
    }
    Ok(value)
}

/// How [`save`] keys the entries it writes.
#[derive(Debug, Clone, Copy)]
pub enum Keying<'a> {
    /// An RFC-043 Change D operation (or a whole replace): [R8]'s two passes. `moved` is the
    /// entry a move operation moves; a whole replace names none and writes the entries out
    /// of stored order.
    Op { moved: Option<&'a str> },
    /// Repair ([R11] step 1): the container's outline is the stored decode with the entries
    /// absent from `container` removed, and detached entries re-attached.
    Repair,
}

/// What [`save`] changed: the entry ids written (in outline order) and removed.
#[derive(Debug, Default, Clone)]
pub struct Saved {
    pub written: Vec<String>,
    pub removed: Vec<String>,
}

fn write_line<S: RepositoryStore + ?Sized>(
    store: &S,
    path: &str,
    value: &Value,
) -> Result<(), RepositoryError> {
    if store.is_file_tree_store() {
        store.save_text_file(path, &line(value))
    } else {
        store.save_instance_json(path, value)
    }
}

/// Persist `container` in directory form: the head only when it differs from the stored one
/// (a member operation never writes it, [R8]), then exactly the entries the write set names,
/// each as one canonical line ([R3]).
pub fn save<S: RepositoryStore + ?Sized>(
    store: &S,
    container: &Container,
    keying: Keying<'_>,
) -> Result<Saved, RepositoryError> {
    let id = container.container_id.as_str();
    if !is_canonical_id(id) {
        return Err(storage_error(
            codes::NAME_MISMATCH,
            format!(
                "container id '{id}' must be a canonical lowercase UUID: it names the container's \
                 directory (RFC-051 [R2])"
            ),
        ));
    }
    let stored = read_entries(store, id)?;
    let outline = container.member_instance_ids.clone().unwrap_or_default();
    let ws = match keying {
        Keying::Op { moved } => {
            container_write::write_set(&stored, None, moved, |_| Ok(outline.clone()))
        }
        Keying::Repair => {
            let keep: HashSet<&str> = outline.iter().map(|e| e.instance_id.as_str()).collect();
            container_write::repair(&stored, None, |i| keep.contains(i))
        }
    }
    .map_err(|v| storage_error(v.code, format!("container {id}: {}", v.message)))?;

    // Every written entry names a file: a non-canonical id would escape the directory or
    // brick the repository ([R2]). Refused before any write.
    if let Some(e) = ws.write.iter().find(|e| !is_canonical_id(&e.instance_id)) {
        return Err(storage_error(
            codes::NAME_MISMATCH,
            format!(
                "member id '{}' must be a canonical lowercase UUID: it names an entry file \
                 (RFC-051 [R2])",
                e.instance_id
            ),
        ));
    }
    let hp = head_path(id);
    let head = head_value(container)?;
    let current = match store.load_instance_json(&hp) {
        Ok(v) => Some(v),
        Err(e) if e.is_not_found() => None,
        Err(e) => return Err(e),
    };
    if current.as_ref() != Some(&head) {
        write_line(store, &hp, &head)?;
    }
    for e in &ws.write {
        write_line(store, &entry_path(id, &e.instance_id), &entry_value(id, e))?;
    }
    for iid in &ws.remove {
        store.delete_instance_file(&entry_path(id, iid))?;
    }
    Ok(Saved {
        written: ws.write.into_iter().map(|e| e.instance_id).collect(),
        removed: ws.remove,
    })
}

/// Remove the whole directory of `container_id` ([R1] delete), the head first: a delete cut
/// short leaves a headless directory, which is reported and repaired ([R11]).
pub fn delete<S: RepositoryStore + ?Sized>(
    store: &S,
    container_id: &str,
) -> Result<(), RepositoryError> {
    let hp = head_path(container_id);
    let mut files = store.list_files_recursive(&format!("containers/{container_id}"));
    files.sort_by_key(|p| *p != hp);
    for path in files {
        store.delete_instance_file(&path)?;
    }
    Ok(())
}

/// `containers/<container_id>/container.json` exists (the container is stored as a directory).
pub fn has_head<S: RepositoryStore + ?Sized>(store: &S, container_id: &str) -> bool {
    store.load_instance_json(&head_path(container_id)).is_ok()
}

/// The detached entries (orphans and loop breaks, [R5]) of a stored directory, in decoding
/// order; empty for a flat container.
pub fn detached<S: RepositoryStore + ?Sized>(
    store: &S,
    container_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let decoded = decode(&read_entries(store, container_id)?, None);
    let found: HashSet<&str> = decoded
        .findings
        .iter()
        .map(|f| f.instance_id.as_str())
        .collect();
    Ok(decoded
        .entries
        .iter()
        .filter(|e| found.contains(e.instance_id.as_str()))
        .map(|e| e.instance_id.clone())
        .collect())
}

/// [R11] step 2 for one directory: when `containers/<container_id>/` has entries but no head,
/// remove it and return the entry ids it held; `None` when there is no such directory.
pub fn remove_headless<S: RepositoryStore + ?Sized>(
    store: &S,
    container_id: &str,
) -> Result<Option<Vec<String>>, RepositoryError> {
    if !is_canonical_id(container_id) {
        return Ok(None);
    }
    let files = store.list_files_recursive(&format!("containers/{container_id}"));
    if files.is_empty() || files.iter().any(|p| *p == head_path(container_id)) {
        return Ok(None);
    }
    let mut ids: Vec<String> = files
        .iter()
        .filter_map(|p| match place(p) {
            Some(Place::Entry { file, .. }) => Some(file.to_string()),
            _ => None,
        })
        .collect();
    ids.sort();
    delete(store, container_id)?;
    Ok(Some(ids))
}

/// [R1] copy: rewrite every entry of `from` into `to` with `to`'s `containerId`, keeping its
/// `parent` and `key` (`rename` maps an instance id the copy swapped). Replaces the entries
/// `to` holds.
pub fn copy_entries<S: RepositoryStore + ?Sized>(
    store: &S,
    from: &str,
    to: &str,
    rename: impl Fn(&str) -> String,
) -> Result<(), RepositoryError> {
    let copied: Vec<StoredEntry> = read_entries(store, from)?
        .into_iter()
        .map(|e| StoredEntry {
            instance_id: rename(&e.instance_id),
            parent: e.parent.as_deref().map(&rename),
            key: e.key,
        })
        .collect();
    if !is_canonical_id(to) || copied.iter().any(|e| !is_canonical_id(&e.instance_id)) {
        return Err(storage_error(
            codes::NAME_MISMATCH,
            format!("copy into {to}: every id must be a canonical lowercase UUID (RFC-051 [R2])"),
        ));
    }
    let keep: HashSet<&str> = copied.iter().map(|e| e.instance_id.as_str()).collect();
    for e in read_entries(store, to)? {
        if !keep.contains(e.instance_id.as_str()) {
            store.delete_instance_file(&entry_path(to, &e.instance_id))?;
        }
    }
    for e in &copied {
        write_line(store, &entry_path(to, &e.instance_id), &entry_value(to, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places() {
        assert_eq!(place("containers/x.json"), Some(Place::Flat));
        assert_eq!(place("containers/c/container.json"), Some(Place::Head("c")));
        assert_eq!(
            place("containers/c/entries/i.json"),
            Some(Place::Entry {
                dir: "c",
                file: "i"
            })
        );
        assert_eq!(place("containers/c/entries/i.txt"), Some(Place::Stray("c")));
        assert_eq!(place("containers/c/notes.md"), Some(Place::Stray("c")));
        assert_eq!(
            place("containers/c/entries/x/y.json"),
            Some(Place::Stray("c"))
        );
        assert_eq!(place("records/x.json"), None);
    }

    #[test]
    fn line_is_sorted_compact_and_terminated() {
        let v = json!({"key": "a0", "containerId": "c", "instanceId": "i", "parent": "p"});
        assert_eq!(
            line(&v),
            "{\"containerId\":\"c\",\"instanceId\":\"i\",\"key\":\"a0\",\"parent\":\"p\"}\n"
        );
    }
}
