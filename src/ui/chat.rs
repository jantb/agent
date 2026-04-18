use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::markdown::markdown_to_lines;
use crate::types::{MessageKind, PlanStatus, Role};

use super::util::{compute_scroll, truncate};

pub(super) fn draw_chat(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mut lines: Vec<Line> = Vec::new();
    let separator = "─".repeat(area.width as usize);

    if let Some(date) = &app.resumed_session {
        lines.push(Line::from(Span::styled(
            format!("  Resumed session from {date}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(""));
    }

    for (msg_idx, msg) in app.messages.iter().enumerate() {
        if msg.role == Role::User
            && matches!(msg.kind, MessageKind::Text | MessageKind::Queued)
            && msg_idx != 0
        {
            lines.push(Line::from(Span::styled(
                separator.clone(),
                Style::default().fg(Color::DarkGray),
            )));
        }

        match &msg.kind {
            MessageKind::Text => match msg.role {
                Role::User => {
                    for (i, text_line) in msg.content.lines().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "❯ ",
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    text_line.to_string(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(
                                    text_line.to_string(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                            ]));
                        }
                    }
                }
                _ => {
                    lines.extend(markdown_to_lines(&msg.content, "  "));
                }
            },
            MessageKind::Queued => {
                for (i, text_line) in msg.content.lines().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled("❯ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                text_line.to_string(),
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                            Span::styled("  [queued]", Style::default().fg(Color::Indexed(243))),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(
                                text_line.to_string(),
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                        ]));
                    }
                }
            }
            MessageKind::ToolCall {
                name, arguments, ..
            } => {
                lines.push(Line::from(Span::styled(
                    format!("  ● {} {}", name, truncate(arguments, 80)),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            MessageKind::Thinking => {
                let content_lines: Vec<&str> = msg.content.lines().collect();
                let total = content_lines.len();
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502} ", Style::default().fg(Color::Indexed(243))),
                    Span::styled(
                        "thinking",
                        Style::default()
                            .fg(Color::Indexed(243))
                            .add_modifier(Modifier::ITALIC)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                let max_visible = 3;
                let show = if total <= max_visible + 1 {
                    total
                } else {
                    max_visible
                };
                for line in &content_lines[..show] {
                    lines.push(Line::from(vec![
                        Span::styled("  \u{2502} ", Style::default().fg(Color::Indexed(243))),
                        Span::styled(
                            line.to_string(),
                            Style::default()
                                .fg(Color::Indexed(245))
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                if total > show {
                    lines.push(Line::from(vec![
                        Span::styled("  \u{2502} ", Style::default().fg(Color::Indexed(243))),
                        Span::styled(
                            format!("[{} more lines]", total - show),
                            Style::default().fg(Color::Indexed(240)),
                        ),
                    ]));
                }
            }
            MessageKind::SubtaskEnter { label, .. } => {
                if msg_idx > 0 {
                    let has_prior_exit = app.messages[..msg_idx]
                        .iter()
                        .rev()
                        .take_while(|m| !matches!(m.kind, MessageKind::SubtaskEnter { .. }))
                        .any(|m| matches!(m.kind, MessageKind::SubtaskExit { .. }));
                    if has_prior_exit {
                        lines.push(Line::from(""));
                    }
                }
                let prefix = "──▶ ";
                let prefix_w = prefix.chars().count();
                let label_budget = (area.width as usize).saturating_sub(prefix_w + 1);
                let label_trunc = truncate(label, label_budget.saturating_sub(3));
                let used = prefix_w + label_trunc.chars().count() + 1;
                let filler = "─".repeat((area.width as usize).saturating_sub(used));
                lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::Cyan)),
                    Span::styled(
                        label_trunc,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {filler}"), Style::default().fg(Color::Cyan)),
                ]));
            }
            MessageKind::SubtaskExit { .. } => {
                let prefix = "──◀ done ";
                let filler =
                    "─".repeat((area.width as usize).saturating_sub(prefix.chars().count()));
                lines.push(Line::from(Span::styled(
                    format!("{prefix}{filler}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            MessageKind::PlanUpdate { items } => {
                lines.push(Line::from(Span::styled(
                    "  Plan:".to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                for item in items {
                    let prefix = match item.status {
                        PlanStatus::Pending => "[ ]",
                        PlanStatus::InProgress => "[>]",
                        PlanStatus::Completed => "[x]",
                    };
                    let color = match item.status {
                        PlanStatus::Pending => Color::White,
                        PlanStatus::InProgress => Color::Yellow,
                        PlanStatus::Completed => Color::DarkGray,
                    };
                    lines.push(Line::from(Span::styled(
                        format!("  {prefix} {}", item.content),
                        Style::default().fg(color),
                    )));
                }
            }
            MessageKind::ToolResult { name, is_error } => {
                let (symbol, color) = if *is_error {
                    ("\u{2717}", Color::Red)
                } else {
                    ("\u{2713}", Color::Green)
                };
                let content_lines: Vec<&str> = msg.content.lines().collect();
                if content_lines.len() <= 1 {
                    lines.push(Line::from(Span::styled(
                        format!("  {} {}: {}", symbol, name, truncate(&msg.content, 200)),
                        Style::default().fg(color),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {} {}:", symbol, name),
                        Style::default().fg(color),
                    )));
                    let max_lines = 10;
                    let total = content_lines.len();
                    for content_line in &content_lines[..total.min(max_lines)] {
                        lines.push(Line::from(Span::styled(
                            format!("    {content_line}"),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    if total > max_lines {
                        lines.push(Line::from(Span::styled(
                            format!("    (+{} more lines)", total - max_lines),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
            }
        }
    }

    if app.thinking {
        let spinner = app.spinner_char();
        lines.push(Line::from(vec![
            Span::styled("  \u{2502} ", Style::default().fg(Color::Indexed(243))),
            Span::styled(
                format!("{spinner} thinking"),
                Style::default()
                    .fg(Color::Indexed(243))
                    .add_modifier(Modifier::ITALIC)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if !app.current_thinking_text.is_empty() {
            for think_line in app.current_thinking_text.lines() {
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502} ", Style::default().fg(Color::Indexed(243))),
                    Span::styled(
                        think_line.to_string(),
                        Style::default()
                            .fg(Color::Indexed(245))
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
    }

    if app.streaming && !app.current_streaming_text.is_empty() {
        let text = &app.current_streaming_text;
        lines.extend(markdown_to_lines(text, "  "));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_height = paragraph.line_count(area.width) as u32;
    let scroll = compute_scroll(
        total_height,
        area.height as u32,
        app.auto_scroll,
        app.scroll_offset,
    );

    frame.render_widget(paragraph.scroll((scroll, 0)), area);
}
