use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::manifest::Manifest;
use crate::store::RepositoryStore;
use srs_core::types::note::Note;
use srs_core::types::relation::Relation;
use srs_core::types::tag_definition::TagDefinition;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use std::collections::{HashMap, HashSet};

/// Validates a relation against the installed package definitions before writing.
pub fn validate_relation_before_write(
    relation: &Relation,
    store: &dyn RepositoryStore,
) -> Result<(), RepositoryError> {
    let pkg = store.load_package()?;
    let manifest = store.load_manifest()?;

    let known_instance_ids: HashSet<String> = manifest
        .instance_index
        .iter()
        .map(|e| e.instance_id().to_string())
        .collect();

    let mut instance_semantic_types: HashMap<String, String> = HashMap::new();
    for entry in &manifest.instance_index {
        if let Ok(val) = store.load_instance_json(entry.path()) {
            if let Some(sot) = val.get("semanticObjectType").and_then(|v| v.as_str()) {
                instance_semantic_types.insert(entry.instance_id().to_string(), sot.to_string());
            }
        }
    }

    let ctx = RelationValidationContext {
        definitions: &pkg.relation_type_definitions,
        known_instance_ids: &known_instance_ids,
        instance_semantic_types: &instance_semantic_types,
    };

    validate_relation(relation, &ctx, true).map_err(|errs| {
        let msg = errs
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; ");
        RepositoryError::RelationValidation {
            relation_id: relation.relation_id.clone(),
            message: msg,
        }
    })
}

/// Generate a new UUID v4 as a string. Only this function generates UUIDs.
pub fn new_instance_id() -> String {
    uuid::Uuid::new_v4().to_string()
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
            serde_json::Value::String(
                "https://srs.semanticops.com/schema/2.0/note.json".to_string(),
            ),
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

/// Write a TagDefinition to the store at the given relative path.
pub fn write_tag_definition(
    store: &dyn RepositoryStore,
    td: &TagDefinition,
    relative_path: &str,
) -> Result<(), RepositoryError> {
    let value = serde_json::to_value(td).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })?;
    store.save_instance_json(relative_path, &value)
}

/// Add or replace the manifest index entry for a TagDefinition (in memory only).
pub fn upsert_tag_definition_index_entry(
    manifest: &mut Manifest,
    td: &TagDefinition,
    relative_path: &str,
) {
    let entry = InstanceIndexEntry {
        instance_id: td.instance_id.clone(),
        tier: 3,
        path: relative_path.to_string(),
        title: td.label.clone().map(serde_json::Value::String),
        tags: None,
    };

    if let Some(pos) = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == td.instance_id)
    {
        manifest.instance_index[pos] = entry;
    } else {
        manifest.instance_index.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::store::memory::MemoryStore;
    use srs_core::types::note::{Note, NoteSection};
    use std::collections::HashMap;
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
            extra: HashMap::new(),
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
            extra: HashMap::new(),
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
