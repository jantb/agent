use ratatui::{
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::autocomplete::COMMANDS;
use crate::markdown::markdown_to_lines;
use crate::types::{AgentMode, MessageKind, NodeStatus, Role};

const TREE_PANEL_WIDTH: u16 = 44;

pub fn draw(frame: &mut Frame, app: &App) {
    let img_count = app.pending_image_count();
    let queue_tag_len = if app.queue_len() > 0 {
        format!("[{}q] ", app.queue_len()).chars().count()
    } else {
        0
    };
    let base_prefix_len = "❯ ".chars().count()
        + queue_tag_len
        + if img_count > 0 {
            format!("[{img_count} img] ").chars().count()
        } else {
            0
        };
    let paste_tag_len = if let Some(p) = &app.input.pasted {
        format!("[{} lines pasted] ", p.lines().count())
            .chars()
            .count()
    } else {
        0
    };
    let first_prefix_len = (base_prefix_len + paste_tag_len) as u16;
    let input_height = app
        .input
        .visual_line_count(first_prefix_len as usize, 2, frame.area().width as usize)
        .min(5) as u16;
    let [title_area, body_area, input_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // Split body into chat + optional right tree panel
    let (chat_area, tree_area_opt) = if app.has_tree() && body_area.width > TREE_PANEL_WIDTH + 20 {
        let [tree, chat] =
            Layout::horizontal([Constraint::Length(TREE_PANEL_WIDTH), Constraint::Min(20)])
                .areas(body_area);
        (chat, Some(tree))
    } else {
        (body_area, None)
    };

    draw_title(frame, app, title_area);
    draw_chat(frame, app, chat_area);
    if let Some(tree_area) = tree_area_opt {
        draw_tree_panel(frame, app, tree_area);
    }
    draw_input(frame, app, input_area, first_prefix_len);
    draw_status(frame, app, status_area);

    if let Some(ac) = &app.autocomplete {
        draw_autocomplete(frame, ac, input_area);
    }
    if let Some(picker) = &app.model_picker {
        draw_model_picker(frame, picker, frame.area());
    }
    if let Some(picker) = &app.interview_picker {
        draw_interview_picker(frame, picker, frame.area());
    }
}

fn draw_tree_panel(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(Span::styled(
        "─ Agent Tree ".to_string() + &"─".repeat(area.width.saturating_sub(14) as usize),
        Style::default().fg(Color::DarkGray),
    )));

    for node in &app.tree {
        let indent = " ".repeat(node.depth);
        let (glyph, color) = match node.status {
            NodeStatus::Active => ("●", Color::Cyan),
            NodeStatus::Suspended => ("⊙", Color::Yellow),
            NodeStatus::Done => ("○", Color::DarkGray),
            NodeStatus::Failed => ("✗", Color::Red),
        };
        let connector = if node.depth > 0 { "└─ " } else { "" };
        let prefix_width = indent.len() + connector.len() + 3; // glyph (up to 2 cols) + space
        let avail = (area.width as usize).saturating_sub(prefix_width).max(1);
        let label_chars: Vec<char> = node.label.chars().collect();
        let chunks: Vec<String> = if label_chars.is_empty() {
            vec![String::new()]
        } else {
            label_chars
                .chunks(avail)
                .map(|c| c.iter().collect())
                .collect()
        };
        for (ci, chunk) in chunks.iter().enumerate() {
            if ci == 0 {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{indent}{connector}"),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(glyph, Style::default().fg(color)),
                    Span::raw(" "),
                    Span::styled(chunk.clone(), Style::default().fg(color)),
                ]));
            } else {
                let pad = " ".repeat(prefix_width);
                lines.push(Line::from(Span::styled(
                    format!("{pad}{chunk}"),
                    Style::default().fg(color),
                )));
            }
        }
    }

    // Show tool call counter for active node
    if app.subtask_tool_calls > 0 {
        lines.push(Line::from(Span::styled(
            format!("  {} tool calls", app.subtask_tool_calls),
            Style::default().fg(Color::Indexed(240)),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
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
                // Separator before enter if a prior subtask just exited
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

fn draw_input(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    first_line_prefix_len: u16,
) {
    let img_count = app.pending_image_count();
    let queue_tag = if app.queue_len() > 0 {
        format!("[{}q] ", app.queue_len())
    } else {
        String::new()
    };
    let base_prefix = if img_count > 0 {
        format!("❯ {queue_tag}[{img_count} img] ")
    } else {
        format!("❯ {queue_tag}")
    };
    let paste_tag = if let Some(pasted) = &app.input.pasted {
        let n = pasted.lines().count();
        format!("[{n} lines pasted] ")
    } else {
        String::new()
    };
    const CONT_PREFIX: u16 = 2;
    let prompt_color = if app.streaming {
        Color::DarkGray
    } else {
        Color::Cyan
    };

    let mut lines: Vec<Line> = Vec::new();
    for (li, text_line) in app.input.text.split('\n').enumerate() {
        let prefix_len = if li == 0 {
            first_line_prefix_len
        } else {
            CONT_PREFIX
        };
        let avail = (area.width as usize)
            .saturating_sub(prefix_len as usize)
            .max(1);
        let chars: Vec<char> = text_line.chars().collect();
        let chunks: Vec<String> = if chars.is_empty() {
            vec![String::new()]
        } else {
            chars.chunks(avail).map(|c| c.iter().collect()).collect()
        };
        for (ci, chunk) in chunks.iter().enumerate() {
            if li == 0 && ci == 0 {
                let mut spans = vec![Span::styled(
                    base_prefix.clone(),
                    Style::default().fg(prompt_color),
                )];
                if !paste_tag.is_empty() {
                    spans.push(Span::styled(
                        paste_tag.clone(),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                spans.push(Span::raw(chunk.clone()));
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("  ".to_string(), Style::default().fg(prompt_color)),
                    Span::raw(chunk.clone()),
                ]));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), area);

    // Wrap-aware cursor positioning
    let text_before_cursor = &app.input.text[..app.input.cursor_pos];
    let logical_row = text_before_cursor.matches('\n').count();
    let last_newline = text_before_cursor.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let cursor_col_in_logical = app.input.text[last_newline..app.input.cursor_pos]
        .chars()
        .count();

    // Count visual rows from logical lines before the cursor's logical line
    let mut visual_row = 0usize;
    for (i, tl) in app.input.text.split('\n').enumerate() {
        if i >= logical_row {
            break;
        }
        let p = if i == 0 {
            first_line_prefix_len as usize
        } else {
            CONT_PREFIX as usize
        };
        let av = (area.width as usize).saturating_sub(p).max(1);
        let cc = tl.chars().count();
        visual_row += if cc == 0 { 1 } else { cc.div_ceil(av) };
    }
    let prefix = if logical_row == 0 {
        first_line_prefix_len as usize
    } else {
        CONT_PREFIX as usize
    };
    let avail = (area.width as usize).saturating_sub(prefix).max(1);
    visual_row += cursor_col_in_logical / avail;
    let visual_col = cursor_col_in_logical % avail;

    let input_height = area.height as usize;
    let visual_row = visual_row.min(input_height.saturating_sub(1)) as u16;

    if app.model_picker.is_none() && app.interview_picker.is_none() {
        frame.set_cursor_position(Position::new(
            area.x + prefix as u16 + visual_col as u16,
            area.y + visual_row,
        ));
    }
}

fn draw_autocomplete(frame: &mut Frame, ac: &crate::autocomplete::Autocomplete, input_area: Rect) {
    if ac.filtered.is_empty() {
        return;
    }
    let max_visible = 6;
    let count = ac.filtered.len().min(max_visible);
    let popup_height = count as u16 + 2;
    let popup_width = 35u16.min(input_area.width);
    let popup_y = input_area.y.saturating_sub(popup_height);
    let area = Rect::new(input_area.x, popup_y, popup_width, popup_height);

    let lines: Vec<Line> = ac
        .filtered
        .iter()
        .take(max_visible)
        .enumerate()
        .map(|(i, &cmd_idx)| {
            let cmd = &COMMANDS[cmd_idx];
            let (bg, fg) = if i == ac.selected {
                (Color::DarkGray, Color::White)
            } else {
                (Color::Reset, Color::White)
            };
            Line::from(vec![
                Span::styled(
                    format!(" {:<10}", cmd.name),
                    Style::default()
                        .bg(bg)
                        .fg(fg)
                        .add_modifier(if i == ac.selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!(" {}", cmd.desc),
                    Style::default().bg(bg).fg(Color::Indexed(245)),
                ),
            ])
        })
        .collect();

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().border_style(Style::default().fg(Color::DarkGray))),
        area,
    );
}

fn draw_model_picker(frame: &mut Frame, picker: &crate::app::ModelPickerState, area: Rect) {
    let max_visible = 10usize;
    let count = picker.models.len().min(max_visible);
    let popup_height = count as u16 + 2;
    let popup_width = 40u16.min(area.width);
    let popup_x = area.x + area.width.saturating_sub(popup_width) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Determine scroll window so selected is always visible
    let start = if picker.selected >= max_visible {
        picker.selected - max_visible + 1
    } else {
        0
    };

    let lines: Vec<Line> = picker
        .models
        .iter()
        .enumerate()
        .skip(start)
        .take(max_visible)
        .map(|(i, name)| {
            let (bg, fg) = if i == picker.selected {
                (Color::DarkGray, Color::White)
            } else {
                (Color::Reset, Color::White)
            };
            Line::from(Span::styled(
                format!(" {name}"),
                Style::default()
                    .bg(bg)
                    .fg(fg)
                    .add_modifier(if i == picker.selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ))
        })
        .collect();

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Select model ")
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        popup_area,
    );
}

fn draw_interview_picker(frame: &mut Frame, picker: &crate::app::InterviewPickerState, area: Rect) {
    let max_width = (area.width * 3 / 4).max(30).min(area.width);
    let inner_width = max_width.saturating_sub(2) as usize;

    // Build lines
    let mut lines: Vec<Line> = Vec::new();

    // Question (word-wrapped, yellow bold)
    for wl in word_wrap(&picker.question, inner_width) {
        lines.push(Line::from(Span::styled(
            wl,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));

    // Suggestions
    for (i, suggestion) in picker.suggestions.iter().enumerate() {
        let selected = !picker.custom_mode && i == picker.selected;
        let (bg, fg) = if selected {
            (Color::DarkGray, Color::White)
        } else {
            (Color::Reset, Color::White)
        };
        let marker = if selected { "▸ " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{marker}{suggestion}"),
            Style::default().bg(bg).fg(fg).add_modifier(if selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        )));
    }

    // Separator + custom input
    lines.push(Line::from(Span::styled(
        "── or type your own ──",
        Style::default().fg(Color::DarkGray),
    )));
    let custom_marker = if picker.custom_mode { "▸ " } else { "  " };
    let cursor = if picker.custom_mode { "█" } else { "" };
    let custom_bg = if picker.custom_mode {
        Color::DarkGray
    } else {
        Color::Reset
    };
    lines.push(Line::from(Span::styled(
        format!("{custom_marker}{}{cursor}", picker.custom_input),
        Style::default()
            .bg(custom_bg)
            .fg(Color::White)
            .add_modifier(if picker.custom_mode {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )));

    // Footer
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab: toggle  Enter: select  Esc: skip",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let popup_height = (lines.len() as u16 + 2).min(area.height);
    let popup_x = area.x + area.width.saturating_sub(max_width) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, max_width, popup_height);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Interview Question ")
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        popup_area,
    );
}

fn draw_status(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let elapsed_str = format_elapsed(app.elapsed_secs());

    // Build optional agent breadcrumb
    let breadcrumb = if app.has_tree() {
        let active_label = app
            .tree
            .iter()
            .rfind(|n| n.status == NodeStatus::Active)
            .map(|n| n.label.as_str())
            .unwrap_or("");
        let calls = app.subtask_tool_calls;
        let call_str = if calls > 0 {
            format!(" ● {calls} tool calls")
        } else {
            String::new()
        };
        format!("[{active_label}]{call_str}  ")
    } else {
        String::new()
    };

    let left_span = if let Some(err) = &app.error_message {
        Span::styled(err.as_str(), Style::default().fg(Color::Red))
    } else if app.thinking {
        let s = app.spinner_char();
        Span::styled(
            format!("{breadcrumb}{s} Thinking...{elapsed_str}"),
            Style::default().fg(Color::Yellow),
        )
    } else if app.streaming {
        let s = app.spinner_char();
        let speed = app
            .tok_per_sec()
            .map(|t| format!(" ({:.0} tok/s)", t))
            .unwrap_or_default();
        let queue = if app.queue_len() > 0 {
            format!(" ({} queued)", app.queue_len())
        } else {
            String::new()
        };
        Span::styled(
            format!("{breadcrumb}{s} Streaming...{elapsed_str}{speed}{queue}"),
            Style::default().fg(Color::Cyan),
        )
    } else {
        Span::raw("")
    };

    let mut right_spans: Vec<Span> = Vec::new();

    if app.mode != AgentMode::Oneshot {
        right_spans.push(Span::styled(
            app.mode.label(),
            Style::default().fg(match app.mode {
                AgentMode::Plan => Color::Blue,
                AgentMode::Thorough => Color::Green,
                AgentMode::Oneshot => Color::DarkGray,
            }),
        ));
        right_spans.push(Span::raw("  "));
    }

    // Cumulative token counters
    {
        let up = fmt_tokens(app.total_tokens_up);
        let dn = fmt_tokens(app.total_tokens_down);
        right_spans.push(Span::styled(
            format!("\u{2191}{up} \u{2193}{dn}"),
            Style::default().fg(Color::DarkGray),
        ));
        right_spans.push(Span::raw("  "));
    }

    if let Some(total) = app.context_window_size {
        let used = app.context_used;
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

fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        let s = format!("{:.1}", n as f64 / 1_000.0);
        format!("{}k", s.trim_end_matches('0').trim_end_matches('.'))
    } else {
        let s = format!("{:.1}", n as f64 / 1_000_000.0);
        format!("{}M", s.trim_end_matches('0').trim_end_matches('.'))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_w = word.chars().count();
        if current.is_empty() {
            if word_w >= max_width {
                let mut rem = word;
                while rem.chars().count() >= max_width {
                    let split = rem
                        .char_indices()
                        .nth(max_width)
                        .map(|(i, _)| i)
                        .unwrap_or(rem.len());
                    lines.push(rem[..split].to_string());
                    rem = &rem[split..];
                }
                current = rem.to_string();
            } else {
                current.push_str(word);
            }
        } else if current.chars().count() + 1 + word_w <= max_width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn compute_scroll(
    total_height: u32,
    area_height: u32,
    auto_scroll: bool,
    scroll_offset: u32,
) -> u16 {
    let max_scroll = total_height.saturating_sub(area_height);
    if auto_scroll {
        max_scroll as u16
    } else {
        let clamped = scroll_offset.min(max_scroll);
        max_scroll.saturating_sub(clamped) as u16
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
    fn fmt_tokens_small() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
    }

    #[test]
    fn fmt_tokens_kilo() {
        assert_eq!(fmt_tokens(1_000), "1k");
        assert_eq!(fmt_tokens(12_345), "12.3k");
        assert_eq!(fmt_tokens(100_000), "100k");
    }

    #[test]
    fn fmt_tokens_mega() {
        assert_eq!(fmt_tokens(1_000_000), "1M");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
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

    #[test]
    fn compute_scroll_auto_scroll_bottom() {
        // 100 visual lines, 20 viewport => scroll to line 80
        assert_eq!(compute_scroll(100, 20, true, 0), 80);
    }

    #[test]
    fn compute_scroll_auto_scroll_content_fits() {
        // Content fits in viewport — no scroll needed
        assert_eq!(compute_scroll(10, 20, true, 0), 0);
    }

    #[test]
    fn compute_scroll_manual_at_bottom() {
        // offset=0 means "at the bottom" => same as auto_scroll
        assert_eq!(compute_scroll(100, 20, false, 0), 80);
    }

    #[test]
    fn compute_scroll_manual_scrolled_up() {
        // offset=30 means 30 lines up from bottom => scroll to line 50
        assert_eq!(compute_scroll(100, 20, false, 30), 50);
    }

    #[test]
    fn compute_scroll_manual_offset_clamped() {
        // offset exceeds max_scroll — clamp to top
        assert_eq!(compute_scroll(100, 20, false, 999), 0);
    }

    #[test]
    fn line_count_accounts_for_wrapping() {
        // A single logical line that's 80 chars wide, rendered in a 20-col area => 4 visual lines
        let long_line = "a".repeat(80);
        let paragraph = Paragraph::new(vec![Line::from(long_line)]).wrap(Wrap { trim: false });
        let visual = paragraph.line_count(20);
        assert_eq!(visual, 4);
    }

    #[test]
    fn scroll_with_wrapped_lines_reaches_bottom() {
        // 5 logical lines, each 40 chars, in a 20-col viewport (10 rows high)
        // Each line wraps to 2 visual lines => 10 visual total, fits exactly
        let lines: Vec<Line> = (0..5).map(|_| Line::from("a".repeat(40))).collect();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let total_height = paragraph.line_count(20) as u32;
        assert_eq!(total_height, 10);
        // With viewport height 6, auto_scroll should be at 4
        assert_eq!(compute_scroll(total_height, 6, true, 0), 4);
    }

    // Helper: collect buffer cells in a row range into a String.
    fn row_text(
        buffer: &ratatui::buffer::Buffer,
        row: u16,
        col_start: u16,
        col_end: u16,
    ) -> String {
        (col_start..col_end)
            .map(|c| {
                buffer
                    .cell(ratatui::layout::Position::new(c, row))
                    .map(|cell| cell.symbol())
                    .unwrap_or(" ")
            })
            .collect()
    }

    #[test]
    fn tree_panel_width_is_44() {
        assert_eq!(TREE_PANEL_WIDTH, 44);
    }

    #[test]
    fn tree_panel_renders_on_left() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();
        app.enter_subtask(1, "test_worker".into());

        assert!(
            app.has_tree(),
            "has_tree() must be true after enter_subtask"
        );

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Row 1 is the body start (row 0 is title). The tree header "─ Agent Tree"
        // must appear somewhere in the LEFT portion (cols 0..TREE_PANEL_WIDTH).
        let found_header = (1..30u16).any(|row| {
            let left = row_text(&buffer, row, 0, TREE_PANEL_WIDTH);
            left.contains("Agent Tree")
        });
        assert!(found_header, "Tree header not found in left panel columns");

        // Verify the header is NOT in the right (chat) portion of the same row.
        let found_in_right = (1..30u16).any(|row| {
            let right = row_text(&buffer, row, TREE_PANEL_WIDTH, 100);
            right.contains("Agent Tree")
        });
        assert!(
            !found_in_right,
            "Tree header must not appear in the chat area"
        );
    }

    #[test]
    fn thinking_renders_after_subtask_output() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();
        // Simulate subtask enter/exit, then orchestrator thinking
        app.enter_subtask(1, "worker".into());
        app.exit_subtask(1);
        app.set_thinking(true);
        app.append_thinking_text("considering next step");

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Find the row containing SubtaskExit marker "◀" and the row containing "thinking"
        let exit_row = (0..30u16).find(|&r| row_text(&buffer, r, 0, 100).contains("◀"));
        let thinking_row = (0..30u16).find(|&r| row_text(&buffer, r, 0, 100).contains("thinking"));

        let exit_row = exit_row.expect("subtask exit marker not found");
        let thinking_row = thinking_row.expect("thinking text not found");

        assert!(
            thinking_row > exit_row,
            "thinking (row {thinking_row}) must render after subtask exit (row {exit_row})"
        );
    }

    #[test]
    fn tree_indent_single_space_per_depth() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();
        app.enter_subtask(1, "coordinator".into());
        app.enter_subtask(2, "worker".into());

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Find the row that contains "worker" in the tree panel (left 44 cols).
        let worker_row =
            (0..30u16).find(|&row| row_text(&buffer, row, 0, TREE_PANEL_WIDTH).contains("worker"));

        let worker_row = worker_row.expect("worker node not found in tree panel");
        let left = row_text(&buffer, worker_row, 0, TREE_PANEL_WIDTH);

        // depth=2 → 2 leading spaces before the connector "└─ "
        // With 1-space-per-depth: "  └─ ●  worker"
        // With 2-space-per-depth: "    └─ ● worker"
        assert!(
            left.starts_with("  └"),
            "Expected 2 leading spaces (1 per depth level) before connector, got: {left:?}"
        );
        assert!(
            !left.starts_with("    └"),
            "Got 4 leading spaces — indent should be 1 space per depth, not 2: {left:?}"
        );
    }

    #[test]
    fn word_wrap_fits_on_one_line() {
        assert_eq!(word_wrap("hello world", 20), vec!["hello world"]);
    }

    #[test]
    fn word_wrap_breaks_at_word_boundary() {
        assert_eq!(word_wrap("hello world", 7), vec!["hello", "world"]);
    }

    #[test]
    fn word_wrap_long_word_char_splits() {
        let result = word_wrap("abcdefgh", 4);
        assert_eq!(result, vec!["abcd", "efgh"]);
    }

    #[test]
    fn word_wrap_empty_string() {
        assert_eq!(word_wrap("", 10), vec![""]);
    }

    #[test]
    fn diagnose_subtask_rendering() {
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();

        // First subtask
        app.enter_subtask(1, "first task: read and summarize".into());
        app.add_tool_call(&crate::types::ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "test.txt"}),
        });
        app.add_tool_result(&crate::types::ToolResult {
            call_id: "c1".into(),
            output: "file contents here".into(),
            is_error: false,
            images: vec![],
        });
        app.exit_subtask(1);

        // Orchestrator delegate_task result + next call
        app.add_tool_result(&crate::types::ToolResult {
            call_id: "d1".into(),
            output: "subtask 1 done".into(),
            is_error: false,
            images: vec![],
        });
        app.add_tool_call(&crate::types::ToolCall {
            id: "d2".into(),
            name: "delegate_task".into(),
            arguments: serde_json::json!({"prompt": "second task"}),
        });

        // Second subtask
        app.enter_subtask(1, "second task: write output".into());
        app.add_tool_call(&crate::types::ToolCall {
            id: "c2".into(),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": "out.txt"}),
        });
        app.add_tool_result(&crate::types::ToolResult {
            call_id: "c2".into(),
            output: "ok".into(),
            is_error: false,
            images: vec![],
        });
        app.exit_subtask(1);

        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let chat_start = TREE_PANEL_WIDTH;
        let chat_end = 100u16;

        // Dump every row with text and color info
        println!(
            "\n=== RENDERED BUFFER (chat area cols {}..{}) ===",
            chat_start, chat_end
        );
        for row in 0..40u16 {
            let text = row_text(&buffer, row, chat_start, chat_end);
            if text.trim().is_empty() {
                println!("row {:2}: [blank]", row);
                continue;
            }
            // Check colors on this row
            let mut colors: Vec<String> = Vec::new();
            let mut prev_fg: Option<Color> = None;
            for col in chat_start..chat_end {
                if let Some(cell) = buffer.cell(ratatui::layout::Position::new(col, row)) {
                    let fg = cell.fg;
                    if prev_fg != Some(fg) {
                        colors.push(format!("c{}:{:?}", col, fg));
                        prev_fg = Some(fg);
                    }
                }
            }
            println!(
                "row {:2}: |{}|  colors: {}",
                row,
                text.trim_end(),
                colors.join(", ")
            );
        }
        println!("=== END ===\n");

        // Find SubtaskEnter rows and check cyan coverage
        let enter_rows: Vec<u16> = (0..40u16)
            .filter(|&r| row_text(&buffer, r, chat_start, chat_end).contains('▶'))
            .collect();
        println!("SubtaskEnter rows: {:?}", enter_rows);

        for &row in &enter_rows {
            let mut non_cyan = Vec::new();
            for col in chat_start..chat_end {
                if let Some(cell) = buffer.cell(ratatui::layout::Position::new(col, row)) {
                    if cell.fg != Color::Cyan {
                        non_cyan.push((col, cell.fg, cell.symbol().to_string()));
                    }
                }
            }
            if !non_cyan.is_empty() {
                println!("Row {} non-cyan cells: {:?}", row, non_cyan);
            } else {
                println!("Row {} is all cyan", row);
            }
        }

        // Find SubtaskExit rows
        let exit_rows: Vec<u16> = (0..40u16)
            .filter(|&r| row_text(&buffer, r, chat_start, chat_end).contains('◀'))
            .collect();
        println!("SubtaskExit rows: {:?}", exit_rows);

        // Check for separator between first exit and second enter
        if enter_rows.len() >= 2 && !exit_rows.is_empty() {
            let first_exit = exit_rows[0];
            let second_enter = enter_rows[1];
            println!(
                "Gap between first exit (row {}) and second enter (row {}): {} rows",
                first_exit,
                second_enter,
                second_enter - first_exit
            );
            for r in first_exit..=second_enter {
                let text = row_text(&buffer, r, chat_start, chat_end);
                println!("  row {}: |{}|", r, text.trim_end());
            }
        }
    }

    #[test]
    fn subtask_enter_long_label_does_not_overflow() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();
        app.enter_subtask(1, "a".repeat(200));

        let term_w = 80u16;
        let backend = TestBackend::new(term_w, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let chat_start = TREE_PANEL_WIDTH;
        let chat_end = term_w;

        let enter_row = (0..10u16)
            .find(|&r| row_text(&buffer, r, chat_start, chat_end).contains('▶'))
            .expect("SubtaskEnter row not found");

        // The SubtaskEnter should render on exactly one line (no wrapping).
        // If the line overflows, Paragraph::wrap pushes content to the next row.
        let next_row = row_text(&buffer, enter_row + 1, chat_start, chat_end);
        assert!(
            next_row.trim().is_empty()
                || next_row.contains('●')
                || next_row.contains('○')
                || next_row.contains('⊙')
                || next_row.contains('✗'),
            "SubtaskEnter wrapped to next line — overflow detected\nnext row: {next_row:?}"
        );
    }

    #[test]
    fn subtask_enter_truncation_arithmetic() {
        // Test current (buggy) formula overflows
        let area_width = 60usize;
        let prefix_w = 4; // "──▶ "
        let label = "x".repeat(100);

        let buggy_trunc = truncate(&label, area_width.saturating_sub(prefix_w + 1));
        let buggy_used = prefix_w + buggy_trunc.chars().count() + 1;
        assert!(
            buggy_used > area_width,
            "Bug should cause overflow: used={buggy_used} should > {area_width}"
        );

        // Test fixed formula fits
        let label_budget = area_width.saturating_sub(prefix_w + 1);
        let fixed_trunc = truncate(&label, label_budget.saturating_sub(3));
        let fixed_used = prefix_w + fixed_trunc.chars().count() + 1;
        assert!(
            fixed_used <= area_width,
            "Fixed: used={fixed_used} > area_width={area_width}"
        );
    }

    #[test]
    fn tree_panel_label_does_not_exceed_panel_width() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();
        app.enter_subtask(1, "a".repeat(60));
        app.enter_subtask(2, "b".repeat(60));
        app.enter_subtask(3, "c".repeat(60));

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        for row in 0..20u16 {
            let tree_row = row_text(&buffer, row, 0, TREE_PANEL_WIDTH);
            let trimmed_len = tree_row.trim_end().chars().count();
            assert!(
                trimmed_len <= TREE_PANEL_WIDTH as usize,
                "Tree row {row} overflows: {trimmed_len} chars > {TREE_PANEL_WIDTH}\nrow: {tree_row:?}"
            );
        }
    }

    #[test]
    fn tree_panel_prefix_arithmetic() {
        let area_width = TREE_PANEL_WIDTH as usize;
        for depth in 0..=5usize {
            let indent_len = depth;
            let connector_len = if depth > 0 { 3 } else { 0 }; // "└─ "
            let prefix_width = indent_len + connector_len + 3;
            let avail = area_width.saturating_sub(prefix_width).max(1);
            let conservative_total = indent_len + connector_len + 2 + 1 + avail;
            assert!(
                conservative_total <= area_width,
                "depth={depth}: {conservative_total} > {area_width}"
            );
        }
    }
}
