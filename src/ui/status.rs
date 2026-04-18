use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;
use crate::types::AgentMode;

pub(super) fn draw_status(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let elapsed_str = format_elapsed(app.elapsed_secs());

    let breadcrumb = if app.has_tree() {
        let active_label = app
            .tree
            .iter()
            .rfind(|n| n.status == crate::types::NodeStatus::Active)
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

    if app.flat {
        right_spans.push(Span::styled("flat", Style::default().fg(Color::Magenta)));
        right_spans.push(Span::raw("  "));
    }

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

pub(super) fn format_elapsed(secs: Option<f64>) -> String {
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

pub(super) fn context_bar(used: u64, total: u64, width: usize) -> String {
    let ratio = (used as f64 / total as f64).clamp(0.0, 1.0);
    let filled = (ratio * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty)
    )
}

pub(super) fn fmt_tokens(n: u64) -> String {
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
}
