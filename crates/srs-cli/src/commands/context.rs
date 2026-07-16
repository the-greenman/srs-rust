use crate::commands::{with_store, CliContext};
use crate::output;
use crate::payload::{ContextFieldPayload, ContextRecordPayload, ContextRevisionTracePayload};
use anyhow::Result;
use clap::Subcommand;
use srs_repository::context_query_service::{self, FieldContextQuery, RecordContextQuery, RevisionTraceQuery};

#[derive(Subcommand)]
pub enum ContextCommand {
    /// Assemble context for a single field: current value, revision history, aiGuidance
    Field {
        /// Record instance ID
        record_id: String,
        /// Field ID
        field_id: String,
    },
    /// Assemble context for a record: all field values and relations
    Record {
        /// Record instance ID
        record_id: String,
    },
    /// Trace a revision: value, source refs, and prior revision chain
    Revision {
        /// Record instance ID
        record_id: String,
        /// Field ID
        field_id: String,
        /// Revision ID to trace
        revision_id: String,
    },
}

pub fn dispatch(ctx: CliContext, cmd: ContextCommand) -> Result<String> {
    match cmd {
        ContextCommand::Field { record_id, field_id } => cmd_context_field(ctx, record_id, field_id),
        ContextCommand::Record { record_id } => cmd_context_record(ctx, record_id),
        ContextCommand::Revision { record_id, field_id, revision_id } => {
            cmd_context_revision(ctx, record_id, field_id, revision_id)
        }
    }
}

fn cmd_context_field(ctx: CliContext, record_id: String, field_id: String) -> Result<String> {
    with_store(&ctx, |store| {
        match context_query_service::get_field_context(
            store,
            FieldContextQuery { record_id: record_id.clone(), field_id: field_id.clone() },
        ) {
            Ok(result) => output::serialize(
                "context field",
                ContextFieldPayload {
                    record_id: result.record_id,
                    field_id: result.field_id,
                    field_name: result.field_name,
                    field_namespace: result.field_namespace,
                    ai_guidance: result.ai_guidance,
                    current_value: result.current_value,
                    revisions: result.revisions,
                    tagged_chunks: result.tagged_chunks,
                },
            ),
            Err(e) => Ok(output::err("context field", vec![e.to_string()])),
        }
    })
}

fn cmd_context_record(ctx: CliContext, record_id: String) -> Result<String> {
    with_store(&ctx, |store| {
        match context_query_service::get_record_context(
            store,
            RecordContextQuery { record_id: record_id.clone() },
        ) {
            Ok(result) => output::serialize(
                "context record",
                ContextRecordPayload {
                    record_id: result.record_id,
                    type_id: result.type_id,
                    type_name: result.type_name,
                    type_namespace: result.type_namespace,
                    display_label: result.display_label,
                    field_values: result.field_values,
                    relations: result.relations,
                    tagged_chunks: result.tagged_chunks,
                    protocol_run_history: result.protocol_run_history,
                },
            ),
            Err(e) => Ok(output::err("context record", vec![e.to_string()])),
        }
    })
}

fn cmd_context_revision(
    ctx: CliContext,
    record_id: String,
    field_id: String,
    revision_id: String,
) -> Result<String> {
    with_store(&ctx, |store| {
        match context_query_service::get_revision_trace(
            store,
            RevisionTraceQuery {
                record_id: record_id.clone(),
                field_id: field_id.clone(),
                revision_id: revision_id.clone(),
            },
        ) {
            Ok(result) => output::serialize(
                "context revision",
                ContextRevisionTracePayload {
                    record_id: result.record_id,
                    field_id: result.field_id,
                    revision: result.revision,
                    prior_chain: result.prior_chain,
                },
            ),
            Err(e) => Ok(output::err("context revision", vec![e.to_string()])),
        }
    })
}
