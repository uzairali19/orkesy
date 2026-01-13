use ratatui::style::{Color, Modifier, Style};

use orkesy_core::model::{HealthStatus, ServiceStatus};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Palette {
    pub bg: Color,
    pub panel_bg: Color,
    pub panel_border: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub accent_dim: Color,
    pub success: Color,
    pub warn: Color,
    pub error: Color,
    pub info: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub key_hint: Color,
    pub key_hint_dim: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

#[allow(dead_code)]
impl Palette {
    pub fn dark() -> Self {
        Self {
            bg: Color::Reset,
            panel_bg: Color::Rgb(30, 30, 30),
            panel_border: Color::Rgb(60, 60, 60),
            text: Color::Rgb(212, 212, 212),
            text_dim: Color::Rgb(150, 150, 150),
            text_muted: Color::Rgb(100, 100, 100),
            accent: Color::Rgb(79, 193, 255), // Light blue
            accent_dim: Color::Rgb(50, 120, 160),
            success: Color::Rgb(78, 201, 176),     // Teal green
            warn: Color::Rgb(220, 180, 100),       // Amber
            error: Color::Rgb(244, 135, 113),      // Coral red
            info: Color::Rgb(156, 220, 254),       // Light cyan
            selection_bg: Color::Rgb(38, 79, 120), // Dark blue
            selection_fg: Color::White,
            key_hint: Color::Rgb(206, 145, 120), // Soft orange
            key_hint_dim: Color::Rgb(140, 100, 80),
        }
    }

    pub fn high_contrast() -> Self {
        Self {
            bg: Color::Black,
            panel_bg: Color::Black,
            panel_border: Color::White,
            text: Color::White,
            text_dim: Color::Rgb(200, 200, 200),
            text_muted: Color::Rgb(150, 150, 150),
            accent: Color::Cyan,
            accent_dim: Color::DarkGray,
            success: Color::Green,
            warn: Color::Yellow,
            error: Color::Red,
            info: Color::Cyan,
            selection_bg: Color::Blue,
            selection_fg: Color::White,
            key_hint: Color::Yellow,
            key_hint_dim: Color::DarkGray,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub palette: Palette,
}

#[allow(dead_code)]
impl Theme {
    pub fn new(palette: Palette) -> Self {
        Self { palette }
    }

    pub fn status_style(&self, status: &ServiceStatus) -> Style {
        let color = match status {
            ServiceStatus::Running => self.palette.success,
            ServiceStatus::Starting | ServiceStatus::Restarting => self.palette.warn,
            ServiceStatus::Stopped => self.palette.text_muted,
            ServiceStatus::Exited { code } => {
                if *code == Some(0) {
                    self.palette.text_dim
                } else {
                    self.palette.error
                }
            }
            ServiceStatus::Errored { .. } => self.palette.error,
            ServiceStatus::Unknown => self.palette.text_muted,
        };
        Style::default().fg(color)
    }

    pub fn status_icon(&self, status: &ServiceStatus) -> &'static str {
        match status {
            ServiceStatus::Running => "●",
            ServiceStatus::Starting => "◐",
            ServiceStatus::Restarting => "⟲",
            ServiceStatus::Stopped => "○",
            ServiceStatus::Exited { code } => {
                if *code == Some(0) {
                    "◌"
                } else {
                    "✗"
                }
            }
            ServiceStatus::Errored { .. } => "✗",
            ServiceStatus::Unknown => "?",
        }
    }

    pub fn health_style(&self, health: &HealthStatus) -> Style {
        let color = match health {
            HealthStatus::Healthy => self.palette.success,
            HealthStatus::Degraded { .. } => self.palette.warn,
            HealthStatus::Unhealthy { .. } => self.palette.error,
            HealthStatus::Unknown => self.palette.text_muted,
        };
        Style::default().fg(color)
    }

    pub fn health_icon(&self, health: &HealthStatus) -> &'static str {
        match health {
            HealthStatus::Healthy => "✓",
            HealthStatus::Degraded { .. } => "~",
            HealthStatus::Unhealthy { .. } => "!",
            HealthStatus::Unknown => "?",
        }
    }

    pub fn tab_style(&self, active: bool) -> Style {
        if active {
            Style::default()
                .fg(self.palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.palette.text_dim)
        }
    }

    pub fn key_hint_style(&self) -> Style {
        Style::default().fg(self.palette.key_hint)
    }

    pub fn key_hint_dim_style(&self) -> Style {
        Style::default().fg(self.palette.key_hint_dim)
    }

    pub fn subtle_border_style(&self) -> Style {
        Style::default().fg(self.palette.panel_border)
    }

    pub fn focused_border_style(&self) -> Style {
        Style::default().fg(self.palette.accent)
    }

    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.palette.selection_bg)
            .fg(self.palette.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn text_style(&self) -> Style {
        Style::default().fg(self.palette.text)
    }

    pub fn text_dim_style(&self) -> Style {
        Style::default().fg(self.palette.text_dim)
    }

    pub fn text_muted_style(&self) -> Style {
        Style::default().fg(self.palette.text_muted)
    }

    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.palette.accent)
    }

    pub fn accent_bold_style(&self) -> Style {
        Style::default()
            .fg(self.palette.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn success_style(&self) -> Style {
        Style::default().fg(self.palette.success)
    }

    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.palette.warn)
    }

    pub fn error_style(&self) -> Style {
        Style::default().fg(self.palette.error)
    }

    pub fn info_style(&self) -> Style {
        Style::default().fg(self.palette.info)
    }

    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.palette.text)
            .add_modifier(Modifier::BOLD)
    }

    pub fn section_header_style(&self) -> Style {
        Style::default()
            .fg(self.palette.accent)
            .add_modifier(Modifier::BOLD)
    }
}

static DEFAULT_THEME: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();

pub fn theme() -> &'static Theme {
    DEFAULT_THEME.get_or_init(Theme::default)
}

#[allow(dead_code)]
pub mod styles {
    use super::*;

    pub fn status(status: &ServiceStatus) -> Style {
        theme().status_style(status)
    }

    pub fn status_icon(status: &ServiceStatus) -> &'static str {
        theme().status_icon(status)
    }

    pub fn health(health: &HealthStatus) -> Style {
        theme().health_style(health)
    }

    pub fn health_icon(health: &HealthStatus) -> &'static str {
        theme().health_icon(health)
    }

    pub fn tab(active: bool) -> Style {
        theme().tab_style(active)
    }

    pub fn key_hint() -> Style {
        theme().key_hint_style()
    }

    pub fn key_hint_dim() -> Style {
        theme().key_hint_dim_style()
    }

    pub fn border_subtle() -> Style {
        theme().subtle_border_style()
    }

    pub fn border_focused() -> Style {
        theme().focused_border_style()
    }

    pub fn selection() -> Style {
        theme().selection_style()
    }

    pub fn text() -> Style {
        theme().text_style()
    }

    pub fn text_dim() -> Style {
        theme().text_dim_style()
    }

    pub fn text_muted() -> Style {
        theme().text_muted_style()
    }

    pub fn accent() -> Style {
        theme().accent_style()
    }

    pub fn accent_bold() -> Style {
        theme().accent_bold_style()
    }

    pub fn success() -> Style {
        theme().success_style()
    }

    pub fn warn() -> Style {
        theme().warn_style()
    }

    pub fn error() -> Style {
        theme().error_style()
    }

    pub fn info() -> Style {
        theme().info_style()
    }

    pub fn title() -> Style {
        theme().title_style()
    }

    pub fn section_header() -> Style {
        theme().section_header_style()
    }
}
