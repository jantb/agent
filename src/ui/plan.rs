use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;
use crate::types::PlanStatus;

pub(super) fn draw_plan_panel(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "─ Plan ".to_string() + &"─".repeat(area.width.saturating_sub(8) as usize),
        Style::default().fg(Color::DarkGray),
    )));
    for item in &app.plan {
        let (marker, color) = match item.status {
            PlanStatus::Pending => ("[ ]", Color::White),
            PlanStatus::InProgress => ("[>]", Color::Yellow),
            PlanStatus::Completed => ("[x]", Color::DarkGray),
        };
        let budget = (area.width as usize)
            .saturating_sub(marker.len() + 1)
            .max(1);
        let content: String = item.content.chars().take(budget).collect();
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(
                content,
                Style::default().fg(color).add_modifier(
                    if matches!(item.status, PlanStatus::InProgress) {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
