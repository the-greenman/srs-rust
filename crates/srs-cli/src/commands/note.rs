use crate::commands::{CliContext, NoteCommand, NoteTagCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::note::Note;
use srs_repository::analysis::{audit_note_tags, collect_foundation_notes};
use srs_repository::services::{
    add_note_tag, create_note, delete_note, get_note_by_id, list_notes, remove_note_tag,
    update_note, AddTagResult, DeleteNoteResult, GetNoteResult, ListNotesFilter, RemoveTagResult,
};
use srs_repository::tag_service::get_foundation_signal_tags;
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: NoteCommand) -> Result<String> {
    match cmd {
        NoteCommand::List { tag, json: _ } => cmd_note_list(ctx, tag),
        NoteCommand::Get { id, json: _ } => cmd_note_get(ctx, id),
        NoteCommand::Create { json: _ } => cmd_note_create(ctx),
        NoteCommand::Update { id, json: _ } => cmd_note_update(ctx, id),
        NoteCommand::Delete { id, json: _ } => cmd_note_delete(ctx, id),
        NoteCommand::Tag(tag_cmd) => cmd_note_tag_dispatch(ctx, tag_cmd),
        NoteCommand::AuditTags { json: _ } => cmd_note_audit_tags(ctx),
        NoteCommand::Foundations { json: _ } => cmd_note_foundations(ctx),
    }
}

fn cmd_note_list(ctx: CliContext, tag: Option<String>) -> Result<String> {
    let filter = ListNotesFilter { tag };
    let result = list_notes(&ctx.repo, filter)?;

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

fn cmd_note_get(ctx: CliContext, id: String) -> Result<String> {
    match get_note_by_id(&ctx.repo, &id)? {
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

fn cmd_note_create(ctx: CliContext) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let note: Note = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse note JSON: {}", e))?;

    // Call service
    let result = create_note(&ctx.repo, note)?;

    Ok(output::ok("note create", json!({ "note": result.note })))
}

fn cmd_note_tag_dispatch(ctx: CliContext, cmd: NoteTagCommand) -> Result<String> {
    match cmd {
        NoteTagCommand::Add { id, tag, json: _ } => cmd_note_tag_add(ctx, id, tag),
        NoteTagCommand::Remove { id, tag, json: _ } => cmd_note_tag_remove(ctx, id, tag),
    }
}

fn cmd_note_tag_add(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match add_note_tag(&ctx.repo, &id, &tag)? {
        AddTagResult::Added { note, .. } | AddTagResult::AlreadyPresent { note, .. } => {
            Ok(output::ok("note tag add", json!({ "note": note, "tag": tag })))
        }
        AddTagResult::NotFound => Ok(output::err(
            "note tag add",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_tag_remove(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match remove_note_tag(&ctx.repo, &id, &tag)? {
        RemoveTagResult::Removed { note, .. } => {
            Ok(output::ok("note tag remove", json!({ "note": note, "tag": tag, "removed": true })))
        }
        RemoveTagResult::NotPresent { note, .. } => {
            Ok(output::ok("note tag remove", json!({ "note": note, "tag": tag, "removed": false })))
        }
        RemoveTagResult::NotFound => Ok(output::err(
            "note tag remove",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_update(ctx: CliContext, id: String) -> Result<String> {
    // Read JSON from stdin
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let note: Note = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse note JSON: {}", e))?;

    // Ensure the ID matches
    if note.instance_id != id {
        return Ok(output::err(
            "note update",
            vec![format!("Note ID in JSON ({}) does not match command argument ({})", note.instance_id, id)],
        ));
    }

    // Call service
    let result = update_note(&ctx.repo, note)?;

    Ok(output::ok("note update", json!({ "note": result.note, "path": result.path })))
}

fn cmd_note_delete(ctx: CliContext, id: String) -> Result<String> {
    match delete_note(&ctx.repo, &id) {
        Ok(DeleteNoteResult { instance_id, path }) => {
            Ok(output::ok("note delete", json!({ "instanceId": instance_id, "path": path })))
        }
        Err(e) => Ok(output::err("note delete", vec![e.to_string()])),
    }
}

fn cmd_note_audit_tags(ctx: CliContext) -> Result<String> {
    let audit = audit_note_tags(&ctx.repo)?;
    Ok(output::ok("note audit-tags", json!({ "tagAudit": audit })))
}

fn cmd_note_foundations(ctx: CliContext) -> Result<String> {
    // Get foundation signal tags from TagDefinition records (data-driven)
    let signal_tags = get_foundation_signal_tags(&ctx.repo)?;

    // If no TagDefinition records with foundation role exist, return empty list
    // (acceptable transitional state until TagDefinition records are created)
    let foundation_notes = collect_foundation_notes(&ctx.repo, &signal_tags)?;

    Ok(output::ok(
        "note foundations",
        json!({ "foundationNotes": foundation_notes }),
    ))
}
