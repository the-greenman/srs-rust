//! # Note Service
//!
//! Public API for note operations. This module is the sole entry point for
//! all note logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, container orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: CreateNoteInput = serde_json::from_reader(io::stdin())?;
//! let result = note_service::create_note(store, input)?;
//! output::ok("note create", result)
//! ```

use crate::container_service;
use crate::error::RepositoryError;
use crate::loader::load_note;
use crate::paths::NOTES_RECORD_DIR;
use crate::record_store::{create_record_in_context, CreateRecordInput};
use crate::store::RepositoryStore;
use crate::writer::{
    new_instance_id, slugify_instance_name, upsert_index_entry, write_manifest, write_note,
};
use serde::{Deserialize, Serialize};
use srs_core::types::note::Note;
use srs_core::types::record::Record;
use srs_core::validation::note::validate_note;
use srs_schema::{SchemaRegistry, NOTE_SCHEMA_ID};

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
    /// If Some, only return notes that are members of this container.
    pub container_id: Option<String>,
}

/// Container-aware create input wrapping the existing Note
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteInput {
    #[serde(flatten)]
    pub note: Note,
    pub container_id: Option<String>,
}

/// Explicit delete input with optional container scoping
#[derive(Debug)]
pub struct DeleteNoteInput {
    pub id: String,
    pub container_id: Option<String>,
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

/// Summary of a tag across all tier-0 notes (note-level only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSummary {
    pub tag: String,
    pub note_count: usize,
}

/// Result of listing note tags
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListNoteTagsResult {
    pub total_notes: usize,
    pub tags: Vec<TagSummary>,
}

/// Service: List notes with optional tag and container filter
pub fn list_notes(
    store: &dyn RepositoryStore,
    filter: ListNotesFilter,
) -> Result<ListNotesResult, RepositoryError> {
    // Resolve container members once if a container filter is set.
    let member_ids: Option<std::collections::HashSet<String>> =
        if let Some(ref cid) = filter.container_id {
            let members = container_service::list_members(store, cid)?;
            Some(members.into_iter().collect())
        } else {
            None
        };

    let manifest = store.load_manifest()?;
    let mut notes = Vec::new();

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }

        // Container membership filter
        if let Some(ref member_set) = member_ids {
            if !member_set.contains(entry.instance_id()) {
                continue;
            }
        }

        if let Some(ref filter_tag) = filter.tag {
            match load_note(store, entry.path()) {
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
            notes.push(NoteSummary {
                instance_id: entry.instance_id().to_string(),
                title: entry.title(),
                tags: Vec::new(),
            });
        }
    }

    Ok(ListNotesResult { notes })
}

/// Service: List distinct tags used across tier-0 notes, with per-tag note counts.
///
/// Uses the manifest index (note-level tags only — section tags are not cached in the index).
/// No note files are loaded. Tags are returned sorted alphabetically.
/// Container scoping is respected when `container_id` is `Some`.
pub fn list_note_tags(
    store: &dyn RepositoryStore,
    container_id: Option<&str>,
) -> Result<ListNoteTagsResult, RepositoryError> {
    let member_ids: Option<std::collections::HashSet<String>> = if let Some(cid) = container_id {
        let members = container_service::list_members(store, cid)?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    let manifest = store.load_manifest()?;
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut total_notes = 0;

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }
        if let Some(ref m) = member_ids {
            if !m.contains(entry.instance_id()) {
                continue;
            }
        }
        total_notes += 1;
        for tag in entry.tags.iter().flatten() {
            *counts.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    let tags = counts
        .into_iter()
        .map(|(tag, note_count)| TagSummary { tag, note_count })
        .collect();

    Ok(ListNoteTagsResult { total_notes, tags })
}

/// Service: Create a note and optionally add it to a container atomically.
///
/// If `input.container_id` is Some, the container is verified to exist before
/// creating the note. If creation succeeds, the note is added to the container.
pub fn create_note_in_context(
    store: &dyn RepositoryStore,
    input: CreateNoteInput,
) -> Result<CreateNoteResult, RepositoryError> {
    if let Some(ref cid) = input.container_id {
        // Validate container exists before writing anything
        container_service::get_container(store, cid)?;
    }

    let result = create_note(store, input.note)?;

    if let Some(ref cid) = input.container_id {
        container_service::add_member(store, cid, &result.note.instance_id)?;
    }

    Ok(result)
}

/// Service: Delete a note with optional container-scoped membership check.
///
/// If `input.container_id` is Some, the note must be a member of that container.
/// The membership is removed before the note is deleted.
pub fn delete_note_in_context(
    store: &dyn RepositoryStore,
    input: DeleteNoteInput,
) -> Result<DeleteNoteResult, RepositoryError> {
    if let Some(ref cid) = input.container_id {
        if !container_service::is_member(store, cid, &input.id)? {
            return Err(RepositoryError::NotFound {
                path: std::path::PathBuf::from(format!(
                    "Instance '{}' is not a member of container '{}'",
                    input.id, cid
                )),
            });
        }
        container_service::remove_member(store, cid, &input.id)?;
    }

    delete_note(store, &input.id)
}

/// Service: Update a note after validating that the ID in the body matches
/// the provided URL/command ID.
///
/// Moves the ID-mismatch check from CLI handlers into the service layer.
pub fn update_note_validated(
    store: &dyn RepositoryStore,
    id: &str,
    note: Note,
) -> Result<UpdateNoteResult, RepositoryError> {
    if note.instance_id != id {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "Note ID in body ({}) does not match path ID ({})",
                note.instance_id, id
            ),
        });
    }
    update_note(store, note)
}

/// Input for `graduate_note`.
#[derive(Debug)]
pub struct GraduateNoteInput {
    /// The instance ID of the Tier-0 Note to graduate.
    pub note_id: String,
    /// Type reference in "namespace/name" format (passed to `create_record_in_context`).
    pub type_ref: String,
    /// Optional version pin for the type. None → latest.
    pub type_version: Option<u32>,
    /// Field values, group values, and tags for the new Record.
    pub record_input: CreateRecordInput,
    /// If Some, the new Record is added to this container after creation.
    pub container_id: Option<String>,
}

/// Result of `graduate_note`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraduateNoteResult {
    /// The Note with `graduated_at` stamped (ISO-8601 UTC timestamp).
    pub note: Note,
    /// The newly created typed Record.
    pub record: Record,
}

/// Promote a Tier-0 Note to a typed Tier-2 Record in one atomic step:
/// creates the Record, stamps `graduated_at` on the Note, and returns both.
pub fn graduate_note(
    store: &dyn RepositoryStore,
    input: GraduateNoteInput,
) -> Result<GraduateNoteResult, RepositoryError> {
    // Step 1: load and validate the note exists and is a Note (tier 0)
    let mut note = match get_note_by_id(store, &input.note_id)? {
        GetNoteResult::Found(n) => *n,
        GetNoteResult::NotFound => {
            return Err(RepositoryError::NoteNotFound {
                path: std::path::PathBuf::new(),
                id: input.note_id.clone(),
            })
        }
        GetNoteResult::NotANote { tier } => {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "Instance '{}' is not a Note (tier {}); cannot graduate",
                    input.note_id, tier
                ),
            })
        }
    };

    // Step 2: guard against re-graduation (graduation is a one-way promotion)
    if note.graduated_at.is_some() {
        return Err(RepositoryError::InvalidInput {
            message: format!(
                "Note '{}' is already graduated; cannot graduate again",
                input.note_id
            ),
        });
    }

    // Step 3: create the typed Record
    let create_result = create_record_in_context(
        store,
        &input.type_ref,
        input.type_version,
        input.record_input,
        input.container_id,
        None,
    )?;

    // Step 4: stamp graduated_at and write the note back
    note.graduated_at = Some(chrono::Utc::now().to_rfc3339());
    let update_result = update_note(store, note)?;

    Ok(GraduateNoteResult {
        note: update_result.note,
        record: create_result.record,
    })
}

/// Service: Get a note by its instance ID
pub fn get_note_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetNoteResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id);

    match entry {
        Some(e) => {
            if !e.is_note() {
                return Ok(GetNoteResult::NotANote { tier: e.tier() });
            }
            let note = load_note(store, e.path())?;
            Ok(GetNoteResult::Found(Box::new(note)))
        }
        None => Ok(GetNoteResult::NotFound),
    }
}

/// Service: Create a new note
pub fn create_note(
    store: &dyn RepositoryStore,
    mut note: Note,
) -> Result<CreateNoteResult, RepositoryError> {
    if note.instance_id.is_empty() {
        note.instance_id = new_instance_id();
    }

    // Schema validation before core validation
    let raw = serde_json::to_value(&note).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from("<stdin>"),
        source: e,
    })?;
    SchemaRegistry::global()
        .validate_by_id(NOTE_SCHEMA_ID, &raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from("<stdin>"),
            message: e.to_string(),
        })?;

    validate_note(&note).map_err(|e| RepositoryError::NoteValidation {
        path: std::path::PathBuf::from("<create>"),
        source: e,
    })?;

    let slug = note
        .title
        .as_ref()
        .map(|t| slugify_instance_name(t))
        .unwrap_or_default();
    let id8 = &note.instance_id[..8];
    let relative_path = if slug.is_empty() {
        format!("{NOTES_RECORD_DIR}/{id8}.json")
    } else {
        format!("{NOTES_RECORD_DIR}/{slug}-{id8}.json")
    };

    store.ensure_instance_dir(NOTES_RECORD_DIR)?;
    write_note(store, &note, &relative_path)?;

    let mut manifest = store.load_manifest()?;
    upsert_index_entry(&mut manifest, &note, &relative_path);
    write_manifest(store, &manifest)?;

    Ok(CreateNoteResult { note })
}

/// Service: Add a tag to a note
pub fn add_note_tag(
    store: &dyn RepositoryStore,
    id: &str,
    tag: &str,
) -> Result<AddTagResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id)
        .cloned();

    match entry {
        Some(e) => {
            let mut note = load_note(store, e.path())?;

            let tags = note.tags.get_or_insert_with(Vec::new);
            if tags.contains(&tag.to_string()) {
                return Ok(AddTagResult::AlreadyPresent {
                    note,
                    tag: tag.to_string(),
                });
            }
            tags.push(tag.to_string());

            write_note(store, &note, e.path())?;
            upsert_index_entry(&mut manifest, &note, e.path());
            write_manifest(store, &manifest)?;

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
    store: &dyn RepositoryStore,
    id: &str,
    tag: &str,
) -> Result<RemoveTagResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id)
        .cloned();

    match entry {
        Some(e) => {
            let mut note = load_note(store, e.path())?;

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

            write_note(store, &note, e.path())?;
            upsert_index_entry(&mut manifest, &note, e.path());
            write_manifest(store, &manifest)?;

            Ok(RemoveTagResult::Removed {
                note,
                tag: tag.to_string(),
            })
        }
        None => Ok(RemoveTagResult::NotFound),
    }
}

/// Service: Update an existing note
pub fn update_note(
    store: &dyn RepositoryStore,
    note: Note,
) -> Result<UpdateNoteResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == note.instance_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NoteNotFound {
            path: std::path::PathBuf::from(NOTES_RECORD_DIR),
            id: note.instance_id.clone(),
        })?;

    // Schema validation before core validation
    let raw = serde_json::to_value(&note).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(entry.path()),
        source: e,
    })?;
    SchemaRegistry::global()
        .validate_by_id(NOTE_SCHEMA_ID, &raw)
        .map_err(|e| RepositoryError::SchemaValidation {
            path: std::path::PathBuf::from(entry.path()),
            message: e.to_string(),
        })?;

    validate_note(&note).map_err(|e| RepositoryError::NoteValidation {
        path: std::path::PathBuf::from(entry.path()),
        source: e,
    })?;

    write_note(store, &note, entry.path())?;
    upsert_index_entry(&mut manifest, &note, entry.path());
    write_manifest(store, &manifest)?;

    Ok(UpdateNoteResult { note })
}

/// Service: Delete a note by ID
pub fn delete_note(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<DeleteNoteResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == id && e.is_note())
        .ok_or_else(|| RepositoryError::NoteNotFound {
            path: std::path::PathBuf::from(NOTES_RECORD_DIR),
            id: id.to_string(),
        })?;

    let path = manifest.instance_index[entry_index].path().to_string();

    store.delete_instance_file(&path)?;
    manifest.instance_index.remove(entry_index);
    write_manifest(store, &manifest)?;

    Ok(DeleteNoteResult {
        instance_id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::InstanceIndexEntry;
    use crate::manifest::Manifest;
    use crate::store::memory::MemoryStore;
    use srs_core::types::note::NoteSection;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_note(id: &str, title: &str) -> Note {
        Note {
            instance_id: id.to_string(),
            title: Some(title.to_string()),
            tags: Some(vec!["test".to_string(), "sample".to_string()]),
            sections: vec![NoteSection {
                name: "body".to_string(),
                label: None,
                content: "content".to_string(),
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

    /// Build a MemoryStore pre-loaded with one note and the matching manifest entry.
    fn store_with_note(note: &Note, path: &str) -> MemoryStore {
        let note_val = {
            let mut v = serde_json::to_value(note).unwrap();
            v.as_object_mut().unwrap().insert(
                "$schema".to_string(),
                serde_json::json!("https://srs.semanticops.com/schema/2.0/note.json"),
            );
            v
        };
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: note.instance_id.clone(),
                tier: 0,
                path: path.to_string(),
                title: note.title.clone().map(serde_json::Value::String),
                tags: note.tags.clone(),
            }],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            },
        )
        .with_data(path, note_val)
    }

    fn store_with_two_notes(
        note_a: &Note,
        path_a: &str,
        note_b: &Note,
        path_b: &str,
    ) -> MemoryStore {
        let make_val = |n: &Note| {
            let mut v = serde_json::to_value(n).unwrap();
            v.as_object_mut().unwrap().insert(
                "$schema".to_string(),
                serde_json::json!("https://srs.semanticops.com/schema/2.0/note.json"),
            );
            v
        };
        let manifest = Manifest {
            instance_index: vec![
                InstanceIndexEntry {
                    instance_id: note_a.instance_id.clone(),
                    tier: 0,
                    path: path_a.to_string(),
                    title: note_a.title.clone().map(serde_json::Value::String),
                    tags: note_a.tags.clone(),
                },
                InstanceIndexEntry {
                    instance_id: note_b.instance_id.clone(),
                    tier: 0,
                    path: path_b.to_string(),
                    title: note_b.title.clone().map(serde_json::Value::String),
                    tags: note_b.tags.clone(),
                },
            ],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            },
        )
        .with_data(path_a, make_val(note_a))
        .with_data(path_b, make_val(note_b))
    }

    #[test]
    fn list_note_tags_empty_store() {
        let store = MemoryStore::default();
        let result = list_note_tags(&store, None).unwrap();
        assert_eq!(result.total_notes, 0);
        assert!(result.tags.is_empty());
    }

    #[test]
    fn list_note_tags_single_note() {
        let note = make_note("11111111-1111-4111-8111-111111111111", "Test");
        let store = store_with_note(&note, "records/notes/test.json");
        let result = list_note_tags(&store, None).unwrap();
        assert_eq!(result.total_notes, 1);
        // make_note sets tags ["test", "sample"] — sorted alphabetically
        assert_eq!(result.tags.len(), 2);
        assert_eq!(result.tags[0].tag, "sample");
        assert_eq!(result.tags[0].note_count, 1);
        assert_eq!(result.tags[1].tag, "test");
        assert_eq!(result.tags[1].note_count, 1);
    }

    #[test]
    fn list_note_tags_shared_tag_counts_correctly() {
        let mut note_a = make_note("aaaaaaaa-aaaa-4aaa-aaaa-aaaaaaaaaaaa", "Alpha");
        note_a.tags = Some(vec!["shared".to_string(), "only-a".to_string()]);
        let mut note_b = make_note("bbbbbbbb-bbbb-4bbb-bbbb-bbbbbbbbbbbb", "Beta");
        note_b.tags = Some(vec!["shared".to_string(), "only-b".to_string()]);

        let store = store_with_two_notes(
            &note_a,
            "records/notes/alpha.json",
            &note_b,
            "records/notes/beta.json",
        );
        let result = list_note_tags(&store, None).unwrap();
        assert_eq!(result.total_notes, 2);
        let shared = result.tags.iter().find(|t| t.tag == "shared").unwrap();
        assert_eq!(shared.note_count, 2);
        let only_a = result.tags.iter().find(|t| t.tag == "only-a").unwrap();
        assert_eq!(only_a.note_count, 1);
    }

    #[test]
    fn list_note_tags_note_with_no_tags_counted_in_total() {
        let mut note = make_note("11111111-1111-4111-8111-111111111111", "Untagged");
        note.tags = None;
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: note.instance_id.clone(),
                tier: 0,
                path: "records/notes/untagged.json".to_string(),
                title: note.title.clone().map(serde_json::Value::String),
                tags: None,
            }],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            },
        );
        let result = list_note_tags(&store, None).unwrap();
        assert_eq!(result.total_notes, 1);
        assert!(result.tags.is_empty());
    }

    #[test]
    fn list_note_tags_non_note_entries_excluded() {
        let note = make_note("11111111-1111-4111-8111-111111111111", "Note");
        let manifest = Manifest {
            instance_index: vec![
                InstanceIndexEntry {
                    instance_id: note.instance_id.clone(),
                    tier: 0,
                    path: "records/notes/note.json".to_string(),
                    title: note.title.clone().map(serde_json::Value::String),
                    tags: note.tags.clone(),
                },
                InstanceIndexEntry {
                    instance_id: "22222222-2222-4222-8222-222222222222".to_string(),
                    tier: 1,
                    path: "records/typed.json".to_string(),
                    title: None,
                    tags: Some(vec!["not-counted".to_string()]),
                },
            ],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            },
        );
        let result = list_note_tags(&store, None).unwrap();
        assert_eq!(result.total_notes, 1);
        assert!(!result.tags.iter().any(|t| t.tag == "not-counted"));
    }

    #[test]
    fn slugify_handles_punctuation_and_collapse() {
        use crate::writer::slugify_instance_name;
        assert_eq!(
            slugify_instance_name("AI-Native SRS Repositories"),
            "ai-native-srs-repositories"
        );
        assert_eq!(
            slugify_instance_name("Meaning: AI + Humans"),
            "meaning-ai-humans"
        );
        assert_eq!(slugify_instance_name("  spaces  "), "spaces");
    }

    #[test]
    fn list_notes_returns_all_notes() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");
        let result = list_notes(&store, ListNotesFilter::default()).unwrap();
        assert_eq!(result.notes.len(), 1);
        assert_eq!(
            result.notes[0].instance_id,
            "11111111-1111-1111-8111-111111111111"
        );
    }

    #[test]
    fn list_notes_filters_by_tag() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");

        let result = list_notes(
            &store,
            ListNotesFilter {
                tag: Some("test".to_string()),
                container_id: None,
            },
        )
        .unwrap();
        assert_eq!(result.notes.len(), 1);

        let result = list_notes(
            &store,
            ListNotesFilter {
                tag: Some("nonexistent".to_string()),
                container_id: None,
            },
        )
        .unwrap();
        assert_eq!(result.notes.len(), 0);
    }

    #[test]
    fn get_note_by_id_finds_note() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");
        let result = get_note_by_id(&store, "11111111-1111-1111-8111-111111111111").unwrap();
        match result {
            GetNoteResult::Found(n) => assert_eq!(n.title, Some("Test Note".to_string())),
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn get_note_by_id_returns_not_found() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");
        let result = get_note_by_id(&store, "nonexistent-id").unwrap();
        match result {
            GetNoteResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn get_note_by_id_refuses_non_note() {
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: "22222222-2222-2222-8222-222222222222".to_string(),
                tier: 1,
                path: "specs/spec.json".to_string(),
                title: None,
                tags: None,
            }],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            },
        );
        let result = get_note_by_id(&store, "22222222-2222-2222-8222-222222222222").unwrap();
        match result {
            GetNoteResult::NotANote { tier } => assert_eq!(tier, 1),
            _ => panic!("Expected NotANote"),
        }
    }

    #[test]
    fn create_note_mints_id_and_stores_note() {
        let store = MemoryStore::default();
        let note = Note {
            instance_id: "".to_string(),
            title: Some("My New Note".to_string()),
            tags: None,
            sections: vec![NoteSection {
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

        let result = create_note(&store, note).unwrap();
        assert!(!result.note.instance_id.is_empty());

        // Note should be loadable from store using slug-id8 path
        let expected_path = format!(
            "records/notes/my-new-note-{}.json",
            &result.note.instance_id[..8]
        );
        let stored = store.load_instance_json(&expected_path).unwrap();
        assert_eq!(
            stored["instanceId"].as_str(),
            Some(result.note.instance_id.as_str())
        );

        // Manifest should be updated
        let manifest = store.load_manifest().unwrap();
        assert_eq!(manifest.instance_index.len(), 1);
        assert_eq!(
            manifest.instance_index[0].instance_id(),
            result.note.instance_id
        );
    }

    #[test]
    fn note_create_no_title_produces_id_only_filename() {
        use srs_core::types::note::NoteSection;
        let store = MemoryStore::default();
        let note = Note {
            instance_id: "".to_string(),
            title: None,
            tags: None,
            sections: vec![NoteSection {
                name: "body".to_string(),
                label: None,
                content: "untitled".to_string(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };

        let result = create_note(&store, note).unwrap();
        let expected_path = format!("records/notes/{}.json", &result.note.instance_id[..8]);
        store
            .load_instance_json(&expected_path)
            .expect("note with no title should use id-only filename");
    }

    #[test]
    fn add_note_tag_adds_tag() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");

        let result =
            add_note_tag(&store, "11111111-1111-1111-8111-111111111111", "new-tag").unwrap();

        match result {
            AddTagResult::Added { note, tag } => {
                assert_eq!(tag, "new-tag");
                assert!(note.tags.as_ref().unwrap().contains(&"new-tag".to_string()));
                assert!(note.tags.as_ref().unwrap().contains(&"test".to_string()));
            }
            _ => panic!("Expected Added"),
        }

        // Manifest should reflect new tag
        let manifest = store.load_manifest().unwrap();
        let entry_tags = manifest.instance_index[0].tags.as_ref().unwrap();
        assert!(entry_tags.contains(&"new-tag".to_string()));
    }

    #[test]
    fn add_note_tag_returns_already_present() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");

        let result = add_note_tag(&store, "11111111-1111-1111-8111-111111111111", "test").unwrap();
        match result {
            AddTagResult::AlreadyPresent { .. } => {}
            _ => panic!("Expected AlreadyPresent"),
        }
    }

    #[test]
    fn add_note_tag_returns_not_found() {
        let store = MemoryStore::default();
        let result = add_note_tag(&store, "nonexistent-id", "tag").unwrap();
        match result {
            AddTagResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn note_update_rewrites_and_updates_manifest() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");

        let updated = Note {
            instance_id: note.instance_id.clone(),
            title: Some("Updated Title".to_string()),
            tags: note.tags.clone(),
            sections: note.sections.clone(),
            graduated_at: None,
            source_refs: None,
            meta: None,
            created_at: None,
            updated_at: Some("2026-01-02T00:00:00Z".to_string()),
        };

        let result = update_note(&store, updated).unwrap();
        assert_eq!(result.note.title, Some("Updated Title".to_string()));

        let stored = store
            .load_instance_json("records/notes/test-note.json")
            .unwrap();
        assert_eq!(stored["title"].as_str(), Some("Updated Title"));

        let manifest = store.load_manifest().unwrap();
        let title_val = manifest.instance_index[0]
            .title
            .as_ref()
            .and_then(|v| v.as_str());
        assert_eq!(title_val, Some("Updated Title"));
    }

    #[test]
    fn note_delete_removes_and_updates_manifest() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");

        let result = delete_note(&store, "11111111-1111-1111-8111-111111111111").unwrap();
        assert_eq!(result.instance_id, "11111111-1111-1111-8111-111111111111");

        let manifest = store.load_manifest().unwrap();
        assert!(manifest.instance_index.is_empty());
    }

    #[test]
    fn note_tag_remove_updates_note() {
        let note = make_note("11111111-1111-1111-8111-111111111111", "Test Note");
        let store = store_with_note(&note, "records/notes/test-note.json");

        // First add a removable tag
        add_note_tag(
            &store,
            "11111111-1111-1111-8111-111111111111",
            "removable-tag",
        )
        .unwrap();

        let result = remove_note_tag(
            &store,
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
    }

    // --- graduate_note tests ---

    /// MemoryStore with one Note AND a package containing one optional-field type
    /// `com.test/simple-type` (version 1). Lets graduate_note tests call
    /// create_record_in_context with empty field_values (no required fields).
    fn store_with_note_and_type(note: &Note, note_path: &str) -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let body_field = Field {
            id: "field-body-00001".to_string(),
            namespace: "com.test".to_string(),
            name: "body".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "Body field".to_string(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let simple_type = RecordType {
            id: "type-simple-00001".to_string(),
            namespace: "com.test".to_string(),
            name: "simple-type".to_string(),
            version: 1,
            description: "Simple test type".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-body-00001".to_string(),
                order: 0,
                required: false,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let note_val = {
            let mut v = serde_json::to_value(note).unwrap();
            v.as_object_mut().unwrap().insert(
                "$schema".to_string(),
                serde_json::json!("https://srs.semanticops.com/schema/2.0/note.json"),
            );
            v
        };
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: note.instance_id.clone(),
                tier: 0,
                path: note_path.to_string(),
                title: note.title.clone().map(serde_json::Value::String),
                tags: note.tags.clone(),
            }],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-00001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![body_field],
            record_types: vec![simple_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package).with_data(note_path, note_val)
    }

    #[test]
    fn graduate_note_creates_record_and_stamps_graduated_at() {
        let note = make_note("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "Graduate Me");
        let store = store_with_note_and_type(&note, "records/notes/graduate-me.json");

        let result = graduate_note(
            &store,
            GraduateNoteInput {
                note_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
                type_ref: "com.test/simple-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .unwrap();

        assert!(
            result.note.graduated_at.is_some(),
            "graduated_at must be stamped"
        );
        assert!(
            !result.record.instance_id.is_empty(),
            "record must have an instance ID"
        );
        assert_ne!(
            result.note.instance_id, result.record.instance_id,
            "note and record must have distinct IDs"
        );
        assert_eq!(
            result.note.instance_id,
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
        );
    }

    #[test]
    fn graduate_note_not_found_returns_error() {
        let store = MemoryStore::default();
        let err = graduate_note(
            &store,
            GraduateNoteInput {
                note_id: "nonexistent-id".to_string(),
                type_ref: "com.test/simple-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::NoteNotFound { .. }),
            "expected NoteNotFound, got {:?}",
            err
        );
    }

    #[test]
    fn graduate_note_not_a_note_returns_error() {
        // Manufacture a store where a tier-2 Record is indexed, not a note.
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
                tier: 2,
                path: "records/typed.json".to_string(),
                title: None,
                tags: None,
            }],
            container: None,
            container_index: None,
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(
            manifest,
            crate::package::Package {
                id: "test-pkg-00001".to_string(),
                namespace: "com.test".to_string(),
                name: "test-pkg".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            },
        );
        let err = graduate_note(
            &store,
            GraduateNoteInput {
                note_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
                type_ref: "com.test/simple-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput for non-Note entity, got {:?}",
            err
        );
    }

    #[test]
    fn graduate_note_cross_store_roundtrip() {
        let note = make_note("cccccccc-cccc-4ccc-8ccc-cccccccccccc", "Roundtrip Note");
        let mem_store = store_with_note_and_type(&note, "records/notes/roundtrip-note.json");

        // Graduate on MemoryStore
        let result = graduate_note(
            &mem_store,
            GraduateNoteInput {
                note_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".to_string(),
                type_ref: "com.test/simple-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .unwrap();

        assert!(result.note.graduated_at.is_some());

        // Copy to FileStore and re-read the note — graduated_at must survive
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&mem_store, &file_store).unwrap();

        let reloaded = get_note_by_id(&file_store, "cccccccc-cccc-4ccc-8ccc-cccccccccccc").unwrap();
        match reloaded {
            GetNoteResult::Found(n) => {
                assert!(
                    n.graduated_at.is_some(),
                    "graduated_at must survive memory → file roundtrip"
                );
                assert_eq!(n.graduated_at, result.note.graduated_at);
            }
            _ => panic!("Expected note to be found in file store after copy"),
        }
    }

    #[test]
    fn graduate_note_already_graduated_returns_error() {
        let mut note = make_note("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee", "Already Graduated");
        note.graduated_at = Some("2026-01-01T00:00:00Z".to_string());
        let store = store_with_note_and_type(&note, "records/notes/already-graduated.json");
        let err = graduate_note(
            &store,
            GraduateNoteInput {
                note_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
                type_ref: "com.test/simple-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput for already-graduated note, got {:?}",
            err
        );
    }
}
