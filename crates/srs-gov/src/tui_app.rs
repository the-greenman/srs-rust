use anyhow::{bail, Result};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::time::Duration;

use crate::tui_data::{load_app_state, refresh_records};
use crate::tui_input::key_to_action;
use crate::tui_render::render;
use crate::tui_state::{reduce, Action};

pub fn run_tui(repo: &str, smoke: bool) -> Result<()> {
    if smoke {
        return smoke_first_frame(repo);
    }

    let _session = TerminalSession::enter()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(repo, &mut terminal);
    terminal.show_cursor()?;
    result
}

fn smoke_first_frame(repo: &str) -> Result<()> {
    let state = load_app_state(repo)?;
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render(frame, &state))?;
    let buffer = terminal.backend().buffer();
    if !buffer.content().iter().any(|cell| cell.symbol() != " ") {
        bail!("tui smoke rendered a blank first frame");
    }
    println!(
        "srs-gov tui smoke ok: {} sections, {} records",
        state.sections.len(),
        state.records.len()
    );
    Ok(())
}

fn run_loop(repo: &str, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut state = load_app_state(repo)?;

    while !state.quit {
        terminal.draw(|frame| render(frame, &state))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        let before_section = state.section_index;
        let before_search = state.search_query.clone();
        let before_show_all = state.show_all;
        let before_sort = state.newest_first;

        let Some(action) = key_to_action(key, state.focus) else {
            continue;
        };
        let refresh_after = matches!(
            action,
            Action::SubmitSearch | Action::ToggleShowAll | Action::ToggleSort
        );
        reduce(&mut state, action);

        if state.section_index != before_section
            || refresh_after
            || state.search_query != before_search && state.search_query.is_empty()
            || state.show_all != before_show_all
            || state.newest_first != before_sort
        {
            if let Err(err) = refresh_records(repo, &mut state) {
                state.status = format!("refresh failed: {err}");
            }
        }
    }

    Ok(())
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}
