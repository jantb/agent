use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use crate::autocomplete::{COMMANDS, HINTS};

use super::util::word_wrap;

pub(super) fn draw_autocomplete(
    frame: &mut Frame,
    ac: &crate::autocomplete::Autocomplete,
    input_area: Rect,
) {
    if ac.filtered.is_empty() {
        return;
    }
    let max_visible = 10;
    let count = ac.filtered.len().min(max_visible);
    // Only show hints when the prefix is just "/" (the full palette).
    let show_hints = ac.filtered.len() == COMMANDS.len();
    let hint_rows = if show_hints { HINTS.len() + 1 } else { 0 };
    let popup_height = (count + hint_rows) as u16 + 2;
    let popup_width = 44u16.min(input_area.width);
    let popup_y = input_area.y.saturating_sub(popup_height);
    let area = Rect::new(input_area.x, popup_y, popup_width, popup_height);

    let mut lines: Vec<Line> = ac
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

    if show_hints {
        lines.push(Line::from(Span::styled(
            " — keys —",
            Style::default().fg(Color::DarkGray),
        )));
        for (k, d) in HINTS {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:<10}", k),
                    Style::default().fg(Color::Indexed(243)),
                ),
                Span::styled(format!(" {}", d), Style::default().fg(Color::Indexed(240))),
            ]));
        }
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().border_style(Style::default().fg(Color::DarkGray))),
        area,
    );
}

pub(super) fn draw_model_picker(
    frame: &mut Frame,
    picker: &crate::app::ModelPickerState,
    area: Rect,
) {
    let max_visible = 10usize;
    let count = picker.filtered.len().min(max_visible);
    // +2 border, +1 filter line
    let popup_height = count as u16 + 3;
    let popup_width = 40u16.min(area.width);
    let popup_x = area.x + area.width.saturating_sub(popup_width) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    let start = if picker.selected >= max_visible {
        picker.selected - max_visible + 1
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::with_capacity(count + 1);
    lines.push(Line::from(vec![
        Span::styled(" ▸ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            picker.filter.clone(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█", Style::default().fg(Color::DarkGray)),
    ]));

    for (row, &model_idx) in picker
        .filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(max_visible)
    {
        let name = &picker.models[model_idx];
        let (bg, fg) = if row == picker.selected {
            (Color::DarkGray, Color::White)
        } else {
            (Color::Reset, Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!(" {name}"),
            Style::default()
                .bg(bg)
                .fg(fg)
                .add_modifier(if row == picker.selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )));
    }

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Select model (type to filter) ")
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        popup_area,
    );
}

pub(super) fn draw_interview_picker(
    frame: &mut Frame,
    picker: &crate::app::InterviewPickerState,
    area: Rect,
) {
    let max_width = (area.width * 3 / 4).max(30).min(area.width);
    let inner_width = max_width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();

    for wl in word_wrap(&picker.question, inner_width) {
        lines.push(Line::from(Span::styled(
            wl,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));

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
        format!("{custom_marker}{}{cursor}", picker.custom_input.text),
        Style::default()
            .bg(custom_bg)
            .fg(Color::White)
            .add_modifier(if picker.custom_mode {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )));

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
