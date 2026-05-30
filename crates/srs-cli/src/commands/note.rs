use crate::commands::{with_store, CliContext, NoteCommand, NoteTagCommand};
use crate::output;
use anyhow::Result;
use serde_json::json;
use srs_core::types::note::Note;
use srs_repository::analysis::{audit_note_tags, collect_foundation_notes};
use srs_repository::container_service::{
    add_member, get_container, is_member, list_members, remove_member,
};
use srs_repository::error::RepositoryError;
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
    let mut result = with_store(&ctx, |store| Ok(list_notes(store, filter.clone())?))?;

    if let Some(ref cid) = ctx.container_id {
        let members = with_store(&ctx, |store| Ok(list_members(store, cid)?))?;
        result
            .notes
            .retain(|n| members.iter().any(|id| id == &n.instance_id));
    }

    let notes: Vec<serde_json::Value> = result
        .notes
        .into_iter()
        .map(|n| {
            json!({
                "instanceId": n.instance_id,
                "title": n.title,
            })
        })
        .collect();

    Ok(output::ok("note list", json!({ "notes": notes })))
}

fn cmd_note_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_note_by_id(store, &id)?))? {
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
    if let Some(ref cid) = ctx.container_id {
        match with_store(&ctx, |store| Ok(get_container(store, cid)?)) {
            Ok(_) => {}
            Err(e) => {
                if let Some(RepositoryError::ContainerNotFound { .. }) =
                    e.downcast_ref::<RepositoryError>()
                {
                    return Ok(output::err(
                        "note create",
                        vec![format!("Container '{}' not found — no note written", cid)],
                    ));
                }
                return Err(e);
            }
        }
    }

    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let note: Note = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse note JSON: {}", e))?;

    let result = with_store(&ctx, |store| Ok(create_note(store, note.clone())?))?;

    if let Some(ref cid) = ctx.container_id {
        if let Err(e) = with_store(&ctx, |store| {
            Ok(add_member(store, cid, &result.note.instance_id)?)
        }) {
            return Ok(output::err(
                "note create",
                vec![format!(
                    "Note created but failed to add to container: {}",
                    e
                )],
            ));
        }
    }

    Ok(output::ok("note create", json!({ "note": result.note })))
}

fn cmd_note_tag_dispatch(ctx: CliContext, cmd: NoteTagCommand) -> Result<String> {
    match cmd {
        NoteTagCommand::Add { id, tag, json: _ } => cmd_note_tag_add(ctx, id, tag),
        NoteTagCommand::Remove { id, tag, json: _ } => cmd_note_tag_remove(ctx, id, tag),
    }
}

fn cmd_note_tag_add(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(add_note_tag(store, &id, &tag)?))? {
        AddTagResult::Added { note, .. } | AddTagResult::AlreadyPresent { note, .. } => Ok(
            output::ok("note tag add", json!({ "note": note, "tag": tag })),
        ),
        AddTagResult::NotFound => Ok(output::err(
            "note tag add",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_tag_remove(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(remove_note_tag(store, &id, &tag)?))? {
        RemoveTagResult::Removed { note, .. } => Ok(output::ok(
            "note tag remove",
            json!({ "note": note, "tag": tag, "removed": true }),
        )),
        RemoveTagResult::NotPresent { note, .. } => Ok(output::ok(
            "note tag remove",
            json!({ "note": note, "tag": tag, "removed": false }),
        )),
        RemoveTagResult::NotFound => Ok(output::err(
            "note tag remove",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let note: Note = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse note JSON: {}", e))?;

    if note.instance_id != id {
        return Ok(output::err(
            "note update",
            vec![format!(
                "Note ID in JSON ({}) does not match command argument ({})",
                note.instance_id, id
            )],
        ));
    }

    let result = with_store(&ctx, |store| Ok(update_note(store, note.clone())?))?;

    Ok(output::ok("note update", json!({ "note": result.note })))
}

fn cmd_note_delete(ctx: CliContext, id: String) -> Result<String> {
    if let Some(ref cid) = ctx.container_id {
        if !with_store(&ctx, |store| Ok(is_member(store, cid, &id)?))? {
            return Ok(output::err(
                "note delete",
                vec![format!(
                    "Instance '{}' is not a member of container '{}' — delete refused",
                    id, cid
                )],
            ));
        }
        with_store(&ctx, |store| Ok(remove_member(store, cid, &id)?))?;
    }

    match with_store(&ctx, |store| Ok(delete_note(store, &id)?)) {
        Ok(DeleteNoteResult { instance_id }) => Ok(output::ok(
            "note delete",
            json!({ "instanceId": instance_id }),
        )),
        Err(e) => Ok(output::err("note delete", vec![e.to_string()])),
    }
}

fn cmd_note_audit_tags(ctx: CliContext) -> Result<String> {
    let audit = with_store(&ctx, |store| Ok(audit_note_tags(store)?))?;
    Ok(output::ok("note audit-tags", json!({ "tagAudit": audit })))
}

fn cmd_note_foundations(ctx: CliContext) -> Result<String> {
    let signal_tags = with_store(&ctx, |store| Ok(get_foundation_signal_tags(store)?))?;
    let foundation_notes = with_store(&ctx, |store| {
        Ok(collect_foundation_notes(store, &signal_tags)?)
    })?;

    Ok(output::ok(
        "note foundations",
        json!({ "foundationNotes": foundation_notes }),
    ))
}
