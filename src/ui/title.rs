use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;

pub(super) fn draw_title(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mcp_count = app.mcp_connected.len();
    let spans = vec![
        Span::raw(app.model_name.as_str()),
        Span::raw("  "),
        Span::styled(
            app.working_dir.display().to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[mcp: {mcp_count}]"),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
