use crate::commands::{parse_type_filter, with_store, CliContext, RecordCommand, RecordTagCommand};
use crate::output;
use crate::payload::{
    RecordAllowedTransitionsPayload, RecordDeletePayload, RecordForkPayload,
    RecordGetAttachmentsPayload, RecordGetPayload, RecordListPayload, RecordPayload,
    RecordSuccessorPayload, RecordTagAddPayload, RecordTagListPayload, RecordTransitionPayload,
    RecordValidatePayload,
};
use anyhow::Result;
use srs_repository::attachment_service::{get_record_attachments, GetRecordAttachmentsInput};
use srs_repository::error::RepositoryError;
use srs_repository::record_store::{
    add_record_tag, create_record_in_context, create_record_successor, delete_record_in_context,
    get_allowed_lifecycle_transitions, get_record_summary_by_id, list_record_summaries,
    list_record_tags, remove_record_tag, transition_record_lifecycle, update_record,
    validate_record_input, AddRecordTagResult, CreateRecordInput, CreateRecordSuccessorInput,
    RecordListFilter, RemoveRecordTagResult, TransitionLifecycleInput, UpdateRecordInput,
    ValidateRecordInput,
};

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
        RecordCommand::Delete {
            id,
            cascade,
            json: _,
        } => cmd_record_delete(ctx, id, cascade),
        RecordCommand::Transition { id } => cmd_record_transition(ctx, id),
        RecordCommand::Successor { id } => cmd_record_successor(ctx, id),
        RecordCommand::Fork { id } => cmd_record_fork(ctx, id),
        RecordCommand::AllowedTransitions { id } => cmd_record_allowed_transitions(ctx, id),
        RecordCommand::Attachments { id } => cmd_record_attachments(ctx, id),
        RecordCommand::Tag(tag_cmd) => dispatch_tag(ctx, tag_cmd),
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
                return Ok(output::repo_err(
                    "record list",
                    &RepositoryError::InvalidInput {
                        message: format!(
                            "Invalid type filter '{}'. Expected format: namespace/name",
                            filter
                        ),
                    },
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
        AddRecordTagResult::NotFound => Ok(output::repo_err(
            "record tag add",
            &RepositoryError::InstanceNotFound { id: id.clone() },
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
        RemoveRecordTagResult::NotFound => Ok(output::repo_err(
            "record tag remove",
            &RepositoryError::InstanceNotFound { id: id.clone() },
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
    match with_store(&ctx, |store| Ok(get_record_summary_by_id(store, &id)?))? {
        Some(summary) => output::serialize("record get", RecordGetPayload::from(summary)),
        None => Ok(output::repo_err(
            "record get",
            &RepositoryError::InstanceNotFound { id: id.clone() },
        )),
    }
}

fn cmd_record_create(
    ctx: CliContext,
    type_filter: String,
    version: Option<u32>,
    dir: Option<String>,
) -> Result<String> {
    let input: CreateRecordInput = match crate::input::from_stdin("record") {
        Ok(v) => v,
        Err(e) => return Ok(output::any_err("record create", &e)),
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
        Err(e) => Ok(output::any_err("record create", &e)),
    }
}

fn cmd_record_validate(ctx: CliContext) -> Result<String> {
    let input: ValidateRecordInput = match crate::input::from_stdin("record") {
        Ok(v) => v,
        Err(e) => return Ok(output::any_err("record validate", &e)),
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
    let input: UpdateRecordInput = crate::input::from_stdin("record")?;
    match with_store(&ctx, |store| Ok(update_record(store, &id, input)?)) {
        Ok(record) => output::serialize("record update", RecordPayload { record }),
        Err(e) => Ok(output::any_err("record update", &e)),
    }
}

fn cmd_record_delete(ctx: CliContext, id: String, cascade: bool) -> Result<String> {
    let container_id = ctx.container_id.clone();
    match with_store(&ctx, |store| {
        Ok(delete_record_in_context(store, id, container_id, cascade)?)
    }) {
        Ok(result) => output::serialize(
            "record delete",
            RecordDeletePayload {
                instance_id: result.instance_id,
                cascaded_relations: result.cascaded_relations,
            },
        ),
        Err(e) => Ok(output::any_err("record delete", &e)),
    }
}

fn cmd_record_transition(ctx: CliContext, id: String) -> Result<String> {
    let input: TransitionLifecycleInput = match crate::input::from_stdin("transition") {
        Ok(v) => v,
        Err(e) => return Ok(output::any_err("record transition", &e)),
    };
    match with_store(&ctx, |store| {
        Ok(transition_record_lifecycle(store, &id, input)?)
    }) {
        Ok(result) => output::serialize(
            "record transition",
            RecordTransitionPayload {
                record: result.record,
                warnings: result.warnings,
                successor: result.successor,
                relation: result.relation,
            },
        ),
        Err(e) => Ok(output::any_err("record transition", &e)),
    }
}

fn cmd_record_successor(ctx: CliContext, id: String) -> Result<String> {
    let input: CreateRecordSuccessorInput = match crate::input::from_stdin("successor") {
        Ok(v) => v,
        Err(e) => return Ok(output::any_err("record successor", &e)),
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
        Err(e) => Ok(output::any_err("record successor", &e)),
    }
}

fn cmd_record_fork(ctx: CliContext, id: String) -> Result<String> {
    let Some(container) = ctx.container_id.clone() else {
        return Ok(output::repo_err(
            "record fork",
            &RepositoryError::InvalidInput {
                message: "--container <ID> is required: the container the fork is swapped into"
                    .into(),
            },
        ));
    };
    match with_store(&ctx, |store| {
        Ok(srs_repository::fork_service::fork_subtree(
            store, &container, &id,
        )?)
    }) {
        Ok(r) => output::serialize(
            "record fork",
            RecordForkPayload {
                container_id: r.container_id,
                forks: r.forks,
                relations: r.relations,
            },
        ),
        Err(e) => Ok(output::any_err("record fork", &e)),
    }
}

fn cmd_record_allowed_transitions(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(get_allowed_lifecycle_transitions(store, &id)?)
    }) {
        Ok(result) => output::serialize(
            "record allowed-transitions",
            RecordAllowedTransitionsPayload::from(result),
        ),
        Err(e) => Ok(output::any_err("record allowed-transitions", &e)),
    }
}

fn cmd_record_attachments(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(get_record_attachments(
            store,
            GetRecordAttachmentsInput {
                instance_id: id.clone(),
            },
        )?)
    })? {
        Some(result) => output::serialize(
            "record attachments",
            RecordGetAttachmentsPayload::from(result),
        ),
        None => Ok(output::repo_err(
            "record attachments",
            &RepositoryError::InstanceNotFound { id: id.clone() },
        )),
    }
}
