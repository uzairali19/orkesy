mod engines;
mod health;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};

use tokio::sync::{broadcast, mpsc, RwLock};

use orkesy_core::config::OrkesyConfig;
use orkesy_core::engine::{Engine, EngineCommand};
use orkesy_core::model::*;
use orkesy_core::reducer::*;
use orkesy_core::state::*;

use engines::{FakeEngine, LocalProcessEngine};

/// Create a demo graph when no orkesy.yaml is found
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

/// Try to load config from orkesy.yaml, return None if not found
fn try_load_config() -> Option<(PathBuf, OrkesyConfig)> {
    let cwd = std::env::current_dir().ok()?;
    OrkesyConfig::discover(&cwd).ok()
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

/// Get visual icon for service status
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

/// Get visual icon for health status
fn health_icon(h: &HealthStatus) -> &'static str {
    match h {
        HealthStatus::Unknown => " ",
        HealthStatus::Healthy => "♥",
        HealthStatus::Degraded { .. } => "♡",
        HealthStatus::Unhealthy { .. } => "✗",
    }
}

/// Get visual icon for service kind
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
    let mut out: String = chars
        .into_iter()
        .take(max - 1)
        .collect();
    out.push('…');
    out
}

fn fit_line(s: &str, width: u16) -> String {
    let max = width as usize;
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
    let mut out: String = chars
        .into_iter()
        .take(max - 1)
        .collect();
    out.push('…');
    out
}

#[derive(Clone, Copy, Debug)]
enum RightPane {
    Logs,
    Graph,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(100);
    let (event_tx, _) = broadcast::channel::<EventEnvelope>(1_000);

    // Try to load config, fall back to demo mode
    let (graph, mut engine): (RuntimeGraph, Box<dyn Engine>) = match try_load_config() {
        Some((path, config)) => {
            eprintln!("Loaded config from: {}", path.display());
            let graph = config.to_graph();
            let configs = config.services.clone();
            let engine = LocalProcessEngine::new().with_configs(configs);
            (graph, Box::new(engine))
        }
        None => {
            eprintln!("No orkesy.yaml found, running in demo mode with fake engine");
            let graph = demo_graph();
            (graph, Box::new(FakeEngine::new()))
        }
    };

    let state = Arc::new(RwLock::new(RuntimeState::new(graph.clone())));

    // Spawn engine task
    let engine_event_tx = event_tx.clone();
    tokio::spawn(async move {
        engine.run(cmd_rx, engine_event_tx, graph).await;
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

    let mut terminal = setup_terminal()?;
    let mut selected = 0usize;
    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let res = tui_loop(&mut terminal, state, cmd_tx, &mut selected, &mut list_state).await;
    restore_terminal(terminal)?;
    res
}

/// Keep the palette input scrolled so the cursor stays visible.
fn fix_input_scroll(input: &str, scroll: &mut usize, visible: usize) {
    let len = input.chars().count();
    if visible == 0 {
        *scroll = 0;
        return;
    }
    if len <= visible {
        *scroll = 0;
        return;
    }
    let max_scroll = len - visible;
    if *scroll > max_scroll {
        *scroll = max_scroll;
    }

    // keep cursor visible on the right edge (simple behavior)
    if len.saturating_sub(*scroll) < visible {
        *scroll = len.saturating_sub(visible);
    }
}

fn build_palette_suggestions(input: &str, service_ids: &[String]) -> Vec<String> {
    let t = input.trim();
    let base = [
        "restart all",
        "stop all",
        "start all",
        "toggle all",
        "kill all",
        "clear logs all",
        "exec api echo hello",
    ];

    let mut out: Vec<String> = vec![];

    if t.is_empty() {
        out.extend(base.iter().map(|s| s.to_string()));
        for id in service_ids.iter().take(8) {
            out.push(format!("restart {id}"));
            out.push(format!("stop {id}"));
            out.push(format!("start {id}"));
            out.push(format!("toggle {id}"));
            out.push(format!("clear logs {id}"));
            out.push(format!("exec {id} echo hello"));
        }
        out.sort();
        out.dedup();
        return out;
    }

    for id in service_ids {
        out.push(format!("restart {id}"));
        out.push(format!("stop {id}"));
        out.push(format!("start {id}"));
        out.push(format!("toggle {id}"));
        out.push(format!("kill {id}"));
        out.push(format!("clear logs {id}"));
        out.push(format!("exec {id} echo hello"));
    }
    out.extend(base.iter().map(|s| s.to_string()));

    let tl = t.to_lowercase();
    out = out
        .into_iter()
        .filter(|s| s.to_lowercase().contains(&tl))
        .collect();

    out.sort();
    out.dedup();
    out
}

fn parse_command(input: &str, service_ids: &[String]) -> Result<Vec<EngineCommand>, String> {
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
        "restart" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| EngineCommand::Restart { id })
            .collect()),

        "stop" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| EngineCommand::Stop { id })
            .collect()),

        "start" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| EngineCommand::Start { id })
            .collect()),

        "toggle" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| EngineCommand::Toggle { id })
            .collect()),

        "kill" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| EngineCommand::Kill { id })
            .collect()),

        "clear" => {
            // clear logs <service|all>
            if parts.get(1).copied() != Some("logs") {
                return Err("Usage: clear logs <service|all>".into());
            }
            let target = parts.get(2).copied().unwrap_or("all");
            Ok(expand_ids(Some(target))?
                .into_iter()
                .map(|id| EngineCommand::ClearLogs { id })
                .collect())
        }

        "exec" => {
            let svc = arg1.ok_or("Usage: exec <service> <cmd...>")?;
            if !exists(svc) {
                return Err(format!("Unknown service: {svc}"));
            }
            let cmd_parts = parts.get(2..).unwrap_or(&[]);
            if cmd_parts.is_empty() {
                return Err("Usage: exec <service> <cmd...>".into());
            }
            Ok(vec![EngineCommand::Exec {
                id: svc.to_string(),
                cmd: cmd_parts.iter().map(|s| s.to_string()).collect(),
            }])
        }

        _ => Err(format!(
            "Unknown command: {cmd} (try: start/stop/restart/toggle/kill/clear/exec)"
        )),
    }
}

async fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<RuntimeState>>,
    cmd_tx: mpsc::Sender<EngineCommand>,
    selected: &mut usize,
    list_state: &mut ListState,
) -> io::Result<()> {
    let mut paused = false;
    let mut frozen_logs: Vec<Line> = vec![];

    let mut right_pane = RightPane::Logs;

    let mut palette_open = false;
    let mut palette_input = String::new();
    let mut palette_error: Option<String> = None;
    let mut palette_pick: usize = 0;

    let mut palette_scroll: usize = 0;

    let mut history: Vec<String> = vec![];
    let mut history_cursor: Option<usize> = None;
    let mut help_open = false;

    loop {
        let snapshot = state.read().await;

        let mut service_ids: Vec<String> = snapshot.graph.nodes.keys().cloned().collect();
        service_ids.sort();

        if service_ids.is_empty() {
            *selected = 0;
            list_state.select(None);
        } else {
            if *selected >= service_ids.len() {
                *selected = service_ids.len() - 1;
            }
            list_state.select(Some(*selected));
        }

        let selected_id: Option<&str> = service_ids.get(*selected).map(|s| s.as_str());

        let items: Vec<ListItem> = service_ids
            .iter()
            .map(|id| {
                let node = snapshot.graph.nodes.get(id).unwrap();
                let status_sym = status_icon(&node.observed.status);
                let health_sym = health_icon(&node.observed.health);
                let kind_sym = kind_icon(&node.kind);
                let port_info = node
                    .port
                    .map(|p| format!(":{}", p))
                    .unwrap_or_default();
                ListItem::new(Line::from(format!(
                    "{} {} {}{} [{}] {}",
                    status_sym,
                    kind_sym,
                    node.display_name,
                    port_info,
                    status_label(&node.observed.status),
                    health_sym
                )))
            })
            .collect();

        let build_logs = |id: &str| -> Vec<Line> {
            if let Some(lines) = snapshot.logs.per_service.get(id) {
                lines
                    .iter()
                    .rev()
                    .take(200)
                    .rev()
                    .map(|l| Line::from(l.text.clone()))
                    .collect()
            } else {
                vec![Line::from("No logs yet.")]
            }
        };

        let build_graph = || -> Vec<Line> {
            let mut out: Vec<Line> = vec![Line::from("Topology"), Line::from("")];
            let mut by_from: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for e in snapshot.graph.edges.iter() {
                by_from.entry(e.from.clone()).or_default().push(format!("{} ({:?})", e.to, e.kind));
            }
            for (from, tos) in by_from {
                out.push(Line::from(from));
                for t in tos {
                    out.push(Line::from(format!("  └─▶ {t}")));
                }
                out.push(Line::from(""));
            }
            if out.len() <= 2 {
                out.push(Line::from("(no edges)"));
            }
            out
        };

        // ✅ Right content (was missing in some versions)
        let right_text: Text = match right_pane {
            RightPane::Graph => Text::from(build_graph()),
            RightPane::Logs => {
                if paused {
                    Text::from(frozen_logs.clone())
                } else if let Some(id) = selected_id {
                    Text::from(build_logs(id))
                } else {
                    Text::from(vec![Line::from("No service selected.")])
                }
            }
        };

        let suggestions = if palette_open {
            build_palette_suggestions(&palette_input, &service_ids)
        } else {
            vec![]
        };

        if palette_pick >= suggestions.len() && !suggestions.is_empty() {
            palette_pick = suggestions.len() - 1;
        }
        if suggestions.is_empty() {
            palette_pick = 0;
        }

        terminal.draw(|f| {
            let area = f.area();

            // Layout:
            // [ main (services + right pane) ]
            // [ footer ]
            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);

            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(18), Constraint::Percentage(82)])
                .split(outer[0]);

            // ---------------- Left: Services ----------------
            let left = Block::default().title("Services").borders(Borders::ALL);
            let list = List::new(items)
                .block(left)
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");
            f.render_stateful_widget(list, main[0], list_state);

            // ---------------- Right: Logs / Graph ----------------
            // Minimal title (no key list here)
            let raw_title = match right_pane {
                RightPane::Graph =>
                    match selected_id {
                        Some(id) => format!("Graph: {id}"),
                        None => "Graph".to_string(),
                    }
                RightPane::Logs => {
                    let p = if paused { "PAUSED" } else { "LIVE" };
                    match selected_id {
                        Some(id) => format!("Logs: {id} [{p}]"),
                        None => format!("Logs [{p}]"),
                    }
                }
            };

            let title = fit_title(&raw_title, main[1].width);

            let right = Paragraph::new(right_text)
                .block(Block::default().title(title).borders(Borders::ALL))
                .wrap(Wrap { trim: false });

            f.render_widget(right, main[1]);

            // ---------------- Footer (pnpm-ish, 1 line, always fits) ----------------
            // Change this string as you like; keep it short.
            let footer =
                "↑↓ select   Space pause   r restart   s stop   t start   Enter toggle   / cmd   ? help   q quit";
            let footer = fit_line(footer, outer[1].width);
            f.render_widget(Paragraph::new(footer), outer[1]);

            // ---------------- Command Palette Drawer (as you had it) ----------------
            if palette_open {
                let drawer_h = (6u16).min(area.height);
                let drawer_rect = Rect {
                    x: area.x,
                    width: area.width,
                    height: drawer_h,
                    y: area.y + area.height.saturating_sub(drawer_h),
                };

                f.render_widget(Clear, drawer_rect);

                let block = Block::default().borders(Borders::ALL).title(" / ");
                let inner = block.inner(drawer_rect).inner(Margin { vertical: 0, horizontal: 1 });
                f.render_widget(block, drawer_rect);

                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(1)])
                    .split(inner);

                // Suggestions list
                let visible_sugg = parts[0].height as usize;
                let mut sugg_items: Vec<ListItem> = vec![];
                for (i, s) in suggestions.iter().take(visible_sugg).enumerate() {
                    let prefix = if i == palette_pick { "▶ " } else { "  " };
                    sugg_items.push(ListItem::new(Line::from(format!("{prefix}{s}"))));
                }
                if sugg_items.is_empty() {
                    sugg_items.push(ListItem::new(Line::from("  (type a command…)")));
                }
                f.render_widget(List::new(sugg_items), parts[0]);

                // Input line + horizontal scroll
                let input_area = parts[1];
                let prompt = "/ ";
                let max_visible = input_area.width.saturating_sub(prompt.len() as u16) as usize;

                let mut local_scroll = palette_scroll;
                fix_input_scroll(&palette_input, &mut local_scroll, max_visible);

                let chars: Vec<char> = palette_input.chars().collect();
                let visible: String = chars.iter().skip(local_scroll).take(max_visible).collect();

                let input_line = if let Some(err) = &palette_error {
                    format!("{prompt}{visible}   ✗ {err}")
                } else {
                    format!("{prompt}{visible}")
                };

                f.render_widget(
                    Paragraph::new(Text::from(Line::from(input_line))).wrap(Wrap { trim: false }),
                    input_area
                );

                // Cursor
                let cursor_in_visible = palette_input
                    .chars()
                    .count()
                    .saturating_sub(local_scroll)
                    .min(max_visible) as u16;

                let cursor_x = input_area.x + (prompt.len() as u16) + cursor_in_visible;
                let cursor_y = input_area.y;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            // ---------------- Help Modal ----------------
            if help_open {
                // centered modal
                let w = ((area.width as f32) * 0.72) as u16;
                let h = ((area.height as f32) * 0.55) as u16;
                let modal = Rect {
                    x: area.x + area.width.saturating_sub(w) / 2,
                    y: area.y + area.height.saturating_sub(h) / 2,
                    width: w.max(20),
                    height: h.max(8),
                };

                f.render_widget(Clear, modal);

                let help_text = vec![
                    Line::from("Keys"),
                    Line::from(""),
                    Line::from("↑/↓        Select service"),
                    Line::from("Space      Pause/Resume log stream"),
                    Line::from("r          Restart selected"),
                    Line::from("s          Stop selected"),
                    Line::from("t          Start selected"),
                    Line::from("Enter      Toggle selected"),
                    Line::from("x          Kill selected"),
                    Line::from("c          Clear logs (selected)"),
                    Line::from("e          Exec demo command (selected)"),
                    Line::from("g          Toggle Graph/Logs"),
                    Line::from("/          Command palette"),
                    Line::from("?          Toggle this help"),
                    Line::from("q          Quit")
                ];

                let block = Block::default().title(" Help ").borders(Borders::ALL);

                f.render_widget(
                    Paragraph::new(Text::from(help_text)).block(block).wrap(Wrap { trim: false }),
                    modal
                );
            }
        })?;

        drop(snapshot);

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        let ev = event::read()?;
        let CEvent::Key(KeyEvent { code, modifiers, .. }) = ev else {
            continue;
        };

        // ---------- PALETTE MODE ----------
        if palette_open {
            match (code, modifiers) {
                (KeyCode::Esc, _) => {
                    palette_open = false;
                    palette_input.clear();
                    palette_error = None;
                    palette_pick = 0;
                    palette_scroll = 0;
                    history_cursor = None;
                    help_open = false;
                }

                (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                    if history.is_empty() {
                        continue;
                    }
                    let idx = match history_cursor {
                        None => history.len().saturating_sub(1),
                        Some(i) => i.saturating_sub(1),
                    };
                    history_cursor = Some(idx);
                    palette_input = history[idx].clone();
                    palette_error = None;
                    palette_pick = 0;
                    fix_input_scroll(&palette_input, &mut palette_scroll, 80);
                }

                (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                    if history.is_empty() {
                        continue;
                    }
                    let idx = match history_cursor {
                        None => history.len(),
                        Some(i) => (i + 1).min(history.len()),
                    };
                    if idx >= history.len() {
                        history_cursor = None;
                        palette_input.clear();
                    } else {
                        history_cursor = Some(idx);
                        palette_input = history[idx].clone();
                    }
                    palette_error = None;
                    palette_pick = 0;
                    fix_input_scroll(&palette_input, &mut palette_scroll, 80);
                }

                (KeyCode::Up, _) => {
                    if palette_pick > 0 {
                        palette_pick -= 1;
                    }
                }
                (KeyCode::Down, _) => {
                    if palette_pick + 1 < suggestions.len() {
                        palette_pick += 1;
                    }
                }

                (KeyCode::Tab, _) => {
                    if let Some(s) = suggestions.get(palette_pick) {
                        palette_input = s.clone();
                        palette_error = None;
                        fix_input_scroll(&palette_input, &mut palette_scroll, 80);
                    }
                }

                (KeyCode::Backspace, _) => {
                    palette_input.pop();
                    palette_error = None;
                    palette_pick = 0;
                    history_cursor = None;
                    fix_input_scroll(&palette_input, &mut palette_scroll, 80);
                }

                (KeyCode::Char(c), _) => {
                    palette_input.push(c);
                    palette_error = None;
                    palette_pick = 0;
                    history_cursor = None;
                    fix_input_scroll(&palette_input, &mut palette_scroll, 80);
                }

                (KeyCode::Enter, _) => {
                    let effective = if palette_input.trim().is_empty() {
                        suggestions.get(palette_pick).cloned().unwrap_or_default()
                    } else {
                        palette_input.clone()
                    };

                    let effective = effective.trim().to_string();
                    if effective.is_empty() {
                        palette_error = Some("Type a command or pick a suggestion.".into());
                        continue;
                    }

                    match parse_command(&effective, &service_ids) {
                        Ok(commands) => {
                            if history.last().map(|s| s.as_str()) != Some(effective.as_str()) {
                                history.push(effective.clone());
                                if history.len() > 50 {
                                    history.remove(0);
                                }
                            }

                            for c in commands {
                                let _ = cmd_tx.send(c).await;
                            }

                            palette_open = false;
                            palette_input.clear();
                            palette_error = None;
                            palette_pick = 0;
                            palette_scroll = 0;
                            history_cursor = None;
                        }
                        Err(e) => {
                            palette_error = Some(e);
                        }
                    }
                }

                _ => {}
            }

            continue;
        }

        // ---------- NORMAL MODE ----------
        match code {
            KeyCode::Char('q') => {
                return Ok(());
            }

            KeyCode::Char('/') => {
                palette_open = true;
                palette_input.clear();
                palette_error = None;
                palette_pick = 0;
                palette_scroll = 0;
                history_cursor = None;
            }

            KeyCode::Char(' ') => {
                paused = !paused;
                if paused {
                    let snap = state.read().await;
                    let mut ids: Vec<String> = snap.graph.nodes.keys().cloned().collect();
                    ids.sort();
                    if let Some(id) = ids.get(*selected) {
                        frozen_logs = if let Some(lines) = snap.logs.per_service.get(id) {
                            lines
                                .iter()
                                .rev()
                                .take(200)
                                .rev()
                                .map(|l| Line::from(l.text.clone()))
                                .collect()
                        } else {
                            vec![Line::from("No logs yet.")]
                        };
                    } else {
                        frozen_logs = vec![Line::from("No service selected.")];
                    }
                }
            }

            KeyCode::Char('g') => {
                right_pane = match right_pane {
                    RightPane::Logs => RightPane::Graph,
                    RightPane::Graph => RightPane::Logs,
                };
            }

            KeyCode::Up => {
                if *selected > 0 {
                    *selected -= 1;
                    list_state.select(Some(*selected));
                }
            }
            KeyCode::Down => {
                *selected += 1;
                list_state.select(Some(*selected));
            }

            KeyCode::Char('r') => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::Restart {
                            id: id.to_string(),
                        })
                        .await;
                }
            }
            KeyCode::Char('s') => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::Stop {
                            id: id.to_string(),
                        })
                        .await;
                }
            }
            KeyCode::Char('t') => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::Start {
                            id: id.to_string(),
                        })
                        .await;
                }
            }
            KeyCode::Enter => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::Toggle {
                            id: id.to_string(),
                        })
                        .await;
                }
            }
            KeyCode::Char('x') => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::Kill {
                            id: id.to_string(),
                        })
                        .await;
                }
            }
            KeyCode::Char('c') => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::ClearLogs {
                            id: id.to_string(),
                        })
                        .await;
                }
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected_id {
                    let _ = cmd_tx
                        .send(EngineCommand::Exec {
                            id: id.to_string(),
                            cmd: vec!["sh".into(), "-lc".into(), "echo hello".into()],
                        })
                        .await;
                }
            }

            KeyCode::Char('?') => {
                help_open = !help_open;
            }

            KeyCode::Esc => {
                help_open = false;
            }

            _ => {}
        }
    }
}
