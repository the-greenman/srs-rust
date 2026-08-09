use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::manifest::Manifest;
use crate::store::RepositoryStore;
use srs_core::types::note::Note;
use srs_schema::NOTE_SCHEMA_ID;
use std::collections::HashMap;

/// Build the `instanceId → semanticObjectType` map used by E4 relation validation.
///
/// Reads each instance file listed in the manifest index and records its
/// top-level `semanticObjectType` when present (instances without the field are
/// simply absent from the map, so E4 is a no-op for them). This is the single
/// source of truth for the map: `relation_service::create_relation` and
/// `repo validate` both consume it, so the write path and the at-rest path
/// enforce E4 over identical inputs (#556).
pub(crate) fn build_instance_semantic_types(
    store: &dyn RepositoryStore,
    manifest: &Manifest,
) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for entry in &manifest.instance_index {
        if let Ok(val) = store.load_instance_json(entry.path()) {
            if let Some(sot) = val.get("semanticObjectType").and_then(|v| v.as_str()) {
                map.insert(entry.instance_id().to_string(), sot.to_string());
            }
        }
    }
    map
}

/// Generate a new UUID v4 as a string. Only this function generates UUIDs.
pub fn new_instance_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Canonical slug algorithm for instance file naming.
///
/// Replaces every non-alphanumeric character with `-`, splits on `-`, filters
/// empty parts, and rejoins. Returns `""` on empty input — callers produce an
/// id-only filename (`{id8}.json`) when the slug is empty.
pub(crate) fn slugify_instance_name(name: &str) -> String {
    let parts: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    parts.join("-").to_lowercase()
}

/// Write a Note to the store at the given relative path.
pub fn write_note(
    store: &dyn RepositoryStore,
    note: &Note,
    relative_path: &str,
) -> Result<(), RepositoryError> {
    let mut value = serde_json::to_value(note).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })?;

    if let serde_json::Value::Object(ref mut obj) = value {
        obj.insert(
            "$schema".to_string(),
            serde_json::Value::String(NOTE_SCHEMA_ID.to_string()),
        );
    }

    store.save_instance_json(relative_path, &value)
}

/// Add or replace the manifest index entry for a Note (in memory only).
pub fn upsert_index_entry(manifest: &mut Manifest, note: &Note, relative_path: &str) {
    let entry = InstanceIndexEntry {
        instance_id: note.instance_id.clone(),
        tier: 0,
        path: relative_path.to_string(),
        title: note.title.clone().map(serde_json::Value::String),
        tags: note.tags.clone(),
    };

    if let Some(pos) = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == note.instance_id)
    {
        manifest.instance_index[pos] = entry;
    } else {
        manifest.instance_index.push(entry);
    }
}

/// Write the manifest back via the store.
pub fn write_manifest(
    store: &dyn RepositoryStore,
    manifest: &Manifest,
) -> Result<(), RepositoryError> {
    store.save_manifest(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::store::memory::MemoryStore;
    use srs_core::types::note::{Note, NoteSection};
    use std::path::PathBuf;

    fn make_note(id: &str, title: &str) -> Note {
        Note {
            instance_id: id.to_string(),
            title: Some(title.to_string()),
            tags: Some(vec!["test".to_string()]),
            sections: vec![NoteSection {
                name: "section1".to_string(),
                label: Some("Section 1".to_string()),
                content: "Test content".to_string(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    fn empty_manifest() -> Manifest {
        Manifest {
            instance_index: vec![],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        }
    }

    #[test]
    fn new_instance_id_produces_unique_uuids() {
        let id1 = new_instance_id();
        let id2 = new_instance_id();
        assert_ne!(id1, id2);
        assert!(uuid::Uuid::parse_str(&id1).is_ok());
        assert!(uuid::Uuid::parse_str(&id2).is_ok());
    }

    #[test]
    fn write_note_stores_with_schema_header() {
        let store = MemoryStore::default();
        let note = make_note("test-123", "Test Note");
        write_note(&store, &note, "records/notes/test.json").unwrap();

        let val = store.load_instance_json("records/notes/test.json").unwrap();
        assert_eq!(
            val.get("$schema").and_then(|v| v.as_str()),
            Some("https://srs.semanticops.com/schema/2.0/note.json")
        );
        assert_eq!(val["instanceId"].as_str(), Some("test-123"));
    }

    #[test]
    fn upsert_index_entry_adds_new_entry() {
        let mut manifest = empty_manifest();
        let note = make_note("new-id", "New Note");
        upsert_index_entry(&mut manifest, &note, "records/notes/new.json");
        assert_eq!(manifest.instance_index.len(), 1);
        assert_eq!(manifest.instance_index[0].instance_id(), "new-id");
    }

    #[test]
    fn upsert_index_entry_replaces_existing_by_id() {
        let mut manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: "existing-id".to_string(),
                tier: 0,
                path: "records/notes/old.json".to_string(),
                title: Some(serde_json::Value::String("Old Title".to_string())),
                tags: None,
            }],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let note = make_note("existing-id", "New Title");
        upsert_index_entry(&mut manifest, &note, "records/notes/new.json");
        assert_eq!(manifest.instance_index.len(), 1);
        assert_eq!(manifest.instance_index[0].path(), "records/notes/new.json");
    }

    #[test]
    fn write_manifest_roundtrips_via_store() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        let note = make_note("some-id", "Some Note");
        upsert_index_entry(&mut manifest, &note, "records/notes/some.json");
        write_manifest(&store, &manifest).unwrap();

        let reloaded = store.load_manifest().unwrap();
        assert_eq!(reloaded.instance_index.len(), 1);
        assert_eq!(reloaded.instance_index[0].instance_id(), "some-id");
    }
}
