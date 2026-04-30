use std::time::SystemTime;

use ratatui::style::{Color, Style};

use orkesy_core::model::{HealthStatus, ServiceKind, ServiceStatus};

pub fn format_timestamp(time: SystemTime) -> String {
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

#[derive(Clone, Debug)]
pub struct DisplayLogLine {
    pub timestamp: Option<SystemTime>,
    pub text: String,
}

pub fn status_label(s: &ServiceStatus) -> &'static str {
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

pub fn status_icon(s: &ServiceStatus) -> &'static str {
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

pub fn health_icon(h: &HealthStatus) -> &'static str {
    match h {
        HealthStatus::Unknown => " ",
        HealthStatus::Healthy => "♥",
        HealthStatus::Degraded { .. } => "♡",
        HealthStatus::Unhealthy { .. } => "✗",
    }
}

pub fn kind_icon(k: &ServiceKind) -> &'static str {
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

pub fn status_style(s: &ServiceStatus) -> Style {
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

pub fn health_style(h: &HealthStatus) -> Style {
    match h {
        HealthStatus::Healthy => Style::default().fg(Color::Green),
        HealthStatus::Degraded { .. } => Style::default().fg(Color::Yellow),
        HealthStatus::Unhealthy { .. } => Style::default().fg(Color::Red),
        HealthStatus::Unknown => Style::default(),
    }
}

pub fn fit_title(s: &str, width: u16) -> String {
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
