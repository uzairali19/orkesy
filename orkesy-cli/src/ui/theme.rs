//! UI Theme Module - Consistent color palette and style helpers
//!
//! Provides a centralized theme system for the Orkesy TUI with:
//! - Palette tokens (not hard-coded colors)
//! - StyleKit helpers for common states
//! - VS Code-esque dark theme defaults

use ratatui::style::{Color, Modifier, Style};

use orkesy_core::model::{HealthStatus, ServiceStatus};

/// Color palette tokens for the theme
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Palette {
    /// Main background color
    pub bg: Color,
    /// Panel/pane background
    pub panel_bg: Color,
    /// Panel border color
    pub panel_border: Color,
    /// Primary text color
    pub text: Color,
    /// Dimmed text (secondary info)
    pub text_dim: Color,
    /// Muted text (tertiary info, disabled)
    pub text_muted: Color,
    /// Accent color (highlights, focus)
    pub accent: Color,
    /// Dimmed accent
    pub accent_dim: Color,
    /// Success state (running, healthy)
    pub success: Color,
    /// Warning state (starting, restarting)
    pub warn: Color,
    /// Error state (failed, errored)
    pub error: Color,
    /// Info state (informational)
    pub info: Color,
    /// Selection background
    pub selection_bg: Color,
    /// Selection foreground
    pub selection_fg: Color,
    /// Key hint text
    pub key_hint: Color,
    /// Dimmed key hint
    pub key_hint_dim: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

#[allow(dead_code)]
impl Palette {
    /// VS Code-esque dark theme
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

    /// High contrast theme variant
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

/// Theme configuration
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Theme {
    pub palette: Palette,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            palette: Palette::dark(),
        }
    }
}

#[allow(dead_code)]
impl Theme {
    pub fn new(palette: Palette) -> Self {
        Self { palette }
    }

    // ========== StyleKit Helper Functions ==========

    /// Style for service status
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

    /// Icon for service status
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

    /// Style for health status
    pub fn health_style(&self, health: &HealthStatus) -> Style {
        let color = match health {
            HealthStatus::Healthy => self.palette.success,
            HealthStatus::Degraded { .. } => self.palette.warn,
            HealthStatus::Unhealthy { .. } => self.palette.error,
            HealthStatus::Unknown => self.palette.text_muted,
        };
        Style::default().fg(color)
    }

    /// Icon for health status
    pub fn health_icon(&self, health: &HealthStatus) -> &'static str {
        match health {
            HealthStatus::Healthy => "✓",
            HealthStatus::Degraded { .. } => "~",
            HealthStatus::Unhealthy { .. } => "!",
            HealthStatus::Unknown => "?",
        }
    }

    /// Style for tab/view labels
    pub fn tab_style(&self, active: bool) -> Style {
        if active {
            Style::default()
                .fg(self.palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.palette.text_dim)
        }
    }

    /// Style for key hints in footer
    pub fn key_hint_style(&self) -> Style {
        Style::default().fg(self.palette.key_hint)
    }

    /// Style for dimmed key hints
    pub fn key_hint_dim_style(&self) -> Style {
        Style::default().fg(self.palette.key_hint_dim)
    }

    /// Style for subtle borders
    pub fn subtle_border_style(&self) -> Style {
        Style::default().fg(self.palette.panel_border)
    }

    /// Style for focused borders
    pub fn focused_border_style(&self) -> Style {
        Style::default().fg(self.palette.accent)
    }

    /// Style for selected items
    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.palette.selection_bg)
            .fg(self.palette.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for primary text
    pub fn text_style(&self) -> Style {
        Style::default().fg(self.palette.text)
    }

    /// Style for dimmed text
    pub fn text_dim_style(&self) -> Style {
        Style::default().fg(self.palette.text_dim)
    }

    /// Style for muted text
    pub fn text_muted_style(&self) -> Style {
        Style::default().fg(self.palette.text_muted)
    }

    /// Style for accent text
    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.palette.accent)
    }

    /// Style for bold accent text
    pub fn accent_bold_style(&self) -> Style {
        Style::default()
            .fg(self.palette.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for success text
    pub fn success_style(&self) -> Style {
        Style::default().fg(self.palette.success)
    }

    /// Style for warning text
    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.palette.warn)
    }

    /// Style for error text
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.palette.error)
    }

    /// Style for info text
    pub fn info_style(&self) -> Style {
        Style::default().fg(self.palette.info)
    }

    /// Style for title text
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.palette.text)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for section headers
    pub fn section_header_style(&self) -> Style {
        Style::default()
            .fg(self.palette.accent)
            .add_modifier(Modifier::BOLD)
    }
}

/// Global theme instance - can be made configurable later
static DEFAULT_THEME: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();

/// Get the default theme
pub fn theme() -> &'static Theme {
    DEFAULT_THEME.get_or_init(Theme::default)
}

/// Convenience re-exports for common use cases
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
