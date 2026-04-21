mod chat;
mod input;
mod picker;
mod plan;
mod status;
mod title;
mod tree;
mod util;

use ratatui::{
    layout::{Constraint, Layout},
    Frame,
};

use crate::app::App;

use tree::TREE_PANEL_WIDTH;

pub fn draw(frame: &mut Frame, app: &App) {
    let img_count = app.pending_image_count();
    let queue_tag_len = if let Some(next) = app.message_queue.front() {
        let first_line = next.0.lines().next().unwrap_or("");
        let preview = util::truncate(first_line, 30);
        if app.queue_len() > 1 {
            format!("[queued +{}: {preview}] ", app.queue_len() - 1)
                .chars()
                .count()
        } else {
            format!("[queued: {preview}] ").chars().count()
        }
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
        let tag = if p.contains('\n') {
            format!("[{} lines pasted] ", p.lines().count())
        } else {
            format!("[{} chars pasted] ", p.chars().count())
        };
        tag.chars().count()
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

    let show_side =
        (app.has_tree() || !app.plan.is_empty()) && body_area.width > TREE_PANEL_WIDTH + 20;
    let (chat_area, side_area_opt) = if show_side {
        let [side, chat] =
            Layout::horizontal([Constraint::Length(TREE_PANEL_WIDTH), Constraint::Min(20)])
                .areas(body_area);
        (chat, Some(side))
    } else {
        (body_area, None)
    };

    title::draw_title(frame, app, title_area);
    chat::draw_chat(frame, app, chat_area);
    if let Some(side_area) = side_area_opt {
        if app.has_tree() && !app.plan.is_empty() {
            let plan_h = (app.plan.len() as u16 + 1).min(side_area.height / 2).max(3);
            let [tree_area, plan_area] =
                Layout::vertical([Constraint::Min(3), Constraint::Length(plan_h)]).areas(side_area);
            tree::draw_tree_panel(frame, app, tree_area);
            plan::draw_plan_panel(frame, app, plan_area);
        } else if app.has_tree() {
            tree::draw_tree_panel(frame, app, side_area);
        } else {
            plan::draw_plan_panel(frame, app, side_area);
        }
    }
    input::draw_input(frame, app, input_area, first_prefix_len);
    status::draw_status(frame, app, status_area);

    if let Some(ac) = &app.autocomplete {
        picker::draw_autocomplete(frame, ac, input_area);
    }
    if let Some(picker) = &app.model_picker {
        picker::draw_model_picker(frame, picker, frame.area());
    }
    if let Some(picker) = &app.interview_picker {
        picker::draw_interview_picker(frame, picker, frame.area());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        text::Line,
        widgets::{Paragraph, Wrap},
    };

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
    fn line_count_accounts_for_wrapping() {
        let long_line = "a".repeat(80);
        let paragraph = Paragraph::new(vec![Line::from(long_line)]).wrap(Wrap { trim: false });
        let visual = paragraph.line_count(20);
        assert_eq!(visual, 4);
    }

    #[test]
    fn scroll_with_wrapped_lines_reaches_bottom() {
        let lines: Vec<Line> = (0..5).map(|_| Line::from("a".repeat(40))).collect();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let total_height = paragraph.line_count(20) as u32;
        assert_eq!(total_height, 10);
        assert_eq!(util::compute_scroll(total_height, 6, true, 0), 4);
    }

    #[test]
    fn thinking_renders_after_subtask_output() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();
        app.enter_subtask(1, "worker".into());
        app.exit_subtask(1);
        app.set_thinking(true);
        app.append_thinking_text("considering next step");

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

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
        use super::util::truncate;

        let area_width = 60usize;
        let prefix_w = 4;
        let label = "x".repeat(100);

        let buggy_trunc = truncate(&label, area_width.saturating_sub(prefix_w + 1));
        let buggy_used = prefix_w + buggy_trunc.chars().count() + 1;
        assert!(
            buggy_used > area_width,
            "Bug should cause overflow: used={buggy_used} should > {area_width}"
        );

        let label_budget = area_width.saturating_sub(prefix_w + 1);
        let fixed_trunc = truncate(&label, label_budget.saturating_sub(3));
        let fixed_used = prefix_w + fixed_trunc.chars().count() + 1;
        assert!(
            fixed_used <= area_width,
            "Fixed: used={fixed_used} > area_width={area_width}"
        );
    }
}
