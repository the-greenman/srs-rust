use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui_state::{AppState, Focus};

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    render_header(frame, rows[0], state);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(28),
            Constraint::Percentage(38),
            Constraint::Min(24),
        ])
        .split(rows[1]);

    render_sections(frame, body[0], state);
    render_records(frame, body[1], state);
    render_detail(frame, body[2], state);
    render_footer(frame, rows[2], state);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let filters = format!(
        "{} | {} | search: {}",
        if state.newest_first {
            "newest first"
        } else {
            "oldest first"
        },
        if state.show_all {
            "show all"
        } else {
            "hide default-hidden"
        },
        if state.search_query.is_empty() {
            "-"
        } else {
            &state.search_query
        }
    );
    let text = vec![Line::from(vec![
        Span::styled(
            "srs-gov ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &state.repo_title,
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(filters, Style::default().fg(Color::Gray)),
    ])];
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_sections(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let prefix = if index == state.section_index {
                "> "
            } else {
                "  "
            };
            ListItem::new(format!("{prefix}{}", section.label))
        })
        .collect();
    let block = pane_block("Sections", state.focus == Focus::Sections);
    frame.render_widget(List::new(items).block(block), area);
}

fn render_records(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let id = &record.instance_id[..8.min(record.instance_id.len())];
            let state_text = record.lifecycle_state.as_deref().unwrap_or("-");
            let prefix = if index == state.record_index {
                "> "
            } else {
                "  "
            };
            ListItem::new(format!("{prefix}{id}  {}  [{state_text}]", record.label))
        })
        .collect();
    let title = state
        .active_document_view_id
        .as_deref()
        .map(short_id)
        .map(|id| format!("Records view {id}"))
        .unwrap_or_else(|| "Records".to_string());
    let block = pane_block_owned(title, state.focus == Focus::Records);
    frame.render_widget(List::new(items).block(block), area);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let body = if state.focus == Focus::Search {
        format!("Search\n\n{}", state.search_query)
    } else if let Some(record) = state.selected_record() {
        let tags = if record.tags.is_empty() {
            "-".to_string()
        } else {
            record.tags.join(", ")
        };
        let mut body = format!(
            "{}\n\nid: {}\ntype: {} @{}\nstate: {}\ntags: {}\ncreated: {}\n",
            record.label,
            record.instance_id,
            record.type_id,
            record.type_version,
            record.lifecycle_state.as_deref().unwrap_or("-"),
            tags,
            record.created_at.as_deref().unwrap_or("-"),
        );
        if record.detail_rows.is_empty() {
            body.push_str("\nNo schema fields resolved");
        } else {
            body.push('\n');
            for row in &record.detail_rows {
                let marker = if row.required { "*" } else { " " };
                let value = row.value.as_deref().unwrap_or("(empty)");
                body.push_str(&format!("{marker} {}: {value}\n", row.label));
            }
        }
        if !state.diagnostics.is_empty() {
            body.push_str("\nDiagnostics\n");
            for diagnostic in &state.diagnostics {
                body.push_str("- ");
                body.push_str(diagnostic);
                body.push('\n');
            }
        }
        body
    } else {
        "No record selected".to_string()
    };

    frame.render_widget(
        Paragraph::new(body)
            .block(pane_block(
                "Detail",
                matches!(state.focus, Focus::Detail | Focus::Search),
            ))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let bindings = AppState::keybindings()
        .iter()
        .map(|binding| format!("{} {}", binding.key, binding.action))
        .collect::<Vec<_>>()
        .join("  ");
    let text = format!("{}  |  {}", state.status, bindings);
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn pane_block(title: &'static str, active: bool) -> Block<'static> {
    pane_block_owned(title.to_string(), active)
}

fn pane_block_owned(title: String, active: bool) -> Block<'static> {
    let style = if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style)
}

fn short_id(id: &str) -> &str {
    &id[..8.min(id.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_state::{RecordItem, SectionItem};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn first_frame_render_is_nonblank() {
        let mut state = AppState::new(
            "Example Governance",
            vec![SectionItem {
                key: "decision_log".to_string(),
                label: "Decision Log".to_string(),
                container_id: Some("c-1".to_string()),
            }],
        );
        state.set_records(vec![RecordItem {
            instance_id: "record-123".to_string(),
            label: "Adopt the policy".to_string(),
            lifecycle_state: Some("ratified".to_string()),
            tags: vec!["tooling".to_string()],
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            type_id: "type-decision".to_string(),
            type_version: 1,
            detail_rows: vec![crate::tui_state::DetailRow {
                label: "Decision Statement".to_string(),
                value: Some("Adopt the policy as written".to_string()),
                required: true,
                order: 1,
            }],
            record: serde_json::json!({}),
        }]);

        let backend = TestBackend::new(90, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal.draw(|frame| render(frame, &state)).expect("draw");

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.symbol() != " "));
        assert!(format!("{buffer:?}").contains("Decision Log"));
        assert!(format!("{buffer:?}").contains("Decision Statement"));
    }
}
