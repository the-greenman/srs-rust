use crate::commands::{with_store, CliContext, ThemeCommand};
use crate::output;
use crate::payload::{ThemeDeletePayload, ThemeListPayload, ThemePayload};
use anyhow::Result;
use srs_core::types::theme::Theme;
use srs_repository::theme_service::{
    create_theme, delete_theme, get_theme_by_id, list_themes_summary, update_theme,
    CreateThemeResult, DeleteThemeResult, GetThemeResult,
};
use std::io::{self, Read};

pub fn dispatch(ctx: CliContext, cmd: ThemeCommand) -> Result<String> {
    match cmd {
        ThemeCommand::List { namespace } => cmd_theme_list(ctx, namespace),
        ThemeCommand::Get { id } => cmd_theme_get(ctx, id),
        ThemeCommand::Create { package } => cmd_theme_create(ctx, package),
        ThemeCommand::Update { id } => cmd_theme_update(ctx, id),
        ThemeCommand::Delete { id } => cmd_theme_delete(ctx, id),
    }
}

fn cmd_theme_list(ctx: CliContext, namespace: Option<String>) -> Result<String> {
    match with_store(&ctx, |store| Ok(list_themes_summary(store)?)) {
        Ok(mut themes) => {
            if let Some(ns) = namespace {
                themes.retain(|t| t.namespace == ns);
            }
            output::serialize("theme list", ThemeListPayload { themes })
        }
        Err(e) => Ok(output::err("theme list", vec![e.to_string()])),
    }
}

fn cmd_theme_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_theme_by_id(store, &id)?))? {
        GetThemeResult::Found(theme) => {
            output::serialize("theme get", ThemePayload { theme: *theme })
        }
        GetThemeResult::NotFound => Ok(output::err(
            "theme get",
            vec![format!("theme not found: {id}")],
        )),
    }
}

fn cmd_theme_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let theme: Theme = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse Theme JSON: {e}"))?;
    match with_store(&ctx, |store| {
        Ok(create_theme(store, theme, package.clone())?)
    }) {
        Ok(CreateThemeResult { theme }) => {
            output::serialize("theme create", ThemePayload { theme })
        }
        Err(e) => Ok(output::err("theme create", vec![e.to_string()])),
    }
}

fn cmd_theme_update(ctx: CliContext, id: String) -> Result<String> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let theme: Theme = serde_json::from_str(&stdin)
        .map_err(|e| anyhow::anyhow!("Failed to parse Theme JSON: {e}"))?;
    match with_store(&ctx, |store| Ok(update_theme(store, &id, theme)?)) {
        Ok(result) => output::serialize(
            "theme update",
            ThemePayload {
                theme: result.theme,
            },
        ),
        Err(e) => Ok(output::err("theme update", vec![e.to_string()])),
    }
}

fn cmd_theme_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_theme(store, &id)?)) {
        Ok(DeleteThemeResult { id }) => {
            output::serialize("theme delete", ThemeDeletePayload { id })
        }
        Err(e) => Ok(output::err("theme delete", vec![e.to_string()])),
    }
}
