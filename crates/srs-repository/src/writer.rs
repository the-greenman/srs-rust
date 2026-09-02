use crate::catalog::RepositoryCatalog;
use crate::error::RepositoryError;
use crate::manifest::Manifest;
use crate::store::RepositoryStore;
use srs_core::types::note::Note;
use srs_schema::NOTE_SCHEMA_ID;
use std::collections::{HashMap, HashSet};

/// Build the known-instance-id set and the `instanceId → typeId` map used by
/// E1/E4 relation validation, from one already-fetched catalog snapshot
/// (RFC-038: no `manifest.instanceIndex` — callers that need both values for
/// one operation must not call `store.catalog()` twice).
///
/// The type-id map records each Tier-2 Record's `typeId` (srs-rust#910: the
/// semanticObjectType collapse re-keys E4 onto the real Type system, srs#372/
/// #383/#524) — a Tier-0 Note has no Type binding and is simply absent from
/// the map, so `requireSameType` is a no-op for it. This is the single source
/// of truth for both values: `relation_service::create_relation` and `repo
/// validate` both consume it, so the write path and the at-rest path enforce
/// E1/E4 over identical inputs (#556).
pub(crate) fn known_instances_and_type_ids(
    store: &dyn RepositoryStore,
    cat: &RepositoryCatalog,
) -> (HashSet<String>, HashMap<String, String>) {
    let mut known_instance_ids = HashSet::with_capacity(cat.instances.len());
    let mut type_ids: HashMap<String, String> = HashMap::new();
    for entry in &cat.instances {
        known_instance_ids.insert(entry.id.clone());
        let Some(locator) = entry.locator.as_deref() else {
            continue;
        };
        if let Ok(val) = store.load_instance_json(locator) {
            // Tier-2 Records carry a resolved `typeId`; Tier-0 Notes have no Type
            // binding and are simply absent from this map (srs-rust#910:
            // semanticObjectType collapse, srs#372/#383/#524).
            if let Some(type_id) = val.get("typeId").and_then(|v| v.as_str()) {
                type_ids.insert(entry.id.clone(), type_id.to_string());
            }
        }
    }
    (known_instance_ids, type_ids)
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
    use crate::store::memory::MemoryStore;
    use srs_core::types::note::{Note, NoteSection};

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
    fn write_manifest_roundtrips_via_store() {
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("title".to_string(), serde_json::json!("Roundtrip Title"));
        write_manifest(&store, &manifest).unwrap();

        let reloaded = store.load_manifest().unwrap();
        assert_eq!(
            reloaded.extra.get("title"),
            Some(&serde_json::json!("Roundtrip Title"))
        );
    }

    #[test]
    fn known_instances_and_type_ids_reads_from_catalog() {
        // RFC-038: no manifest.instanceIndex — the known-id set is built from
        // one catalog snapshot, reading each entity body's locator directly.
        // A Tier-0 Note has no `typeId` (srs-rust#910: semanticObjectType
        // collapse, srs#372/#383/#524) — the type-id map is empty for it;
        // exercised against real Tier-2 data by relation_service's own E4 tests.
        let store = MemoryStore::default();
        let note = make_note("sem-000000-0000-4000-8000-000000000001", "Typed Note");
        write_note(&store, &note, "records/notes/sem-00000000.json").unwrap();

        let cat = store.catalog().unwrap();
        let (known_ids, map) = known_instances_and_type_ids(&store, &cat);
        assert!(known_ids.contains("sem-000000-0000-4000-8000-000000000001"));
        assert!(map.is_empty());
    }
}
