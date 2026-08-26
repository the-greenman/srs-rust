use crate::commands::{with_store, CliContext, ProtocolCommand, ProtocolRunCommand};
use crate::output;
use crate::payload::{
    self as payload, ProtocolDeletePayload, ProtocolFindByTargetTypePayload, ProtocolListEntry,
    ProtocolListPayload, ProtocolPayload, ProtocolRunListEntry, ProtocolRunListPayload,
    ProtocolRunPayload, ProtocolStageEntry, ProtocolStagesPayload, ProtocolValidatePayload,
};
use anyhow::Result;
use srs_repository::error::RepositoryError;
use srs_repository::protocol_run_service::{
    abandon_run, advance_stage, complete_run, create_run, get_run, list_runs, AdvanceStageInput,
    CreateRunInput, GetRunResult, RunListFilter,
};
use srs_repository::protocol_service::{
    delete_protocol, export_protocol, find_protocol_by_target_type, get_protocol_by_id,
    import_protocol, list_protocol_stages, list_protocols, update_protocol,
    validate_protocol_definition, GetProtocolResult, ImportProtocolInput,
};

pub fn dispatch(ctx: CliContext, cmd: ProtocolCommand) -> Result<String> {
    match cmd {
        ProtocolCommand::List { json: _ } => cmd_protocol_list(ctx),
        ProtocolCommand::Get { id, json: _ } => cmd_protocol_get(ctx, id),
        ProtocolCommand::Stages { id, json: _ } => cmd_protocol_stages(ctx, id),
        ProtocolCommand::Validate { id, json: _ } => cmd_protocol_validate(ctx, id),
        ProtocolCommand::Export { id, json: _ } => cmd_protocol_export(ctx, id),
        ProtocolCommand::Create { package, json: _ } => cmd_protocol_create(ctx, package),
        ProtocolCommand::Import { package, json: _ } => cmd_protocol_import(ctx, package),
        ProtocolCommand::Update { id } => cmd_protocol_update(ctx, id),
        ProtocolCommand::Delete { id } => cmd_protocol_delete(ctx, id),
        ProtocolCommand::FindByTargetType { type_id } => {
            cmd_protocol_find_by_target_type(ctx, type_id)
        }
        ProtocolCommand::Run(run_cmd) => dispatch_run(ctx, run_cmd),
    }
}

pub fn dispatch_run(ctx: CliContext, cmd: ProtocolRunCommand) -> Result<String> {
    match cmd {
        ProtocolRunCommand::Create => cmd_run_create(ctx),
        ProtocolRunCommand::Advance => cmd_run_advance(ctx),
        ProtocolRunCommand::Get { run_id } => cmd_run_get(ctx, run_id),
        ProtocolRunCommand::List {
            protocol_id,
            container_id,
            status,
        } => cmd_run_list(ctx, protocol_id, container_id, status),
        ProtocolRunCommand::Complete { run_id } => cmd_run_complete(ctx, run_id),
        ProtocolRunCommand::Abandon { run_id } => cmd_run_abandon(ctx, run_id),
    }
}

/// Read a JSON object from stdin, accepting either a bare object or `{ "protocol": { ... } }`.
fn read_protocol_value_from_stdin() -> Result<serde_json::Value> {
    let raw = crate::input::value_from_stdin("protocol")?;
    Ok(raw.get("protocol").cloned().unwrap_or(raw))
}

fn cmd_protocol_list(ctx: CliContext) -> Result<String> {
    let protocols = with_store(&ctx, |store| Ok(list_protocols(store)?))?;

    let protocols = protocols
        .into_iter()
        .map(|p| ProtocolListEntry {
            protocol_id: p.protocol_id,
            namespace: p.protocol_namespace,
            name: p.protocol_name,
            version: p.protocol_version,
            stage_count: p.stage_count,
            source_package: p.source_package,
        })
        .collect();

    output::serialize("protocol list", ProtocolListPayload { protocols })
}

fn cmd_protocol_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_protocol_by_id(store, &id)?))? {
        GetProtocolResult::Found(protocol) => {
            output::serialize("protocol get", ProtocolPayload { protocol })
        }
        GetProtocolResult::NotFound => Ok(output::err(
            "protocol get",
            vec![format!("Protocol '{}' not found", id)],
        )),
    }
}

fn cmd_protocol_stages(ctx: CliContext, id: String) -> Result<String> {
    let stages = match with_store(&ctx, |store| Ok(list_protocol_stages(store, &id)?)) {
        Ok(stages) => stages,
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol stages",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            return Err(e);
        }
    };

    let stages = stages
        .into_iter()
        .map(|s| ProtocolStageEntry {
            stage_id: s.stage_id,
            name: s.name,
            purpose: s.purpose,
            order: s.order,
            depends_on: s.depends_on,
            question: s.question,
            completion_criteria: s.completion_criteria,
            contributes_to: s.contributes_to.map(|refs| {
                refs.into_iter()
                    .map(|r| payload::FieldRef {
                        field_id: r.field_id,
                        type_id: r.type_id,
                    })
                    .collect()
            }),
            ai_guidance: s.ai_guidance,
            output_type: s.output_type,
        })
        .collect();

    output::serialize("protocol stages", ProtocolStagesPayload { stages })
}

fn cmd_protocol_validate(ctx: CliContext, id: String) -> Result<String> {
    let result = match with_store(&ctx, |store| Ok(validate_protocol_definition(store, &id)?)) {
        Ok(result) => result,
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol validate",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            return Err(e);
        }
    };

    output::serialize(
        "protocol validate",
        ProtocolValidatePayload {
            protocol_id: result.protocol_id,
            valid: result.valid,
            diagnostics: result.diagnostics,
        },
    )
}

fn cmd_protocol_export(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(export_protocol(store, &id)?))? {
        GetProtocolResult::Found(protocol) => {
            output::serialize("protocol export", ProtocolPayload { protocol })
        }
        GetProtocolResult::NotFound => Ok(output::err(
            "protocol export",
            vec![format!("Protocol '{}' not found", id)],
        )),
    }
}

fn cmd_protocol_write(
    ctx: CliContext,
    package: Option<String>,
    label: &'static str,
) -> Result<String> {
    let raw = crate::input::value_from_stdin("protocol")?;
    let result = with_store(&ctx, |store| {
        Ok(import_protocol(
            store,
            ImportProtocolInput { raw },
            package,
        )?)
    })?;
    output::serialize(
        label,
        ProtocolPayload {
            protocol: result.protocol,
        },
    )
}

fn cmd_protocol_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    cmd_protocol_write(ctx, package, "protocol create")
}

fn cmd_protocol_import(ctx: CliContext, package: Option<String>) -> Result<String> {
    cmd_protocol_write(ctx, package, "protocol import")
}

fn cmd_protocol_update(ctx: CliContext, id: String) -> Result<String> {
    let value = read_protocol_value_from_stdin()?;

    let result = match with_store(&ctx, |store| {
        Ok(update_protocol(store, &id, value.clone())?)
    }) {
        Ok(r) => r,
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol update",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            return Err(e);
        }
    };

    output::serialize(
        "protocol update",
        ProtocolPayload {
            protocol: result.protocol,
        },
    )
}

fn cmd_protocol_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_protocol(store, &id)?)) {
        Ok(result) => output::serialize(
            "protocol delete",
            ProtocolDeletePayload {
                protocol_id: result.protocol_id,
            },
        ),
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol delete",
                    vec![format!("Protocol '{}' not found", id)],
                ));
            }
            Err(e)
        }
    }
}

fn cmd_protocol_find_by_target_type(ctx: CliContext, type_id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(find_protocol_by_target_type(store, &type_id)?)
    })? {
        Some(result) => {
            let stages = result
                .stages
                .into_iter()
                .map(|s| ProtocolStageEntry {
                    stage_id: s.stage_id,
                    name: s.name,
                    purpose: s.purpose,
                    order: s.order,
                    depends_on: s.depends_on,
                    question: s.question,
                    completion_criteria: s.completion_criteria,
                    contributes_to: s.contributes_to.map(|refs| {
                        refs.into_iter()
                            .map(|r| payload::FieldRef {
                                field_id: r.field_id,
                                type_id: r.type_id,
                            })
                            .collect()
                    }),
                    ai_guidance: s.ai_guidance,
                    output_type: s.output_type,
                })
                .collect();
            output::serialize(
                "protocol find-by-target-type",
                ProtocolFindByTargetTypePayload {
                    protocol_id: result.protocol_id,
                    protocol_name: result.protocol_name,
                    stages,
                    diagnostics: result.diagnostics,
                },
            )
        }
        None => Ok(output::err(
            "protocol find-by-target-type",
            vec![format!("No protocol found with target type '{}'", type_id)],
        )),
    }
}

// ── Protocol run handlers ─────────────────────────────────────────────────────

fn cmd_run_create(ctx: CliContext) -> Result<String> {
    let input: CreateRunInput = serde_json::from_reader(std::io::stdin())?;
    let result = with_store(&ctx, |store| Ok(create_run(store, input)?))?;
    let run = serde_json::to_value(&result.run)?;
    output::serialize("protocol run create", ProtocolRunPayload { run })
}

fn cmd_run_advance(ctx: CliContext) -> Result<String> {
    let input: AdvanceStageInput = serde_json::from_reader(std::io::stdin())?;
    match with_store(&ctx, |store| Ok(advance_stage(store, input)?)) {
        Ok(result) => {
            let run = serde_json::to_value(&result.run)?;
            output::serialize("protocol run advance", ProtocolRunPayload { run })
        }
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol run advance",
                    vec!["Protocol run not found".to_string()],
                ));
            }
            if let Some(RepositoryError::RunInvalidState { run_id, message }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "protocol run advance",
                    vec![format!(
                        "Protocol run '{}' cannot be advanced: {}",
                        run_id, message
                    )],
                ));
            }
            Err(e)
        }
    }
}

fn cmd_run_get(ctx: CliContext, run_id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_run(store, &run_id)?))? {
        GetRunResult::Found(run) => {
            let run = serde_json::to_value(&*run)?;
            output::serialize("protocol run get", ProtocolRunPayload { run })
        }
        GetRunResult::NotFound => Ok(output::err(
            "protocol run get",
            vec![format!("Protocol run '{}' not found", run_id)],
        )),
    }
}

fn cmd_run_list(
    ctx: CliContext,
    protocol_id: Option<String>,
    container_id: Option<String>,
    status: Option<String>,
) -> Result<String> {
    let filter = RunListFilter {
        protocol_id,
        container_id,
        status,
    };
    let runs = with_store(&ctx, |store| Ok(list_runs(store, filter)?))?
        .into_iter()
        .map(ProtocolRunListEntry::from)
        .collect();
    output::serialize("protocol run list", ProtocolRunListPayload { runs })
}

fn cmd_run_complete(ctx: CliContext, run_id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(complete_run(store, &run_id)?)) {
        Ok(result) => {
            let run = serde_json::to_value(&result.run)?;
            output::serialize("protocol run complete", ProtocolRunPayload { run })
        }
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol run complete",
                    vec![format!("Protocol run '{}' not found", run_id)],
                ));
            }
            if let Some(RepositoryError::RunInvalidState { message, .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "protocol run complete",
                    vec![format!(
                        "Protocol run '{}' cannot be completed: {}",
                        run_id, message
                    )],
                ));
            }
            Err(e)
        }
    }
}

fn cmd_run_abandon(ctx: CliContext, run_id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(abandon_run(store, &run_id)?)) {
        Ok(result) => {
            let run = serde_json::to_value(&result.run)?;
            output::serialize("protocol run abandon", ProtocolRunPayload { run })
        }
        Err(e) => {
            if let Some(RepositoryError::NotFound { .. }) = e.downcast_ref::<RepositoryError>() {
                return Ok(output::err(
                    "protocol run abandon",
                    vec![format!("Protocol run '{}' not found", run_id)],
                ));
            }
            if let Some(RepositoryError::RunInvalidState { message, .. }) =
                e.downcast_ref::<RepositoryError>()
            {
                return Ok(output::err(
                    "protocol run abandon",
                    vec![format!(
                        "Protocol run '{}' cannot be abandoned: {}",
                        run_id, message
                    )],
                ));
            }
            Err(e)
        }
    }
}
