use orkesy_core::log_filter::LogFilterMode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Focus, InspectSection, UiState, View};
use crate::ui::styles;

pub fn draw(f: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let view_tabs = view_tab_spans(ui.view);
    let context = context_hint_spans(ui);
    let globals = global_hint_spans();

    let mut spans: Vec<Span> = view_tabs;
    spans.extend(context);
    spans.extend(globals);

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn view_tab_spans(active: View) -> Vec<Span<'static>> {
    fn tab(label: &'static str, key: &'static str, active: bool) -> [Span<'static>; 2] {
        [
            Span::styled(key, styles::key_hint()),
            Span::styled(
                label,
                if active {
                    styles::accent()
                } else {
                    styles::text_dim()
                },
            ),
        ]
    }
    let mut v = Vec::with_capacity(9);
    v.extend(tab(" Logs  ", "l", active == View::Logs));
    v.extend(tab(" Inspect  ", "i", active == View::Inspect));
    v.extend(tab(" Deps  ", "d", active == View::Deps));
    v.extend(tab(" Metrics", "m", active == View::Metrics));
    v.push(Span::styled("  |  ", styles::text_muted()));
    v
}

fn context_hint_spans(ui: &UiState) -> Vec<Span<'static>> {
    match (ui.focus, ui.view) {
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
                Span::styled(format!("[{filter_label}]"), filter_style),
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
                Span::styled(format!("[{section_name}]"), styles::accent()),
            ]
        }
        (Focus::RightPane, _) => vec![
            Span::styled("↑↓", styles::key_hint()),
            Span::styled(" scroll", styles::text_dim()),
        ],
    }
}

fn global_hint_spans() -> Vec<Span<'static>> {
    vec![
        Span::styled("  Tab", styles::key_hint()),
        Span::styled(" focus  ", styles::text_dim()),
        Span::styled("/", styles::key_hint()),
        Span::styled(" cmd  ", styles::text_dim()),
        Span::styled("?", styles::key_hint()),
        Span::styled(" help  ", styles::text_dim()),
        Span::styled("q", styles::key_hint()),
        Span::styled(" quit", styles::text_dim()),
    ]
}
