use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn draw(
    f: &mut Frame<'_>,
    right_pane: Rect,
    query: &str,
    match_idx: usize,
    total_matches: usize,
) {
    let search_h = 3u16;
    let search_rect = Rect {
        x: right_pane.x,
        width: right_pane.width,
        height: search_h,
        y: right_pane.y + right_pane.height.saturating_sub(search_h),
    };

    f.render_widget(Clear, search_rect);

    let match_info = if total_matches == 0 {
        if query.is_empty() {
            String::new()
        } else {
            " (no matches)".to_string()
        }
    } else {
        format!(" ({}/{})", match_idx + 1, total_matches)
    };

    let title = format!(" Search{match_info} ");
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let search_text = format!("/{query}");
    f.render_widget(Paragraph::new(search_text).block(block), search_rect);

    let cursor_x = search_rect.x + 2 + query.len() as u16;
    let cursor_y = search_rect.y + 1;
    f.set_cursor_position((cursor_x, cursor_y));
}
