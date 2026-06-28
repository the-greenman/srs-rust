use crate::commands::{
    with_store, CliContext, RecordCommand, RecordRevisionCommand, RecordTagCommand,
};
use crate::output::{self, OutputDTO};
use crate::payload::{
    DeletedPayload, RecordListPayload, RecordPayload, RecordSuccessorPayload, RecordTagAddPayload,
    RecordTagListPayload, RecordValidatePayload, RevisionListPayload, RevisionPayload,
};
use anyhow::Result;
use srs_repository::record_store::{
    add_record_tag, create_record_in_context, create_record_successor, delete_record_in_context,
    get_record_by_id, get_record_revision, list_record_revisions, list_record_summaries,
    list_record_tags, remove_record_tag, transition_record_lifecycle, update_record,
    validate_record_input, AddRecordTagResult, CreateRecordInput, CreateRecordSuccessorInput,
    RecordListFilter, RemoveRecordTagResult, TransitionLifecycleInput, ValidateRecordInput,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: RecordCommand) -> Result<String> {
    match cmd {
        RecordCommand::List {
            type_filter,
            tag,
            json: _,
        } => cmd_record_list(ctx, type_filter, tag),
        RecordCommand::Get { id, json: _ } => cmd_record_get(ctx, id),
        RecordCommand::Create {
            type_filter,
            version,
            dir,
            json: _,
        } => cmd_record_create(ctx, type_filter, version, dir),
        RecordCommand::Update { id, json: _ } => cmd_record_update(ctx, id),
        RecordCommand::Validate => cmd_record_validate(ctx),
        RecordCommand::Delete { id, json: _ } => cmd_record_delete(ctx, id),
        RecordCommand::Transition { id } => cmd_record_transition(ctx, id),
        RecordCommand::Successor { id } => cmd_record_successor(ctx, id),
        RecordCommand::Revision(rev_cmd) => dispatch_revision(ctx, rev_cmd),
        RecordCommand::Tag(tag_cmd) => dispatch_tag(ctx, tag_cmd),
    }
}

fn dispatch_revision(ctx: CliContext, cmd: RecordRevisionCommand) -> Result<String> {
    match cmd {
        RecordRevisionCommand::List {
            id,
            field_id,
            limit,
            offset,
        } => cmd_revision_list(ctx, id, field_id, limit, offset),
        RecordRevisionCommand::Get { id, revision_id } => cmd_revision_get(ctx, id, revision_id),
    }
}

fn dispatch_tag(ctx: CliContext, cmd: RecordTagCommand) -> Result<String> {
    match cmd {
        RecordTagCommand::Add { id, tag } => cmd_record_tag_add(ctx, id, tag),
        RecordTagCommand::Remove { id, tag } => cmd_record_tag_remove(ctx, id, tag),
        RecordTagCommand::List => cmd_record_tag_list(ctx),
    }
}

fn cmd_record_list(
    ctx: CliContext,
    type_filter: Option<String>,
    tag: Option<String>,
) -> Result<String> {
    let (type_namespace, type_name) = match type_filter {
        None => (None, None),
        Some(ref filter) => match parse_type_filter(filter) {
            Some((namespace, name)) => (Some(namespace), Some(name)),
            None => {
                return Ok(output::err(
                    "record list",
                    vec![format!(
                        "Invalid type filter '{}'. Expected format: namespace/name",
                        filter
                    )],
                ))
            }
        },
    };

    let records = with_store(&ctx, |store| {
        Ok(list_record_summaries(
            store,
            RecordListFilter {
                type_namespace,
                type_name,
                container_id: ctx.container_id.clone(),
                tag,
            },
        )?)
    })?;

    output::serialize("record list", RecordListPayload { records })
}

fn cmd_record_tag_add(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(add_record_tag(store, &id, &tag)?))? {
        AddRecordTagResult::Added { record, .. }
        | AddRecordTagResult::AlreadyPresent { record, .. } => {
            output::serialize("record tag add", RecordTagAddPayload { record, tag })
        }
        AddRecordTagResult::NotFound => Ok(output::err(
            "record tag add",
            vec![format!("No tier-2 record with id '{}' found", id)],
        )),
    }
}

fn cmd_record_tag_remove(ctx: CliContext, id: String, tag: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(remove_record_tag(store, &id, &tag)?))? {
        RemoveRecordTagResult::Removed { record, .. } => {
            output::serialize("record tag remove", RecordTagAddPayload { record, tag })
        }
        RemoveRecordTagResult::NotPresent { record, .. } => {
            output::serialize("record tag remove", RecordTagAddPayload { record, tag })
        }
        RemoveRecordTagResult::NotFound => Ok(output::err(
            "record tag remove",
            vec![format!("No tier-2 record with id '{}' found", id)],
        )),
    }
}

fn cmd_record_tag_list(ctx: CliContext) -> Result<String> {
    let result = with_store(&ctx, |store| {
        Ok(list_record_tags(store, ctx.container_id.as_deref())?)
    })?;
    output::serialize("record tag list", RecordTagListPayload::from(result))
}

fn cmd_record_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_record_by_id(store, &id)?))? {
        Some(record) => output::serialize("record get", RecordPayload { record }),
        None => Ok(output::err(
            "record get",
            vec![format!("Record with id '{}' not found", id)],
        )),
    }
}

fn cmd_record_create(
    ctx: CliContext,
    type_filter: String,
    version: Option<u32>,
    dir: Option<String>,
) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: CreateRecordInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record create",
                vec![format!("Failed to parse record JSON from stdin: {}", e)],
            ))
        }
    };

    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(create_record_in_context(
            store,
            &type_filter,
            version,
            input,
            container_id,
            dir.as_deref(),
        )?)
    }) {
        Ok(result) => output::serialize(
            "record create",
            RecordPayload {
                record: result.record,
            },
        ),
        Err(e) => Ok(output::err("record create", vec![e.to_string()])),
    }
}

fn cmd_record_validate(ctx: CliContext) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: ValidateRecordInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record validate",
                vec![format!("Failed to parse record JSON from stdin: {}", e)],
            ))
        }
    };

    let report = with_store(&ctx, |store| Ok(validate_record_input(store, input)?))?;
    if report.ok {
        output::serialize(
            "record validate",
            RecordValidatePayload {
                ok: true,
                errors: vec![],
            },
        )
    } else {
        Ok(output::err("record validate", report.errors))
    }
}

fn cmd_record_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: CreateRecordInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record update",
                vec![format!("Failed to parse record JSON from stdin: {}", e)],
            ))
        }
    };

    match with_store(&ctx, |store| {
        Ok(update_record(
            store,
            &id,
            input.field_values,
            input.group_values.map(Some),
            input.tags,
        )?)
    }) {
        Ok(record) => output::serialize("record update", RecordPayload { record }),
        Err(e) => Ok(output::err("record update", vec![e.to_string()])),
    }
}

fn cmd_record_delete(ctx: CliContext, id: String) -> Result<String> {
    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(delete_record_in_context(store, id, container_id)?)
    }) {
        Ok(result) => output::serialize(
            "record delete",
            DeletedPayload {
                instance_id: result.instance_id,
            },
        ),
        Err(e) => Ok(output::err("record delete", vec![e.to_string()])),
    }
}

fn cmd_record_transition(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: TransitionLifecycleInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record transition",
                vec![format!("Failed to parse transition JSON from stdin: {}", e)],
            ))
        }
    };

    match with_store(&ctx, |store| {
        Ok(transition_record_lifecycle(store, &id, input)?)
    }) {
        Ok(result) => {
            let payload = serde_json::to_value(RecordPayload {
                record: result.record,
            })?;
            let dto = OutputDTO {
                ok: true,
                command: "record transition".to_string(),
                version: output::VERSION.to_string(),
                payload: Some(payload),
                diagnostics: if result.warnings.is_empty() {
                    None
                } else {
                    Some(result.warnings)
                },
            };
            Ok(dto.render(ctx.format, ctx.pretty))
        }
        Err(e) => Ok(output::err("record transition", vec![e.to_string()])),
    }
}

fn cmd_record_successor(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let input: CreateRecordSuccessorInput = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(e) => {
            return Ok(output::err(
                "record successor",
                vec![format!("Failed to parse successor JSON from stdin: {}", e)],
            ))
        }
    };

    match with_store(&ctx, |store| {
        Ok(create_record_successor(store, &id, input)?)
    }) {
        Ok(result) => output::serialize(
            "record successor",
            RecordSuccessorPayload {
                record: result.record,
                relation: result.relation,
            },
        ),
        Err(e) => Ok(output::err("record successor", vec![e.to_string()])),
    }
}

fn cmd_revision_list(
    ctx: CliContext,
    id: String,
    field_id: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(list_record_revisions(
            store,
            &id,
            field_id.as_deref(),
            limit,
            offset,
        )?)
    }) {
        Ok(revisions) => output::serialize(
            "record revision list",
            RevisionListPayload {
                instance_id: id,
                revisions,
            },
        ),
        Err(e) => Ok(output::err("record revision list", vec![e.to_string()])),
    }
}

fn cmd_revision_get(ctx: CliContext, id: String, revision_id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(get_record_revision(store, &id, &revision_id)?)
    }) {
        Ok(Some(revision)) => {
            output::serialize("record revision get", RevisionPayload { revision })
        }
        Ok(None) => Ok(output::err(
            "record revision get",
            vec![format!(
                "Revision '{}' not found for record '{}'",
                revision_id, id
            )],
        )),
        Err(e) => Ok(output::err("record revision get", vec![e.to_string()])),
    }
}

fn parse_type_filter(type_filter: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = type_filter.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
}
