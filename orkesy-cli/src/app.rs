use orkesy_core::log_filter::LogFilterMode;

use crate::render::DisplayLogLine;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum View {
    #[default]
    Logs,
    Inspect,
    Exec,
    Deps,
    Metrics,
}

#[allow(dead_code)]
impl View {
    pub fn label(&self) -> &'static str {
        match self {
            View::Logs => "Logs",
            View::Inspect => "Inspect",
            View::Exec => "Exec",
            View::Deps => "Deps",
            View::Metrics => "Metrics",
        }
    }

    pub fn key(&self) -> char {
        match self {
            View::Logs => 'l',
            View::Inspect => 'i',
            View::Exec => 'e',
            View::Deps => 'd',
            View::Metrics => 'm',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InspectSection {
    #[default]
    Summary,
    Metrics,
    Health,
}

impl InspectSection {
    pub fn next(&self) -> Self {
        match self {
            InspectSection::Summary => InspectSection::Metrics,
            InspectSection::Metrics => InspectSection::Health,
            InspectSection::Health => InspectSection::Summary,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            InspectSection::Summary => InspectSection::Health,
            InspectSection::Metrics => InspectSection::Summary,
            InspectSection::Health => InspectSection::Metrics,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Units,
    RightPane,
    InspectPanel(InspectSection),
    Palette,
}

#[allow(dead_code)]
impl Focus {
    pub fn toggle(&self) -> Self {
        match self {
            Focus::Units => Focus::RightPane,
            Focus::RightPane => Focus::Units,
            Focus::InspectPanel(_) => Focus::Units,
            Focus::Palette => Focus::Palette,
        }
    }

    pub fn is_right(&self) -> bool {
        matches!(self, Focus::RightPane | Focus::InspectPanel(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LeftMode {
    #[default]
    Services,
    Commands,
    Runs,
}

impl LeftMode {
    pub fn label(&self) -> &'static str {
        match self {
            LeftMode::Services => "Units",
            LeftMode::Commands => "Commands",
            LeftMode::Runs => "Runs",
        }
    }

    pub fn key(&self) -> char {
        match self {
            LeftMode::Services => '1',
            LeftMode::Commands => '2',
            LeftMode::Runs => '3',
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LogsUiState {
    pub follow: bool,
    pub paused: bool,
    pub scroll: usize,
    pub search: Option<String>,
    pub matches: Vec<usize>,
    pub match_idx: usize,
    pub frozen_logs: Vec<DisplayLogLine>,
    pub log_filter: LogFilterMode,
}

impl LogsUiState {
    pub fn new() -> Self {
        Self {
            follow: true,
            ..Default::default()
        }
    }

    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    pub fn enter_search(&mut self) {
        self.search = Some(String::new());
        self.matches.clear();
        self.match_idx = 0;
    }

    pub fn exit_search(&mut self) {
        self.search = None;
        self.matches.clear();
        self.match_idx = 0;
    }

    pub fn toggle_follow(&mut self) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll = 0;
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.follow = false;
        self.scroll = self.scroll.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
        if self.scroll == 0 {
            self.follow = true;
        }
    }

    pub fn next_match(&mut self) {
        if !self.matches.is_empty() {
            self.match_idx = (self.match_idx + 1) % self.matches.len();
        }
    }

    pub fn prev_match(&mut self) {
        if !self.matches.is_empty() {
            self.match_idx = if self.match_idx == 0 {
                self.matches.len() - 1
            } else {
                self.match_idx - 1
            };
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct UiState {
    pub focus: Focus,
    pub view: View,
    pub left_mode: LeftMode,
    pub selected_command: usize,
    pub selected_run: usize,
    pub logs: LogsUiState,
    pub inspect_scroll: usize,
    pub deps_scroll: usize,
    pub palette_open: bool,
    pub palette_input: String,
    pub palette_error: Option<String>,
    pub palette_pick: usize,
    pub palette_scroll: usize,
    pub palette_sugg_offset: usize,
    pub help_open: bool,
    pub metrics_paused: bool,
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            focus: Focus::Units,
            view: View::Logs,
            left_mode: LeftMode::Services,
            selected_command: 0,
            selected_run: 0,
            logs: LogsUiState::new(),
            inspect_scroll: 0,
            deps_scroll: 0,
            palette_open: false,
            palette_input: String::new(),
            palette_error: None,
            palette_pick: 0,
            palette_scroll: 0,
            palette_sugg_offset: 0,
            help_open: false,
            metrics_paused: false,
            history: Vec::new(),
            history_cursor: None,
        }
    }
}

impl UiState {
    pub fn scroll_offset(&self) -> usize {
        self.logs.scroll
    }

    pub fn is_following(&self) -> bool {
        self.logs.follow
    }

    pub fn enter_follow(&mut self) {
        self.logs.follow = true;
        self.logs.scroll = 0;
    }

    pub fn search_query(&self) -> Option<&str> {
        self.logs.search.as_deref()
    }
}
