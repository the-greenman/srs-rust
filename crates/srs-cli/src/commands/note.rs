use crate::commands::{with_store, CliContext, NoteCommand, NoteTagCommand};
use crate::output;
use crate::payload::{
    DeletedPayload, NoteFoundationsPayload, NoteListEntry, NoteListPayload, NotePayload,
    NoteTagAddPayload, NoteTagListPayload, NoteTagMapPayload, NoteTagRemovePayload,
};
use anyhow::Result;
use srs_core::types::note::Note;
use srs_repository::analysis::{
    audit_note_tags, audit_note_tags_for_note, collect_foundation_notes,
};
use srs_repository::services::{
    add_note_tag, create_note_in_context, delete_note_in_context, get_note_by_id, list_note_tags,
    list_notes, remove_note_tag, update_note_validated, AddTagResult, CreateNoteInput,
    DeleteNoteInput, DeleteNoteResult, GetNoteResult, ListNotesFilter, RemoveTagResult,
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
        NoteCommand::Foundations { json: _ } => cmd_note_foundations(ctx),
    }
}

fn cmd_note_list(ctx: CliContext, tag: Option<String>) -> Result<String> {
    let filter = ListNotesFilter {
        tag,
        container_id: ctx.container_id.clone(),
    };
    let result = with_store(&ctx, |store| Ok(list_notes(store, filter.clone())?))?;
    let notes = result.notes.into_iter().map(NoteListEntry::from).collect();
    output::serialize("note list", NoteListPayload { notes })
}

fn cmd_note_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_note_by_id(store, &id)?))? {
        GetNoteResult::Found(note) => output::serialize("note get", NotePayload { note: *note }),
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
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let note: Note = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse note JSON: {}", e))?;

    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(create_note_in_context(
            store,
            CreateNoteInput { note, container_id },
        )?)
    }) {
        Ok(result) => output::serialize("note create", NotePayload { note: result.note }),
        Err(e) => Ok(output::err("note create", vec![e.to_string()])),
    }
}

fn cmd_note_tag_dispatch(ctx: CliContext, cmd: NoteTagCommand) -> Result<String> {
    match cmd {
        NoteTagCommand::Add { id, tag, json: _ } => cmd_note_tag_add(ctx, id, tag),
        NoteTagCommand::Remove { id, tag, json: _ } => cmd_note_tag_remove(ctx, id, tag),
        NoteTagCommand::List => cmd_note_tag_list(ctx),
        NoteTagCommand::Map { id } => cmd_note_tag_map(ctx, id),
    }
}

fn cmd_note_tag_add(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(add_note_tag(store, &id, &tag)?))? {
        AddTagResult::Added { note, .. } | AddTagResult::AlreadyPresent { note, .. } => {
            output::serialize("note tag add", NoteTagAddPayload { note, tag })
        }
        AddTagResult::NotFound => Ok(output::err(
            "note tag add",
            vec![format!("Note with id '{}' not found", id)],
        )),
    }
}

fn cmd_note_tag_remove(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(remove_note_tag(store, &id, &tag)?))? {
        RemoveTagResult::Removed { note, .. } => output::serialize(
            "note tag remove",
            NoteTagRemovePayload {
                note,
                tag,
                removed: true,
            },
        ),
        RemoveTagResult::NotPresent { note, .. } => output::serialize(
            "note tag remove",
            NoteTagRemovePayload {
                note,
                tag,
                removed: false,
            },
        ),
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

    match with_store(&ctx, |store| Ok(update_note_validated(store, &id, note)?)) {
        Ok(result) => output::serialize("note update", NotePayload { note: result.note }),
        Err(e) => Ok(output::err("note update", vec![e.to_string()])),
    }
}

fn cmd_note_delete(ctx: CliContext, id: String) -> Result<String> {
    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(delete_note_in_context(
            store,
            DeleteNoteInput { id, container_id },
        )?)
    }) {
        Ok(DeleteNoteResult { instance_id }) => {
            output::serialize("note delete", DeletedPayload { instance_id })
        }
        Err(e) => Ok(output::err("note delete", vec![e.to_string()])),
    }
}

fn cmd_note_tag_list(ctx: CliContext) -> Result<String> {
    let container_id = ctx.container_id.clone();
    let result = with_store(&ctx, |store| {
        Ok(list_note_tags(store, container_id.as_deref())?)
    })?;
    output::serialize("note tag list", NoteTagListPayload::from(result))
}

fn cmd_note_tag_map(ctx: CliContext, id: Option<String>) -> Result<String> {
    let audit = with_store(&ctx, |store| {
        Ok(if let Some(ref note_id) = id {
            audit_note_tags_for_note(store, note_id)?
        } else {
            audit_note_tags(store)?
        })
    })?;
    output::serialize("note tag map", NoteTagMapPayload { tag_audit: audit })
}

fn cmd_note_foundations(ctx: CliContext) -> Result<String> {
    let signal_tags = with_store(&ctx, |store| Ok(get_foundation_signal_tags(store)?))?;
    let foundation_notes = with_store(&ctx, |store| {
        Ok(collect_foundation_notes(store, &signal_tags)?)
    })?;
    output::serialize(
        "note foundations",
        NoteFoundationsPayload { foundation_notes },
    )
}
