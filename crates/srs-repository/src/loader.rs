use crate::error::RepositoryError;
use srs_core::types::note::Note;
use srs_core::types::tag_definition::TagDefinition;
use srs_core::validation::note::validate_note;
use srs_core::validation::tag_definition::validate_tag_definition;
use std::path::Path;

/// Load a Note from a path, validating it after deserialization.
pub fn load_note(path: &Path) -> Result<Note, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|e| RepositoryError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let note: Note = serde_json::from_str(&content).map_err(|e| RepositoryError::NoteLoad {
        path: path.to_path_buf(),
        source: e,
    })?;

    validate_note(&note).map_err(|e| RepositoryError::NoteValidation {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(note)
}

/// Load a Note from a relative path within a repo.
pub fn load_note_relative(repo_root: &Path, relative_path: &str) -> Result<Note, RepositoryError> {
    let path = repo_root.join(relative_path);
    load_note(&path)
}

/// Load a TagDefinition from a path, validating it after deserialization.
pub fn load_tag_definition(path: &Path) -> Result<TagDefinition, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|e| RepositoryError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let td: TagDefinition =
        serde_json::from_str(&content).map_err(|e| RepositoryError::TagDefinitionLoad {
            path: path.to_path_buf(),
            source: e,
        })?;

    validate_tag_definition(&td).map_err(|e| RepositoryError::TagDefinitionValidation {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(td)
}

/// Load a TagDefinition from a relative path within a repo.
pub fn load_tag_definition_relative(
    repo_root: &Path,
    relative_path: &str,
) -> Result<TagDefinition, RepositoryError> {
    let path = repo_root.join(relative_path);
    load_tag_definition(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn origin_purpose_loads_with_correct_id_and_sections() {
        let path =
            Path::new("/home/greenman/dev/semanticops/srs/srs/records/notes/origin-purpose.json");
        let note = load_note(path).unwrap();
        assert_eq!(note.instance_id, "d5c7e536-5f7d-491a-8166-5ee25a954377");
        assert_eq!(note.sections.len(), 6);
    }

    #[test]
    fn load_validates_on_read() {
        let temp = TempDir::new().unwrap();
        let note_path = temp.path().join("invalid-note.json");

        // Create a note with duplicate section names
        let json = r#"{
            "instanceId": "test-123",
            "sections": [
                {"name": "section1", "content": "content1"},
                {"name": "section1", "content": "content2"}
            ]
        }"#;

        fs::write(&note_path, json).unwrap();

        let result = load_note(&note_path);
        assert!(matches!(
            result,
            Err(RepositoryError::NoteValidation { .. })
        ));
    }
}
