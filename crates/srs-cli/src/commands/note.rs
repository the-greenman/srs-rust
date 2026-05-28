use crate::commands::NoteCommand;
use crate::output;
use anyhow::{Context, Result};
use serde_json::json;
use srs_core::types::note::Note;
use srs_core::validation::note::validate_note;
use srs_repository::detect::find_repo_root;
use srs_repository::loader::load_note_relative;
use srs_repository::manifest::load_manifest;
use srs_repository::writer::{new_instance_id, upsert_index_entry, write_manifest, write_note};
use std::io::{self, Read};
use std::path::PathBuf;

pub fn dispatch(cmd: NoteCommand) -> Result<String> {
    match cmd {
        NoteCommand::List { repo, tag, json: _ } => cmd_note_list(repo, tag),
        NoteCommand::Get { repo, id, json: _ } => cmd_note_get(repo, id),
        NoteCommand::Create { repo, json: _ } => cmd_note_create(repo),
        NoteCommand::Tag {
            repo,
            id,
            add_tag,
            json: _,
        } => cmd_note_tag(repo, id, add_tag),
    }
}

fn resolve_repo(repo: Option<PathBuf>) -> Result<PathBuf> {
    match repo {
        Some(path) => Ok(path),
        None => {
            let cwd = std::env::current_dir()?;
            find_repo_root(&cwd).context("Failed to find repository root")
        }
    }
}

fn cmd_note_list(repo: Option<PathBuf>, tag: Option<String>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let manifest = load_manifest(&repo_root)?;

    let mut notes: Vec<serde_json::Value> = Vec::new();

    for entry in &manifest.instance_index {
        // Skip non-note entries (only include Tier 0)
        if !entry.is_note() {
            continue;
        }

        let path = entry.path();

        // Load note to check tags if filtering
        if let Some(ref filter_tag) = tag {
            match load_note_relative(&repo_root, path) {
                Ok(note) => {
                    let has_tag = note
                        .tags
                        .as_ref()
                        .map_or(false, |tags| tags.contains(filter_tag));
                    if !has_tag {
                        continue;
                    }
                    notes.push(json!({
                        "instanceId": entry.instance_id(),
                        "path": path,
                        "title": entry.title(),
                    }));
                }
                Err(_) => continue, // Skip notes that fail to load
            }
        } else {
            notes.push(json!({
                "instanceId": entry.instance_id(),
                "path": path,
                "title": entry.title(),
            }));
        }
    }

    Ok(output::ok("note list", json!({ "notes": notes })))
}

fn cmd_note_get(repo: Option<PathBuf>, id: String) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let manifest = load_manifest(&repo_root)?;

    // Find the note in the manifest
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == Some(&id));

    match entry {
        Some(e) => {
            // Refuse non-note entries (tier != 0)
            if !e.is_note() {
                return Ok(output::err(
                    "note get",
                    vec![format!("Instance '{}' is not a Note (tier != 0)", id)],
                ));
            }
            let note = load_note_relative(&repo_root, e.path())?;
            Ok(output::ok("note get", json!({ "note": note })))
        }
        None => Ok(output::err(
            "note get",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_create(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let mut note: Note = serde_json::from_str(&stdin).context("Failed to parse note JSON")?;

    // Mint instance_id if absent
    if note.instance_id.is_empty() {
        note.instance_id = new_instance_id();
    }

    // Validate the note
    validate_note(&note).context("Note validation failed")?;

    // Determine path: records/notes/<slug>.json
    let slug = note
        .title
        .as_ref()
        .map(|t| slugify(t))
        .unwrap_or_else(|| note.instance_id.clone());
    let relative_path = format!("records/notes/{}.json", slug);
    let full_path = repo_root.join(&relative_path);

    // Ensure directory exists
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write the note
    write_note(&note, &full_path)?;

    // Update manifest
    let mut manifest = load_manifest(&repo_root)?;
    upsert_index_entry(&mut manifest, &note, &relative_path);
    write_manifest(&manifest)?;

    Ok(output::ok("note create", json!({ "note": note })))
}

fn cmd_note_tag(repo: Option<PathBuf>, id: String, add_tag: String) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let manifest = load_manifest(&repo_root)?;

    // Find the note in the manifest
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == Some(&id));

    match entry {
        Some(e) => {
            let mut note = load_note_relative(&repo_root, e.path())?;

            // Add tag if not already present
            let tags = note.tags.get_or_insert_with(Vec::new);
            if !tags.contains(&add_tag) {
                tags.push(add_tag.clone());
            }

            // Write back
            let full_path = repo_root.join(e.path());
            write_note(&note, &full_path)?;

            // Update manifest to reflect new tags
            let mut manifest = load_manifest(&repo_root)?;
            upsert_index_entry(&mut manifest, &note, e.path());
            write_manifest(&manifest)?;

            Ok(output::ok("note tag", json!({ "note": note })))
        }
        None => Ok(output::err(
            "note tag",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn slugify(title: &str) -> String {
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

    #[test]
    fn slugify_preserves_word_boundaries_from_punctuation() {
        assert_eq!(
            slugify("AI-Native SRS Repositories"),
            "ai-native-srs-repositories"
        );
    }

    #[test]
    fn slugify_collapses_repeated_separators() {
        assert_eq!(slugify("Meaning: AI + Humans"), "meaning-ai-humans");
    }
}
