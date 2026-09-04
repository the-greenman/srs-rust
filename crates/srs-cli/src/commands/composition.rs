use crate::commands::{with_store, CliContext, CompositionCommand};
use crate::output;
use crate::payload::{
    CompositionDeletePayload, CompositionListPayload, CompositionPayload,
    CompositionsForContainerPayload,
};
use anyhow::Result;
use srs_core::types::view::Composition;
use srs_repository::view_service::{
    compositions_for_container_summary, create_composition_normalized, delete_composition,
    get_composition_by_id, list_compositions_summary, update_composition, CompositionListFilter,
    CreateCompositionResult, DeleteCompositionResult, GetCompositionResult,
};

pub fn dispatch(ctx: CliContext, cmd: CompositionCommand) -> Result<String> {
    match cmd {
        CompositionCommand::List {
            namespace,
            name,
            container_type,
            root_type,
        } => cmd_composition_list(ctx, namespace, name, container_type, root_type),
        CompositionCommand::Get { id } => cmd_composition_get(ctx, id),
        CompositionCommand::Create { package } => cmd_composition_create(ctx, package),
        CompositionCommand::Update { id } => cmd_composition_update(ctx, id),
        CompositionCommand::Delete { id } => cmd_composition_delete(ctx, id),
        CompositionCommand::ListForContainer { container_id } => {
            cmd_composition_list_for_container(ctx, container_id)
        }
    }
}

fn cmd_composition_list(
    ctx: CliContext,
    namespace: Option<String>,
    name: Option<String>,
    container_type: Option<String>,
    root_type: Option<String>,
) -> Result<String> {
    let filter = CompositionListFilter {
        namespace,
        name,
        container_type,
        root_type_id: root_type,
    };
    match with_store(&ctx, |store| Ok(list_compositions_summary(store, &filter)?)) {
        Ok(compositions) => {
            output::serialize("composition list", CompositionListPayload { compositions })
        }
        Err(e) => Ok(output::err("composition list", vec![e.to_string()])),
    }
}

fn cmd_composition_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_composition_by_id(store, &id)?))? {
        GetCompositionResult::Found(dv) => {
            output::serialize("composition get", CompositionPayload { composition: *dv })
        }
        GetCompositionResult::NotFound => Ok(output::err(
            "composition get",
            vec![format!("composition not found: {id}")],
        )),
    }
}

fn cmd_composition_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let raw = crate::input::value_from_stdin("Composition")?;
    match with_store(&ctx, |store| {
        Ok(create_composition_normalized(store, raw, package.clone())?)
    }) {
        Ok(CreateCompositionResult { composition }) => {
            output::serialize("composition create", CompositionPayload { composition })
        }
        Err(e) => Ok(output::err("composition create", vec![e.to_string()])),
    }
}

fn cmd_composition_update(ctx: CliContext, id: String) -> Result<String> {
    let dv: Composition = crate::input::from_stdin("Composition")?;
    match with_store(&ctx, |store| Ok(update_composition(store, &id, dv)?)) {
        Ok(result) => output::serialize(
            "composition update",
            CompositionPayload {
                composition: result.composition,
            },
        ),
        Err(e) => Ok(output::err("composition update", vec![e.to_string()])),
    }
}

fn cmd_composition_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_composition(store, &id)?)) {
        Ok(DeleteCompositionResult { id }) => {
            output::serialize("composition delete", CompositionDeletePayload { id })
        }
        Err(e) => Ok(output::err("composition delete", vec![e.to_string()])),
    }
}

fn cmd_composition_list_for_container(ctx: CliContext, container_id: String) -> Result<String> {
    match with_store(&ctx, |store| {
        Ok(compositions_for_container_summary(store, &container_id)?)
    }) {
        Ok(compositions) => output::serialize(
            "composition list-for-container",
            CompositionsForContainerPayload {
                container_id,
                compositions,
            },
        ),
        Err(e) => Ok(output::err(
            "composition list-for-container",
            vec![e.to_string()],
        )),
    }
}
