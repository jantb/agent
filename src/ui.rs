use ratatui::{
    layout::{Constraint, Layout, Position},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::app::{App, MessageKind};
use crate::markdown::markdown_to_lines;
use crate::types::Role;

pub fn draw(frame: &mut Frame, app: &App) {
    let input_height = if app.streaming {
        1
    } else {
        app.input.line_count().min(5) as u16
    };
    let [title_area, chat_area, input_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_title(frame, app, title_area);
    draw_chat(frame, app, chat_area);
    draw_input(frame, app, input_area);
    draw_status(frame, app, status_area);
}

fn draw_title(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
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

fn draw_chat(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
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
        // Separator before each user message except the first message overall
        if msg.role == Role::User && matches!(msg.kind, MessageKind::Text) && msg_idx != 0 {
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
                let max_lines = 5;
                let total = content_lines.len();
                for content_line in &content_lines[..total.min(max_lines)] {
                    lines.push(Line::from(Span::styled(
                        format!("  {content_line}"),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                if total > max_lines {
                    lines.push(Line::from(Span::styled(
                        format!("  (+{} more lines)", total - max_lines),
                        Style::default().fg(Color::DarkGray),
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
                    let max_lines = 5;
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
        if !app.current_thinking_text.is_empty() {
            let think_lines: Vec<&str> = app.current_thinking_text.lines().collect();
            let max_lines = 10;
            let total = think_lines.len();
            let start = total.saturating_sub(max_lines);
            for think_line in &think_lines[start..] {
                lines.push(Line::from(Span::styled(
                    format!("  {think_line}"),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            if total > max_lines {
                let spinner = app.spinner_char();
                lines.push(Line::from(Span::styled(
                    format!("  {spinner} ...thinking ({total} lines)"),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
        } else {
            let spinner = app.spinner_char();
            lines.push(Line::from(Span::styled(
                format!("  {spinner} Thinking..."),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
    }

    if app.streaming && !app.current_streaming_text.is_empty() {
        let text = &app.current_streaming_text;
        lines.extend(markdown_to_lines(text, "  "));
        if let Some(last) = lines.last_mut() {
            last.spans
                .push(Span::styled("\u{258b}", Style::default().fg(Color::Cyan)));
        }
    } else if app.streaming {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("\u{258b}", Style::default().fg(Color::Cyan)),
        ]));
    }

    let total_height = lines.len() as u32;
    let area_height = area.height as u32;
    let max_scroll = total_height.saturating_sub(area_height);

    let scroll: u16 = if app.auto_scroll {
        max_scroll as u16
    } else {
        let clamped = app.scroll_offset.min(max_scroll);
        max_scroll.saturating_sub(clamped) as u16
    };

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

fn draw_input(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.streaming {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "❯ ",
                Style::default().fg(Color::DarkGray),
            ))),
            area,
        );
    } else {
        let img_count = app.pending_image_count();
        let base_prefix = if img_count > 0 {
            format!("❯ [{img_count} img] ")
        } else {
            "❯ ".to_string()
        };
        let paste_tag = if let Some(pasted) = &app.input.pasted {
            let n = pasted.lines().count();
            format!("[{n} lines pasted] ")
        } else {
            String::new()
        };
        let first_line_prefix_len =
            (base_prefix.chars().count() + paste_tag.chars().count()) as u16;
        let continuation = "  ".to_string();

        let mut lines: Vec<Line> = Vec::new();
        for (i, text_line) in app.input.text.split('\n').enumerate() {
            if i == 0 {
                let mut spans = vec![Span::styled(
                    base_prefix.clone(),
                    Style::default().fg(Color::Cyan),
                )];
                if !paste_tag.is_empty() {
                    spans.push(Span::styled(
                        paste_tag.clone(),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                spans.push(Span::raw(text_line.to_string()));
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(continuation.clone(), Style::default().fg(Color::Cyan)),
                    Span::raw(text_line.to_string()),
                ]));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                base_prefix,
                Style::default().fg(Color::Cyan),
            )]));
        }
        frame.render_widget(Paragraph::new(lines), area);
        let text_before_cursor = &app.input.text[..app.input.cursor_pos];
        let cursor_row = text_before_cursor.matches('\n').count() as u16;
        let last_newline = text_before_cursor.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let cursor_col = app.input.text[last_newline..app.input.cursor_pos].len() as u16;
        let row_prefix_len = if cursor_row == 0 {
            first_line_prefix_len
        } else {
            continuation.chars().count() as u16
        };
        frame.set_cursor_position(Position::new(
            area.x + row_prefix_len + cursor_col,
            area.y + cursor_row,
        ));
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let elapsed_str = format_elapsed(app.elapsed_secs());

    let left_span = if let Some(err) = &app.error_message {
        Span::styled(err.as_str(), Style::default().fg(Color::Red))
    } else if app.thinking {
        let s = app.spinner_char();
        Span::styled(
            format!("{s} Thinking...{elapsed_str}"),
            Style::default().fg(Color::Yellow),
        )
    } else if app.streaming {
        let s = app.spinner_char();
        let speed = app
            .tok_per_sec()
            .map(|t| format!(" ({:.0} tok/s)", t))
            .unwrap_or_default();
        Span::styled(
            format!("{s} Streaming...{elapsed_str}{speed}"),
            Style::default().fg(Color::Cyan),
        )
    } else {
        Span::raw("")
    };

    let mut right_spans: Vec<Span> = Vec::new();

    if let (Some(used), Some(total)) = (app.last_prompt_eval_count, app.context_window_size) {
        if total > 0 {
            let pct = (used as f64 / total as f64 * 100.0).round() as u64;
            right_spans.push(Span::styled(
                format!(
                    "ctx {} {}% {}/{}",
                    context_bar(used, total, 10),
                    pct,
                    used,
                    total
                ),
                Style::default().fg(Color::DarkGray),
            ));
            right_spans.push(Span::raw("  "));
        }
    }

    for name in &app.mcp_connected {
        if !right_spans.is_empty() {
            right_spans.push(Span::raw("  "));
        }
        right_spans.push(Span::styled(
            format!("\u{25cf} {name}"),
            Style::default().fg(Color::Green),
        ));
    }
    for (name, _reason) in &app.mcp_failed {
        if !right_spans.is_empty() {
            right_spans.push(Span::raw("  "));
        }
        right_spans.push(Span::styled(
            format!("\u{25cf} {name}"),
            Style::default().fg(Color::Red),
        ));
    }

    let right_width: u16 = right_spans
        .iter()
        .map(|s| s.content.chars().count() as u16)
        .sum();
    let left_width = area.width.saturating_sub(right_width);

    let mut spans = vec![left_span.clone()];
    let left_chars = left_span.content.chars().count() as u16;
    let pad = left_width.saturating_sub(left_chars);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad as usize)));
    }
    spans.extend(right_spans);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn format_elapsed(secs: Option<f64>) -> String {
    match secs {
        None => String::new(),
        Some(s) => {
            let total = s as u64;
            if total < 60 {
                format!(" {total}s")
            } else if total < 3600 {
                format!(" {}m{}s", total / 60, total % 60)
            } else {
                format!(" {}h{}m{}s", total / 3600, (total % 3600) / 60, total % 60)
            }
        }
    }
}

fn context_bar(used: u64, total: u64, width: usize) -> String {
    let ratio = (used as f64 / total as f64).clamp(0.0, 1.0);
    let filled = (ratio * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty)
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_bar_empty() {
        assert_eq!(
            context_bar(0, 100, 10),
            "[\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}]"
        );
    }

    #[test]
    fn context_bar_full() {
        assert_eq!(
            context_bar(100, 100, 10),
            "[\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}]"
        );
    }

    #[test]
    fn context_bar_half() {
        assert_eq!(
            context_bar(50, 100, 10),
            "[\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}]"
        );
    }

    #[test]
    fn context_bar_clamps_over_100_percent() {
        let bar = context_bar(200, 100, 10);
        assert_eq!(
            bar,
            "[\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}]"
        );
    }

    #[test]
    fn format_elapsed_none() {
        assert_eq!(format_elapsed(None), "");
    }

    #[test]
    fn format_elapsed_seconds() {
        assert_eq!(format_elapsed(Some(5.0)), " 5s");
    }

    #[test]
    fn format_elapsed_minutes() {
        assert_eq!(format_elapsed(Some(125.0)), " 2m5s");
    }

    #[test]
    fn format_elapsed_hours() {
        assert_eq!(format_elapsed(Some(3661.0)), " 1h1m1s");
    }

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_gets_ellipsis() {
        assert_eq!(truncate("hello world", 5), "hello...");
    }

    #[test]
    fn truncate_multibyte_chars() {
        // Each emoji is a single char but multiple bytes; truncating at 3 chars must not panic
        let s = "😀😁😂😃😄";
        assert_eq!(truncate(s, 3), "😀😁😂...");
        // CJK characters
        let cjk = "你好世界测试";
        assert_eq!(truncate(cjk, 4), "你好世界...");
    }
}
