#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sections,
    Records,
    Detail,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionItem {
    pub key: String,
    pub label: String,
    pub container_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRow {
    pub label: String,
    pub value: Option<String>,
    pub required: bool,
    pub order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordItem {
    pub instance_id: String,
    pub label: String,
    pub lifecycle_state: Option<String>,
    pub tags: Vec<String>,
    pub created_at: Option<String>,
    pub type_id: String,
    pub type_version: u64,
    pub detail_rows: Vec<DetailRow>,
    pub record: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnItem {
    pub field_id: String,
    pub field_name: String,
    pub display_label: String,
    pub order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: &'static str,
    pub action: &'static str,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub repo_title: String,
    pub sections: Vec<SectionItem>,
    pub records: Vec<RecordItem>,
    pub active_document_view_id: Option<String>,
    pub columns: Vec<ColumnItem>,
    pub diagnostics: Vec<String>,
    pub section_index: usize,
    pub record_index: usize,
    pub focus: Focus,
    pub search_query: String,
    pub show_all: bool,
    pub newest_first: bool,
    pub quit: bool,
    pub status: String,
}

impl AppState {
    pub fn new(repo_title: impl Into<String>, sections: Vec<SectionItem>) -> Self {
        let status = if sections.is_empty() {
            "No governance sections discovered".to_string()
        } else {
            "Ready".to_string()
        };
        Self {
            repo_title: repo_title.into(),
            sections,
            records: Vec::new(),
            active_document_view_id: None,
            columns: Vec::new(),
            diagnostics: Vec::new(),
            section_index: 0,
            record_index: 0,
            focus: Focus::Sections,
            search_query: String::new(),
            show_all: false,
            newest_first: true,
            quit: false,
            status,
        }
    }

    pub fn set_records(&mut self, records: Vec<RecordItem>) {
        self.records = records;
        self.record_index = self.record_index.min(self.records.len().saturating_sub(1));
    }

    pub fn set_view_context(
        &mut self,
        document_view_id: Option<String>,
        columns: Vec<ColumnItem>,
        diagnostics: Vec<String>,
    ) {
        self.active_document_view_id = document_view_id;
        self.columns = columns;
        self.diagnostics = diagnostics;
    }

    pub fn selected_section(&self) -> Option<&SectionItem> {
        self.sections.get(self.section_index)
    }

    pub fn selected_record(&self) -> Option<&RecordItem> {
        self.records.get(self.record_index)
    }

    pub fn keybindings() -> &'static [KeyBinding] {
        &[
            KeyBinding {
                key: "q",
                action: "quit",
            },
            KeyBinding {
                key: "j/k",
                action: "move",
            },
            KeyBinding {
                key: "Tab",
                action: "switch pane",
            },
            KeyBinding {
                key: "Enter",
                action: "open",
            },
            KeyBinding {
                key: "Esc",
                action: "back",
            },
            KeyBinding {
                key: "/",
                action: "search",
            },
            KeyBinding {
                key: "s",
                action: "sort",
            },
            KeyBinding {
                key: "a",
                action: "show all",
            },
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    NextPane,
    Open,
    Back,
    SearchMode,
    SearchInput(char),
    SearchBackspace,
    SubmitSearch,
    ToggleSort,
    ToggleShowAll,
    Quit,
}

pub fn reduce(state: &mut AppState, action: Action) {
    match action {
        Action::Up => match state.focus {
            Focus::Sections => state.section_index = state.section_index.saturating_sub(1),
            Focus::Records => state.record_index = state.record_index.saturating_sub(1),
            Focus::Detail | Focus::Search => {}
        },
        Action::Down => match state.focus {
            Focus::Sections => {
                state.section_index =
                    (state.section_index + 1).min(state.sections.len().saturating_sub(1));
            }
            Focus::Records => {
                state.record_index =
                    (state.record_index + 1).min(state.records.len().saturating_sub(1));
            }
            Focus::Detail | Focus::Search => {}
        },
        Action::NextPane => {
            state.focus = match state.focus {
                Focus::Sections => Focus::Records,
                Focus::Records => Focus::Sections,
                Focus::Detail => Focus::Records,
                Focus::Search => Focus::Records,
            };
        }
        Action::Open => {
            if state.focus == Focus::Sections {
                state.focus = Focus::Records;
            } else if state.focus == Focus::Records && state.selected_record().is_some() {
                state.focus = Focus::Detail;
            }
        }
        Action::Back => {
            state.focus = match state.focus {
                Focus::Search => Focus::Records,
                Focus::Detail => Focus::Records,
                Focus::Records => Focus::Sections,
                Focus::Sections => Focus::Sections,
            };
        }
        Action::SearchMode => state.focus = Focus::Search,
        Action::SearchInput(ch) => {
            if state.focus == Focus::Search {
                state.search_query.push(ch);
            }
        }
        Action::SearchBackspace => {
            if state.focus == Focus::Search {
                state.search_query.pop();
            }
        }
        Action::SubmitSearch => {
            if state.focus == Focus::Search {
                state.focus = Focus::Records;
                state.status = if state.search_query.is_empty() {
                    "Search cleared".to_string()
                } else {
                    format!("Search: {}", state.search_query)
                };
            }
        }
        Action::ToggleSort => {
            state.newest_first = !state.newest_first;
            state.status = if state.newest_first {
                "Newest first".to_string()
            } else {
                "Oldest first".to_string()
            };
        }
        Action::ToggleShowAll => {
            state.show_all = !state.show_all;
            state.status = if state.show_all {
                "Showing default-hidden records".to_string()
            } else {
                "Hiding default-hidden records".to_string()
            };
        }
        Action::Quit => state.quit = true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        let mut state = AppState::new(
            "Example",
            vec![
                SectionItem {
                    key: "articles".to_string(),
                    label: "Articles".to_string(),
                    container_id: Some("c-articles".to_string()),
                },
                SectionItem {
                    key: "decision_log".to_string(),
                    label: "Decision Log".to_string(),
                    container_id: Some("c-decisions".to_string()),
                },
            ],
        );
        state.set_records(vec![
            RecordItem {
                instance_id: "r-1".to_string(),
                label: "First".to_string(),
                lifecycle_state: Some("draft".to_string()),
                tags: vec![],
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                type_id: "type-decision".to_string(),
                type_version: 1,
                detail_rows: vec![DetailRow {
                    label: "Decision Statement".to_string(),
                    value: Some("Use the field detail".to_string()),
                    required: true,
                    order: 1,
                }],
                record: serde_json::json!({}),
            },
            RecordItem {
                instance_id: "r-2".to_string(),
                label: "Second".to_string(),
                lifecycle_state: Some("ratified".to_string()),
                tags: vec!["tooling".to_string()],
                created_at: Some("2026-01-02T00:00:00Z".to_string()),
                type_id: "type-decision".to_string(),
                type_version: 1,
                detail_rows: vec![],
                record: serde_json::json!({}),
            },
        ]);
        state
    }

    #[test]
    fn section_navigation_is_bounded() {
        let mut state = state();
        reduce(&mut state, Action::Down);
        reduce(&mut state, Action::Down);
        assert_eq!(state.section_index, 1);
        reduce(&mut state, Action::Up);
        reduce(&mut state, Action::Up);
        assert_eq!(state.section_index, 0);
    }

    #[test]
    fn record_navigation_open_and_back() {
        let mut state = state();
        reduce(&mut state, Action::NextPane);
        reduce(&mut state, Action::Down);
        assert_eq!(state.record_index, 1);
        reduce(&mut state, Action::Open);
        assert_eq!(state.focus, Focus::Detail);
        reduce(&mut state, Action::Back);
        assert_eq!(state.focus, Focus::Records);
        reduce(&mut state, Action::Back);
        assert_eq!(state.focus, Focus::Sections);
    }

    #[test]
    fn search_mode_collects_and_submits_text() {
        let mut state = state();
        reduce(&mut state, Action::SearchMode);
        reduce(&mut state, Action::SearchInput('b'));
        reduce(&mut state, Action::SearchInput('u'));
        reduce(&mut state, Action::SearchBackspace);
        reduce(&mut state, Action::SubmitSearch);
        assert_eq!(state.search_query, "b");
        assert_eq!(state.focus, Focus::Records);
        assert_eq!(state.status, "Search: b");
    }

    #[test]
    fn toggles_and_quit_update_state() {
        let mut state = state();
        reduce(&mut state, Action::ToggleSort);
        reduce(&mut state, Action::ToggleShowAll);
        reduce(&mut state, Action::Quit);
        assert!(!state.newest_first);
        assert!(state.show_all);
        assert!(state.quit);
    }
}
