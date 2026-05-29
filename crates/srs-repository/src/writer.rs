use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::manifest::load_manifest;
use crate::manifest::Manifest;
use crate::package::load_package;
use srs_core::types::note::Note;
use srs_core::types::relation::Relation;
use srs_core::types::tag_definition::TagDefinition;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Validates a relation against the installed package definitions before writing.
///
/// Loads the package, collects all known instance IDs and semanticObjectType map,
/// constructs a `RelationValidationContext`, and runs `validate_relation` with
/// `is_write: true`. Returns `Ok(())` if validation passes.
pub fn validate_relation_before_write(
    relation: &Relation,
    repo_root: &Path,
) -> Result<(), RepositoryError> {
    let pkg = load_package(repo_root)?;
    let manifest = load_manifest(repo_root)?;

    let known_instance_ids: HashSet<String> = manifest
        .instance_index
        .iter()
        .map(|e| e.instance_id().to_string())
        .collect();

    let mut instance_semantic_types: HashMap<String, String> = HashMap::new();
    for entry in &manifest.instance_index {
        let inst_path = repo_root.join(entry.path());
        if let Ok(raw) = std::fs::read_to_string(&inst_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(sot) = val.get("semanticObjectType").and_then(|v| v.as_str()) {
                    instance_semantic_types
                        .insert(entry.instance_id().to_string(), sot.to_string());
                }
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

/// Write a Note to disk as pretty-printed JSON.
pub fn write_note(note: &Note, path: &Path) -> Result<(), RepositoryError> {
    let mut value = serde_json::to_value(note).map_err(|e| RepositoryError::Serialize {
        path: path.to_path_buf(),
        source: e,
    })?;

    if let serde_json::Value::Object(ref mut object) = value {
        object.insert(
            "$schema".to_string(),
            serde_json::Value::String(
                "https://srs.semanticops.com/schema/2.0/note.json".to_string(),
            ),
        );
    }

    let json = serde_json::to_string_pretty(&value).map_err(|e| RepositoryError::Serialize {
        path: path.to_path_buf(),
        source: e,
    })?;

    std::fs::write(path, json).map_err(|e| RepositoryError::NoteWrite {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(())
}

/// Add or replace the manifest index entry for a Note (in memory only).
pub fn upsert_index_entry(manifest: &mut Manifest, note: &Note, relative_path: &str) {
    let entry = InstanceIndexEntry {
        instance_id: note.instance_id.clone(),
        tier: 0, // Default tier for new notes
        path: relative_path.to_string(),
        title: note.title.clone().map(serde_json::Value::String),
        tags: note.tags.clone(),
    };

    // Check if entry with same instance_id exists and replace it
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

/// Write the manifest back to disk, preserving all original fields.
pub fn write_manifest(manifest: &Manifest) -> Result<(), RepositoryError> {
    let manifest_path = manifest.root.join("manifest.json");

    let json = serde_json::to_string_pretty(manifest).map_err(|e| RepositoryError::Serialize {
        path: manifest_path.clone(),
        source: e,
    })?;

    std::fs::write(&manifest_path, json).map_err(|e| RepositoryError::Io {
        path: manifest_path,
        source: e,
    })?;

    Ok(())
}

/// Write a TagDefinition to disk as pretty-printed JSON.
pub fn write_tag_definition(td: &TagDefinition, path: &Path) -> Result<(), RepositoryError> {
    let json = serde_json::to_string_pretty(td).map_err(|e| RepositoryError::Serialize {
        path: path.to_path_buf(),
        source: e,
    })?;

    std::fs::write(path, json).map_err(|e| RepositoryError::TagDefinitionWrite {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(())
}

/// Add or replace the manifest index entry for a TagDefinition (in memory only).
/// Uses tier: 3 for TagDefinition instances.
pub fn upsert_tag_definition_index_entry(
    manifest: &mut Manifest,
    td: &TagDefinition,
    relative_path: &str,
) {
    let entry = InstanceIndexEntry {
        instance_id: td.instance_id.clone(),
        tier: 3, // TagDefinition tier
        path: relative_path.to_string(),
        title: td.label.clone().map(serde_json::Value::String),
        tags: None, // TagDefinitions don't have tags in the index
    };

    // Check if entry with same instance_id exists and replace it
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
    use crate::manifest::load_manifest;
    use srs_core::types::note::{Note, NoteSection};
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn new_instance_id_produces_unique_uuids() {
        let id1 = new_instance_id();
        let id2 = new_instance_id();
        assert_ne!(id1, id2);

        // Verify they are valid UUIDs
        assert!(uuid::Uuid::parse_str(&id1).is_ok());
        assert!(uuid::Uuid::parse_str(&id2).is_ok());
    }

    #[test]
    fn write_note_roundtrips_and_includes_schema_header() {
        let temp = TempDir::new().unwrap();
        let note_path = temp.path().join("test-note.json");

        let note = Note {
            instance_id: "test-123".to_string(),
            title: Some("Test Note".to_string()),
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
        };

        write_note(&note, &note_path).unwrap();

        // Read back and verify
        let loaded = crate::loader::load_note(&note_path).unwrap();
        assert_eq!(loaded.instance_id, note.instance_id);
        assert_eq!(loaded.title, note.title);
        assert_eq!(loaded.sections.len(), 1);

        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&note_path).unwrap()).unwrap();
        assert_eq!(
            raw.get("$schema").and_then(|schema| schema.as_str()),
            Some("https://srs.semanticops.com/schema/2.0/note.json")
        );
    }

    #[test]
    fn upsert_index_entry_adds_new_entry() {
        let mut manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: Path::new("/tmp").to_path_buf(),
        };

        let note = Note {
            instance_id: "new-id".to_string(),
            title: Some("New Note".to_string()),
            tags: Some(vec!["tag1".to_string()]),
            sections: vec![],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };

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
            root: Path::new("/tmp").to_path_buf(),
        };

        let note = Note {
            instance_id: "existing-id".to_string(),
            title: Some("New Title".to_string()),
            tags: None,
            sections: vec![],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };

        upsert_index_entry(&mut manifest, &note, "records/notes/new.json");

        assert_eq!(manifest.instance_index.len(), 1);
        assert_eq!(manifest.instance_index[0].path(), "records/notes/new.json");
    }

    #[test]
    fn write_manifest_preserves_extra_fields() {
        let temp = TempDir::new().unwrap();

        // Create initial manifest
        let manifest_json = r#"{
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo",
            "instanceIndex": [
                {
                    "instanceId": "11111111-1111-4111-8111-111111111111",
                    "tier": 0,
                    "path": "records/notes/test.json"
                }
            ]
        }"#;
        fs::write(temp.path().join("manifest.json"), manifest_json).unwrap();

        // Load, modify, and write back
        let mut manifest = load_manifest(temp.path()).unwrap();
        assert!(manifest.extra.contains_key("srsVersion"));
        assert!(manifest.extra.contains_key("repositoryId"));

        // Add a new entry
        let note = Note {
            instance_id: "new-note".to_string(),
            title: None,
            tags: None,
            sections: vec![],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };
        upsert_index_entry(&mut manifest, &note, "records/notes/new.json");

        write_manifest(&manifest).unwrap();

        // Reload and verify extra fields preserved
        let reloaded = load_manifest(temp.path()).unwrap();
        assert!(reloaded.extra.contains_key("srsVersion"));
        assert!(reloaded.extra.contains_key("repositoryId"));
        assert_eq!(
            reloaded.extra.get("srsVersion").unwrap().as_str(),
            Some("2.0-draft")
        );
    }
}
