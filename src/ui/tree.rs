use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;
use crate::types::NodeStatus;

pub(super) const TREE_PANEL_WIDTH: u16 = 44;

/// Render a compact 6-cell context bar: ` [████░░] 42%`
/// Returns a fixed-width string of exactly 13 chars.
fn context_bar(used: u64, total: u64) -> String {
    let pct = if total > 0 {
        ((used as f64 / total as f64) * 100.0).round() as u64
    } else {
        0
    };
    let filled = if total > 0 {
        ((used as f64 / total as f64) * 6.0).round() as usize
    } else {
        0
    }
    .min(6);
    let empty = 6 - filled;
    let bar: String = "█".repeat(filled) + &"░".repeat(empty);
    format!(" [{bar}]{pct:3}%")
}

pub(super) fn draw_tree_panel(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let ctx_total = app.context_window_size.unwrap_or(crate::ollama::NUM_CTX);
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "─ Agent Tree ".to_string() + &"─".repeat(area.width.saturating_sub(14) as usize),
        Style::default().fg(Color::DarkGray),
    )));

    // bar string is always 13 chars: ` [██████] NNN%`
    const BAR_WIDTH: usize = 13;

    for node in &app.tree {
        let indent = " ".repeat(node.depth);
        let (glyph, color) = match node.status {
            NodeStatus::Active => ("●", Color::Cyan),
            NodeStatus::Suspended => ("⊙", Color::Yellow),
            NodeStatus::Done => ("○", Color::DarkGray),
            NodeStatus::Failed => ("✗", Color::Red),
        };
        let connector = if node.depth > 0 { "└─ " } else { "" };
        let prefix_width = indent.len() + connector.len() + 3; // indent + connector + glyph + space
        let bar_str = context_bar(node.context_used, ctx_total);
        // Available chars for label on first line (leave room for bar)
        let first_line_label_avail = (area.width as usize)
            .saturating_sub(prefix_width + BAR_WIDTH)
            .max(1);
        let avail = (area.width as usize).saturating_sub(prefix_width).max(1);
        let label_chars: Vec<char> = node.label.chars().collect();
        let chunks: Vec<String> = if label_chars.is_empty() {
            vec![String::new()]
        } else {
            // First chunk is constrained by bar, subsequent by full avail
            let first: String = label_chars.iter().take(first_line_label_avail).collect();
            let mut v = vec![first];
            let rest: Vec<char> = label_chars
                .into_iter()
                .skip(first_line_label_avail)
                .collect();
            for ch in rest.chunks(avail) {
                v.push(ch.iter().collect());
            }
            v
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
                    Span::styled(bar_str.clone(), Style::default().fg(Color::Indexed(240))),
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

    if app.subtask_tool_calls > 0 {
        lines.push(Line::from(Span::styled(
            format!("  {} tool calls", app.subtask_tool_calls),
            Style::default().fg(Color::Indexed(240)),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::super::draw;
    use super::TREE_PANEL_WIDTH;

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

        let found_header = (1..30u16).any(|row| {
            let left = row_text(&buffer, row, 0, TREE_PANEL_WIDTH);
            left.contains("Agent Tree")
        });
        assert!(found_header, "Tree header not found in left panel columns");

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

        let worker_row =
            (0..30u16).find(|&row| row_text(&buffer, row, 0, TREE_PANEL_WIDTH).contains("worker"));

        let worker_row = worker_row.expect("worker node not found in tree panel");
        let left = row_text(&buffer, worker_row, 0, TREE_PANEL_WIDTH);

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
    fn tree_panel_renders_per_node_context_bar() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = crate::app::App::new("model".into(), std::path::PathBuf::from("."));
        app.context_window_size = Some(1000);
        app.start_assistant_turn();
        app.update_turn_stats(10, 1_000_000_000, 500, 0); // 50% for orchestrator
        app.enter_subtask(1, "w".into());
        app.update_turn_stats(20, 1_000_000_000, 250, 1); // 25% for subtask

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::super::draw(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        let mut found_50 = false;
        let mut found_25 = false;
        for row in 0..30u16 {
            let txt = row_text(&buffer, row, 0, TREE_PANEL_WIDTH);
            if txt.contains("50%") {
                found_50 = true;
            }
            if txt.contains("25%") {
                found_25 = true;
            }
        }
        assert!(found_50, "orchestrator row should show 50% bar");
        assert!(found_25, "subtask row should show 25% bar");
    }

    #[test]
    fn tree_panel_prefix_arithmetic() {
        let area_width = TREE_PANEL_WIDTH as usize;
        for depth in 0..=5usize {
            let indent_len = depth;
            let connector_len = if depth > 0 { 3 } else { 0 };
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
