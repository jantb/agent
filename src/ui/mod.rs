mod chat;
mod input;
mod picker;
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

    let (chat_area, tree_area_opt) = if app.has_tree() && body_area.width > TREE_PANEL_WIDTH + 20 {
        let [tree, chat] =
            Layout::horizontal([Constraint::Length(TREE_PANEL_WIDTH), Constraint::Min(20)])
                .areas(body_area);
        (chat, Some(tree))
    } else {
        (body_area, None)
    };

    title::draw_title(frame, app, title_area);
    chat::draw_chat(frame, app, chat_area);
    if let Some(tree_area) = tree_area_opt {
        tree::draw_tree_panel(frame, app, tree_area);
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

    #[test]
    #[ignore = "debug helper: prints buffer contents, not a real assertion"]
    fn diagnose_subtask_rendering() {
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.start_assistant_turn();

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

        let exit_rows: Vec<u16> = (0..40u16)
            .filter(|&r| row_text(&buffer, r, chat_start, chat_end).contains('◀'))
            .collect();
        println!("SubtaskExit rows: {:?}", exit_rows);

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
}
