use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::ui::styles;

pub fn draw(f: &mut Frame<'_>, area: Rect) {
    let help_width = 50u16.min(area.width.saturating_sub(4));
    let help_height = 24u16.min(area.height.saturating_sub(4));
    let help_x = area.width.saturating_sub(help_width) / 2;
    let help_y = area.height.saturating_sub(help_height) / 2;

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

    let lines = vec![
        Line::from(vec![Span::styled("VIEWS", styles::section_header())]),
        kv("  l ", "Logs view"),
        kv("  i ", "Inspect view"),
        kv("  e ", "Exec (commands) view"),
        kv("  d ", "Dependencies view"),
        kv("  m ", "Metrics view"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "FOCUS & NAVIGATION",
            styles::section_header(),
        )]),
        kv("  Tab ", "Switch focus (Services ↔ Right)"),
        kv("  j/k ↑↓ ", "Move selection / scroll"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "SERVICE ACTIONS",
            styles::section_header(),
        )]),
        kv("  Enter ", "Toggle service (start/stop)"),
        kv("  r     ", "Restart service"),
        kv("  s     ", "Stop service"),
        kv("  t     ", "Start service"),
        kv("  x     ", "Kill service"),
        kv("  c     ", "Clear logs"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "COMMANDS & SEARCH",
            styles::section_header(),
        )]),
        kv("  /     ", "Open command picker"),
        kv("  s     ", "Search logs (in Logs view)"),
        kv("  ?     ", "Toggle this help"),
        kv("  q     ", "Quit Orkesy"),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

fn kv(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(key, styles::key_hint()),
        Span::styled(desc, styles::text()),
    ])
}
