use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::tui_state::{Action, Focus};

pub fn key_to_action(key: KeyEvent, focus: Focus) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    match (focus, key.code) {
        (_, KeyCode::Char('q')) => Some(Action::Quit),
        (_, KeyCode::Up) | (_, KeyCode::Char('k')) => Some(Action::Up),
        (_, KeyCode::Down) | (_, KeyCode::Char('j')) => Some(Action::Down),
        (_, KeyCode::Tab) => Some(Action::NextPane),
        (_, KeyCode::Enter) if focus == Focus::Search => Some(Action::SubmitSearch),
        (_, KeyCode::Enter) => Some(Action::Open),
        (_, KeyCode::Esc) => Some(Action::Back),
        (_, KeyCode::Char('/')) if focus != Focus::Search => Some(Action::SearchMode),
        (_, KeyCode::Char('s')) if focus != Focus::Search => Some(Action::ToggleSort),
        (_, KeyCode::Char('a')) if focus != Focus::Search => Some(Action::ToggleShowAll),
        (Focus::Search, KeyCode::Backspace) => Some(Action::SearchBackspace),
        (Focus::Search, KeyCode::Char(ch)) => Some(Action::SearchInput(ch)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_navigation_keys() {
        assert_eq!(
            key_to_action(key(KeyCode::Char('j')), Focus::Records),
            Some(Action::Down)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Char('k')), Focus::Records),
            Some(Action::Up)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Tab), Focus::Sections),
            Some(Action::NextPane)
        );
    }

    #[test]
    fn search_focus_turns_chars_into_search_input() {
        assert_eq!(
            key_to_action(key(KeyCode::Char('x')), Focus::Search),
            Some(Action::SearchInput('x'))
        );
        assert_eq!(
            key_to_action(key(KeyCode::Enter), Focus::Search),
            Some(Action::SubmitSearch)
        );
    }
}
