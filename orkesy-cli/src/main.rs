// Suppress clippy warnings that require extensive refactoring
#![allow(clippy::collapsible_if)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]

mod adapters;
mod commands;
mod detectors;
mod engines;
mod health;
mod runner;
mod sampler;
mod ui;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, GraphType, List, ListItem, ListState,
        Paragraph, Wrap,
    },
};

use tokio::sync::{RwLock, broadcast, mpsc};

use orkesy_core::adapter::{Adapter, AdapterCommand, AdapterEvent, LogStream};
use orkesy_core::config::OrkesyConfig;
use orkesy_core::log_filter::{LogFilterMode, detect_level};
use orkesy_core::model::*;
use orkesy_core::reducer::*;
use orkesy_core::state::*;
use orkesy_core::unit::{Unit, UnitStatus as AdapterUnitStatus};

use adapters::ProcessAdapter;
use engines::FakeEngine;
use ui::styles;

/// Format a SystemTime as HH:MM:SS for log display
fn format_timestamp(time: SystemTime) -> String {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            let hours = (secs / 3600) % 24;
            let minutes = (secs / 60) % 60;
            let seconds = secs % 60;
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        }
        Err(_) => "??:??:??".to_string(),
    }
}

/// A log line with optional timestamp for display
#[derive(Clone, Debug)]
struct DisplayLogLine {
    timestamp: Option<SystemTime>,
    text: String,
}

#[derive(Parser)]
#[command(name = "orkesy")]
#[command(about = "Manage and orchestrate local services", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(short, long)]
        yes: bool,
    },
    Doctor,
    Tui,
    Up {
        #[arg(required = true)]
        units: Vec<String>,
    },
    Down {
        #[arg(required = true)]
        units: Vec<String>,
    },
    Restart {
        #[arg(required = true)]
        units: Vec<String>,
    },
    Logs {
        unit: String,
        #[arg(short, long, default_value = "true")]
        follow: bool,
    },
    Install {
        units: Vec<String>,
    },
    Exec {
        unit: String,
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
}

fn demo_graph() -> RuntimeGraph {
    let mut nodes = BTreeMap::new();

    for (id, kind, port) in [
        ("api", ServiceKind::HttpApi, Some(8000u16)),
        ("worker", ServiceKind::Worker, None),
        ("postgres", ServiceKind::Database, Some(5432)),
    ] {
        nodes.insert(
            id.to_string(),
            ServiceNode {
                id: id.to_string(),
                display_name: id.to_string(),
                kind,
                desired: DesiredState::Running,
                observed: ObservedState {
                    instance_id: None,
                    status: ServiceStatus::Stopped,
                    health: HealthStatus::Unknown,
                },
                port,
                description: None,
            },
        );
    }

    let mut edges = BTreeSet::new();
    edges.insert(Edge {
        from: "api".into(),
        to: "postgres".into(),
        kind: EdgeKind::DependsOn,
    });
    edges.insert(Edge {
        from: "worker".into(),
        to: "postgres".into(),
        kind: EdgeKind::DependsOn,
    });

    RuntimeGraph { nodes, edges }
}

fn try_load_config() -> Option<(PathBuf, OrkesyConfig)> {
    let cwd = std::env::current_dir().ok()?;
    let names = ["orkesy.yml", "orkesy.yaml", ".orkesy.yml", ".orkesy.yaml"];

    for name in &names {
        let path = cwd.join(name);
        if path.exists() {
            match OrkesyConfig::load(&path) {
                Ok(config) => return Some((path, config)),
                Err(e) => {
                    eprintln!("Error loading {}: {}", path.display(), e);
                    continue;
                }
            }
        }
    }
    None
}

fn units_to_graph(units: &[Unit], edges: &[orkesy_core::unit::UnitEdge]) -> RuntimeGraph {
    let mut nodes = BTreeMap::new();

    for unit in units {
        let kind = match unit.kind {
            orkesy_core::unit::UnitKind::Process => {
                // Infer from name/description
                let name_lower = unit.id.to_lowercase();
                if name_lower.contains("api") || name_lower.contains("server") {
                    ServiceKind::HttpApi
                } else if name_lower.contains("worker") || name_lower.contains("celery") {
                    ServiceKind::Worker
                } else if name_lower.contains("db")
                    || name_lower.contains("postgres")
                    || name_lower.contains("mysql")
                {
                    ServiceKind::Database
                } else if name_lower.contains("redis") || name_lower.contains("cache") {
                    ServiceKind::Cache
                } else if name_lower.contains("web") || name_lower.contains("frontend") {
                    ServiceKind::Frontend
                } else {
                    ServiceKind::Generic
                }
            }
            orkesy_core::unit::UnitKind::Docker => {
                let name_lower = unit.id.to_lowercase();
                if name_lower.contains("db")
                    || name_lower.contains("postgres")
                    || name_lower.contains("mysql")
                {
                    ServiceKind::Database
                } else if name_lower.contains("redis") {
                    ServiceKind::Cache
                } else if name_lower.contains("rabbit") || name_lower.contains("kafka") {
                    ServiceKind::Queue
                } else {
                    ServiceKind::Generic
                }
            }
            orkesy_core::unit::UnitKind::Generic => ServiceKind::Generic,
        };

        nodes.insert(
            unit.id.clone(),
            ServiceNode {
                id: unit.id.clone(),
                display_name: unit.display_name().to_string(),
                kind,
                desired: DesiredState::Stopped, // Will be updated by adapter events
                observed: ObservedState {
                    instance_id: None,
                    status: ServiceStatus::Stopped,
                    health: HealthStatus::Unknown,
                },
                port: unit.port,
                description: unit.description.clone(),
            },
        );
    }

    let mut edge_set = BTreeSet::new();
    for edge in edges {
        edge_set.insert(Edge {
            from: edge.from.clone(),
            to: edge.to.clone(),
            kind: match edge.kind {
                orkesy_core::unit::EdgeKind::DependsOn => EdgeKind::DependsOn,
                orkesy_core::unit::EdgeKind::TalksTo => EdgeKind::TalksTo,
                orkesy_core::unit::EdgeKind::Produces => EdgeKind::Produces,
                orkesy_core::unit::EdgeKind::Consumes => EdgeKind::Consumes,
            },
        });
    }

    RuntimeGraph {
        nodes,
        edges: edge_set,
    }
}

fn adapter_event_to_runtime(event: AdapterEvent) -> RuntimeEvent {
    match event {
        AdapterEvent::StatusChanged { id, status } => {
            let service_status = match status {
                AdapterUnitStatus::Unknown => ServiceStatus::Unknown,
                AdapterUnitStatus::Starting => ServiceStatus::Starting,
                AdapterUnitStatus::Running => ServiceStatus::Running,
                AdapterUnitStatus::Stopping => ServiceStatus::Stopped, // Map stopping to stopped for now
                AdapterUnitStatus::Stopped => ServiceStatus::Stopped,
                AdapterUnitStatus::Exited { code } => ServiceStatus::Exited { code },
                AdapterUnitStatus::Errored { message } => ServiceStatus::Errored { message },
            };
            RuntimeEvent::StatusChanged {
                id,
                status: service_status,
            }
        }
        AdapterEvent::HealthChanged { id, health } => {
            let health_status = match health {
                orkesy_core::unit::UnitHealth::Unknown => HealthStatus::Unknown,
                orkesy_core::unit::UnitHealth::Healthy => HealthStatus::Healthy,
                orkesy_core::unit::UnitHealth::Degraded { reason } => {
                    HealthStatus::Degraded { reason }
                }
                orkesy_core::unit::UnitHealth::Unhealthy { reason } => {
                    HealthStatus::Unhealthy { reason }
                }
            };
            RuntimeEvent::HealthChanged {
                id,
                health: health_status,
            }
        }
        AdapterEvent::LogLine { id, stream, text } => RuntimeEvent::LogLine { id, stream, text },
        AdapterEvent::MetricsUpdated { id, metrics } => {
            RuntimeEvent::MetricsUpdated { id, metrics }
        }
    }
}

// --- Terminal setup/teardown ---
fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn status_label(s: &ServiceStatus) -> &'static str {
    match s {
        ServiceStatus::Unknown => "unknown",
        ServiceStatus::Starting => "starting",
        ServiceStatus::Running => "running",
        ServiceStatus::Stopped => "stopped",
        ServiceStatus::Exited { .. } => "exited",
        ServiceStatus::Restarting => "restarting",
        ServiceStatus::Errored { .. } => "error",
    }
}

fn status_icon(s: &ServiceStatus) -> &'static str {
    match s {
        ServiceStatus::Unknown => "?",
        ServiceStatus::Starting => "◐",
        ServiceStatus::Running => "●",
        ServiceStatus::Stopped => "○",
        ServiceStatus::Exited { code: Some(0) } => "◌",
        ServiceStatus::Exited { .. } => "✗",
        ServiceStatus::Restarting => "↻",
        ServiceStatus::Errored { .. } => "✗",
    }
}

fn health_icon(h: &HealthStatus) -> &'static str {
    match h {
        HealthStatus::Unknown => " ",
        HealthStatus::Healthy => "♥",
        HealthStatus::Degraded { .. } => "♡",
        HealthStatus::Unhealthy { .. } => "✗",
    }
}

fn kind_icon(k: &ServiceKind) -> &'static str {
    match k {
        ServiceKind::HttpApi => "⚡",
        ServiceKind::Worker => "⚙",
        ServiceKind::Database => "◆",
        ServiceKind::Cache => "⚡",
        ServiceKind::Queue => "≡",
        ServiceKind::Frontend => "◉",
        ServiceKind::Generic => "•",
    }
}

fn status_style(s: &ServiceStatus) -> Style {
    match s {
        ServiceStatus::Running => Style::default().fg(Color::Green),
        ServiceStatus::Starting | ServiceStatus::Restarting => Style::default().fg(Color::Yellow),
        ServiceStatus::Stopped => Style::default().fg(Color::DarkGray),
        ServiceStatus::Errored { .. } => Style::default().fg(Color::Red),
        ServiceStatus::Exited { code: Some(0) } => Style::default().fg(Color::DarkGray),
        ServiceStatus::Exited { .. } => Style::default().fg(Color::Red),
        ServiceStatus::Unknown => Style::default().fg(Color::DarkGray),
    }
}

fn health_style(h: &HealthStatus) -> Style {
    match h {
        HealthStatus::Healthy => Style::default().fg(Color::Green),
        HealthStatus::Degraded { .. } => Style::default().fg(Color::Yellow),
        HealthStatus::Unhealthy { .. } => Style::default().fg(Color::Red),
        HealthStatus::Unknown => Style::default(),
    }
}

fn fit_title(s: &str, width: u16) -> String {
    // width includes borders; keep safe margin
    let max = width.saturating_sub(4) as usize;
    if max == 0 {
        return "".into();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".into();
    }
    let mut out: String = chars.into_iter().take(max - 1).collect();
    out.push('…');
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
enum View {
    #[default]
    Logs,
    Inspect,
    Exec, // Commands explorer
    Deps,
    Metrics,
}

#[allow(dead_code)]
impl View {
    fn label(&self) -> &'static str {
        match self {
            View::Logs => "Logs",
            View::Inspect => "Inspect",
            View::Exec => "Exec",
            View::Deps => "Deps",
            View::Metrics => "Metrics",
        }
    }

    fn key(&self) -> char {
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
enum InspectSection {
    #[default]
    Summary,
    Metrics,
    Health,
}

impl InspectSection {
    fn next(&self) -> Self {
        match self {
            InspectSection::Summary => InspectSection::Metrics,
            InspectSection::Metrics => InspectSection::Health,
            InspectSection::Health => InspectSection::Summary,
        }
    }

    fn prev(&self) -> Self {
        match self {
            InspectSection::Summary => InspectSection::Health,
            InspectSection::Metrics => InspectSection::Summary,
            InspectSection::Health => InspectSection::Metrics,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Focus {
    #[default]
    Units,
    RightPane,
    InspectPanel(InspectSection),
    Palette,
}

#[allow(dead_code)]
impl Focus {
    fn toggle(&self) -> Self {
        match self {
            Focus::Units => Focus::RightPane,
            Focus::RightPane => Focus::Units,
            Focus::InspectPanel(_) => Focus::Units,
            Focus::Palette => Focus::Palette,
        }
    }

    fn is_right(&self) -> bool {
        matches!(self, Focus::RightPane | Focus::InspectPanel(_))
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum PickerCategory {
    ServiceAction,
    ProjectAction,
    DetectedCommand,
    Navigation,
}

#[allow(dead_code)]
impl PickerCategory {
    fn label(&self) -> &'static str {
        match self {
            PickerCategory::ServiceAction => "Service Actions",
            PickerCategory::ProjectAction => "Project Actions",
            PickerCategory::DetectedCommand => "Commands",
            PickerCategory::Navigation => "Navigation",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            PickerCategory::ServiceAction => "●",
            PickerCategory::ProjectAction => "◉",
            PickerCategory::DetectedCommand => "▶",
            PickerCategory::Navigation => "◇",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct PickerItem {
    label: String,
    detail: Option<String>,
    category: PickerCategory,
    command: Option<String>,
    target_view: Option<View>,
    service_id: Option<String>,
}

#[allow(dead_code)]
impl PickerItem {
    fn new_service_action(
        label: &str,
        detail: Option<&str>,
        command: &str,
        service_id: &str,
    ) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.map(|s| s.to_string()),
            category: PickerCategory::ServiceAction,
            command: Some(command.to_string()),
            target_view: None,
            service_id: Some(service_id.to_string()),
        }
    }

    fn new_project_action(label: &str, detail: Option<&str>, command: &str) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.map(|s| s.to_string()),
            category: PickerCategory::ProjectAction,
            command: Some(command.to_string()),
            target_view: None,
            service_id: None,
        }
    }

    fn new_detected_command(label: &str, command: &str, detail: Option<&str>) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.map(|s| s.to_string()),
            category: PickerCategory::DetectedCommand,
            command: Some(command.to_string()),
            target_view: None,
            service_id: None,
        }
    }

    fn new_navigation(label: &str, view: View) -> Self {
        Self {
            label: label.to_string(),
            detail: Some(format!("Press '{}' for quick access", view.key())),
            category: PickerCategory::Navigation,
            command: None,
            target_view: Some(view),
            service_id: None,
        }
    }

    fn fuzzy_matches(&self, pattern: &str) -> bool {
        if pattern.is_empty() {
            return true;
        }
        let label_lower = self.label.to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        let mut pattern_chars = pattern_lower.chars().peekable();
        for c in label_lower.chars() {
            if pattern_chars.peek() == Some(&c) {
                pattern_chars.next();
            }
        }
        pattern_chars.peek().is_none()
    }

    fn fuzzy_score(&self, pattern: &str) -> i32 {
        if pattern.is_empty() {
            return 0;
        }
        let label_lower = self.label.to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        // Exact prefix match: highest score
        if label_lower.starts_with(&pattern_lower) {
            return 1000 - self.label.len() as i32;
        }
        // Contains match
        if label_lower.contains(&pattern_lower) {
            return 500 - self.label.len() as i32;
        }
        // Fuzzy match score
        if self.fuzzy_matches(pattern) {
            return 100 - self.label.len() as i32;
        }
        -1000
    }
}

fn build_picker_items(
    service_ids: &[String],
    selected_service: Option<&str>,
    _detected_commands: &[String], // For future: parsed from package.json etc.
) -> Vec<PickerItem> {
    let mut items = Vec::new();

    // Service actions for selected service
    if let Some(sid) = selected_service {
        items.push(PickerItem::new_service_action(
            &format!("Start {}", sid),
            Some("Start the service"),
            &format!("start {}", sid),
            sid,
        ));
        items.push(PickerItem::new_service_action(
            &format!("Stop {}", sid),
            Some("Stop the service"),
            &format!("stop {}", sid),
            sid,
        ));
        items.push(PickerItem::new_service_action(
            &format!("Restart {}", sid),
            Some("Restart the service"),
            &format!("restart {}", sid),
            sid,
        ));
        items.push(PickerItem::new_service_action(
            &format!("Kill {}", sid),
            Some("Force kill the service"),
            &format!("kill {}", sid),
            sid,
        ));
        items.push(PickerItem::new_service_action(
            &format!("Clear logs for {}", sid),
            Some("Clear the log buffer"),
            &format!("clear {}", sid),
            sid,
        ));
    }

    // Other service actions (not selected)
    for id in service_ids {
        if selected_service == Some(id.as_str()) {
            continue; // Already added above
        }
        items.push(PickerItem::new_service_action(
            &format!("Start {}", id),
            None,
            &format!("start {}", id),
            id,
        ));
        items.push(PickerItem::new_service_action(
            &format!("Stop {}", id),
            None,
            &format!("stop {}", id),
            id,
        ));
        items.push(PickerItem::new_service_action(
            &format!("Restart {}", id),
            None,
            &format!("restart {}", id),
            id,
        ));
    }

    // Project-wide actions
    items.push(PickerItem::new_project_action(
        "Start all services",
        Some("Start all defined services"),
        "start all",
    ));
    items.push(PickerItem::new_project_action(
        "Stop all services",
        Some("Stop all running services"),
        "stop all",
    ));
    items.push(PickerItem::new_project_action(
        "Restart all services",
        Some("Restart all services"),
        "restart all",
    ));
    items.push(PickerItem::new_project_action(
        "Kill all services",
        Some("Force kill all services"),
        "kill all",
    ));
    items.push(PickerItem::new_project_action(
        "Clear all logs",
        Some("Clear log buffers for all services"),
        "clear all",
    ));

    // Navigation actions
    items.push(PickerItem::new_navigation("Open Logs view", View::Logs));
    items.push(PickerItem::new_navigation(
        "Open Inspect view",
        View::Inspect,
    ));
    items.push(PickerItem::new_navigation("Open Exec view", View::Exec));
    items.push(PickerItem::new_navigation(
        "Open Dependencies view",
        View::Deps,
    ));
    items.push(PickerItem::new_navigation(
        "Open Metrics view",
        View::Metrics,
    ));

    items
}

fn filter_picker_items(items: &[PickerItem], query: &str) -> Vec<PickerItem> {
    if query.is_empty() {
        // Return all items, grouped by category
        let mut result = items.to_vec();
        result.sort_by(|a, b| {
            // Sort by category order, then by label
            let cat_order = |c: &PickerCategory| match c {
                PickerCategory::ServiceAction => 0,
                PickerCategory::ProjectAction => 1,
                PickerCategory::DetectedCommand => 2,
                PickerCategory::Navigation => 3,
            };
            cat_order(&a.category)
                .cmp(&cat_order(&b.category))
                .then_with(|| a.label.cmp(&b.label))
        });
        return result;
    }

    // Filter by fuzzy match and sort by score
    let mut filtered: Vec<(PickerItem, i32)> = items
        .iter()
        .filter(|item| item.fuzzy_matches(query))
        .map(|item| (item.clone(), item.fuzzy_score(query)))
        .collect();

    filtered.sort_by(|a, b| b.1.cmp(&a.1));
    filtered.into_iter().map(|(item, _)| item).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum LeftMode {
    #[default]
    Services,
    Commands,
    Runs,
}

impl LeftMode {
    fn label(&self) -> &'static str {
        match self {
            LeftMode::Services => "Units",
            LeftMode::Commands => "Commands",
            LeftMode::Runs => "Runs",
        }
    }

    fn key(&self) -> char {
        match self {
            LeftMode::Services => '1',
            LeftMode::Commands => '2',
            LeftMode::Runs => '3',
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LogsUiState {
    follow: bool,
    paused: bool,
    scroll: usize,
    search: Option<String>,
    matches: Vec<usize>,
    match_idx: usize,
    frozen_logs: Vec<DisplayLogLine>,
    log_filter: LogFilterMode,
}

impl LogsUiState {
    fn new() -> Self {
        Self {
            follow: true,
            ..Default::default()
        }
    }

    fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    fn enter_search(&mut self) {
        self.search = Some(String::new());
        self.matches.clear();
        self.match_idx = 0;
    }

    fn exit_search(&mut self) {
        self.search = None;
        self.matches.clear();
        self.match_idx = 0;
    }

    fn toggle_follow(&mut self) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll = 0;
        }
    }

    fn scroll_up(&mut self, lines: usize) {
        self.follow = false;
        self.scroll = self.scroll.saturating_add(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
        if self.scroll == 0 {
            self.follow = true;
        }
    }

    fn next_match(&mut self) {
        if !self.matches.is_empty() {
            self.match_idx = (self.match_idx + 1) % self.matches.len();
        }
    }

    fn prev_match(&mut self) {
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
struct UiState {
    focus: Focus,
    view: View,
    left_mode: LeftMode,
    selected_command: usize,
    selected_run: usize,
    logs: LogsUiState,
    inspect_scroll: usize,
    deps_scroll: usize,
    palette_open: bool,
    palette_input: String,
    palette_error: Option<String>,
    palette_pick: usize,
    palette_scroll: usize,
    palette_sugg_offset: usize,
    help_open: bool,
    metrics_paused: bool,
    history: Vec<String>,
    history_cursor: Option<usize>,
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
    fn scroll_offset(&self) -> usize {
        self.logs.scroll
    }

    fn is_following(&self) -> bool {
        self.logs.follow
    }

    fn enter_follow(&mut self) {
        self.logs.follow = true;
        self.logs.scroll = 0;
    }

    fn search_query(&self) -> Option<&str> {
        self.logs.search.as_deref()
    }
}

enum RuntimeBackend {
    Adapter {
        cmd_tx: mpsc::Sender<AdapterCommand>,
    },
    LegacyEngine {
        cmd_tx: mpsc::Sender<orkesy_core::engine::EngineCommand>,
    },
}

impl RuntimeBackend {
    async fn send_start(&self, id: String) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::Start { id }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::Start { id })
                    .await;
            }
        }
    }

    async fn send_stop(&self, id: String) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::Stop { id }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::Stop { id })
                    .await;
            }
        }
    }

    async fn send_restart(&self, id: String) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::Restart { id }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::Restart { id })
                    .await;
            }
        }
    }

    async fn send_kill(&self, id: String) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::Kill { id }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::Kill { id })
                    .await;
            }
        }
    }

    async fn send_toggle(&self, id: String) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::Toggle { id }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::Toggle { id })
                    .await;
            }
        }
    }

    async fn send_clear_logs(&self, id: String) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::ClearLogs { id }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::ClearLogs { id })
                    .await;
            }
        }
    }

    async fn send_exec(&self, id: String, cmd: Vec<String>) {
        match self {
            RuntimeBackend::Adapter { cmd_tx } => {
                let _ = cmd_tx.send(AdapterCommand::Exec { id, cmd }).await;
            }
            RuntimeBackend::LegacyEngine { cmd_tx } => {
                let _ = cmd_tx
                    .send(orkesy_core::engine::EngineCommand::Exec { id, cmd })
                    .await;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum CliAction {
    Start,
    Stop,
    Restart,
    Install,
}

async fn run_cli_command(action: CliAction, unit_args: Vec<String>) -> io::Result<()> {
    let Some((path, config)) = try_load_config() else {
        eprintln!("Error: No orkesy.yml found. Run `orkesy init` first.");
        std::process::exit(1);
    };

    let units = config.to_units();
    let unit_ids: Vec<String> = units.iter().map(|u| u.id.clone()).collect();

    // Expand "all" to all unit IDs
    let target_ids: Vec<String> = if unit_args.iter().any(|u| u == "all") {
        unit_ids.clone()
    } else {
        // Validate unit IDs exist
        for id in &unit_args {
            if !unit_ids.contains(id) {
                eprintln!(
                    "Error: Unknown unit '{}'. Available: {}",
                    id,
                    unit_ids.join(", ")
                );
                std::process::exit(1);
            }
        }
        unit_args
    };

    if target_ids.is_empty() {
        eprintln!("No units to process.");
        return Ok(());
    }

    let action_name = match action {
        CliAction::Start => "Starting",
        CliAction::Stop => "Stopping",
        CliAction::Restart => "Restarting",
        CliAction::Install => "Installing",
    };

    println!("Loaded config from: {}", path.display());
    println!("{} {} unit(s)...\n", action_name, target_ids.len());

    // Set up adapter
    let (cmd_tx, cmd_rx) = mpsc::channel::<AdapterCommand>(100);
    let (event_tx, mut event_rx) = broadcast::channel::<AdapterEvent>(1_000);

    let mut adapter = ProcessAdapter::new();
    let units_clone = units.clone();
    tokio::spawn(async move {
        adapter.run(cmd_rx, event_tx, units_clone).await;
    });

    // Send commands
    for id in &target_ids {
        let cmd = match action {
            CliAction::Start => AdapterCommand::Start { id: id.clone() },
            CliAction::Stop => AdapterCommand::Stop { id: id.clone() },
            CliAction::Restart => AdapterCommand::Restart { id: id.clone() },
            CliAction::Install => AdapterCommand::Install { id: id.clone() },
        };
        let _ = cmd_tx.send(cmd).await;
    }

    // For start/restart, wait a bit and stream initial output
    let wait_time = match action {
        CliAction::Start | CliAction::Restart => Duration::from_secs(2),
        CliAction::Stop => Duration::from_millis(500),
        CliAction::Install => Duration::from_secs(30), // Longer for install
    };

    let deadline = tokio::time::Instant::now() + wait_time;
    let mut completed = std::collections::HashSet::new();

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                break;
            }
            event = event_rx.recv() => {
                if let Ok(event) = event {
                    match &event {
                        AdapterEvent::LogLine { id, stream, text } => {
                            if target_ids.contains(id) {
                                let prefix = match stream {
                                    LogStream::Stdout => "",
                                    LogStream::Stderr => "[stderr] ",
                                    LogStream::System => "[system] ",
                                };
                                println!("[{}] {}{}", id, prefix, text);
                            }
                        }
                        AdapterEvent::StatusChanged { id, status } => {
                            if target_ids.contains(id) {
                                let status_str = match status {
                                    orkesy_core::unit::UnitStatus::Running => "running",
                                    orkesy_core::unit::UnitStatus::Stopped => "stopped",
                                    orkesy_core::unit::UnitStatus::Exited { code } => {
                                        completed.insert(id.clone());
                                        if code == &Some(0) { "exited (0)" } else { "exited (error)" }
                                    }
                                    orkesy_core::unit::UnitStatus::Errored { message } => {
                                        eprintln!("[{}] Error: {}", id, message);
                                        "error"
                                    }
                                    _ => continue,
                                };
                                println!("[{}] Status: {}", id, status_str);

                                // For stop action, track completion
                                if matches!(action, CliAction::Stop) && matches!(status, orkesy_core::unit::UnitStatus::Stopped) {
                                    completed.insert(id.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Check if all units completed for stop/install
                if matches!(action, CliAction::Stop | CliAction::Install) && completed.len() >= target_ids.len() {
                    break;
                }
            }
        }
    }

    // Shutdown adapter for stop action
    if matches!(action, CliAction::Stop) {
        let _ = cmd_tx.send(AdapterCommand::Shutdown).await;
    }

    println!("\nDone.");
    Ok(())
}

async fn run_cli_logs(unit_id: &str, follow: bool) -> io::Result<()> {
    let Some((path, config)) = try_load_config() else {
        eprintln!("Error: No orkesy.yml found. Run `orkesy init` first.");
        std::process::exit(1);
    };

    let units = config.to_units();
    let unit_ids: Vec<String> = units.iter().map(|u| u.id.clone()).collect();

    if !unit_ids.contains(&unit_id.to_string()) {
        eprintln!(
            "Error: Unknown unit '{}'. Available: {}",
            unit_id,
            unit_ids.join(", ")
        );
        std::process::exit(1);
    }

    println!("Loaded config from: {}", path.display());
    println!("Streaming logs for '{}'... (Ctrl+C to stop)\n", unit_id);

    // Set up adapter
    let (cmd_tx, cmd_rx) = mpsc::channel::<AdapterCommand>(100);
    let (event_tx, mut event_rx) = broadcast::channel::<AdapterEvent>(1_000);

    let mut adapter = ProcessAdapter::new();
    let units_clone = units.clone();
    tokio::spawn(async move {
        adapter.run(cmd_rx, event_tx, units_clone).await;
    });

    // Start the unit
    let _ = cmd_tx
        .send(AdapterCommand::Start {
            id: unit_id.to_string(),
        })
        .await;

    // Stream logs until Ctrl+C
    let unit_id_owned = unit_id.to_string();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n\nStopping...");
                let _ = cmd_tx.send(AdapterCommand::Stop { id: unit_id_owned.clone() }).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                break;
            }
            event = event_rx.recv() => {
                if let Ok(event) = event {
                    match event {
                        AdapterEvent::LogLine { id, stream, text } if id == unit_id_owned => {
                            let prefix = match stream {
                                LogStream::Stdout => "",
                                LogStream::Stderr => "\x1b[33m[stderr]\x1b[0m ",
                                LogStream::System => "\x1b[36m[system]\x1b[0m ",
                            };
                            println!("{}{}", prefix, text);
                        }
                        AdapterEvent::StatusChanged { id, status } if id == unit_id_owned => {
                            match status {
                                orkesy_core::unit::UnitStatus::Exited { code } => {
                                    println!("\n\x1b[33mProcess exited with code: {:?}\x1b[0m", code);
                                    if !follow {
                                        break;
                                    }
                                }
                                orkesy_core::unit::UnitStatus::Errored { message } => {
                                    eprintln!("\n\x1b[31mError: {}\x1b[0m", message);
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_cli_exec(unit_id: &str, cmd: Vec<String>) -> io::Result<()> {
    let Some((_path, config)) = try_load_config() else {
        eprintln!("Error: No orkesy.yml found. Run `orkesy init` first.");
        std::process::exit(1);
    };

    let units = config.to_units();
    let unit = units.iter().find(|u| u.id == unit_id);

    let Some(unit) = unit else {
        let unit_ids: Vec<String> = units.iter().map(|u| u.id.clone()).collect();
        eprintln!(
            "Error: Unknown unit '{}'. Available: {}",
            unit_id,
            unit_ids.join(", ")
        );
        std::process::exit(1);
    };

    println!("Executing in context of '{}': {}", unit_id, cmd.join(" "));

    // Run the command with unit's cwd and env
    let mut command = tokio::process::Command::new(&cmd[0]);
    command.args(&cmd[1..]);

    if let Some(cwd) = &unit.cwd {
        command.current_dir(cwd);
    }
    for (k, v) in &unit.env {
        command.env(k, v);
    }

    let status = command.status().await?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();

    // Handle subcommands
    match cli.command {
        Some(Commands::Init { yes }) => match commands::run_init(yes) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },
        Some(Commands::Doctor) => match commands::run_doctor() {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },
        Some(Commands::Up { units }) => {
            return run_cli_command(CliAction::Start, units).await;
        }
        Some(Commands::Down { units }) => {
            return run_cli_command(CliAction::Stop, units).await;
        }
        Some(Commands::Restart { units }) => {
            return run_cli_command(CliAction::Restart, units).await;
        }
        Some(Commands::Logs { unit, follow }) => {
            return run_cli_logs(&unit, follow).await;
        }
        Some(Commands::Install { units }) => {
            return run_cli_command(CliAction::Install, units).await;
        }
        Some(Commands::Exec { unit, cmd }) => {
            return run_cli_exec(&unit, cmd).await;
        }
        Some(Commands::Tui) | None => {
            // Fall through to TUI
        }
    }

    // Run TUI
    run_tui().await
}

async fn run_tui() -> io::Result<()> {
    // Track when we started for uptime display
    let start_time = std::time::Instant::now();

    // Event channel for reducer (using RuntimeEvent for TUI compatibility)
    let (event_tx, _) = broadcast::channel::<EventEnvelope>(1_000);

    // Try to load config, fall back to demo mode
    let (graph, backend, autostart_ids, units_map, project_name): (
        RuntimeGraph,
        RuntimeBackend,
        Vec<String>,
        BTreeMap<String, Unit>,
        String,
    ) = match try_load_config() {
        Some((path, config)) => {
            eprintln!("Loaded config from: {}", path.display());
            let proj_name = config
                .project_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "orkesy".to_string());

            // Get units and edges from config
            let units = config.to_units();
            let edges = config.to_edges();
            let graph = units_to_graph(&units, &edges);

            // Store units by ID for Inspect view
            let units_map: BTreeMap<String, Unit> =
                units.iter().map(|u| (u.id.clone(), u.clone())).collect();

            // Collect units that should autostart
            let autostart_ids: Vec<String> = units
                .iter()
                .filter(|u| u.autostart)
                .map(|u| u.id.clone())
                .collect();

            // Use new ProcessAdapter
            let (adapter_cmd_tx, adapter_cmd_rx) = mpsc::channel::<AdapterCommand>(100);
            let (adapter_event_tx, mut adapter_event_rx) =
                broadcast::channel::<AdapterEvent>(1_000);

            // Spawn adapter
            let mut adapter = ProcessAdapter::new();
            let units_for_health = units.clone();
            tokio::spawn(async move {
                adapter.run(adapter_cmd_rx, adapter_event_tx, units).await;
            });

            // Spawn health checkers for units with health config
            let health_event_tx = event_tx.clone();
            let next_health_id = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000_000));
            health::spawn_health_checkers(&units_for_health, health_event_tx, next_health_id);

            // Bridge adapter events to runtime events
            let event_tx_clone = event_tx.clone();
            tokio::spawn(async move {
                let mut event_id = 1u64;
                while let Ok(adapter_event) = adapter_event_rx.recv().await {
                    let runtime_event = adapter_event_to_runtime(adapter_event);
                    let _ = event_tx_clone.send(EventEnvelope {
                        id: event_id,
                        at: std::time::SystemTime::now(),
                        event: runtime_event,
                    });
                    event_id += 1;
                }
            });

            // Emit initial topology
            let _ = event_tx.send(EventEnvelope {
                id: 0,
                at: std::time::SystemTime::now(),
                event: RuntimeEvent::TopologyLoaded {
                    graph: graph.clone(),
                },
            });

            (
                graph,
                RuntimeBackend::Adapter {
                    cmd_tx: adapter_cmd_tx,
                },
                autostart_ids,
                units_map,
                proj_name,
            )
        }
        None => {
            eprintln!("No orkesy.yml found, running in demo mode with fake engine");
            let graph = demo_graph();
            let units_map = BTreeMap::new(); // Empty in demo mode

            // Use legacy FakeEngine for demo mode
            let (engine_cmd_tx, engine_cmd_rx) =
                mpsc::channel::<orkesy_core::engine::EngineCommand>(100);
            let engine_event_tx = event_tx.clone();
            let graph_clone = graph.clone();

            let mut engine = FakeEngine::new();
            tokio::spawn(async move {
                use orkesy_core::engine::Engine;
                engine
                    .run(engine_cmd_rx, engine_event_tx, graph_clone)
                    .await;
            });

            (
                graph,
                RuntimeBackend::LegacyEngine {
                    cmd_tx: engine_cmd_tx,
                },
                vec![],
                units_map,
                "demo".to_string(),
            )
        }
    };

    // Autostart units that have autostart: true
    if !autostart_ids.is_empty() {
        eprintln!("Auto-starting {} unit(s)...", autostart_ids.len());
        for id in autostart_ids {
            backend.send_start(id).await;
        }
    }

    // Index project for Commands + Runs feature
    let cwd = std::env::current_dir().unwrap_or_default();
    eprintln!("Indexing project at: {}", cwd.display());
    let project_index = detectors::index_project(&cwd).await;
    eprintln!(
        "Found {} commands from {} tool(s)",
        project_index.commands.len(),
        project_index.tools.len()
    );

    // Spawn CommandRunner for Commands + Runs feature
    let (runner_cmd_tx, runner_cmd_rx) = mpsc::channel::<runner::RunnerCommand>(100);
    let runner_event_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut cmd_runner = runner::CommandRunner::new();
        cmd_runner.run(runner_cmd_rx, runner_event_tx).await;
    });

    let state = Arc::new(RwLock::new(RuntimeState::new(graph.clone())));

    // Emit ProjectIndexed event
    let _ = event_tx.send(EventEnvelope {
        id: 2,
        at: std::time::SystemTime::now(),
        event: RuntimeEvent::ProjectIndexed {
            project: project_index,
        },
    });

    // Reducer task
    let state_for_reducer = state.clone();
    let mut reducer_rx = event_tx.subscribe();
    tokio::spawn(async move {
        while let Ok(env) = reducer_rx.recv().await {
            let mut s = state_for_reducer.write().await;
            reduce(&mut s, &env);
        }
    });

    // Metrics sampler task (collects system stats + log rates every 500ms)
    sampler::spawn_sampler(event_tx.clone(), state.clone());

    let mut terminal = setup_terminal()?;
    let mut selected = 0usize;
    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let res = tui_loop(
        &mut terminal,
        state,
        backend,
        &units_map,
        &mut selected,
        &mut list_state,
        runner_cmd_tx,
        &project_name,
        start_time,
    )
    .await;
    restore_terminal(terminal)?;
    res
}

#[derive(Clone, Debug)]
enum TuiCommand {
    Start { id: String },
    Stop { id: String },
    Restart { id: String },
    Kill { id: String },
    Toggle { id: String },
    ClearLogs { id: String },
    Exec { id: String, cmd: Vec<String> },
}

impl TuiCommand {
    async fn execute(self, backend: &RuntimeBackend) {
        match self {
            TuiCommand::Start { id } => backend.send_start(id).await,
            TuiCommand::Stop { id } => backend.send_stop(id).await,
            TuiCommand::Restart { id } => backend.send_restart(id).await,
            TuiCommand::Kill { id } => backend.send_kill(id).await,
            TuiCommand::Toggle { id } => backend.send_toggle(id).await,
            TuiCommand::ClearLogs { id } => backend.send_clear_logs(id).await,
            TuiCommand::Exec { id, cmd } => backend.send_exec(id, cmd).await,
        }
    }
}

fn parse_command(input: &str, service_ids: &[String]) -> Result<Vec<TuiCommand>, String> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".into());
    }

    let cmd = parts[0].to_lowercase();
    let arg1 = parts.get(1).copied();

    let exists = |id: &str| service_ids.iter().any(|s| s == id);

    let expand_ids = |arg: Option<&str>| -> Result<Vec<String>, String> {
        match arg {
            Some("all") => Ok(service_ids.to_vec()),
            Some(id) if exists(id) => Ok(vec![id.to_string()]),
            Some(id) => Err(format!("Unknown service: {id}")),
            None => Err("Missing target (service id or 'all')".into()),
        }
    };

    match cmd.as_str() {
        // Aliases for common operations
        "up" | "start" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Start { id })
            .collect()),

        "down" | "stop" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Stop { id })
            .collect()),

        "restart" | "rs" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Restart { id })
            .collect()),

        "toggle" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Toggle { id })
            .collect()),

        "kill" | "k" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Kill { id })
            .collect()),

        "clear" | "cl" => {
            // clear <service|all> or clear logs <service|all>
            let target = if arg1 == Some("logs") {
                parts.get(2).copied().unwrap_or("all")
            } else {
                arg1.unwrap_or("all")
            };
            Ok(expand_ids(Some(target))?
                .into_iter()
                .map(|id| TuiCommand::ClearLogs { id })
                .collect())
        }

        "exec" | "run" => {
            let svc = arg1.ok_or("Usage: exec <service> <cmd...>")?;
            if !exists(svc) {
                return Err(format!("Unknown service: {svc}"));
            }
            let cmd_parts = parts.get(2..).unwrap_or(&[]);
            if cmd_parts.is_empty() {
                return Err("Usage: exec <service> <cmd...>".into());
            }
            Ok(vec![TuiCommand::Exec {
                id: svc.to_string(),
                cmd: cmd_parts.iter().map(|s| s.to_string()).collect(),
            }])
        }

        _ => Err(format!(
            "Unknown command: {cmd}\nTry: up/down/restart/toggle/kill/clear/exec"
        )),
    }
}

async fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<RuntimeState>>,
    backend: RuntimeBackend,
    units_map: &BTreeMap<String, Unit>,
    selected: &mut usize,
    list_state: &mut ListState,
    runner_cmd_tx: mpsc::Sender<runner::RunnerCommand>,
    project_name: &str,
    start_time: std::time::Instant,
) -> io::Result<()> {
    let mut ui = UiState::default();
    let mut command_list_state = ListState::default();
    let mut run_list_state = ListState::default();

    loop {
        let snapshot = state.read().await;

        let mut service_ids: Vec<String> = snapshot.graph.nodes.keys().cloned().collect();
        service_ids.sort();

        // Prepend "all" to the list for merged view
        let mut display_ids = vec!["all".to_string()];
        display_ids.extend(service_ids.iter().cloned());

        if display_ids.is_empty() {
            *selected = 0;
            list_state.select(None);
        } else {
            if *selected >= display_ids.len() {
                *selected = display_ids.len() - 1;
            }
            list_state.select(Some(*selected));
        }

        let selected_id: Option<&str> = display_ids.get(*selected).map(|s| s.as_str());

        // Build items for Services mode with numeric indices
        let service_items: Vec<ListItem> = display_ids
            .iter()
            .enumerate()
            .map(|(idx, id)| {
                // Display index (1-based for user, 0 is "all")
                let index_str = if id == "all" {
                    "  ".to_string() // No index for "all"
                } else {
                    format!("{:2}", idx) // 1-based index
                };

                if id == "all" {
                    // Special "all" item showing running/total count
                    let running_count = snapshot
                        .graph
                        .nodes
                        .values()
                        .filter(|n| n.observed.status == ServiceStatus::Running)
                        .count();
                    let total = snapshot.graph.nodes.len();
                    ListItem::new(Line::from(vec![
                        Span::styled(index_str, styles::text_muted()),
                        Span::styled(" ◉ ", Style::default().fg(Color::Magenta)),
                        Span::raw(format!("all ({}/{})", running_count, total)),
                    ]))
                } else {
                    let node = snapshot.graph.nodes.get(id).unwrap();
                    let status_sym = status_icon(&node.observed.status);
                    let health_sym = health_icon(&node.observed.health);
                    let kind_sym = kind_icon(&node.kind);
                    let port_info = node.port.map(|p| format!(":{}", p)).unwrap_or_default();

                    let style = status_style(&node.observed.status);
                    let health_st = health_style(&node.observed.health);

                    // Get metrics for this unit
                    let metrics_info = if let Some(metrics) = snapshot.metrics.get(id) {
                        if node.observed.status == ServiceStatus::Running {
                            format!(
                                " {:.1}% {}",
                                metrics.cpu_percent,
                                adapters::format_bytes(metrics.memory_bytes)
                            )
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(index_str, styles::text_muted()),
                        Span::styled(format!(" {} ", status_sym), style),
                        Span::raw(format!("{} {}{} ", kind_sym, node.display_name, port_info)),
                        Span::styled(format!("[{}]", status_label(&node.observed.status)), style),
                        Span::styled(metrics_info, Style::default().fg(Color::DarkGray)),
                        Span::raw(" "),
                        Span::styled(health_sym, health_st),
                    ]))
                }
            })
            .collect();

        // Build items for Commands mode
        let command_items: Vec<ListItem> = if let Some(project) = &snapshot.project {
            project
                .commands_sorted()
                .iter()
                .map(|cmd| {
                    let cat_icon = cmd.category.icon();
                    let tool_icon = cmd.tool.icon();
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{} ", cat_icon), Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("[{}] ", tool_icon),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::raw(&cmd.display_name),
                    ]))
                })
                .collect()
        } else {
            vec![ListItem::new(Line::from("No commands detected."))]
        };

        // Build items for Runs mode
        let run_items: Vec<ListItem> = snapshot
            .runs_ordered()
            .iter()
            .map(|run| {
                let status_icon = run.status.icon();
                let style = match &run.status {
                    orkesy_core::command::RunStatus::Running => Style::default().fg(Color::Green),
                    orkesy_core::command::RunStatus::Exited { code: Some(0) } => {
                        Style::default().fg(Color::DarkGray)
                    }
                    orkesy_core::command::RunStatus::Exited { .. } => {
                        Style::default().fg(Color::Red)
                    }
                    orkesy_core::command::RunStatus::Killed => Style::default().fg(Color::Yellow),
                    orkesy_core::command::RunStatus::Failed { .. } => {
                        Style::default().fg(Color::Red)
                    }
                };
                let duration = run.duration_str();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", status_icon), style),
                    Span::raw(&run.display_name),
                    Span::styled(
                        format!(" ({})", duration),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect();

        // Update list state selection based on mode (before borrowing)
        match ui.left_mode {
            LeftMode::Services => {
                if !display_ids.is_empty() {
                    list_state.select(Some(*selected));
                }
            }
            LeftMode::Commands => {
                let cmd_count = snapshot
                    .project
                    .as_ref()
                    .map(|p| p.commands.len())
                    .unwrap_or(0);
                if cmd_count > 0 {
                    if ui.selected_command >= cmd_count {
                        ui.selected_command = cmd_count.saturating_sub(1);
                    }
                    command_list_state.select(Some(ui.selected_command));
                }
            }
            LeftMode::Runs => {
                let run_count = snapshot.run_order.len();
                if run_count > 0 {
                    if ui.selected_run >= run_count {
                        ui.selected_run = run_count.saturating_sub(1);
                    }
                    run_list_state.select(Some(ui.selected_run));
                }
            }
        }

        // Select items based on current left mode
        let items: Vec<ListItem> = match ui.left_mode {
            LeftMode::Services => service_items,
            LeftMode::Commands => command_items,
            LeftMode::Runs => run_items,
        };

        let build_logs = |id: &str| -> Vec<Line> {
            // Helper to get service color for merged view
            let service_color = |svc: &str| -> Style {
                // Hash-based color assignment for consistent colors
                let colors = [
                    Color::Cyan,
                    Color::Green,
                    Color::Yellow,
                    Color::Magenta,
                    Color::Blue,
                    Color::Red,
                ];
                let hash: usize = svc.bytes().map(|b| b as usize).sum();
                Style::default().fg(colors[hash % colors.len()])
            };

            if id == "all" {
                // Show merged logs with service prefixes
                if snapshot.logs.merged.is_empty() {
                    return vec![Line::from("No logs yet.")];
                }
                snapshot
                    .logs
                    .merged
                    .iter()
                    .map(|l| {
                        let prefix = format!("{:8}│ ", l.service_id);
                        let style = service_color(&l.service_id);
                        // Add stream indicator
                        let stream_prefix = match l.stream {
                            LogStream::Stdout => "",
                            LogStream::Stderr => "[stderr] ",
                            LogStream::System => "[system] ",
                        };
                        Line::from(vec![
                            Span::styled(prefix, style),
                            Span::styled(stream_prefix, Style::default().fg(Color::DarkGray)),
                            Span::raw(&l.text),
                        ])
                    })
                    .collect()
            } else if let Some(lines) = snapshot.logs.per_service.get(id) {
                if lines.is_empty() {
                    return vec![Line::from("No logs yet.")];
                }
                lines
                    .iter()
                    .map(|l| {
                        // Add stream indicator
                        let stream_prefix = match l.stream {
                            LogStream::Stdout => "",
                            LogStream::Stderr => "[stderr] ",
                            LogStream::System => "[system] ",
                        };
                        Line::from(vec![
                            Span::styled(stream_prefix, Style::default().fg(Color::DarkGray)),
                            Span::raw(&l.text),
                        ])
                    })
                    .collect()
            } else {
                vec![Line::from("No logs yet.")]
            }
        };

        let _build_graph = || -> Vec<Line> {
            let cyan = Style::default().fg(Color::Cyan);
            let dim = Style::default().fg(Color::DarkGray);
            let green = Style::default().fg(Color::Green);
            let red = Style::default().fg(Color::Red);
            let yellow = Style::default().fg(Color::Yellow);

            let mut out: Vec<Line> = vec![
                Line::from(vec![Span::styled(
                    "Dependency Graph",
                    Style::default().add_modifier(Modifier::BOLD),
                )]),
                Line::from(""),
            ];

            // Build dependency map
            let mut by_from: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
            let mut has_deps: std::collections::HashSet<String> = std::collections::HashSet::new();

            for e in snapshot.graph.edges.iter() {
                by_from
                    .entry(e.from.clone())
                    .or_default()
                    .push((e.to.clone(), format!("{:?}", e.kind)));
                has_deps.insert(e.to.clone());
            }

            // Show all units with their status and dependencies
            let service_ids: Vec<_> = snapshot.graph.nodes.keys().cloned().collect();

            for id in &service_ids {
                let node = snapshot.graph.nodes.get(id).unwrap();

                // Status icon and style
                let (status_icon, unit_style) = match &node.observed.status {
                    ServiceStatus::Running => ("●", green),
                    ServiceStatus::Stopped => ("○", dim),
                    ServiceStatus::Starting => ("◐", yellow),
                    ServiceStatus::Errored { .. } => ("✗", red),
                    ServiceStatus::Exited { .. } => ("◌", dim),
                    _ => ("?", dim),
                };

                // Health icon
                let health_icon = match &node.observed.health {
                    HealthStatus::Healthy => " ♥",
                    HealthStatus::Degraded { .. } => " △",
                    HealthStatus::Unhealthy { .. } => " ✗",
                    _ => "",
                };

                // Metrics if available
                let metrics_str = if let Some(m) = snapshot.metrics.get(id) {
                    if node.observed.status == ServiceStatus::Running {
                        format!(
                            " [{:.1}% {}]",
                            m.cpu_percent,
                            adapters::format_bytes(m.memory_bytes)
                        )
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                // Unit line with status
                out.push(Line::from(vec![
                    Span::styled(format!("{} ", status_icon), unit_style),
                    Span::styled(&node.display_name, cyan),
                    Span::styled(health_icon, green),
                    Span::styled(metrics_str, dim),
                ]));

                // Dependencies from this unit
                if let Some(deps) = by_from.get(id) {
                    let deps_clone: Vec<_> =
                        deps.iter().map(|(t, k)| (t.clone(), k.clone())).collect();
                    for (i, (to, kind)) in deps_clone.iter().enumerate() {
                        let connector = if i == deps_clone.len() - 1 {
                            "└"
                        } else {
                            "├"
                        };
                        let to_node = snapshot.graph.nodes.get(to);
                        let to_status = to_node
                            .map(|n| match &n.observed.status {
                                ServiceStatus::Running => "●",
                                ServiceStatus::Stopped => "○",
                                _ => "?",
                            })
                            .unwrap_or("?");
                        let to_style = to_node
                            .map(|n| status_style(&n.observed.status))
                            .unwrap_or(dim);

                        out.push(Line::from(vec![
                            Span::raw(format!("  {}── ", connector)),
                            Span::styled(to_status, to_style),
                            Span::raw(" "),
                            Span::raw(to.clone()),
                            Span::styled(format!(" ({})", kind), dim),
                        ]));
                    }
                }

                out.push(Line::from(""));
            }

            if service_ids.is_empty() {
                out.push(Line::from("(no units)"));
            } else if snapshot.graph.edges.is_empty() {
                out.push(Line::styled("No dependencies defined.", dim));
                out.push(Line::from(""));
                out.push(Line::styled("Define edges in orkesy.yml:", dim));
                out.push(Line::styled("  edges:", dim));
                out.push(Line::styled("    - from: api", dim));
                out.push(Line::styled("      to: db", dim));
                out.push(Line::styled("      kind: depends_on", dim));
            }

            out
        };

        let build_inspect = |id: &str| -> Vec<Line> {
            let cyan = Style::default().fg(Color::Cyan);
            let dim = Style::default().fg(Color::DarkGray);
            let green = Style::default().fg(Color::Green);
            let yellow = Style::default().fg(Color::Yellow);
            let red = Style::default().fg(Color::Red);
            let bold = Style::default().add_modifier(Modifier::BOLD);
            let mut out: Vec<Line> = vec![];

            // Handle "all" - show summary of all services
            if id == "all" {
                out.push(Line::from(vec![Span::styled(
                    "All Services Summary",
                    bold.fg(Color::White),
                )]));
                out.push(Line::from(""));

                // Count statuses
                let running = snapshot
                    .graph
                    .nodes
                    .values()
                    .filter(|n| n.observed.status == ServiceStatus::Running)
                    .count();
                let stopped = snapshot
                    .graph
                    .nodes
                    .values()
                    .filter(|n| matches!(n.observed.status, ServiceStatus::Stopped))
                    .count();
                let errored = snapshot
                    .graph
                    .nodes
                    .values()
                    .filter(|n| matches!(n.observed.status, ServiceStatus::Errored { .. }))
                    .count();
                let total = snapshot.graph.nodes.len();

                out.push(Line::from(vec![Span::styled(
                    "OVERVIEW",
                    cyan.add_modifier(Modifier::BOLD),
                )]));
                out.push(Line::from(vec![
                    Span::styled("  Total     ", dim),
                    Span::styled(format!("{}", total), Style::default().fg(Color::White)),
                ]));
                out.push(Line::from(vec![
                    Span::styled("  Running   ", dim),
                    Span::styled(format!("{}", running), green),
                ]));
                out.push(Line::from(vec![
                    Span::styled("  Stopped   ", dim),
                    Span::styled(format!("{}", stopped), dim),
                ]));
                if errored > 0 {
                    out.push(Line::from(vec![
                        Span::styled("  Errored   ", dim),
                        Span::styled(format!("{}", errored), red),
                    ]));
                }

                out.push(Line::from(""));

                // Aggregate metrics
                let total_cpu: f64 = snapshot
                    .metrics
                    .values()
                    .map(|m| m.cpu_percent as f64)
                    .sum();
                let total_mem: u64 = snapshot.metrics.values().map(|m| m.memory_bytes).sum();

                out.push(Line::from(vec![Span::styled(
                    "RESOURCES",
                    cyan.add_modifier(Modifier::BOLD),
                )]));
                out.push(Line::from(vec![
                    Span::styled("  CPU       ", dim),
                    Span::styled(format!("{:.1}%", total_cpu.abs()), green),
                ]));
                out.push(Line::from(vec![
                    Span::styled("  Memory    ", dim),
                    Span::styled(adapters::format_bytes(total_mem), green),
                ]));

                out.push(Line::from(""));

                // Per-service status list
                out.push(Line::from(vec![Span::styled(
                    "SERVICES",
                    cyan.add_modifier(Modifier::BOLD),
                )]));

                for (svc_id, node) in snapshot.graph.nodes.iter() {
                    let icon = status_icon(&node.observed.status);
                    let style = status_style(&node.observed.status);
                    let label = status_label(&node.observed.status);

                    let mut line_spans = vec![
                        Span::styled(format!("  {} ", icon), style),
                        Span::styled(format!("{:12}", svc_id), Style::default().fg(Color::White)),
                        Span::styled(format!(" {}", label), style),
                    ];

                    // Add metrics if running
                    if let Some(metrics) = snapshot.metrics.get(svc_id) {
                        line_spans.push(Span::styled(
                            format!(
                                "  {:.1}% / {}",
                                metrics.cpu_percent.abs(),
                                adapters::format_bytes(metrics.memory_bytes)
                            ),
                            dim,
                        ));
                    }

                    out.push(Line::from(line_spans));
                }

                return out;
            }

            let node = snapshot.graph.nodes.get(id);
            let unit = units_map.get(id);

            if node.is_none() && unit.is_none() {
                out.push(Line::from("No unit selected".to_string()));
            } else {
                // ─────────────── Header ───────────────
                let display_name = node
                    .map(|n| n.display_name.clone())
                    .or(unit.and_then(|u| u.name.clone()))
                    .unwrap_or_else(|| id.to_string());
                out.push(Line::from(vec![Span::styled(
                    display_name,
                    bold.fg(Color::White),
                )]));
                out.push(Line::from(""));

                // ─────────────── Status ───────────────
                if let Some(node) = node {
                    out.push(Line::from(vec![Span::styled(
                        "STATUS",
                        cyan.add_modifier(Modifier::BOLD),
                    )]));

                    // State with icon
                    let state_str = format!(
                        "{} {}",
                        status_icon(&node.observed.status),
                        status_label(&node.observed.status)
                    );
                    out.push(Line::from(vec![
                        Span::styled("  State   ", dim),
                        Span::styled(state_str, status_style(&node.observed.status)),
                    ]));

                    // Health
                    let health_str = match &node.observed.health {
                        HealthStatus::Healthy => {
                            format!("{} healthy", health_icon(&node.observed.health))
                        }
                        HealthStatus::Unhealthy { reason } => format!(
                            "{} unhealthy: {}",
                            health_icon(&node.observed.health),
                            reason
                        ),
                        HealthStatus::Degraded { reason } => format!(
                            "{} degraded: {}",
                            health_icon(&node.observed.health),
                            reason
                        ),
                        HealthStatus::Unknown => {
                            format!("{} unknown", health_icon(&node.observed.health))
                        }
                    };
                    out.push(Line::from(vec![
                        Span::styled("  Health  ", dim),
                        Span::styled(health_str, health_style(&node.observed.health)),
                    ]));

                    // Metrics (if running)
                    if let Some(metrics) = snapshot.metrics.get(id) {
                        if node.observed.status == ServiceStatus::Running {
                            out.push(Line::from(vec![
                                Span::styled("  CPU     ", dim),
                                Span::styled(format!("{:.1}%", metrics.cpu_percent), green),
                            ]));
                            out.push(Line::from(vec![
                                Span::styled("  Memory  ", dim),
                                Span::styled(adapters::format_bytes(metrics.memory_bytes), green),
                            ]));
                            let mins = metrics.uptime_secs / 60;
                            let secs = metrics.uptime_secs % 60;
                            let uptime_str = if mins > 0 {
                                format!("{}m {}s", mins, secs)
                            } else {
                                format!("{}s", secs)
                            };
                            out.push(Line::from(vec![
                                Span::styled("  Uptime  ", dim),
                                Span::raw(uptime_str),
                            ]));
                            if let Some(pid) = metrics.pid {
                                out.push(Line::from(vec![
                                    Span::styled("  PID     ", dim),
                                    Span::raw(format!("{}", pid)),
                                ]));
                            }
                        }
                    }

                    // Error message if errored
                    if let ServiceStatus::Errored { message } = &node.observed.status {
                        out.push(Line::from(""));
                        out.push(Line::from(vec![
                            Span::styled(
                                "  Error: ",
                                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(message.clone(), Style::default().fg(Color::Red)),
                        ]));
                    }

                    out.push(Line::from(""));
                }

                // ─────────────── Configuration ───────────────
                out.push(Line::from(vec![Span::styled(
                    "CONFIGURATION",
                    cyan.add_modifier(Modifier::BOLD),
                )]));

                if let Some(unit) = unit {
                    // Kind
                    out.push(Line::from(vec![
                        Span::styled("  Kind    ", dim),
                        Span::raw(format!("{:?}", unit.kind)),
                    ]));

                    // Port
                    if let Some(port) = unit.port {
                        out.push(Line::from(vec![
                            Span::styled("  Port    ", dim),
                            Span::raw(format!("{}", port)),
                        ]));
                    }

                    // Working directory
                    if let Some(cwd) = &unit.cwd {
                        out.push(Line::from(vec![
                            Span::styled("  Cwd     ", dim),
                            Span::raw(cwd.display().to_string()),
                        ]));
                    }

                    // Start command
                    out.push(Line::from(vec![
                        Span::styled("  Start   ", dim),
                        Span::styled(&unit.start, yellow),
                    ]));

                    // Stop behavior
                    let stop_str = match &unit.stop {
                        orkesy_core::unit::StopBehavior::Signal(sig) => format!("{:?}", sig),
                        orkesy_core::unit::StopBehavior::Command(cmd) => cmd.clone(),
                    };
                    out.push(Line::from(vec![
                        Span::styled("  Stop    ", dim),
                        Span::raw(stop_str),
                    ]));

                    // Install commands
                    if !unit.install.is_empty() {
                        out.push(Line::from(vec![Span::styled("  Install ", dim)]));
                        for cmd in &unit.install {
                            out.push(Line::from(vec![
                                Span::styled("    ", dim),
                                Span::styled(cmd, yellow),
                            ]));
                        }
                    }

                    // Environment variables
                    if !unit.env.is_empty() {
                        out.push(Line::from(vec![Span::styled("  Env     ", dim)]));
                        for (k, v) in &unit.env {
                            out.push(Line::from(vec![
                                Span::styled("    ", dim),
                                Span::raw(format!("{}={}", k, v)),
                            ]));
                        }
                    }

                    // Description
                    if let Some(desc) = &unit.description {
                        out.push(Line::from(vec![
                            Span::styled("  Desc    ", dim),
                            Span::raw(desc.clone()),
                        ]));
                    }

                    // Health check config
                    if let Some(health) = &unit.health {
                        let health_cfg = match health {
                            orkesy_core::unit::HealthCheck::Tcp { port, interval_ms } => {
                                format!("tcp:{} every {}ms", port, interval_ms)
                            }
                            orkesy_core::unit::HealthCheck::Http {
                                url, interval_ms, ..
                            } => {
                                format!("http {} every {}ms", url, interval_ms)
                            }
                            orkesy_core::unit::HealthCheck::Exec {
                                command,
                                interval_ms,
                            } => {
                                format!("exec \"{}\" every {}ms", command, interval_ms)
                            }
                        };
                        out.push(Line::from(vec![
                            Span::styled("  Health  ", dim),
                            Span::raw(health_cfg),
                        ]));
                    }
                } else if let Some(node) = node {
                    // Fallback to node info if unit not available
                    out.push(Line::from(vec![
                        Span::styled("  Kind    ", dim),
                        Span::raw(format!("{:?}", node.kind)),
                    ]));
                    if let Some(port) = node.port {
                        out.push(Line::from(vec![
                            Span::styled("  Port    ", dim),
                            Span::raw(format!("{}", port)),
                        ]));
                    }
                    if let Some(desc) = &node.description {
                        out.push(Line::from(vec![
                            Span::styled("  Desc    ", dim),
                            Span::raw(desc.clone()),
                        ]));
                    }
                }

                out.push(Line::from(""));

                // ─────────────── Dependencies ───────────────
                let deps: Vec<&str> = snapshot
                    .graph
                    .edges
                    .iter()
                    .filter(|e| e.from == id)
                    .map(|e| e.to.as_str())
                    .collect();
                if !deps.is_empty() {
                    out.push(Line::from(vec![Span::styled(
                        "DEPENDS ON",
                        cyan.add_modifier(Modifier::BOLD),
                    )]));
                    for dep in deps {
                        let dep_node = snapshot.graph.nodes.get(dep);
                        let dep_icon = dep_node
                            .map(|n| status_icon(&n.observed.status))
                            .unwrap_or("?");
                        let dep_style = dep_node
                            .map(|n| status_style(&n.observed.status))
                            .unwrap_or(dim);
                        out.push(Line::from(vec![
                            Span::styled(format!("  {} ", dep_icon), dep_style),
                            Span::raw(dep),
                        ]));
                    }
                    out.push(Line::from(""));
                }

                // ─────────────── Dependents ───────────────
                let dependents: Vec<&str> = snapshot
                    .graph
                    .edges
                    .iter()
                    .filter(|e| e.to == id)
                    .map(|e| e.from.as_str())
                    .collect();
                if !dependents.is_empty() {
                    out.push(Line::from(vec![Span::styled(
                        "DEPENDED ON BY",
                        cyan.add_modifier(Modifier::BOLD),
                    )]));
                    for dep in dependents {
                        let dep_node = snapshot.graph.nodes.get(dep);
                        let dep_icon = dep_node
                            .map(|n| status_icon(&n.observed.status))
                            .unwrap_or("?");
                        let dep_style = dep_node
                            .map(|n| status_style(&n.observed.status))
                            .unwrap_or(dim);
                        out.push(Line::from(vec![
                            Span::styled(format!("  {} ", dep_icon), dep_style),
                            Span::raw(dep),
                        ]));
                    }
                    out.push(Line::from(""));
                }

                // ─────────────── Quick Actions ───────────────
                out.push(Line::from(""));
                out.push(Line::from(vec![Span::styled(
                    "ACTIONS",
                    dim.add_modifier(Modifier::BOLD),
                )]));
                out.push(Line::from(vec![
                    Span::styled("  Enter", Style::default().fg(Color::Cyan)),
                    Span::styled(" toggle  ", dim),
                    Span::styled("r", Style::default().fg(Color::Cyan)),
                    Span::styled(" restart  ", dim),
                    Span::styled("k", Style::default().fg(Color::Cyan)),
                    Span::styled(" kill  ", dim),
                    Span::styled("c", Style::default().fg(Color::Cyan)),
                    Span::styled(" clear logs", dim),
                ]));
                out.push(Line::from(vec![
                    Span::styled("  l", Style::default().fg(Color::Cyan)),
                    Span::styled(" logs view  ", dim),
                    Span::styled("d", Style::default().fg(Color::Cyan)),
                    Span::styled(" graph view  ", dim),
                    Span::styled("/", Style::default().fg(Color::Cyan)),
                    Span::styled(" command", dim),
                ]));
            } // end of else block

            out
        };

        // Build dependency tree view for selected service
        let build_deps = |id: &str| -> Vec<Line> {
            let cyan = Style::default().fg(Color::Cyan);
            let dim = Style::default().fg(Color::DarkGray);
            let bold = Style::default().add_modifier(Modifier::BOLD);
            let mut out: Vec<Line> = vec![];

            // Header
            out.push(Line::from(vec![Span::styled(
                "Dependencies",
                bold.fg(Color::White),
            )]));
            out.push(Line::from(""));

            // Special case for "all"
            if id == "all" {
                out.push(Line::from(vec![Span::styled(
                    "Select a specific service to view its dependencies.",
                    dim,
                )]));
                return out;
            }

            // Upstream dependencies (what this service depends on)
            let deps: Vec<String> = snapshot
                .graph
                .edges
                .iter()
                .filter(|e| e.from == id)
                .map(|e| e.to.clone())
                .collect();

            out.push(Line::from(vec![Span::styled(
                "DEPENDS ON",
                cyan.add_modifier(Modifier::BOLD),
            )]));

            if deps.is_empty() {
                out.push(Line::from(vec![Span::styled("  (none)", dim)]));
            } else {
                for dep in &deps {
                    let dep_node = snapshot.graph.nodes.get(dep);
                    let dep_icon = dep_node
                        .map(|n| status_icon(&n.observed.status))
                        .unwrap_or("?");
                    let dep_style = dep_node
                        .map(|n| status_style(&n.observed.status))
                        .unwrap_or(dim);
                    out.push(Line::from(vec![
                        Span::styled(format!("  {} ", dep_icon), dep_style),
                        Span::raw(dep.clone()),
                    ]));
                }
            }
            out.push(Line::from(""));

            // Downstream dependents (what depends on this service)
            let dependents: Vec<String> = snapshot
                .graph
                .edges
                .iter()
                .filter(|e| e.to == id)
                .map(|e| e.from.clone())
                .collect();

            out.push(Line::from(vec![Span::styled(
                "USED BY",
                cyan.add_modifier(Modifier::BOLD),
            )]));

            if dependents.is_empty() {
                out.push(Line::from(vec![Span::styled("  (none)", dim)]));
            } else {
                for dep in &dependents {
                    let dep_node = snapshot.graph.nodes.get(dep);
                    let dep_icon = dep_node
                        .map(|n| status_icon(&n.observed.status))
                        .unwrap_or("?");
                    let dep_style = dep_node
                        .map(|n| status_style(&n.observed.status))
                        .unwrap_or(dim);
                    out.push(Line::from(vec![
                        Span::styled(format!("  {} ", dep_icon), dep_style),
                        Span::raw(dep.clone()),
                    ]));
                }
            }

            out
        };

        // Build Exec view content (commands explorer)
        let build_exec = || -> Vec<Line> {
            let cyan = Style::default().fg(Color::Cyan);
            let dim = Style::default().fg(Color::DarkGray);
            let bold = Style::default().add_modifier(Modifier::BOLD);
            let green = Style::default().fg(Color::Green);
            let yellow = Style::default().fg(Color::Yellow);
            let mut out: Vec<Line> = vec![];

            // Header
            out.push(Line::from(vec![
                Span::styled("Commands Explorer", bold.fg(Color::White)),
                Span::styled("  •  Press ", dim),
                Span::styled(":", cyan),
                Span::styled(" to open command picker", dim),
            ]));
            out.push(Line::from(""));

            // Currently selected service (if any)
            if let Some(sid) = selected_id {
                if sid != "all" {
                    out.push(Line::from(vec![
                        Span::styled("SELECTED SERVICE: ", cyan.add_modifier(Modifier::BOLD)),
                        Span::styled(
                            sid,
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    out.push(Line::from(vec![
                        Span::styled("  ● ", green),
                        Span::styled("start", Style::default().fg(Color::White)),
                        Span::styled(format!("  → start {}", sid), dim),
                    ]));
                    out.push(Line::from(vec![
                        Span::styled("  ○ ", Style::default().fg(Color::Red)),
                        Span::styled("stop", Style::default().fg(Color::White)),
                        Span::styled(format!("   → stop {}", sid), dim),
                    ]));
                    out.push(Line::from(vec![
                        Span::styled("  ⟲ ", yellow),
                        Span::styled("restart", Style::default().fg(Color::White)),
                        Span::styled(format!(" → restart {}", sid), dim),
                    ]));
                    out.push(Line::from(vec![
                        Span::styled("  ✗ ", Style::default().fg(Color::Red)),
                        Span::styled("kill", Style::default().fg(Color::White)),
                        Span::styled(format!("   → kill {}", sid), dim),
                    ]));
                    out.push(Line::from(""));
                }
            }

            // Project Commands (from detected commands)
            if let Some(project) = &snapshot.project {
                if !project.commands.is_empty() {
                    out.push(Line::from(vec![Span::styled(
                        "PROJECT COMMANDS",
                        cyan.add_modifier(Modifier::BOLD),
                    )]));

                    for cmd in project.commands_sorted().iter().take(15) {
                        let cat_icon = cmd.category.icon();
                        out.push(Line::from(vec![
                            Span::styled(format!("  {} ", cat_icon), green),
                            Span::styled(&cmd.display_name, Style::default().fg(Color::White)),
                            Span::styled(format!("  ({})", cmd.tool.short_name()), dim),
                        ]));
                    }
                    if project.commands.len() > 15 {
                        out.push(Line::from(vec![Span::styled(
                            format!("  ... and {} more", project.commands.len() - 15),
                            dim,
                        )]));
                    }
                    out.push(Line::from(""));
                }
            }

            // All Service Commands
            out.push(Line::from(vec![Span::styled(
                "ALL SERVICES",
                cyan.add_modifier(Modifier::BOLD),
            )]));

            for (id, node) in snapshot.graph.nodes.iter() {
                let status_icon = match &node.observed.status {
                    ServiceStatus::Running => "●",
                    ServiceStatus::Stopped => "○",
                    ServiceStatus::Starting => "◐",
                    ServiceStatus::Restarting => "⟲",
                    ServiceStatus::Exited { .. } => "◌",
                    ServiceStatus::Errored { .. } => "✗",
                    ServiceStatus::Unknown => "?",
                };
                let status_style = match &node.observed.status {
                    ServiceStatus::Running => green,
                    ServiceStatus::Stopped => dim,
                    ServiceStatus::Starting | ServiceStatus::Restarting => yellow,
                    ServiceStatus::Exited { .. } => dim,
                    ServiceStatus::Errored { .. } => Style::default().fg(Color::Red),
                    ServiceStatus::Unknown => dim,
                };

                out.push(Line::from(vec![
                    Span::styled(format!("  {} ", status_icon), status_style),
                    Span::styled(id, Style::default().fg(Color::White)),
                    Span::styled("  start • stop • restart • kill • clear", dim),
                ]));
            }
            out.push(Line::from(""));

            // Quick actions
            out.push(Line::from(vec![Span::styled(
                "QUICK ACTIONS",
                cyan.add_modifier(Modifier::BOLD),
            )]));
            out.push(Line::from(vec![
                Span::styled("  ◉ ", green),
                Span::styled("Start all", Style::default().fg(Color::White)),
                Span::styled("    → start all", dim),
            ]));
            out.push(Line::from(vec![
                Span::styled("  ○ ", Style::default().fg(Color::Red)),
                Span::styled("Stop all", Style::default().fg(Color::White)),
                Span::styled("     → stop all", dim),
            ]));
            out.push(Line::from(vec![
                Span::styled("  ⟲ ", yellow),
                Span::styled("Restart all", Style::default().fg(Color::White)),
                Span::styled("  → restart all", dim),
            ]));
            out.push(Line::from(""));

            // Controls hint
            out.push(Line::from(vec![
                Span::styled("Tip: ", dim),
                Span::styled("Press ", dim),
                Span::styled(":", cyan),
                Span::styled(" to open the command picker and run any command.", dim),
            ]));

            out
        };

        // Build Metrics view content with ASCII gauges
        let build_metrics = || -> Vec<Line> {
            let cyan = Style::default().fg(Color::Cyan);
            let dim = Style::default().fg(Color::DarkGray);
            let _bold = Style::default().add_modifier(Modifier::BOLD);
            let _green = Style::default().fg(Color::Green);
            let yellow = Style::default().fg(Color::Yellow);
            let mut out: Vec<Line> = vec![];

            // Helper to create ASCII gauge: [████████░░] 80%
            let make_gauge = |percent: f64, width: usize| -> Vec<Span> {
                let filled = ((percent / 100.0) * width as f64).round() as usize;
                let empty = width.saturating_sub(filled);
                let bar: String = "█".repeat(filled) + &"░".repeat(empty);
                let color = if percent > 80.0 {
                    Color::Red
                } else if percent > 50.0 {
                    Color::Yellow
                } else {
                    Color::Green
                };
                vec![
                    Span::raw("["),
                    Span::styled(bar, Style::default().fg(color)),
                    Span::raw("] "),
                    Span::styled(format!("{:5.1}%", percent), Style::default().fg(color)),
                ]
            };

            // Header with summary and pause state
            let running_count = snapshot
                .graph
                .nodes
                .values()
                .filter(|n| n.observed.status == ServiceStatus::Running)
                .count();
            let total = snapshot.graph.nodes.len();

            let pause_hint = if ui.metrics_paused {
                vec![
                    Span::styled("  |  ", dim),
                    Span::styled(
                        "[PAUSED]",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  Press ", dim),
                    Span::styled("p", Style::default().fg(Color::Cyan)),
                    Span::styled(" to resume", dim),
                ]
            } else {
                vec![
                    Span::styled("  |  ", dim),
                    Span::styled("[LIVE]", Style::default().fg(Color::Green)),
                    Span::styled("  Press ", dim),
                    Span::styled("p", Style::default().fg(Color::Cyan)),
                    Span::styled(" to pause", dim),
                ]
            };

            let mut header = vec![Span::styled(
                format!("RUN {}/{}", running_count, total),
                cyan.add_modifier(Modifier::BOLD),
            )];
            header.extend(pause_hint);
            out.push(Line::from(header));
            out.push(Line::from(""));

            // Aggregate metrics with gauges
            let total_cpu: f64 = snapshot
                .metrics
                .values()
                .map(|m| m.cpu_percent as f64)
                .sum();
            let total_mem: u64 = snapshot.metrics.values().map(|m| m.memory_bytes).sum();
            // Estimate total memory as percentage (assume 8GB system)
            let mem_percent: f64 = (total_mem as f64 / (8.0 * 1024.0 * 1024.0 * 1024.0)) * 100.0;

            out.push(Line::from(vec![Span::styled(
                "SYSTEM TOTALS",
                cyan.add_modifier(Modifier::BOLD),
            )]));

            // CPU gauge
            let mut cpu_line = vec![Span::styled("  CPU ", dim)];
            cpu_line.extend(make_gauge(total_cpu.min(100.0), 20));
            out.push(Line::from(cpu_line));

            // Memory gauge
            let mut mem_line = vec![Span::styled("  MEM ", dim)];
            mem_line.extend(make_gauge(mem_percent.min(100.0), 20));
            mem_line.push(Span::styled(
                format!("  ({})", adapters::format_bytes(total_mem)),
                dim,
            ));
            out.push(Line::from(mem_line));

            out.push(Line::from(""));

            // Per-service metrics with mini gauges
            out.push(Line::from(vec![Span::styled(
                "PER SERVICE",
                cyan.add_modifier(Modifier::BOLD),
            )]));

            for (id, node) in snapshot.graph.nodes.iter() {
                let s_icon = status_icon(&node.observed.status);
                let s_style = status_style(&node.observed.status);

                if let Some(metrics) = snapshot.metrics.get(id) {
                    // Mini gauge (10 chars wide)
                    let cpu_pct = metrics.cpu_percent.abs(); // Avoid -0.0
                    let cpu_filled = ((cpu_pct as f64 / 100.0) * 10.0).round() as usize;
                    let cpu_bar: String = "▓".repeat(cpu_filled.min(10))
                        + &"░".repeat(10_usize.saturating_sub(cpu_filled));

                    out.push(Line::from(vec![
                        Span::styled(format!("  {} ", s_icon), s_style),
                        Span::styled(format!("{:10}", id), Style::default().fg(Color::White)),
                        Span::styled(format!(" {:5.1}% ", cpu_pct), yellow),
                        Span::styled(cpu_bar, yellow),
                        Span::styled(
                            format!("  {}", adapters::format_bytes(metrics.memory_bytes)),
                            dim,
                        ),
                        if let Some(pid) = metrics.pid {
                            Span::styled(format!("  pid:{}", pid), dim)
                        } else {
                            Span::raw("")
                        },
                    ]));
                } else {
                    // No metrics - show "stopped" for stopped services
                    let status_label = match &node.observed.status {
                        ServiceStatus::Stopped => "stopped",
                        ServiceStatus::Exited { code } => {
                            if *code == Some(0) {
                                "exited"
                            } else {
                                "failed"
                            }
                        }
                        ServiceStatus::Errored { .. } => "error",
                        _ => "—",
                    };
                    out.push(Line::from(vec![
                        Span::styled(format!("  {} ", s_icon), s_style),
                        Span::styled(format!("{:10}", id), Style::default().fg(Color::White)),
                        Span::styled(format!("  {}", status_label), dim),
                    ]));
                }
            }

            out
        };

        // Right content
        let right_text: Text = match ui.view {
            View::Deps => {
                if let Some(id) = selected_id {
                    Text::from(build_deps(id))
                } else {
                    Text::from(vec![Line::from("No service selected.")])
                }
            }
            View::Inspect => {
                if let Some(id) = selected_id {
                    Text::from(build_inspect(id))
                } else {
                    Text::from(vec![Line::from("No service selected.")])
                }
            }
            View::Exec => Text::from(build_exec()),
            View::Metrics => Text::from(build_metrics()),
            View::Logs => {
                if ui.logs.paused {
                    Text::from(
                        ui.logs
                            .frozen_logs
                            .iter()
                            .map(|log_line| {
                                if let Some(ts) = log_line.timestamp {
                                    Line::from(vec![
                                        Span::raw(log_line.text.clone()),
                                        Span::styled(
                                            format!(" {}", format_timestamp(ts)),
                                            Style::default().fg(Color::DarkGray),
                                        ),
                                    ])
                                } else {
                                    Line::from(log_line.text.clone())
                                }
                            })
                            .collect::<Vec<_>>(),
                    )
                } else if let Some(id) = selected_id {
                    Text::from(build_logs(id))
                } else {
                    Text::from(vec![Line::from("No service selected.")])
                }
            }
        };

        // Build picker items for command picker modal
        let picker_items: Vec<PickerItem> = if ui.palette_open {
            let all_items = build_picker_items(&service_ids, selected_id, &[]);
            filter_picker_items(&all_items, &ui.palette_input)
        } else {
            vec![]
        };

        if ui.palette_pick >= picker_items.len() && !picker_items.is_empty() {
            ui.palette_pick = picker_items.len() - 1;
        }
        if picker_items.is_empty() {
            ui.palette_pick = 0;
        }

        terminal.draw(|f| {
            let area = f.area();

            // Layout:
            // [ top bar ]
            // [ main (services + right pane) ]
            // [ footer ]
            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Top bar
                    Constraint::Min(1),    // Main area
                    Constraint::Length(1), // Footer
                ])
                .split(area);

            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
                .split(outer[1]);

            // ---------------- Top Status Bar ----------------
            // Calculate aggregate metrics
            let total_cpu: f64 = snapshot
                .metrics
                .values()
                .map(|m| m.cpu_percent as f64)
                .sum();
            let total_mem: u64 = snapshot.metrics.values().map(|m| m.memory_bytes).sum();

            // Calculate uptime (time since start)
            let uptime_secs = start_time.elapsed().as_secs();
            let uptime_mins = uptime_secs / 60;
            let uptime_hrs = uptime_mins / 60;
            let uptime_str = format!(
                "{:02}:{:02}:{:02}",
                uptime_hrs,
                uptime_mins % 60,
                uptime_secs % 60
            );

            let running_count = snapshot
                .graph
                .nodes
                .values()
                .filter(|n| n.observed.status == ServiceStatus::Running)
                .count();
            let total_services = snapshot.graph.nodes.len();

            let top_bar = Line::from(vec![
                Span::styled(" Orkesy ", styles::accent_bold()),
                Span::styled(project_name, styles::text()),
                Span::raw("  "),
                Span::styled(
                    format!("{}/{} running", running_count, total_services),
                    styles::success(),
                ),
                Span::raw("    "),
                Span::styled(format!("CPU {:.0}%", total_cpu), styles::warn()),
                Span::raw("  "),
                Span::styled(
                    format!("MEM {}", adapters::format_bytes(total_mem)),
                    styles::warn(),
                ),
                Span::raw("  "),
                Span::styled(format!("⏱ {}", uptime_str), styles::text_muted()),
            ]);
            f.render_widget(Paragraph::new(top_bar), outer[0]);

            // ---------------- Left: Mode-aware pane ----------------
            let left_focused = ui.focus == Focus::Units;
            let left_border_style = if left_focused {
                styles::border_focused()
            } else {
                styles::border_subtle()
            };
            let mode_label = ui.left_mode.label();
            let mode_key = ui.left_mode.key();
            let left_title = if left_focused {
                format!("▸ {} [{}]", mode_label, mode_key)
            } else {
                format!("{} [{}]", mode_label, mode_key)
            };
            let left = Block::default()
                .title(left_title)
                .borders(Borders::ALL)
                .border_style(left_border_style);
            let list = List::new(items)
                .block(left)
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");
            // Render with mode-appropriate list state
            match ui.left_mode {
                LeftMode::Services => f.render_stateful_widget(list, main[0], list_state),
                LeftMode::Commands => {
                    f.render_stateful_widget(list, main[0], &mut command_list_state)
                }
                LeftMode::Runs => f.render_stateful_widget(list, main[0], &mut run_list_state),
            }

            // ---------------- Right: Workspace ----------------
            let right_focused = ui.focus == Focus::RightPane;
            let right_border_style = if right_focused {
                styles::border_focused()
            } else {
                styles::border_subtle()
            };

            // Calculate visible area for scroll
            let right_inner_height = main[1].height.saturating_sub(2) as usize; // minus borders
            let _total_lines = match ui.view {
                View::Logs => {
                    if ui.logs.paused {
                        ui.logs.frozen_logs.len()
                    } else if let Some(id) = selected_id {
                        snapshot
                            .logs
                            .per_service
                            .get(id)
                            .map(|l| l.len())
                            .unwrap_or(0)
                    } else {
                        1
                    }
                }
                _ => 0,
            };

            // Build title - clean format per spec
            let unit_name = selected_id.unwrap_or("all");
            let raw_title = match ui.view {
                View::Deps => format!("Deps: {}", unit_name),
                View::Inspect => format!("Inspect: {}", unit_name),
                View::Exec => "Commands".to_string(),
                View::Metrics => format!("Metrics: {}", unit_name),
                View::Logs => {
                    // Format: "Logs: api [LIVE]" or "Logs: all [PAUSED]"
                    let status = if ui.logs.paused {
                        " [PAUSED]"
                    } else if ui.is_following() {
                        " [LIVE]"
                    } else {
                        ""
                    };
                    // Search indicator
                    let search_info = if let Some(query) = ui.search_query() {
                        if !query.is_empty() {
                            let match_count = ui.logs.matches.len();
                            let current = if match_count > 0 {
                                ui.logs.match_idx + 1
                            } else {
                                0
                            };
                            format!(" [/{} ({}/{})]", query, current, match_count)
                        } else {
                            " [/]".to_string()
                        }
                    } else {
                        String::new()
                    };
                    format!("Logs: {}{}{}", unit_name, status, search_info)
                }
            };

            let title = fit_title(&raw_title, main[1].width);

            // Color the title based on log state for logs view
            let title_style = match ui.view {
                View::Logs => {
                    if ui.logs.paused {
                        Style::default().fg(Color::Yellow)
                    } else if ui.is_following() {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Cyan)
                    }
                }
                _ => selected_id
                    .and_then(|id| snapshot.graph.nodes.get(id))
                    .map(|node| status_style(&node.observed.status))
                    .unwrap_or_default(),
            };

            // Apply scroll to logs - scroll from bottom, with optional search/select highlighting
            let log_scroll = ui.scroll_offset();
            let scrolled_text: Text = match ui.view {
                View::Logs => {
                    let raw_lines: Vec<DisplayLogLine> = if ui.logs.paused {
                        ui.logs.frozen_logs.clone()
                    } else if ui.left_mode == LeftMode::Runs {
                        // Runs mode: show logs for selected run
                        if let Some(run_id) = snapshot.run_order.get(ui.selected_run) {
                            if let Some(log_lines) = snapshot.logs.per_run.get(run_id) {
                                log_lines
                                    .iter()
                                    .map(|l| {
                                        let prefix = match l.stream {
                                            LogStream::Stderr => "[stderr] ",
                                            LogStream::System => "[system] ",
                                            LogStream::Stdout => "",
                                        };
                                        DisplayLogLine {
                                            timestamp: Some(l.at),
                                            text: format!("{}{}", prefix, l.text),
                                        }
                                    })
                                    .collect()
                            } else {
                                vec![DisplayLogLine {
                                    timestamp: None,
                                    text: "No output yet.".to_string(),
                                }]
                            }
                        } else {
                            vec![DisplayLogLine {
                                timestamp: None,
                                text: "No run selected.".to_string(),
                            }]
                        }
                    } else if let Some(id) = selected_id {
                        if id == "all" {
                            // Merged logs from all services
                            if snapshot.logs.merged.is_empty() {
                                vec![DisplayLogLine {
                                    timestamp: None,
                                    text: "No logs yet.".to_string(),
                                }]
                            } else {
                                snapshot
                                    .logs
                                    .merged
                                    .iter()
                                    .map(|l| {
                                        let prefix = format!("{:8}| ", l.service_id);
                                        let stream_prefix = match l.stream {
                                            LogStream::Stderr => "[stderr] ",
                                            LogStream::System => "[system] ",
                                            LogStream::Stdout => "",
                                        };
                                        DisplayLogLine {
                                            timestamp: Some(l.at),
                                            text: format!("{}{}{}", prefix, stream_prefix, l.text),
                                        }
                                    })
                                    .collect()
                            }
                        } else if let Some(log_lines) = snapshot.logs.per_service.get(id) {
                            log_lines
                                .iter()
                                .map(|l| DisplayLogLine {
                                    timestamp: Some(l.at),
                                    text: l.text.clone(),
                                })
                                .collect()
                        } else {
                            vec![DisplayLogLine {
                                timestamp: None,
                                text: "No logs yet.".to_string(),
                            }]
                        }
                    } else {
                        vec![DisplayLogLine {
                            timestamp: None,
                            text: "No service selected.".to_string(),
                        }]
                    };

                    // Apply log level filter
                    let filtered_lines: Vec<DisplayLogLine> =
                        if ui.logs.log_filter == LogFilterMode::All {
                            raw_lines
                        } else {
                            raw_lines
                                .into_iter()
                                .filter(|log_line| {
                                    let level = detect_level(&log_line.text);
                                    ui.logs.log_filter.matches(level)
                                })
                                .collect()
                        };

                    // Build lines with search highlighting and timestamps
                    let search_query = ui.search_query().map(|s| s.to_lowercase());
                    let search_match_idx = ui.logs.match_idx;

                    let all_lines: Vec<Line> = filtered_lines
                        .iter()
                        .enumerate()
                        .map(|(idx, log_line)| {
                            // Format timestamp if available
                            let ts_span = log_line.timestamp.map(|t| {
                                Span::styled(
                                    format!(" {}", format_timestamp(t)),
                                    Style::default().fg(Color::DarkGray),
                                )
                            });

                            // Check if line matches search
                            if let Some(ref query) = search_query {
                                if !query.is_empty() && log_line.text.to_lowercase().contains(query)
                                {
                                    let is_current =
                                        ui.logs.matches.get(search_match_idx) == Some(&idx);
                                    let style = if is_current {
                                        Style::default().bg(Color::Yellow).fg(Color::Black)
                                    } else {
                                        Style::default().bg(Color::DarkGray).fg(Color::White)
                                    };
                                    let mut spans =
                                        vec![Span::styled(log_line.text.clone(), style)];
                                    if let Some(ts) = ts_span {
                                        spans.push(ts);
                                    }
                                    return Line::from(spans);
                                }
                            }

                            // Normal line with timestamp
                            if let Some(ts) = ts_span {
                                Line::from(vec![Span::raw(log_line.text.clone()), ts])
                            } else {
                                Line::from(log_line.text.clone())
                            }
                        })
                        .collect();

                    if all_lines.is_empty() {
                        Text::from(vec![Line::from("No logs yet.")])
                    } else if ui.is_following() && !ui.logs.is_searching() {
                        // Follow mode: show last N lines (newest at bottom)
                        let start = all_lines.len().saturating_sub(right_inner_height);
                        Text::from(all_lines[start..].to_vec())
                    } else {
                        // Scroll mode: log_scroll = lines scrolled UP from bottom
                        let end = all_lines.len().saturating_sub(log_scroll);
                        let start = end.saturating_sub(right_inner_height);
                        Text::from(all_lines[start..end].to_vec())
                    }
                }
                _ => right_text,
            };

            // Special rendering for Inspect view with charts
            if ui.view == View::Inspect && selected_id.is_some() {
                let id = selected_id.unwrap();
                let right_height = main[1].height;

                // Adaptive layout based on available height
                // - Full layout (>= 30): Summary + 4 charts + Health
                // - Medium layout (20-29): Summary + 2 charts + Health
                // - Compact layout (< 20): Summary only
                let layout_mode = if right_height >= 30 {
                    "full"
                } else if right_height >= 20 {
                    "medium"
                } else {
                    "compact"
                };

                // Split right pane into sections based on available height
                let inspect_layout = match layout_mode {
                    "full" => Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(8), // Summary
                            Constraint::Min(12),   // Metrics charts
                            Constraint::Length(5), // Health
                        ])
                        .split(main[1]),
                    "medium" => Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(6), // Summary (compact)
                            Constraint::Min(8),    // Single chart row
                            Constraint::Length(4), // Health (compact)
                        ])
                        .split(main[1]),
                    _ => Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(1), // Summary fills space
                        ])
                        .split(main[1]),
                };

                let cyan = Style::default().fg(Color::Cyan);
                let dim = Style::default().fg(Color::DarkGray);
                let green = Style::default().fg(Color::Green);

                // Determine focused section for visual indicators
                let focused_section = match ui.focus {
                    Focus::InspectPanel(section) => Some(section),
                    _ => None,
                };
                let summary_border = if focused_section == Some(InspectSection::Summary) {
                    cyan
                } else {
                    dim
                };
                let metrics_border = if focused_section == Some(InspectSection::Metrics) {
                    cyan
                } else {
                    dim
                };
                let health_border = if focused_section == Some(InspectSection::Health) {
                    cyan
                } else {
                    dim
                };

                // ─────────────── Summary Section ───────────────
                let node = snapshot.graph.nodes.get(id);
                let unit = units_map.get(id);

                let mut summary_lines: Vec<Line> = vec![];

                if let Some(node) = node {
                    // Status
                    let status_str = format!(
                        "{} {}",
                        status_icon(&node.observed.status),
                        status_label(&node.observed.status)
                    );
                    summary_lines.push(Line::from(vec![
                        Span::styled("Status   ", dim),
                        Span::styled(status_str, status_style(&node.observed.status)),
                    ]));

                    // Health
                    let health_str = match &node.observed.health {
                        HealthStatus::Healthy => {
                            format!("{} healthy", health_icon(&node.observed.health))
                        }
                        HealthStatus::Unhealthy { reason } => {
                            format!("{} {}", health_icon(&node.observed.health), reason)
                        }
                        HealthStatus::Degraded { reason } => {
                            format!("{} {}", health_icon(&node.observed.health), reason)
                        }
                        HealthStatus::Unknown => {
                            format!("{} unknown", health_icon(&node.observed.health))
                        }
                    };
                    summary_lines.push(Line::from(vec![
                        Span::styled("Health   ", dim),
                        Span::styled(health_str, health_style(&node.observed.health)),
                    ]));

                    // PID and Uptime if running
                    if let Some(metrics) = snapshot.metrics.get(id) {
                        if let Some(pid) = metrics.pid {
                            summary_lines.push(Line::from(vec![
                                Span::styled("PID      ", dim),
                                Span::raw(format!("{}", pid)),
                            ]));
                        }
                        let mins = metrics.uptime_secs / 60;
                        let secs = metrics.uptime_secs % 60;
                        summary_lines.push(Line::from(vec![
                            Span::styled("Uptime   ", dim),
                            Span::raw(format!("{}m {}s", mins, secs)),
                        ]));
                        summary_lines.push(Line::from(vec![
                            Span::styled("CPU      ", dim),
                            Span::styled(format!("{:.1}%", metrics.cpu_percent.abs()), green),
                        ]));
                        summary_lines.push(Line::from(vec![
                            Span::styled("Memory   ", dim),
                            Span::styled(adapters::format_bytes(metrics.memory_bytes), green),
                        ]));
                    }
                }

                // Command from unit config
                if let Some(unit) = unit {
                    summary_lines.push(Line::from(vec![
                        Span::styled("Command  ", dim),
                        Span::raw(unit.start.clone()),
                    ]));
                }

                let summary_title = if focused_section == Some(InspectSection::Summary) {
                    format!(" {} ★ ", id)
                } else {
                    format!(" {} ", id)
                };
                let summary = Paragraph::new(summary_lines).block(
                    Block::default()
                        .title(Span::styled(
                            summary_title,
                            title_style.add_modifier(Modifier::BOLD),
                        ))
                        .borders(Borders::ALL)
                        .border_style(summary_border),
                );
                f.render_widget(summary, inspect_layout[0]);

                // ─────────────── Metrics Charts Section ───────────────
                // Only render charts in full or medium layout modes
                if layout_mode != "compact" && inspect_layout.len() > 1 {
                    // Get time-series data
                    let cpu_data = snapshot.metrics_series.system_cpu.as_vec();
                    let mem_data = snapshot.metrics_series.system_mem.as_vec();
                    let net_data = snapshot.metrics_series.system_net.as_vec();
                    let log_rate_data = snapshot
                        .metrics_series
                        .logs_rate
                        .get(id)
                        .map(|s| s.as_vec())
                        .unwrap_or_default();

                    // Calculate bounds with time labels
                    let (t_min, t_max) = if !cpu_data.is_empty() {
                        let min_t = cpu_data.first().map(|(t, _)| *t).unwrap_or(0.0);
                        let max_t = cpu_data.last().map(|(t, _)| *t).unwrap_or(60.0);
                        (min_t, max_t.max(min_t + 1.0))
                    } else {
                        (0.0, 60.0)
                    };

                    // Build x-axis labels: show relative time (e.g., "-60s", "-30s", "now")
                    let time_range = t_max - t_min;
                    let x_labels = if time_range > 0.0 {
                        vec![
                            Span::styled(format!("-{:.0}s", time_range), dim),
                            Span::styled("now", dim),
                        ]
                    } else {
                        vec![Span::styled("0s", dim), Span::styled("60s", dim)]
                    };

                    let metrics_title_suffix = if focused_section == Some(InspectSection::Metrics) {
                        " ★"
                    } else {
                        ""
                    };

                    if layout_mode == "full" {
                        // Full layout: 2x2 grid of charts
                        let chart_rows = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .split(inspect_layout[1]);

                        let chart_top = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .split(chart_rows[0]);

                        let chart_bottom = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .split(chart_rows[1]);

                        // CPU Chart
                        let cpu_dataset = Dataset::default()
                            .name("CPU")
                            .marker(symbols::Marker::Braille)
                            .graph_type(GraphType::Line)
                            .style(Style::default().fg(Color::Cyan))
                            .data(&cpu_data);

                        let cpu_chart = Chart::new(vec![cpu_dataset])
                            .block(
                                Block::default()
                                    .title(format!(" CPU %{} ", metrics_title_suffix))
                                    .borders(Borders::ALL)
                                    .border_style(metrics_border),
                            )
                            .x_axis(
                                Axis::default()
                                    .bounds([t_min, t_max])
                                    .labels(x_labels.clone()),
                            )
                            .y_axis(
                                Axis::default()
                                    .bounds([0.0, 100.0])
                                    .labels(vec![Span::raw("0"), Span::raw("100")]),
                            );

                        f.render_widget(cpu_chart, chart_top[0]);

                        // Memory Chart
                        let mem_max = mem_data.iter().map(|(_, v)| *v).fold(100.0_f64, f64::max);
                        let mem_dataset = Dataset::default()
                            .name("MEM")
                            .marker(symbols::Marker::Braille)
                            .graph_type(GraphType::Line)
                            .style(Style::default().fg(Color::Green))
                            .data(&mem_data);

                        let mem_chart = Chart::new(vec![mem_dataset])
                            .block(
                                Block::default()
                                    .title(" Memory MB ")
                                    .borders(Borders::ALL)
                                    .border_style(metrics_border),
                            )
                            .x_axis(
                                Axis::default()
                                    .bounds([t_min, t_max])
                                    .labels(x_labels.clone()),
                            )
                            .y_axis(Axis::default().bounds([0.0, mem_max.max(100.0)]).labels(
                                vec![Span::raw("0"), Span::raw(format!("{:.0}", mem_max))],
                            ));

                        f.render_widget(mem_chart, chart_top[1]);

                        // Network Chart
                        let net_max = net_data.iter().map(|(_, v)| *v).fold(10.0_f64, f64::max);
                        let net_dataset = Dataset::default()
                            .name("NET")
                            .marker(symbols::Marker::Braille)
                            .graph_type(GraphType::Line)
                            .style(Style::default().fg(Color::Yellow))
                            .data(&net_data);

                        let net_chart =
                            Chart::new(vec![net_dataset])
                                .block(
                                    Block::default()
                                        .title(" Network KB/s ")
                                        .borders(Borders::ALL)
                                        .border_style(metrics_border),
                                )
                                .x_axis(
                                    Axis::default()
                                        .bounds([t_min, t_max])
                                        .labels(x_labels.clone()),
                                )
                                .y_axis(Axis::default().bounds([0.0, net_max.max(10.0)]).labels(
                                    vec![Span::raw("0"), Span::raw(format!("{:.0}", net_max))],
                                ));

                        f.render_widget(net_chart, chart_bottom[0]);

                        // Log Rate Chart
                        let log_max = log_rate_data
                            .iter()
                            .map(|(_, v)| *v)
                            .fold(10.0_f64, f64::max);
                        let log_dataset = Dataset::default()
                            .name("LOGS")
                            .marker(symbols::Marker::Braille)
                            .graph_type(GraphType::Line)
                            .style(Style::default().fg(Color::Magenta))
                            .data(&log_rate_data);

                        let log_chart = Chart::new(vec![log_dataset])
                            .block(
                                Block::default()
                                    .title(" Logs/s ")
                                    .borders(Borders::ALL)
                                    .border_style(metrics_border),
                            )
                            .x_axis(Axis::default().bounds([t_min, t_max]).labels(x_labels))
                            .y_axis(Axis::default().bounds([0.0, log_max.max(1.0)]).labels(vec![
                                Span::raw("0"),
                                Span::raw(format!("{:.0}", log_max)),
                            ]));

                        f.render_widget(log_chart, chart_bottom[1]);
                    } else {
                        // Medium layout: 1x2 grid (CPU + Log rate only)
                        let chart_cols = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .split(inspect_layout[1]);

                        // CPU Chart
                        let cpu_dataset = Dataset::default()
                            .name("CPU")
                            .marker(symbols::Marker::Braille)
                            .graph_type(GraphType::Line)
                            .style(Style::default().fg(Color::Cyan))
                            .data(&cpu_data);

                        let cpu_chart = Chart::new(vec![cpu_dataset])
                            .block(
                                Block::default()
                                    .title(format!(" CPU %{} ", metrics_title_suffix))
                                    .borders(Borders::ALL)
                                    .border_style(metrics_border),
                            )
                            .x_axis(
                                Axis::default()
                                    .bounds([t_min, t_max])
                                    .labels(x_labels.clone()),
                            )
                            .y_axis(
                                Axis::default()
                                    .bounds([0.0, 100.0])
                                    .labels(vec![Span::raw("0"), Span::raw("100")]),
                            );

                        f.render_widget(cpu_chart, chart_cols[0]);

                        // Log Rate Chart
                        let log_max = log_rate_data
                            .iter()
                            .map(|(_, v)| *v)
                            .fold(10.0_f64, f64::max);
                        let log_dataset = Dataset::default()
                            .name("LOGS")
                            .marker(symbols::Marker::Braille)
                            .graph_type(GraphType::Line)
                            .style(Style::default().fg(Color::Magenta))
                            .data(&log_rate_data);

                        let log_chart = Chart::new(vec![log_dataset])
                            .block(
                                Block::default()
                                    .title(" Logs/s ")
                                    .borders(Borders::ALL)
                                    .border_style(metrics_border),
                            )
                            .x_axis(Axis::default().bounds([t_min, t_max]).labels(x_labels))
                            .y_axis(Axis::default().bounds([0.0, log_max.max(1.0)]).labels(vec![
                                Span::raw("0"),
                                Span::raw(format!("{:.0}", log_max)),
                            ]));

                        f.render_widget(log_chart, chart_cols[1]);
                    }
                }

                // ─────────────── Health Section ───────────────
                // Only render Health in full/medium layouts (index 2 exists)
                if layout_mode != "compact" && inspect_layout.len() > 2 {
                    let mut health_lines: Vec<Line> = vec![];

                    if let Some(node) = node {
                        let health_str = match &node.observed.health {
                            HealthStatus::Healthy => "All checks passing".to_string(),
                            HealthStatus::Unhealthy { reason } => format!("Failed: {}", reason),
                            HealthStatus::Degraded { reason } => format!("Degraded: {}", reason),
                            HealthStatus::Unknown => "No health checks configured".to_string(),
                        };
                        health_lines.push(Line::from(vec![
                            Span::styled("Status  ", dim),
                            Span::styled(health_str, health_style(&node.observed.health)),
                        ]));

                        // Show check interval if configured
                        if let Some(unit) = unit {
                            if unit.health.is_some() {
                                health_lines.push(Line::from(vec![
                                    Span::styled("Check   ", dim),
                                    Span::raw("HTTP health endpoint"),
                                ]));
                            }
                        }
                    }

                    let health_title = if focused_section == Some(InspectSection::Health) {
                        " Health ★ "
                    } else {
                        " Health "
                    };
                    let health_section = Paragraph::new(health_lines).block(
                        Block::default()
                            .title(health_title)
                            .borders(Borders::ALL)
                            .border_style(health_border),
                    );
                    f.render_widget(health_section, inspect_layout[2]);
                }
            } else {
                // Default rendering for other views
                let right = Paragraph::new(scrolled_text)
                    .block(
                        Block::default()
                            .title(Span::styled(title, title_style))
                            .borders(Borders::ALL)
                            .border_style(right_border_style),
                    )
                    .wrap(Wrap { trim: false });

                f.render_widget(right, main[1]);
            }

            // ---------------- Footer (always visible, context-sensitive) ----------------
            // Build footer: l Logs  i Inspect  d Deps  m Metrics  |  context hints  |  / cmd  q quit
            let view_tabs = vec![
                Span::styled("l", styles::key_hint()),
                Span::styled(
                    " Logs  ",
                    if ui.view == View::Logs {
                        styles::accent()
                    } else {
                        styles::text_dim()
                    },
                ),
                Span::styled("i", styles::key_hint()),
                Span::styled(
                    " Inspect  ",
                    if ui.view == View::Inspect {
                        styles::accent()
                    } else {
                        styles::text_dim()
                    },
                ),
                Span::styled("d", styles::key_hint()),
                Span::styled(
                    " Deps  ",
                    if ui.view == View::Deps {
                        styles::accent()
                    } else {
                        styles::text_dim()
                    },
                ),
                Span::styled("m", styles::key_hint()),
                Span::styled(
                    " Metrics",
                    if ui.view == View::Metrics {
                        styles::accent()
                    } else {
                        styles::text_dim()
                    },
                ),
                Span::styled("  |  ", styles::text_muted()),
            ];

            // Context-sensitive hints based on focus + view (keys in key_hint, labels in text_dim)
            let context_hints: Vec<Span> = match (ui.focus, ui.view) {
                (Focus::Palette, _) => vec![
                    Span::styled("↑↓", styles::key_hint()),
                    Span::styled(" select  ", styles::text_dim()),
                    Span::styled("Enter", styles::key_hint()),
                    Span::styled(" run  ", styles::text_dim()),
                    Span::styled("Esc", styles::key_hint()),
                    Span::styled(" close", styles::text_dim()),
                ],
                (Focus::Units, _) => vec![
                    Span::styled("↑↓", styles::key_hint()),
                    Span::styled(" select  ", styles::text_dim()),
                    Span::styled("r", styles::key_hint()),
                    Span::styled(" restart  ", styles::text_dim()),
                    Span::styled("s", styles::key_hint()),
                    Span::styled(" stop  ", styles::text_dim()),
                    Span::styled("t", styles::key_hint()),
                    Span::styled(" start", styles::text_dim()),
                ],
                (Focus::RightPane, View::Logs) if ui.logs.is_searching() => vec![
                    Span::styled("n/N", styles::key_hint()),
                    Span::styled(" match  ", styles::text_dim()),
                    Span::styled("Esc", styles::key_hint()),
                    Span::styled(" clear", styles::text_dim()),
                ],
                (Focus::RightPane, View::Logs) if ui.logs.paused => vec![
                    Span::styled("Space", styles::key_hint()),
                    Span::styled(" resume  ", styles::text_dim()),
                    Span::styled("↑↓", styles::key_hint()),
                    Span::styled(" scroll", styles::text_dim()),
                ],
                (Focus::RightPane, View::Logs) => {
                    let filter_label = ui.logs.log_filter.label();
                    let filter_style = if ui.logs.log_filter == LogFilterMode::All {
                        styles::text_dim()
                    } else {
                        styles::warn()
                    };
                    vec![
                        Span::styled("Space", styles::key_hint()),
                        Span::styled(" pause  ", styles::text_dim()),
                        Span::styled("f", styles::key_hint()),
                        Span::styled(" follow  ", styles::text_dim()),
                        Span::styled("s", styles::key_hint()),
                        Span::styled(" search  ", styles::text_dim()),
                        Span::styled("e/w/a", styles::key_hint()),
                        Span::styled(" filter  ", styles::text_dim()),
                        Span::styled(format!("[{}]", filter_label), filter_style),
                    ]
                }
                (Focus::RightPane, View::Metrics) => {
                    if ui.metrics_paused {
                        vec![
                            Span::styled("p", styles::key_hint()),
                            Span::styled(" resume  ", styles::text_dim()),
                            Span::styled("[PAUSED]", styles::warn()),
                        ]
                    } else {
                        vec![
                            Span::styled("p", styles::key_hint()),
                            Span::styled(" pause", styles::text_dim()),
                        ]
                    }
                }
                (Focus::InspectPanel(section), _) => {
                    let section_name = match section {
                        InspectSection::Summary => "Summary",
                        InspectSection::Metrics => "Metrics",
                        InspectSection::Health => "Health",
                    };
                    vec![
                        Span::styled("Tab", styles::key_hint()),
                        Span::styled(" section  ", styles::text_dim()),
                        Span::styled(format!("[{}]", section_name), styles::accent()),
                    ]
                }
                (Focus::RightPane, _) => vec![
                    Span::styled("↑↓", styles::key_hint()),
                    Span::styled(" scroll", styles::text_dim()),
                ],
            };

            // Global hints (keys in key_hint, labels in text_dim)
            let global_hints: Vec<Span> = vec![
                Span::styled("  Tab", styles::key_hint()),
                Span::styled(" focus  ", styles::text_dim()),
                Span::styled("/", styles::key_hint()),
                Span::styled(" cmd  ", styles::text_dim()),
                Span::styled("?", styles::key_hint()),
                Span::styled(" help  ", styles::text_dim()),
                Span::styled("q", styles::key_hint()),
                Span::styled(" quit", styles::text_dim()),
            ];

            // Combine all footer spans
            let mut footer_spans: Vec<Span> = view_tabs;
            footer_spans.extend(context_hints);
            footer_spans.extend(global_hints);

            f.render_widget(Paragraph::new(Line::from(footer_spans)), outer[2]);

            // ---------------- VS Code Style Command Picker Modal ----------------
            if ui.palette_open {
                // Centered modal - 60% width, max 50 chars, vertically centered
                let modal_width = (area.width * 60 / 100).min(60).max(30);
                let modal_height = (picker_items.len() as u16 + 4).min(area.height - 4).max(6);
                let modal_x = (area.width.saturating_sub(modal_width)) / 2;
                let modal_y = (area.height.saturating_sub(modal_height)) / 2;

                let modal_rect = Rect {
                    x: modal_x,
                    y: modal_y,
                    width: modal_width,
                    height: modal_height,
                };

                // Clear background and draw modal
                f.render_widget(Clear, modal_rect);

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(styles::border_focused())
                    .title(" Commands ");

                let inner = block.inner(modal_rect);
                f.render_widget(block, modal_rect);

                // Layout: input line at top, then items
                let modal_parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(inner);

                // Input line with > prompt
                let input_area = modal_parts[0];
                let prompt = "> ";
                let max_input_chars =
                    input_area.width.saturating_sub(prompt.len() as u16 + 1) as usize;

                let input_display: String = if ui.palette_input.len() > max_input_chars {
                    ui.palette_input.chars().take(max_input_chars).collect()
                } else {
                    ui.palette_input.clone()
                };

                let input_line = Line::from(vec![
                    Span::styled(prompt, styles::accent()),
                    Span::styled(&input_display, styles::text()),
                ]);
                f.render_widget(Paragraph::new(input_line), input_area);

                // Items list
                let items_area = modal_parts[1];
                let visible_count = items_area.height as usize;

                // Adjust scroll offset to keep selection visible
                if ui.palette_pick < ui.palette_sugg_offset {
                    ui.palette_sugg_offset = ui.palette_pick;
                } else if visible_count > 0
                    && ui.palette_pick >= ui.palette_sugg_offset + visible_count
                {
                    ui.palette_sugg_offset = ui.palette_pick.saturating_sub(visible_count - 1);
                }

                // Build list items with category grouping
                let mut list_items: Vec<ListItem> = vec![];
                let mut last_category: Option<PickerCategory> = None;

                for (i, item) in picker_items
                    .iter()
                    .skip(ui.palette_sugg_offset)
                    .take(visible_count)
                    .enumerate()
                {
                    let actual_idx = ui.palette_sugg_offset + i;
                    let is_selected = actual_idx == ui.palette_pick;

                    // Category header (only when not filtering and category changes)
                    if ui.palette_input.is_empty() && last_category.as_ref() != Some(&item.category)
                    {
                        if last_category.is_some() && list_items.len() < visible_count {
                            // Add separator line between categories
                            list_items.push(ListItem::new(Line::from("")));
                        }
                        last_category = Some(item.category.clone());
                    }

                    // Build item line
                    let icon = item.category.icon();
                    let prefix = if is_selected { "▸ " } else { "  " };

                    let item_style = if is_selected {
                        styles::selection()
                    } else {
                        styles::text()
                    };

                    let mut spans = vec![
                        Span::raw(prefix),
                        Span::styled(format!("{} ", icon), styles::text_muted()),
                        Span::styled(&item.label, item_style),
                    ];

                    // Add detail if present and space allows
                    if let Some(detail) = &item.detail {
                        let remaining = modal_width.saturating_sub(
                            prefix.len() as u16 + icon.len() as u16 + item.label.len() as u16 + 6,
                        );
                        if remaining > 10 {
                            let truncated_detail: String =
                                detail.chars().take(remaining as usize).collect();
                            spans.push(Span::styled(
                                format!("  {}", truncated_detail),
                                styles::text_dim(),
                            ));
                        }
                    }

                    list_items.push(ListItem::new(Line::from(spans)));
                }

                if list_items.is_empty() {
                    list_items.push(ListItem::new(Line::from(vec![Span::styled(
                        "  No matching commands",
                        styles::text_muted(),
                    )])));
                }

                f.render_widget(List::new(list_items), items_area);

                // Position cursor at end of input
                let cursor_x = input_area.x + prompt.len() as u16 + input_display.len() as u16;
                let cursor_y = input_area.y;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            // ---------------- Search Bar (when in search mode) ----------------
            if let Some(query) = &ui.logs.search {
                let search_h = 3u16;
                let search_rect = Rect {
                    x: main[1].x,
                    width: main[1].width,
                    height: search_h,
                    y: main[1].y + main[1].height.saturating_sub(search_h),
                };

                f.render_widget(Clear, search_rect);

                let match_info = if ui.logs.matches.is_empty() {
                    if query.is_empty() {
                        String::new()
                    } else {
                        " (no matches)".to_string()
                    }
                } else {
                    format!(" ({}/{})", ui.logs.match_idx + 1, ui.logs.matches.len())
                };

                let title = format!(" Search{} ", match_info);
                let block = Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow));

                let search_text = format!("/{}", query);
                f.render_widget(Paragraph::new(search_text).block(block), search_rect);

                let cursor_x = search_rect.x + 2 + query.len() as u16;
                let cursor_y = search_rect.y + 1;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            // ---------------- Help Overlay ----------------
            if ui.help_open {
                // Centered modal
                let help_width = 50u16.min(area.width - 4);
                let help_height = 24u16.min(area.height - 4);
                let help_x = (area.width.saturating_sub(help_width)) / 2;
                let help_y = (area.height.saturating_sub(help_height)) / 2;

                let help_rect = Rect {
                    x: help_x,
                    y: help_y,
                    width: help_width,
                    height: help_height,
                };

                f.render_widget(Clear, help_rect);

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(styles::border_focused())
                    .title(" Help - Press ? or Esc to close ");

                let inner = block.inner(help_rect);
                f.render_widget(block, help_rect);

                let help_lines = vec![
                    Line::from(vec![Span::styled("VIEWS", styles::section_header())]),
                    Line::from(vec![
                        Span::styled("  l ", styles::key_hint()),
                        Span::styled("Logs view", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  i ", styles::key_hint()),
                        Span::styled("Inspect view", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  e ", styles::key_hint()),
                        Span::styled("Exec (commands) view", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  d ", styles::key_hint()),
                        Span::styled("Dependencies view", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  m ", styles::key_hint()),
                        Span::styled("Metrics view", styles::text()),
                    ]),
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "FOCUS & NAVIGATION",
                        styles::section_header(),
                    )]),
                    Line::from(vec![
                        Span::styled("  Tab ", styles::key_hint()),
                        Span::styled("Switch focus (Services ↔ Right)", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  j/k ↑↓ ", styles::key_hint()),
                        Span::styled("Move selection / scroll", styles::text()),
                    ]),
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "SERVICE ACTIONS",
                        styles::section_header(),
                    )]),
                    Line::from(vec![
                        Span::styled("  Enter ", styles::key_hint()),
                        Span::styled("Toggle service (start/stop)", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  r     ", styles::key_hint()),
                        Span::styled("Restart service", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  s     ", styles::key_hint()),
                        Span::styled("Stop service", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  t     ", styles::key_hint()),
                        Span::styled("Start service", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  x     ", styles::key_hint()),
                        Span::styled("Kill service", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  c     ", styles::key_hint()),
                        Span::styled("Clear logs", styles::text()),
                    ]),
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "COMMANDS & SEARCH",
                        styles::section_header(),
                    )]),
                    Line::from(vec![
                        Span::styled("  /     ", styles::key_hint()),
                        Span::styled("Open command picker", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  s     ", styles::key_hint()),
                        Span::styled("Search logs (in Logs view)", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  ?     ", styles::key_hint()),
                        Span::styled("Toggle this help", styles::text()),
                    ]),
                    Line::from(vec![
                        Span::styled("  q     ", styles::key_hint()),
                        Span::styled("Quit Orkesy", styles::text()),
                    ]),
                ];

                f.render_widget(Paragraph::new(help_lines), inner);
            }
        })?;

        drop(snapshot);

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        let ev = event::read()?;
        let CEvent::Key(KeyEvent {
            code, modifiers, ..
        }) = ev
        else {
            continue;
        };

        // ========== KEY HANDLING (new state machine) ==========

        // Helper to get the total number of log lines for scrolling calculations
        let get_log_line_count = |snapshot: &RuntimeState,
                                  sid: Option<&str>,
                                  left_mode: LeftMode,
                                  selected_run: usize,
                                  paused: bool,
                                  frozen_logs: &[DisplayLogLine]|
         -> usize {
            if paused {
                frozen_logs.len()
            } else if left_mode == LeftMode::Runs {
                snapshot
                    .run_order
                    .get(selected_run)
                    .and_then(|run_id| snapshot.logs.per_run.get(run_id))
                    .map(|l| l.len())
                    .unwrap_or(0)
            } else if sid == Some("all") {
                snapshot.logs.merged.len()
            } else {
                sid.and_then(|id| snapshot.logs.per_service.get(id))
                    .map(|l| l.len())
                    .unwrap_or(0)
            }
        };

        // Helper to update search matches - searches the correct buffer based on view mode
        let update_search_matches = |query: &str,
                                     snapshot: &RuntimeState,
                                     sid: Option<&str>,
                                     left_mode: LeftMode,
                                     selected_run: usize,
                                     paused: bool,
                                     frozen_logs: &[DisplayLogLine]|
         -> Vec<usize> {
            if query.is_empty() {
                return vec![];
            }
            let search_lower = query.to_lowercase();

            // Get the log texts based on current view mode (matching the display logic)
            let logs: Vec<String> = if paused {
                frozen_logs.iter().map(|l| l.text.clone()).collect()
            } else if left_mode == LeftMode::Runs {
                // Search per_run logs
                snapshot
                    .run_order
                    .get(selected_run)
                    .and_then(|run_id| snapshot.logs.per_run.get(run_id))
                    .map(|l| l.iter().map(|x| x.text.clone()).collect())
                    .unwrap_or_default()
            } else if sid == Some("all") {
                // Search merged logs
                snapshot
                    .logs
                    .merged
                    .iter()
                    .map(|l| l.text.clone())
                    .collect()
            } else {
                // Search per_service logs
                sid.and_then(|id| snapshot.logs.per_service.get(id))
                    .map(|l| l.iter().map(|x| x.text.clone()).collect())
                    .unwrap_or_default()
            };

            logs.iter()
                .enumerate()
                .filter(|(_, text)| text.to_lowercase().contains(&search_lower))
                .map(|(idx, _)| idx)
                .collect()
        };

        // ---------- HELP MODE ----------
        if ui.help_open {
            match code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    ui.help_open = false;
                }
                _ => {}
            }
            continue;
        }

        // ---------- PALETTE MODE ----------
        if ui.palette_open {
            match (code, modifiers) {
                (KeyCode::Esc, _) => {
                    ui.palette_open = false;
                    ui.focus = Focus::Units; // Return focus to units
                    ui.palette_input.clear();
                    ui.palette_error = None;
                    ui.palette_pick = 0;
                    ui.palette_scroll = 0;
                    ui.palette_sugg_offset = 0;
                    ui.history_cursor = None;
                }
                (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                    if !ui.history.is_empty() {
                        let idx = ui
                            .history_cursor
                            .map(|i| i.saturating_sub(1))
                            .unwrap_or(ui.history.len().saturating_sub(1));
                        ui.history_cursor = Some(idx);
                        ui.palette_input = ui.history[idx].clone();
                        ui.palette_error = None;
                        ui.palette_pick = 0;
                    }
                }
                (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                    if !ui.history.is_empty() {
                        let idx = ui
                            .history_cursor
                            .map(|i| (i + 1).min(ui.history.len()))
                            .unwrap_or(ui.history.len());
                        if idx >= ui.history.len() {
                            ui.history_cursor = None;
                            ui.palette_input.clear();
                        } else {
                            ui.history_cursor = Some(idx);
                            ui.palette_input = ui.history[idx].clone();
                        }
                        ui.palette_error = None;
                        ui.palette_pick = 0;
                    }
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                    if ui.palette_pick > 0 {
                        ui.palette_pick -= 1;
                        // Keep scroll in sync - if selection goes above visible area
                        if ui.palette_pick < ui.palette_sugg_offset {
                            ui.palette_sugg_offset = ui.palette_pick;
                        }
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                    // Use picker_items for bounds check
                    let all_items = build_picker_items(&service_ids, selected_id, &[]);
                    let filtered = filter_picker_items(&all_items, &ui.palette_input);
                    if ui.palette_pick + 1 < filtered.len() {
                        ui.palette_pick += 1;
                        // Keep scroll in sync - estimate visible area (~15 items typical)
                        // This will be corrected by the draw loop for exact positioning
                        let estimated_visible = 15usize;
                        if ui.palette_pick >= ui.palette_sugg_offset + estimated_visible {
                            ui.palette_sugg_offset =
                                ui.palette_pick.saturating_sub(estimated_visible - 1);
                        }
                    }
                }
                (KeyCode::PageUp, _) => {
                    let page_size = 10usize;
                    ui.palette_pick = ui.palette_pick.saturating_sub(page_size);
                    ui.palette_sugg_offset = ui.palette_sugg_offset.saturating_sub(page_size);
                }
                (KeyCode::PageDown, _) => {
                    let all_items = build_picker_items(&service_ids, selected_id, &[]);
                    let filtered = filter_picker_items(&all_items, &ui.palette_input);
                    let page_size = 10usize;
                    ui.palette_pick =
                        (ui.palette_pick + page_size).min(filtered.len().saturating_sub(1));
                    ui.palette_sugg_offset =
                        (ui.palette_sugg_offset + page_size).min(filtered.len().saturating_sub(15));
                }
                (KeyCode::Home, _) => {
                    ui.palette_pick = 0;
                    ui.palette_sugg_offset = 0;
                }
                (KeyCode::End, _) => {
                    let all_items = build_picker_items(&service_ids, selected_id, &[]);
                    let filtered = filter_picker_items(&all_items, &ui.palette_input);
                    ui.palette_pick = filtered.len().saturating_sub(1);
                    ui.palette_sugg_offset = filtered.len().saturating_sub(15);
                }
                (KeyCode::Tab, _) => {
                    // Tab autocomplete: fill input with selected item's label
                    let all_items = build_picker_items(&service_ids, selected_id, &[]);
                    let filtered = filter_picker_items(&all_items, &ui.palette_input);
                    if let Some(item) = filtered.get(ui.palette_pick) {
                        ui.palette_input = item.label.clone();
                        ui.palette_error = None;
                    }
                }
                (KeyCode::Backspace, _) => {
                    ui.palette_input.pop();
                    ui.palette_error = None;
                    ui.palette_pick = 0;
                    ui.palette_sugg_offset = 0;
                    ui.history_cursor = None;
                }
                (KeyCode::Char(c), _) => {
                    ui.palette_input.push(c);
                    ui.palette_error = None;
                    ui.palette_pick = 0;
                    ui.palette_sugg_offset = 0;
                    ui.history_cursor = None;
                }
                (KeyCode::Enter, _) => {
                    // Execute the selected picker item
                    let all_items = build_picker_items(&service_ids, selected_id, &[]);
                    let filtered = filter_picker_items(&all_items, &ui.palette_input);

                    if let Some(item) = filtered.get(ui.palette_pick) {
                        // Handle navigation items
                        if let Some(view) = item.target_view {
                            ui.view = view;
                            if view == View::Logs {
                                ui.enter_follow();
                            }
                            ui.palette_open = false;
                            ui.focus = Focus::Units;
                            ui.palette_input.clear();
                            ui.palette_error = None;
                            ui.palette_pick = 0;
                            ui.palette_scroll = 0;
                            ui.palette_sugg_offset = 0;
                            ui.history_cursor = None;
                            continue;
                        }

                        // Handle command items
                        if let Some(ref cmd_str) = item.command {
                            match parse_command(cmd_str, &service_ids) {
                                Ok(commands) => {
                                    // Add to history
                                    if ui.history.last().map(|s| s.as_str()) != Some(&item.label) {
                                        ui.history.push(item.label.clone());
                                        if ui.history.len() > 50 {
                                            ui.history.remove(0);
                                        }
                                    }
                                    // Execute commands
                                    for c in commands {
                                        c.execute(&backend).await;
                                    }
                                    // Switch to logs view after command execution
                                    ui.view = View::Logs;
                                    ui.enter_follow();
                                    // Close picker and return focus
                                    ui.palette_open = false;
                                    ui.focus = Focus::Units;
                                    ui.palette_input.clear();
                                    ui.palette_error = None;
                                    ui.palette_pick = 0;
                                    ui.palette_scroll = 0;
                                    ui.palette_sugg_offset = 0;
                                    ui.history_cursor = None;
                                }
                                Err(e) => {
                                    ui.palette_error = Some(e);
                                }
                            }
                        } else {
                            ui.palette_error = Some("No action for this item".into());
                        }
                    } else {
                        ui.palette_error = Some("No command selected".into());
                    }
                }
                _ => {}
            }
            continue;
        }

        // ---------- SEARCH MODE (logs) ----------
        if ui.logs.is_searching() {
            match code {
                KeyCode::Esc => {
                    ui.logs.exit_search();
                }
                KeyCode::Enter => {
                    // Close search, keep at current scroll position
                    ui.logs.exit_search();
                }
                KeyCode::Backspace => {
                    if let Some(ref mut query) = ui.logs.search {
                        query.pop();
                        let snap = state.read().await;
                        ui.logs.matches = update_search_matches(
                            query,
                            &snap,
                            selected_id,
                            ui.left_mode,
                            ui.selected_run,
                            ui.logs.paused,
                            &ui.logs.frozen_logs,
                        );
                        ui.logs.match_idx = 0;

                        // Auto-scroll to first match
                        if let Some(&match_line) = ui.logs.matches.first() {
                            let total_lines = get_log_line_count(
                                &snap,
                                selected_id,
                                ui.left_mode,
                                ui.selected_run,
                                ui.logs.paused,
                                &ui.logs.frozen_logs,
                            );
                            let viewport_approx = 20;
                            let from_bottom = total_lines.saturating_sub(match_line + 1);
                            ui.logs.scroll = from_bottom.saturating_sub(viewport_approx / 2);
                            ui.logs.follow = false;
                        }
                    }
                }
                KeyCode::Down | KeyCode::Char('n') => {
                    ui.logs.next_match();
                    // Scroll to show the match
                    if let Some(&match_line) = ui.logs.matches.get(ui.logs.match_idx) {
                        let snap = state.read().await;
                        let total_lines = get_log_line_count(
                            &snap,
                            selected_id,
                            ui.left_mode,
                            ui.selected_run,
                            ui.logs.paused,
                            &ui.logs.frozen_logs,
                        );
                        // Scroll to center the match (scroll is lines from bottom)
                        let viewport_approx = 20; // Approximate viewport height
                        let from_bottom = total_lines.saturating_sub(match_line + 1);
                        ui.logs.scroll = from_bottom.saturating_sub(viewport_approx / 2);
                        ui.logs.follow = false;
                    }
                }
                KeyCode::Up | KeyCode::Char('N') => {
                    ui.logs.prev_match();
                    // Scroll to show the match
                    if let Some(&match_line) = ui.logs.matches.get(ui.logs.match_idx) {
                        let snap = state.read().await;
                        let total_lines = get_log_line_count(
                            &snap,
                            selected_id,
                            ui.left_mode,
                            ui.selected_run,
                            ui.logs.paused,
                            &ui.logs.frozen_logs,
                        );
                        // Scroll to center the match (scroll is lines from bottom)
                        let viewport_approx = 20;
                        let from_bottom = total_lines.saturating_sub(match_line + 1);
                        ui.logs.scroll = from_bottom.saturating_sub(viewport_approx / 2);
                        ui.logs.follow = false;
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut query) = ui.logs.search {
                        query.push(c);
                        let snap = state.read().await;
                        ui.logs.matches = update_search_matches(
                            query,
                            &snap,
                            selected_id,
                            ui.left_mode,
                            ui.selected_run,
                            ui.logs.paused,
                            &ui.logs.frozen_logs,
                        );
                        ui.logs.match_idx = 0;

                        // Auto-scroll to first match
                        if let Some(&match_line) = ui.logs.matches.first() {
                            let total_lines = get_log_line_count(
                                &snap,
                                selected_id,
                                ui.left_mode,
                                ui.selected_run,
                                ui.logs.paused,
                                &ui.logs.frozen_logs,
                            );
                            let viewport_approx = 20;
                            let from_bottom = total_lines.saturating_sub(match_line + 1);
                            ui.logs.scroll = from_bottom.saturating_sub(viewport_approx / 2);
                            ui.logs.follow = false;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        // ---------- GLOBAL KEYS ----------
        match (code, modifiers) {
            (KeyCode::Char('q'), _) => {
                return Ok(());
            }
            // Help overlay: ?
            (KeyCode::Char('?'), _) => {
                ui.help_open = true;
                continue;
            }
            // Command palette: '/' or Ctrl+K (VS Code Quick Pick style)
            (KeyCode::Char('/'), _) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                ui.palette_open = true;
                ui.focus = Focus::Palette;
                ui.palette_input.clear();
                ui.palette_error = None;
                ui.palette_pick = 0;
                ui.palette_scroll = 0;
                ui.palette_sugg_offset = 0;
                ui.history_cursor = None;
                continue;
            }
            // Tab: toggle focus (Units <-> RightPane), or cycle sections in Inspect view
            (KeyCode::Tab, _) => {
                ui.focus = match (ui.focus, ui.view) {
                    // From Units to right pane
                    (Focus::Units, View::Inspect) => Focus::InspectPanel(InspectSection::Summary),
                    (Focus::Units, _) => Focus::RightPane,
                    // From Inspect panel, cycle sections
                    (Focus::InspectPanel(section), View::Inspect) => {
                        Focus::InspectPanel(section.next())
                    }
                    // From other right pane, back to Units
                    (Focus::RightPane, _) => Focus::Units,
                    (Focus::InspectPanel(_), _) => Focus::Units, // Fallback if view changed
                    (Focus::Palette, _) => Focus::Palette, // Don't toggle when palette is open
                };
                continue;
            }
            // Shift+Tab: reverse cycle
            (KeyCode::BackTab, _) => {
                ui.focus = match (ui.focus, ui.view) {
                    (Focus::Units, View::Inspect) => Focus::InspectPanel(InspectSection::Health),
                    (Focus::Units, _) => Focus::RightPane,
                    (Focus::InspectPanel(section), View::Inspect) => {
                        Focus::InspectPanel(section.prev())
                    }
                    (Focus::RightPane, _) => Focus::Units,
                    (Focus::InspectPanel(_), _) => Focus::Units,
                    (Focus::Palette, _) => Focus::Palette,
                };
                continue;
            }
            // Esc: global back - exit search/selection, return to follow
            (KeyCode::Esc, _) => {
                ui.logs.exit_search();
                ui.enter_follow();
                continue;
            }
            // Left pane mode switching: 1/2/3
            (KeyCode::Char('1'), _) => {
                ui.left_mode = LeftMode::Services;
                continue;
            }
            (KeyCode::Char('2'), _) => {
                ui.left_mode = LeftMode::Commands;
                continue;
            }
            (KeyCode::Char('3'), _) => {
                ui.left_mode = LeftMode::Runs;
                continue;
            }
            // Direct view switching: l/i/d/m (global, works from any focus)
            (KeyCode::Char('l'), _) => {
                ui.view = View::Logs;
                ui.enter_follow();
                continue;
            }
            (KeyCode::Char('i'), _) => {
                ui.view = View::Inspect;
                // If focus was on right pane, switch to InspectPanel
                if ui.focus.is_right() {
                    ui.focus = Focus::InspectPanel(InspectSection::Summary);
                }
                continue;
            }
            (KeyCode::Char('d'), _) => {
                ui.view = View::Deps;
                continue;
            }
            (KeyCode::Char('m'), _) => {
                ui.view = View::Metrics;
                continue;
            }
            _ => {}
        }

        // ---------- FOCUS-SPECIFIC KEYS ----------
        match ui.focus {
            Focus::Units => {
                // Left pane: mode-specific navigation and control
                match ui.left_mode {
                    LeftMode::Services => {
                        // Services mode: service navigation and control
                        match code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                if *selected > 0 {
                                    *selected -= 1;
                                    list_state.select(Some(*selected));
                                    ui.enter_follow(); // Reset to follow when changing selection
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if *selected + 1 < display_ids.len() {
                                    *selected += 1;
                                    list_state.select(Some(*selected));
                                    ui.enter_follow();
                                }
                            }
                            KeyCode::Enter => {
                                // "all" is virtual - just shows merged logs, no toggle action
                                if let Some(id) = selected_id {
                                    if id != "all" {
                                        backend.send_toggle(id.to_string()).await;
                                    }
                                }
                            }
                            KeyCode::Char('r') => {
                                if let Some(id) = selected_id {
                                    if id == "all" {
                                        // Restart all services
                                        for service_id in display_ids.iter().filter(|s| *s != "all")
                                        {
                                            backend.send_restart(service_id.to_string()).await;
                                        }
                                    } else {
                                        backend.send_restart(id.to_string()).await;
                                    }
                                }
                            }
                            KeyCode::Char('s') => {
                                if let Some(id) = selected_id {
                                    if id == "all" {
                                        // Stop all services
                                        for service_id in display_ids.iter().filter(|s| *s != "all")
                                        {
                                            backend.send_stop(service_id.to_string()).await;
                                        }
                                    } else {
                                        backend.send_stop(id.to_string()).await;
                                    }
                                }
                            }
                            KeyCode::Char('t') => {
                                if let Some(id) = selected_id {
                                    if id == "all" {
                                        // Start all services
                                        for service_id in display_ids.iter().filter(|s| *s != "all")
                                        {
                                            backend.send_start(service_id.to_string()).await;
                                        }
                                    } else {
                                        backend.send_start(id.to_string()).await;
                                    }
                                }
                            }
                            KeyCode::Char('x') => {
                                if let Some(id) = selected_id {
                                    if id == "all" {
                                        // Kill all services
                                        for service_id in display_ids.iter().filter(|s| *s != "all")
                                        {
                                            backend.send_kill(service_id.to_string()).await;
                                        }
                                    } else {
                                        backend.send_kill(id.to_string()).await;
                                    }
                                }
                            }
                            KeyCode::Char('c') => {
                                if let Some(id) = selected_id {
                                    if id == "all" {
                                        // Clear logs for all services
                                        for service_id in display_ids.iter().filter(|s| *s != "all")
                                        {
                                            backend.send_clear_logs(service_id.to_string()).await;
                                        }
                                    } else {
                                        backend.send_clear_logs(id.to_string()).await;
                                    }
                                }
                            }
                            // View keys (also work from left)
                            KeyCode::Char('l') => {
                                ui.view = View::Logs;
                                ui.enter_follow();
                            }
                            KeyCode::Char('i') => {
                                ui.view = View::Inspect;
                            }
                            KeyCode::Char('e') => {
                                ui.view = View::Exec;
                            }
                            KeyCode::Char('d') => {
                                ui.view = View::Deps;
                            }
                            KeyCode::Char('m') => {
                                ui.view = View::Metrics;
                            }
                            _ => {}
                        }
                    }
                    LeftMode::Commands => {
                        // Commands mode: command navigation and execution
                        // Re-acquire state lock for command data
                        let snap = state.read().await;
                        let cmd_count =
                            snap.project.as_ref().map(|p| p.commands.len()).unwrap_or(0);
                        match code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                if ui.selected_command > 0 {
                                    ui.selected_command -= 1;
                                    command_list_state.select(Some(ui.selected_command));
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if ui.selected_command + 1 < cmd_count {
                                    ui.selected_command += 1;
                                    command_list_state.select(Some(ui.selected_command));
                                }
                            }
                            KeyCode::Enter => {
                                // Run selected command
                                if let Some(project) = &snap.project {
                                    let cmds: Vec<_> = project.commands_sorted();
                                    if let Some(cmd) = cmds.get(ui.selected_command) {
                                        let spec = (*cmd).clone();
                                        drop(snap); // Release lock before sending
                                        let _ = runner_cmd_tx
                                            .send(runner::RunnerCommand::Run { spec })
                                            .await;
                                        // Switch to Runs mode to see output
                                        ui.left_mode = LeftMode::Runs;
                                        ui.selected_run = 0;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    LeftMode::Runs => {
                        // Runs mode: run navigation and control
                        // Re-acquire state lock for run data
                        let snap = state.read().await;
                        let run_count = snap.run_order.len();
                        match code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                if ui.selected_run > 0 {
                                    ui.selected_run -= 1;
                                    run_list_state.select(Some(ui.selected_run));
                                    ui.enter_follow();
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if ui.selected_run + 1 < run_count {
                                    ui.selected_run += 1;
                                    run_list_state.select(Some(ui.selected_run));
                                    ui.enter_follow();
                                }
                            }
                            KeyCode::Enter => {
                                // View logs for selected run (switch to logs view)
                                ui.view = View::Logs;
                                ui.enter_follow();
                            }
                            KeyCode::Char('x') => {
                                // Kill selected run
                                if let Some(run_id) = snap.run_order.get(ui.selected_run) {
                                    let run_id = run_id.clone();
                                    drop(snap); // Release lock before sending
                                    let _ = runner_cmd_tx
                                        .send(runner::RunnerCommand::Kill { run_id })
                                        .await;
                                }
                            }
                            KeyCode::Char('r') => {
                                // Rerun selected command
                                if let Some(run_id) = snap.run_order.get(ui.selected_run) {
                                    let run_id = run_id.clone();
                                    drop(snap); // Release lock before sending
                                    let _ = runner_cmd_tx
                                        .send(runner::RunnerCommand::Rerun { run_id })
                                        .await;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Focus::RightPane => {
                // Right pane: view-specific keys
                match ui.view {
                    View::Logs => {
                        match code {
                            // Note: '/' opens command palette globally, not search here
                            // Use Ctrl+F or 's' for search instead
                            KeyCode::Char('s') => {
                                ui.logs.enter_search();
                            }
                            // Follow toggle: 'f'
                            KeyCode::Char('f') => {
                                ui.logs.toggle_follow();
                            }
                            // Scroll up
                            KeyCode::Up | KeyCode::Char('k') => {
                                ui.logs.scroll_up(1);
                            }
                            // Scroll down
                            KeyCode::Down | KeyCode::Char('j') => {
                                ui.logs.scroll_down(1);
                            }
                            // Page up
                            KeyCode::PageUp => {
                                ui.logs.scroll_up(20);
                            }
                            // Page down
                            KeyCode::PageDown => {
                                ui.logs.scroll_down(20);
                            }
                            // Jump to top (oldest)
                            KeyCode::Home => {
                                let snap = state.read().await;
                                let total = selected_id
                                    .and_then(|id| snap.logs.per_service.get(id))
                                    .map(|l| l.len())
                                    .unwrap_or(0);
                                ui.logs.scroll = total.saturating_sub(20);
                                ui.logs.follow = false;
                            }
                            // Jump to bottom (newest) / return to follow mode
                            KeyCode::End | KeyCode::Char('g') | KeyCode::Char('G') => {
                                ui.enter_follow();
                            }
                            // Pause/resume
                            KeyCode::Char(' ') => {
                                ui.logs.paused = !ui.logs.paused;
                                if ui.logs.paused {
                                    let snap = state.read().await;
                                    if let Some(id) = selected_id {
                                        ui.logs.frozen_logs = snap
                                            .logs
                                            .per_service
                                            .get(id)
                                            .map(|l| {
                                                l.iter()
                                                    .map(|x| DisplayLogLine {
                                                        timestamp: Some(x.at),
                                                        text: x.text.clone(),
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                    }
                                }
                            }
                            // Log level filter keys
                            KeyCode::Char('e') => {
                                ui.logs.log_filter = LogFilterMode::ErrorOnly;
                            }
                            KeyCode::Char('w') => {
                                ui.logs.log_filter = LogFilterMode::WarnAndAbove;
                            }
                            KeyCode::Char('a') => {
                                ui.logs.log_filter = LogFilterMode::All;
                            }
                            // Legacy view keys
                            KeyCode::Char('l') => {
                                ui.view = View::Logs;
                                ui.enter_follow();
                            }
                            KeyCode::Char('i') => {
                                ui.view = View::Inspect;
                            }
                            KeyCode::Char('d') => {
                                ui.view = View::Deps;
                            }
                            _ => {}
                        }
                    }
                    View::Metrics => match code {
                        KeyCode::Char('p') => {
                            ui.metrics_paused = !ui.metrics_paused;
                        }
                        KeyCode::Char('l') => {
                            ui.view = View::Logs;
                            ui.enter_follow();
                        }
                        KeyCode::Char('i') => {
                            ui.view = View::Inspect;
                        }
                        KeyCode::Char('e') => {
                            ui.view = View::Exec;
                        }
                        KeyCode::Char('d') => {
                            ui.view = View::Deps;
                        }
                        KeyCode::Char('m') => {
                            ui.view = View::Metrics;
                        }
                        _ => {}
                    },
                    View::Inspect | View::Deps | View::Exec => {
                        // Simple scroll for these views (future enhancement)
                        match code {
                            KeyCode::Char('l') => {
                                ui.view = View::Logs;
                                ui.enter_follow();
                            }
                            KeyCode::Char('i') => {
                                ui.view = View::Inspect;
                            }
                            KeyCode::Char('e') => {
                                ui.view = View::Exec;
                            }
                            KeyCode::Char('d') => {
                                ui.view = View::Deps;
                            }
                            KeyCode::Char('m') => {
                                ui.view = View::Metrics;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Focus::InspectPanel(_section) => {
                // Inspect panel focus - Tab cycles sections
                // The Tab key is handled globally earlier, so this is for any section-specific keys
                match code {
                    // In the future, sections could have their own key bindings
                    // For now, just acknowledge the focus state
                    _ => {}
                }
            }
            Focus::Palette => {
                // Palette keys are handled earlier in the loop
                // This should not be reached, but included for exhaustiveness
            }
        }
    }
}
