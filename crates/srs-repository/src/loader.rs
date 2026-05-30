use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::note::Note;
use srs_core::types::tag_definition::TagDefinition;
use srs_core::validation::note::validate_note;
use srs_core::validation::tag_definition::validate_tag_definition;
use std::path::PathBuf;

/// Load a Note from the store by relative path, validating after deserialization.
pub fn load_note(
    store: &dyn RepositoryStore,
    relative_path: &str,
) -> Result<Note, RepositoryError> {
    let value = store.load_instance_json(relative_path)?;
    let note: Note = serde_json::from_value(value).map_err(|e| RepositoryError::NoteLoad {
        path: PathBuf::from(relative_path),
        source: e,
    })?;
    validate_note(&note).map_err(|e| RepositoryError::NoteValidation {
        path: PathBuf::from(relative_path),
        source: e,
    })?;
    Ok(note)
}

/// Load a TagDefinition from the store by relative path, validating after deserialization.
pub fn load_tag_definition(
    store: &dyn RepositoryStore,
    relative_path: &str,
) -> Result<TagDefinition, RepositoryError> {
    let value = store.load_instance_json(relative_path)?;
    let td: TagDefinition =
        serde_json::from_value(value).map_err(|e| RepositoryError::TagDefinitionLoad {
            path: PathBuf::from(relative_path),
            source: e,
        })?;
    validate_tag_definition(&td).map_err(|e| RepositoryError::TagDefinitionValidation {
        path: PathBuf::from(relative_path),
        source: e,
    })?;
    Ok(td)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use srs_core::types::note::NoteSection;

    #[test]
    fn load_note_roundtrips_from_memory() {
        let store = MemoryStore::default();
        let note = srs_core::types::note::Note {
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
        let val = serde_json::to_value(&note).unwrap();
        let store = store.with_data("records/notes/test.json", val);

        let loaded = load_note(&store, "records/notes/test.json").unwrap();
        assert_eq!(loaded.instance_id, "test-123");
        assert_eq!(loaded.title, Some("Test Note".to_string()));
    }

    #[test]
    fn load_note_validates_on_read() {
        let store = MemoryStore::default();
        // Duplicate section names → validation error
        let invalid = serde_json::json!({
            "instanceId": "test-123",
            "sections": [
                {"name": "section1", "content": "content1"},
                {"name": "section1", "content": "content2"}
            ]
        });
        let store = store.with_data("records/notes/bad.json", invalid);

        let result = load_note(&store, "records/notes/bad.json");
        assert!(matches!(
            result,
            Err(RepositoryError::NoteValidation { .. })
        ));
    }
}
