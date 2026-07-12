use crate::commands::{with_store, CliContext, FederationCommand};
use crate::output;
use crate::payload::{
    FederationAppendEventPayload, FederationEventPayload, FederationEventsListPayload,
    FederationRegistryEntryPayload, FederationResolvePayload,
};
use anyhow::Result;
use srs_repository::federation_service::{
    append_federation_event, list_federation_events, resolve_repository,
    AppendFederationEventInput, ListFederationEventsFilter, ListFederationEventsInput,
    ResolveRepositoryInput,
};

pub fn dispatch(ctx: CliContext, cmd: FederationCommand) -> Result<String> {
    match cmd {
        FederationCommand::Resolve { repository_id } => cmd_federation_resolve(ctx, repository_id),
        FederationCommand::EventsList {
            source,
            target,
            kind,
        } => {
            let input = ListFederationEventsInput {
                filter: ListFederationEventsFilter {
                    source_repository_id: source,
                    target_repository_id: target,
                    kind,
                },
            };
            cmd_federation_events_list(ctx, input)
        }
        FederationCommand::EventsAppend => cmd_federation_events_append(ctx),
    }
}

fn cmd_federation_resolve(ctx: CliContext, repository_id: String) -> Result<String> {
    let result = with_store(&ctx, |store| {
        Ok(resolve_repository(
            store,
            ResolveRepositoryInput { repository_id },
        )?)
    })?;
    output::serialize(
        "federation resolve",
        FederationResolvePayload {
            found: result.found,
            registry_id: result.registry_id,
            entry: result.entry.map(FederationRegistryEntryPayload::from),
        },
    )
}

fn cmd_federation_events_list(ctx: CliContext, input: ListFederationEventsInput) -> Result<String> {
    let result = with_store(&ctx, |store| Ok(list_federation_events(store, input)?))?;
    output::serialize(
        "federation events list",
        FederationEventsListPayload {
            repository_id: result.repository_id,
            events: result
                .events
                .into_iter()
                .map(FederationEventPayload::from)
                .collect(),
            total_count: result.total_count,
            filtered_count: result.filtered_count,
        },
    )
}

fn cmd_federation_events_append(ctx: CliContext) -> Result<String> {
    let input: AppendFederationEventInput = serde_json::from_reader(std::io::stdin())?;
    let result = with_store(&ctx, |store| Ok(append_federation_event(store, input)?))?;
    output::serialize(
        "federation events append",
        FederationAppendEventPayload {
            event_id: result.event_id,
            total_events: result.total_events,
        },
    )
}
