use crate::commands::{with_store, CliContext};
use crate::output;
use crate::payload::{ContextFieldPayload, ContextRecordMarkdownPayload, ContextRecordPayload};
use anyhow::Result;
use clap::Subcommand;
use srs_core::types::relation_type_definition::RelationTypeCategory;
use srs_repository::context_query_service::{
    self, ContextProjection, FieldContextQuery, RecordContextQuery,
};

#[derive(Subcommand)]
pub enum ContextCommand {
    /// Assemble context for a single field: current value, aiGuidance
    Field {
        /// Record instance ID
        record_id: String,
        /// Field ID
        field_id: String,
    },
    /// Assemble context for a record: field values, inbound and outbound relations with
    /// neighbours inline; with the global --container <ID>, also its arrangement subtree there
    Record {
        /// Record instance ID
        record_id: String,
        /// Omit edges whose relation type has this category (repeatable), e.g.
        /// `--exclude-category composition --exclude-category sequence` drops structural edges
        #[arg(long = "exclude-category", value_name = "CATEGORY", value_parser = clap::value_parser!(RelationTypeCategory))]
        exclude_category: Vec<RelationTypeCategory>,
        /// How much of each neighbour to inline: `full` (whole record, default), `card`
        /// (id, label, type, lifecycle state, one summary line) or `label` (id and label)
        #[arg(long = "projection", value_name = "PROJECTION", default_value = "full")]
        projection: ContextProjection,
        /// Return the context as compact markdown in `rendered` (card projection unless
        /// `--projection label`): one line per edge with label, type and instance id
        #[arg(long = "markdown")]
        markdown: bool,
    },
}

pub fn dispatch(ctx: CliContext, cmd: ContextCommand) -> Result<String> {
    match cmd {
        ContextCommand::Field {
            record_id,
            field_id,
        } => cmd_context_field(ctx, record_id, field_id),
        ContextCommand::Record {
            record_id,
            exclude_category,
            projection,
            markdown: true,
        } => cmd_context_record_markdown(ctx, record_id, exclude_category, projection),
        ContextCommand::Record {
            record_id,
            exclude_category,
            projection,
            markdown: false,
        } => cmd_context_record(ctx, record_id, exclude_category, projection),
    }
}

fn cmd_context_field(ctx: CliContext, record_id: String, field_id: String) -> Result<String> {
    with_store(
        &ctx,
        |store| match context_query_service::get_field_context(
            store,
            FieldContextQuery {
                record_id: record_id.clone(),
                field_id: field_id.clone(),
            },
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
                    tagged_chunks: result.tagged_chunks,
                },
            ),
            Err(e) => Ok(output::err("context field", vec![e.to_string()])),
        },
    )
}

fn cmd_context_record(
    ctx: CliContext,
    record_id: String,
    exclude_relation_categories: Vec<RelationTypeCategory>,
    projection: ContextProjection,
) -> Result<String> {
    with_store(
        &ctx,
        |store| match context_query_service::get_record_context(
            store,
            RecordContextQuery {
                record_id: record_id.clone(),
                container_id: ctx.container_id.clone(),
                exclude_relation_categories,
                projection,
            },
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
                    container_id: result.container_id,
                    entry: result.entry,
                    subtree: result.subtree,
                    tagged_chunks: result.tagged_chunks,
                    protocol_run_history: result.protocol_run_history,
                },
            ),
            Err(e) => Ok(output::err("context record", vec![e.to_string()])),
        },
    )
}

fn cmd_context_record_markdown(
    ctx: CliContext,
    record_id: String,
    exclude_relation_categories: Vec<RelationTypeCategory>,
    projection: ContextProjection,
) -> Result<String> {
    with_store(
        &ctx,
        |store| match context_query_service::render_record_context_markdown(
            store,
            RecordContextQuery {
                record_id: record_id.clone(),
                container_id: ctx.container_id.clone(),
                exclude_relation_categories,
                projection,
            },
        ) {
            Ok(rendered) => output::serialize(
                "context record",
                ContextRecordMarkdownPayload {
                    record_id: record_id.clone(),
                    rendered,
                },
            ),
            Err(e) => Ok(output::err("context record", vec![e.to_string()])),
        },
    )
}
