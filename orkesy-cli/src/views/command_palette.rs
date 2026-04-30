use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app::{UiState, View};
use crate::ui::styles;

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerCategory {
    ServiceAction,
    ProjectAction,
    DetectedCommand,
    Navigation,
}

#[allow(dead_code)]
impl PickerCategory {
    pub fn label(&self) -> &'static str {
        match self {
            PickerCategory::ServiceAction => "Service Actions",
            PickerCategory::ProjectAction => "Project Actions",
            PickerCategory::DetectedCommand => "Commands",
            PickerCategory::Navigation => "Navigation",
        }
    }

    pub fn icon(&self) -> &'static str {
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
pub struct PickerItem {
    pub label: String,
    pub detail: Option<String>,
    pub category: PickerCategory,
    pub command: Option<String>,
    pub target_view: Option<View>,
    pub service_id: Option<String>,
}

#[allow(dead_code)]
impl PickerItem {
    pub fn new_service_action(
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

    pub fn new_project_action(label: &str, detail: Option<&str>, command: &str) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.map(|s| s.to_string()),
            category: PickerCategory::ProjectAction,
            command: Some(command.to_string()),
            target_view: None,
            service_id: None,
        }
    }

    pub fn new_detected_command(label: &str, command: &str, detail: Option<&str>) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.map(|s| s.to_string()),
            category: PickerCategory::DetectedCommand,
            command: Some(command.to_string()),
            target_view: None,
            service_id: None,
        }
    }

    pub fn new_navigation(label: &str, view: View) -> Self {
        Self {
            label: label.to_string(),
            detail: Some(format!("Press '{}' for quick access", view.key())),
            category: PickerCategory::Navigation,
            command: None,
            target_view: Some(view),
            service_id: None,
        }
    }

    pub fn fuzzy_matches(&self, pattern: &str) -> bool {
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

    pub fn fuzzy_score(&self, pattern: &str) -> i32 {
        if pattern.is_empty() {
            return 0;
        }
        let label_lower = self.label.to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        if label_lower.starts_with(&pattern_lower) {
            return 1000 - self.label.len() as i32;
        }
        if label_lower.contains(&pattern_lower) {
            return 500 - self.label.len() as i32;
        }
        if self.fuzzy_matches(pattern) {
            return 100 - self.label.len() as i32;
        }
        -1000
    }
}

pub fn build_picker_items(
    service_ids: &[String],
    selected_service: Option<&str>,
    _detected_commands: &[String],
) -> Vec<PickerItem> {
    let mut items = Vec::new();

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

    for id in service_ids {
        if selected_service == Some(id.as_str()) {
            continue;
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

pub fn filter_picker_items(items: &[PickerItem], query: &str) -> Vec<PickerItem> {
    if query.is_empty() {
        let mut result = items.to_vec();
        result.sort_by(|a, b| {
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

    let mut filtered: Vec<(PickerItem, i32)> = items
        .iter()
        .filter(|item| item.fuzzy_matches(query))
        .map(|item| (item.clone(), item.fuzzy_score(query)))
        .collect();

    filtered.sort_by(|a, b| b.1.cmp(&a.1));
    filtered.into_iter().map(|(item, _)| item).collect()
}

// Mutates `ui.palette_sugg_offset` to keep the selected item visible —
// a render-time clamp, not user-visible behaviour.
pub fn draw_modal(f: &mut Frame<'_>, area: Rect, ui: &mut UiState, picker_items: &[PickerItem]) {
    let modal_width = (area.width * 60 / 100).clamp(30, 60);
    let modal_height = ((picker_items.len() as u16 + 4).min(area.height.saturating_sub(4))).max(6);
    let modal_x = area.width.saturating_sub(modal_width) / 2;
    let modal_y = area.height.saturating_sub(modal_height) / 2;

    let modal_rect = Rect {
        x: modal_x,
        y: modal_y,
        width: modal_width,
        height: modal_height,
    };

    f.render_widget(Clear, modal_rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles::border_focused())
        .title(" Commands ");

    let inner = block.inner(modal_rect);
    f.render_widget(block, modal_rect);

    let modal_parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let input_area = modal_parts[0];
    let prompt = "> ";
    let max_input_chars = input_area.width.saturating_sub(prompt.len() as u16 + 1) as usize;

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

    let items_area = modal_parts[1];
    let visible_count = items_area.height as usize;

    if ui.palette_pick < ui.palette_sugg_offset {
        ui.palette_sugg_offset = ui.palette_pick;
    } else if visible_count > 0 && ui.palette_pick >= ui.palette_sugg_offset + visible_count {
        ui.palette_sugg_offset = ui.palette_pick.saturating_sub(visible_count - 1);
    }

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

        if ui.palette_input.is_empty() && last_category.as_ref() != Some(&item.category) {
            if last_category.is_some() && list_items.len() < visible_count {
                list_items.push(ListItem::new(Line::from("")));
            }
            last_category = Some(item.category.clone());
        }

        let icon = item.category.icon();
        let prefix = if is_selected { "▸ " } else { "  " };

        let item_style = if is_selected {
            styles::selection()
        } else {
            styles::text()
        };

        let mut spans = vec![
            Span::raw(prefix),
            Span::styled(format!("{icon} "), styles::text_muted()),
            Span::styled(&item.label, item_style),
        ];

        if let Some(detail) = &item.detail {
            let remaining = modal_width.saturating_sub(
                prefix.len() as u16 + icon.len() as u16 + item.label.len() as u16 + 6,
            );
            if remaining > 10 {
                let truncated_detail: String = detail.chars().take(remaining as usize).collect();
                spans.push(Span::styled(
                    format!("  {truncated_detail}"),
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

    let cursor_x = input_area.x + prompt.len() as u16 + input_display.len() as u16;
    let cursor_y = input_area.y;
    f.set_cursor_position((cursor_x, cursor_y));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_subsequence() {
        let item = PickerItem::new_navigation("Open Logs view", View::Logs);
        assert!(item.fuzzy_matches("ologs"));
        assert!(item.fuzzy_matches("logs"));
        assert!(!item.fuzzy_matches("xyz"));
    }

    #[test]
    fn fuzzy_score_ranks_prefix_highest() {
        let exact = PickerItem::new_navigation("Logs view", View::Logs);
        let later = PickerItem::new_navigation("Open Logs view", View::Logs);
        assert!(exact.fuzzy_score("Logs") > later.fuzzy_score("Logs"));
    }

    #[test]
    fn empty_query_returns_all_grouped() {
        let items = build_picker_items(&["api".into()], None, &[]);
        let out = filter_picker_items(&items, "");
        assert_eq!(out.len(), items.len());
    }
}
