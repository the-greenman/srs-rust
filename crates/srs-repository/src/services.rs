use crate::error::RepositoryError;
use crate::loader::load_note_relative;
use crate::manifest::{load_manifest, Manifest};
use crate::writer::{new_instance_id, upsert_index_entry, write_manifest, write_note};
use serde::{Deserialize, Serialize};
use srs_core::types::note::Note;
use srs_core::validation::note::validate_note;
use std::path::Path;

/// Summary of a note for list operations
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    pub instance_id: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

/// Filter options for listing notes
#[derive(Debug, Clone, Default)]
pub struct ListNotesFilter {
    pub tag: Option<String>,
}

/// Result of listing notes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListNotesResult {
    pub notes: Vec<NoteSummary>,
}

/// Result of getting a note
#[derive(Debug, Clone)]
pub enum GetNoteResult {
    Found(Box<Note>),
    NotFound,
    NotANote { tier: u8 },
}

/// Result of creating a note
#[derive(Debug, Clone)]
pub struct CreateNoteResult {
    pub note: Note,
}

/// Result of adding a tag
#[derive(Debug, Clone)]
pub enum AddTagResult {
    Added { note: Note, tag: String },
    AlreadyPresent { note: Note, tag: String },
    NotFound,
}

/// Result of removing a tag
#[derive(Debug, Clone)]
pub enum RemoveTagResult {
    Removed { note: Note, tag: String },
    NotPresent { note: Note, tag: String },
    NotFound,
}

/// Result of updating a note
#[derive(Debug, Clone)]
pub struct UpdateNoteResult {
    pub note: Note,
}

/// Result of deleting a note
#[derive(Debug, Clone)]
pub struct DeleteNoteResult {
    pub instance_id: String,
}

/// Service: List notes with optional tag filter
pub fn list_notes(
    repo_root: &Path,
    filter: ListNotesFilter,
) -> Result<ListNotesResult, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    list_notes_from_manifest(repo_root, &manifest, filter)
}

fn list_notes_from_manifest(
    repo_root: &Path,
    manifest: &Manifest,
    filter: ListNotesFilter,
) -> Result<ListNotesResult, RepositoryError> {
    let mut notes = Vec::new();

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }

        let path = entry.path();

        // If filtering by tag, load note to check tags
        if let Some(ref filter_tag) = filter.tag {
            match load_note_relative(repo_root, path) {
                Ok(note) => {
                    let has_tag = note
                        .tags
                        .as_ref()
                        .is_some_and(|tags| tags.contains(filter_tag));
                    if !has_tag {
                        continue;
                    }
                    notes.push(NoteSummary {
                        instance_id: entry.instance_id().to_string(),
                        title: entry.title(),
                        tags: note.tags.unwrap_or_default(),
                    });
                }
                Err(_) => continue,
            }
        } else {
            // No filter, include all notes (just from manifest for efficiency)
            notes.push(NoteSummary {
                instance_id: entry.instance_id().to_string(),
                title: entry.title(),
                tags: Vec::new(), // Tags not loaded for efficiency
            });
        }
    }

    Ok(ListNotesResult { notes })
}

/// Service: Get a note by its instance ID
pub fn get_note_by_id(repo_root: &Path, id: &str) -> Result<GetNoteResult, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    get_note_by_id_from_manifest(repo_root, &manifest, id)
}

fn get_note_by_id_from_manifest(
    repo_root: &Path,
    manifest: &Manifest,
    id: &str,
) -> Result<GetNoteResult, RepositoryError> {
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id);

    match entry {
        Some(e) => {
            if !e.is_note() {
                return Ok(GetNoteResult::NotANote { tier: e.tier() });
            }
            let note = load_note_relative(repo_root, e.path())?;
            Ok(GetNoteResult::Found(Box::new(note)))
        }
        None => Ok(GetNoteResult::NotFound),
    }
}

/// Service: Create a new note
pub fn create_note(repo_root: &Path, mut note: Note) -> Result<CreateNoteResult, RepositoryError> {
    // Mint instance_id if absent
    if note.instance_id.is_empty() {
        note.instance_id = new_instance_id();
    }

    // Validate the note
    validate_note(&note).map_err(|e| RepositoryError::NoteValidation {
        path: repo_root.join("<create>"),
        source: e,
    })?;

    // Determine path: records/notes/<slug>.json
    let slug = note
        .title
        .as_ref()
        .map(|t| slugify_title(t))
        .unwrap_or_else(|| note.instance_id.clone());
    let relative_path = format!("records/notes/{}.json", slug);
    let full_path = repo_root.join(&relative_path);

    // Ensure directory exists
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| RepositoryError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    // Write the note
    write_note(&note, &full_path)?;

    // Update manifest
    let mut manifest = load_manifest(repo_root)?;
    upsert_index_entry(&mut manifest, &note, &relative_path);
    write_manifest(&manifest)?;

    Ok(CreateNoteResult { note })
}

/// Service: Add a tag to a note
pub fn add_note_tag(
    repo_root: &Path,
    id: &str,
    tag: &str,
) -> Result<AddTagResult, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Find the note in the manifest
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id)
        .cloned();

    match entry {
        Some(e) => {
            let mut note = load_note_relative(repo_root, e.path())?;

            // Add tag if not already present
            let tags = note.tags.get_or_insert_with(Vec::new);
            if tags.contains(&tag.to_string()) {
                return Ok(AddTagResult::AlreadyPresent {
                    note,
                    tag: tag.to_string(),
                });
            }
            tags.push(tag.to_string());

            // Write back
            let full_path = repo_root.join(e.path());
            write_note(&note, &full_path)?;

            // Update manifest to reflect new tags (reusing the loaded manifest)
            upsert_index_entry(&mut manifest, &note, e.path());
            write_manifest(&manifest)?;

            Ok(AddTagResult::Added {
                note,
                tag: tag.to_string(),
            })
        }
        None => Ok(AddTagResult::NotFound),
    }
}

/// Service: Remove a tag from a note
pub fn remove_note_tag(
    repo_root: &Path,
    id: &str,
    tag: &str,
) -> Result<RemoveTagResult, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Find the note in the manifest
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id)
        .cloned();

    match entry {
        Some(e) => {
            let mut note = load_note_relative(repo_root, e.path())?;

            // Remove tag if present
            let tags = note.tags.get_or_insert_with(Vec::new);
            if !tags.contains(&tag.to_string()) {
                return Ok(RemoveTagResult::NotPresent {
                    note,
                    tag: tag.to_string(),
                });
            }
            tags.retain(|t| t != tag);
            if tags.is_empty() {
                note.tags = None;
            }

            // Write back
            let full_path = repo_root.join(e.path());
            write_note(&note, &full_path)?;

            // Update manifest to reflect new tags
            upsert_index_entry(&mut manifest, &note, e.path());
            write_manifest(&manifest)?;

            Ok(RemoveTagResult::Removed {
                note,
                tag: tag.to_string(),
            })
        }
        None => Ok(RemoveTagResult::NotFound),
    }
}

/// Service: Update an existing note
/// Accepts a full Note object, replaces the stored note, and updates manifest if title changed
pub fn update_note(repo_root: &Path, note: Note) -> Result<UpdateNoteResult, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Find the note in the manifest
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == note.instance_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NoteNotFound {
            path: repo_root.join("records/notes"),
            id: note.instance_id.clone(),
        })?;

    // Validate the note
    validate_note(&note).map_err(|e| RepositoryError::NoteValidation {
        path: repo_root.join(entry.path()),
        source: e,
    })?;

    // Write the note
    let full_path = repo_root.join(entry.path());
    write_note(&note, &full_path)?;

    // Update manifest if title changed
    upsert_index_entry(&mut manifest, &note, entry.path());
    write_manifest(&manifest)?;

    Ok(UpdateNoteResult { note })
}

/// Service: Delete a note by ID
/// Removes the file and updates the manifest
pub fn delete_note(repo_root: &Path, id: &str) -> Result<DeleteNoteResult, RepositoryError> {
    let mut manifest = load_manifest(repo_root)?;

    // Find the note in the manifest
    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == id && e.is_note())
        .ok_or_else(|| RepositoryError::NoteNotFound {
            path: repo_root.join("records/notes"),
            id: id.to_string(),
        })?;

    let entry = manifest.instance_index[entry_index].clone();
    let path = entry.path().to_string();

    // Remove the file
    let full_path = repo_root.join(&path);
    if full_path.exists() {
        std::fs::remove_file(&full_path).map_err(|e| RepositoryError::Io {
            path: full_path.clone(),
            source: e,
        })?;
    }

    // Remove from manifest
    manifest.instance_index.remove(entry_index);
    write_manifest(&manifest)?;

    Ok(DeleteNoteResult {
        instance_id: id.to_string(),
    })
}

/// Library-owned slugification for note paths
pub fn slugify_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    fn create_temp_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".srs")).unwrap();
        std::fs::write(
            temp.path().join("manifest.json"),
            json!({
                "instanceIndex": [
                    {
                        "instanceId": "11111111-1111-1111-8111-111111111111",
                        "tier": 0,
                        "path": "records/notes/test-note.json",
                        "title": "Test Note"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("records/notes")).unwrap();
        std::fs::write(
            temp.path().join("records/notes/test-note.json"),
            json!({
                "instanceId": "11111111-1111-1111-8111-111111111111",
                "title": "Test Note",
                "tags": ["test", "sample"],
                "sections": [{"name": "test", "content": "test content"}]
            })
            .to_string(),
        )
        .unwrap();
        temp
    }

    #[test]
    fn slugify_handles_punctuation_and_collapse() {
        assert_eq!(
            slugify_title("AI-Native SRS Repositories"),
            "ai-native-srs-repositories"
        );
        assert_eq!(slugify_title("Meaning: AI + Humans"), "meaning-ai-humans");
        assert_eq!(slugify_title("  spaces  "), "spaces");
    }

    #[test]
    fn list_notes_returns_all_notes() {
        let temp = create_temp_repo();
        let result = list_notes(temp.path(), ListNotesFilter::default()).unwrap();
        assert_eq!(result.notes.len(), 1);
        assert_eq!(
            result.notes[0].instance_id,
            "11111111-1111-1111-8111-111111111111"
        );
    }

    #[test]
    fn list_notes_filters_by_tag() {
        let temp = create_temp_repo();
        let result = list_notes(
            temp.path(),
            ListNotesFilter {
                tag: Some("test".to_string()),
            },
        )
        .unwrap();
        assert_eq!(result.notes.len(), 1);

        let result = list_notes(
            temp.path(),
            ListNotesFilter {
                tag: Some("nonexistent".to_string()),
            },
        )
        .unwrap();
        assert_eq!(result.notes.len(), 0);
    }

    #[test]
    fn get_note_by_id_finds_note() {
        let temp = create_temp_repo();
        let result = get_note_by_id(temp.path(), "11111111-1111-1111-8111-111111111111").unwrap();
        match result {
            GetNoteResult::Found(note) => {
                assert_eq!(note.title, Some("Test Note".to_string()));
            }
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn get_note_by_id_returns_not_found() {
        let temp = create_temp_repo();
        let result = get_note_by_id(temp.path(), "nonexistent-id").unwrap();
        match result {
            GetNoteResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn get_note_by_id_refuses_non_note() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".srs")).unwrap();
        std::fs::write(
            temp.path().join("manifest.json"),
            json!({
                "instanceIndex": [
                    {
                        "instanceId": "22222222-2222-2222-8222-222222222222",
                        "tier": 1,
                        "path": "specs/spec.json"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let result = get_note_by_id(temp.path(), "22222222-2222-2222-8222-222222222222").unwrap();
        match result {
            GetNoteResult::NotANote { tier } => assert_eq!(tier, 1),
            _ => panic!("Expected NotANote"),
        }
    }

    #[test]
    fn create_note_mints_id_and_creates_file() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".srs")).unwrap();
        std::fs::write(
            temp.path().join("manifest.json"),
            json!({ "instanceIndex": [] }).to_string(),
        )
        .unwrap();

        let note = Note {
            instance_id: "".to_string(),
            title: Some("My New Note".to_string()),
            tags: None,
            sections: vec![srs_core::types::note::NoteSection {
                name: "intro".to_string(),
                label: None,
                content: "Hello world".to_string(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };

        let result = create_note(temp.path(), note).unwrap();
        assert!(!result.note.instance_id.is_empty());

        // Verify file exists at the expected path
        assert!(temp.path().join("records/notes/my-new-note.json").exists());

        // Verify manifest updated
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let index = manifest["instanceIndex"].as_array().unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0]["instanceId"], result.note.instance_id);
    }

    #[test]
    fn add_note_tag_adds_and_updates_manifest() {
        let temp = create_temp_repo();

        let result = add_note_tag(
            temp.path(),
            "11111111-1111-1111-8111-111111111111",
            "new-tag",
        )
        .unwrap();

        match result {
            AddTagResult::Added { note, tag } => {
                assert_eq!(tag, "new-tag");
                assert!(note.tags.as_ref().unwrap().contains(&"new-tag".to_string()));
                assert!(note.tags.as_ref().unwrap().contains(&"test".to_string()));
            }
            _ => panic!("Expected Added"),
        }

        // Verify manifest was updated
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let index = manifest["instanceIndex"].as_array().unwrap();
        let tags = index[0]["tags"].as_array().unwrap();
        assert!(tags.iter().any(|t| t.as_str() == Some("new-tag")));
    }

    #[test]
    fn add_note_tag_returns_already_present() {
        let temp = create_temp_repo();

        let result = add_note_tag(
            temp.path(),
            "11111111-1111-1111-8111-111111111111",
            "test", // already present
        )
        .unwrap();

        match result {
            AddTagResult::AlreadyPresent { .. } => {}
            _ => panic!("Expected AlreadyPresent"),
        }
    }

    #[test]
    fn add_note_tag_returns_not_found() {
        let temp = create_temp_repo();

        let result = add_note_tag(temp.path(), "nonexistent-id", "tag").unwrap();
        match result {
            AddTagResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    // Acceptance Criteria Tests for Phase 2

    #[test]
    fn note_update_rewrites_file_and_manifest_title() {
        let temp = create_temp_repo();

        // Get the existing note
        let existing =
            match get_note_by_id(temp.path(), "11111111-1111-1111-8111-111111111111").unwrap() {
                GetNoteResult::Found(n) => n,
                _ => panic!("Should find note"),
            };

        // Update the note
        let updated_note = Note {
            instance_id: existing.instance_id.clone(),
            title: Some("Updated Title".to_string()),
            tags: existing.tags.clone(),
            sections: existing.sections.clone(),
            graduated_at: existing.graduated_at.clone(),
            source_refs: existing.source_refs.clone(),
            meta: existing.meta.clone(),
            created_at: existing.created_at.clone(),
            updated_at: Some("2026-01-02T00:00:00Z".to_string()),
        };

        let result = update_note(temp.path(), updated_note).unwrap();
        assert_eq!(result.note.title, Some("Updated Title".to_string()));

        // Verify file was rewritten
        let file_content =
            std::fs::read_to_string(temp.path().join("records/notes/test-note.json")).unwrap();
        let file_note: Note = serde_json::from_str(&file_content).unwrap();
        assert_eq!(file_note.title, Some("Updated Title".to_string()));

        // Verify manifest was updated with new title
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let index = manifest["instanceIndex"].as_array().unwrap();
        assert_eq!(index[0]["title"], "Updated Title");
    }

    #[test]
    fn note_delete_removes_file_and_manifest_entry() {
        let temp = create_temp_repo();

        let result = delete_note(temp.path(), "11111111-1111-1111-8111-111111111111").unwrap();
        assert_eq!(result.instance_id, "11111111-1111-1111-8111-111111111111");

        // Verify file was removed
        assert!(!temp.path().join("records/notes/test-note.json").exists());

        // Verify manifest entry was removed
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let index = manifest["instanceIndex"].as_array().unwrap();
        assert!(index.is_empty());
    }

    #[test]
    fn note_tag_remove_updates_note() {
        let temp = create_temp_repo();

        // First add a tag we can remove
        add_note_tag(
            temp.path(),
            "11111111-1111-1111-8111-111111111111",
            "removable-tag",
        )
        .unwrap();

        // Now remove it
        let result = remove_note_tag(
            temp.path(),
            "11111111-1111-1111-8111-111111111111",
            "removable-tag",
        )
        .unwrap();

        match result {
            RemoveTagResult::Removed { note, tag } => {
                assert_eq!(tag, "removable-tag");
                assert!(!note
                    .tags
                    .as_ref()
                    .unwrap()
                    .contains(&"removable-tag".to_string()));
            }
            _ => panic!("Expected Removed"),
        }

        // Verify file was updated
        let file_content =
            std::fs::read_to_string(temp.path().join("records/notes/test-note.json")).unwrap();
        let file_note: Note = serde_json::from_str(&file_content).unwrap();
        assert!(!file_note
            .tags
            .as_ref()
            .unwrap()
            .contains(&"removable-tag".to_string()));
    }
}
