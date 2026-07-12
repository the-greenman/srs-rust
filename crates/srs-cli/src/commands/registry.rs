use crate::commands::{CliContext, RegistryCommand};
use crate::output;
use crate::payload::{RegistryEntryPayload, RegistryGetPayload, RegistryListPayload};
use anyhow::Result;
use srs_repository::registry_service::{
    get_registry_entry, list_registry, GetRegistryEntryInput, ListRegistryInput, RegistryListFilter,
};
use std::path::PathBuf;

pub fn dispatch(_ctx: CliContext, cmd: RegistryCommand) -> Result<String> {
    match cmd {
        RegistryCommand::List {
            path,
            publisher,
            tag,
        } => cmd_registry_list(path, publisher, tag),
        RegistryCommand::Get { path, package_name } => cmd_registry_get(path, package_name),
    }
}

fn cmd_registry_list(
    path: PathBuf,
    publisher: Option<String>,
    tag: Option<String>,
) -> Result<String> {
    let result = list_registry(ListRegistryInput {
        path,
        filter: RegistryListFilter { publisher, tag },
    })
    .map_err(anyhow::Error::from)?;
    let entries = result
        .entries
        .into_iter()
        .map(RegistryEntryPayload::from)
        .collect();
    output::serialize(
        "registry list",
        RegistryListPayload {
            registry_id: result.registry_id,
            registry_name: result.registry_name,
            catalog_version: result.catalog_version,
            updated_at: result.updated_at,
            homepage: result.homepage,
            entries,
            total_count: result.total_count,
            filtered_count: result.filtered_count,
        },
    )
}

fn cmd_registry_get(path: PathBuf, package_name: String) -> Result<String> {
    let result = get_registry_entry(GetRegistryEntryInput { path, package_name })
        .map_err(anyhow::Error::from)?;
    output::serialize(
        "registry get",
        RegistryGetPayload {
            registry_id: result.registry_id,
            entry: RegistryEntryPayload::from(result.entry),
        },
    )
}
