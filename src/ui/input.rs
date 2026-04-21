use ratatui::{
    layout::Position,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;

pub(super) fn draw_input(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    first_line_prefix_len: u16,
) {
    let img_count = app.pending_image_count();
    let queue_tag = if let Some(next) = app.message_queue.front() {
        let first_line = next.0.lines().next().unwrap_or("");
        let preview = super::util::truncate(first_line, 30);
        if app.queue_len() > 1 {
            format!("[queued +{}: {preview}] ", app.queue_len() - 1)
        } else {
            format!("[queued: {preview}] ")
        }
    } else {
        String::new()
    };
    let base_prefix = if img_count > 0 {
        format!("❯ {queue_tag}[{img_count} img] ")
    } else {
        format!("❯ {queue_tag}")
    };
    let paste_tag = if let Some(pasted) = &app.input.pasted {
        if pasted.contains('\n') {
            format!("[{} lines pasted] ", pasted.lines().count())
        } else {
            format!("[{} chars pasted] ", pasted.chars().count())
        }
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
