use crate::commands::{resolve_repo, NoteCommand};
use crate::output;
use anyhow::{Context, Result};
use serde_json::json;
use srs_core::types::note::Note;
use srs_repository::analysis::{audit_note_tags, collect_foundation_notes};
use srs_repository::services::{
    add_note_tag, create_note, get_note_by_id, list_notes, AddTagResult, GetNoteResult,
    ListNotesFilter,
};
use srs_repository::tag_service::get_foundation_signal_tags;
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
        NoteCommand::AuditTags { repo, json: _ } => cmd_note_audit_tags(repo),
        NoteCommand::Foundations { repo, json: _ } => cmd_note_foundations(repo),
    }
}

fn cmd_note_list(repo: Option<PathBuf>, tag: Option<String>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let filter = ListNotesFilter { tag };
    let result = list_notes(&repo_root, filter)?;

    // Convert NoteSummary to JSON for output
    let notes: Vec<serde_json::Value> = result
        .notes
        .into_iter()
        .map(|n| {
            json!({
                "instanceId": n.instance_id,
                "path": n.path,
                "title": n.title,
            })
        })
        .collect();

    Ok(output::ok("note list", json!({ "notes": notes })))
}

fn cmd_note_get(repo: Option<PathBuf>, id: String) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    match get_note_by_id(&repo_root, &id)? {
        GetNoteResult::Found(note) => Ok(output::ok("note get", json!({ "note": note }))),
        GetNoteResult::NotFound => Ok(output::err(
            "note get",
            vec![format!("Note with id '{}' not found", id)],
        )),
        GetNoteResult::NotANote { tier: _ } => Ok(output::err(
            "note get",
            vec![format!("Instance '{}' is not a Note (tier != 0)", id,)],
        )),
    }
}

fn cmd_note_create(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let note: Note = serde_json::from_str(&stdin).context("Failed to parse note JSON")?;

    // Call service
    let result = create_note(&repo_root, note)?;

    Ok(output::ok("note create", json!({ "note": result.note })))
}

fn cmd_note_tag(repo: Option<PathBuf>, id: String, add_tag: String) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    match add_note_tag(&repo_root, &id, &add_tag)? {
        AddTagResult::Added { note, .. } | AddTagResult::AlreadyPresent { note, .. } => {
            Ok(output::ok("note tag", json!({ "note": note })))
        }
        AddTagResult::NotFound => Ok(output::err(
            "note tag",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_audit_tags(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;
    let audit = audit_note_tags(&repo_root)?;
    Ok(output::ok("note audit-tags", json!({ "tagAudit": audit })))
}

fn cmd_note_foundations(repo: Option<PathBuf>) -> Result<String> {
    let repo_root = resolve_repo(repo)?;

    // Get foundation signal tags from TagDefinition records (data-driven)
    let signal_tags = get_foundation_signal_tags(&repo_root)?;

    // If no TagDefinition records with foundation role exist, return empty list
    // (acceptable transitional state until TagDefinition records are created)
    let foundation_notes = collect_foundation_notes(&repo_root, &signal_tags)?;

    Ok(output::ok(
        "note foundations",
        json!({ "foundationNotes": foundation_notes }),
    ))
}
