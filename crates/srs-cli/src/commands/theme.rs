use crate::commands::{with_store, CliContext, ThemeCommand};
use crate::output;
use crate::payload::{ThemeDeletePayload, ThemeListPayload, ThemePayload};
use anyhow::Result;
use srs_core::types::theme::Theme;
use srs_repository::error::RepositoryError;
use srs_repository::theme_service::{
    create_theme_normalized, delete_theme, get_theme_by_id, list_themes_summary, update_theme,
    CreateThemeResult, DeleteThemeResult, GetThemeResult,
};

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
        Err(e) => Ok(output::any_err("theme list", &e)),
    }
}

fn cmd_theme_get(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(get_theme_by_id(store, &id)?))? {
        GetThemeResult::Found(theme) => {
            output::serialize("theme get", ThemePayload { theme: *theme })
        }
        GetThemeResult::NotFound => Ok(output::repo_err(
            "theme get",
            &RepositoryError::ThemeNotFound {
                theme_id: id.clone(),
            },
        )),
    }
}

fn cmd_theme_create(ctx: CliContext, package: Option<String>) -> Result<String> {
    let raw = crate::input::value_from_stdin("Theme")?;
    match with_store(&ctx, |store| {
        Ok(create_theme_normalized(store, raw, package.clone())?)
    }) {
        Ok(CreateThemeResult { theme }) => {
            output::serialize("theme create", ThemePayload { theme })
        }
        Err(e) => Ok(output::any_err("theme create", &e)),
    }
}

fn cmd_theme_update(ctx: CliContext, id: String) -> Result<String> {
    let theme: Theme = crate::input::from_stdin("Theme")?;
    match with_store(&ctx, |store| Ok(update_theme(store, &id, theme)?)) {
        Ok(result) => output::serialize(
            "theme update",
            ThemePayload {
                theme: result.theme,
            },
        ),
        Err(e) => Ok(output::any_err("theme update", &e)),
    }
}

fn cmd_theme_delete(ctx: CliContext, id: String) -> Result<String> {
    match with_store(&ctx, |store| Ok(delete_theme(store, &id)?)) {
        Ok(DeleteThemeResult { id }) => {
            output::serialize("theme delete", ThemeDeletePayload { id })
        }
        Err(e) => Ok(output::any_err("theme delete", &e)),
    }
}
