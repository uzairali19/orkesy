use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use orkesy_core::model::ServiceStatus;
use orkesy_core::state::RuntimeState;

use crate::adapters::format_bytes;
use crate::ui::styles;

pub fn draw(
    f: &mut Frame<'_>,
    area: Rect,
    snapshot: &RuntimeState,
    start_time: Instant,
    project_name: &str,
) {
    let total_cpu: f64 = snapshot
        .metrics
        .values()
        .map(|m| m.cpu_percent as f64)
        .sum();
    let total_mem: u64 = snapshot.metrics.values().map(|m| m.memory_bytes).sum();

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
        Span::styled(project_name.to_string(), styles::text()),
        Span::raw("  "),
        Span::styled(
            format!("{running_count}/{total_services} running"),
            styles::success(),
        ),
        Span::raw("    "),
        Span::styled(format!("CPU {total_cpu:.0}%"), styles::warn()),
        Span::raw("  "),
        Span::styled(format!("MEM {}", format_bytes(total_mem)), styles::warn()),
        Span::raw("  "),
        Span::styled(format!("⏱ {uptime_str}"), styles::text_muted()),
    ]);

    f.render_widget(Paragraph::new(top_bar), area);
}
