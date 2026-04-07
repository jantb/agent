use crate::highlight::highlight_code;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

struct RenderState {
    lines: Vec<Line<'static>>,
    current_spans: Vec<Span<'static>>,
    indent: String,
    bold: bool,
    italic: bool,
    in_code_block: bool,
    in_blockquote: bool,
    in_heading: Option<HeadingLevel>,
    list_depth: u32,
    ordered_counters: Vec<u64>,
    in_link: bool,
    code_block_lang: Option<String>,
    code_block_buffer: String,
}

impl RenderState {
    fn new(indent: &str) -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            indent: indent.to_owned(),
            bold: false,
            italic: false,
            in_code_block: false,
            in_blockquote: false,
            in_heading: None,
            list_depth: 0,
            ordered_counters: Vec::new(),
            in_link: false,
            code_block_lang: None,
            code_block_buffer: String::new(),
        }
    }

    fn current_style(&self) -> Style {
        if self.in_code_block {
            return Style::default().fg(Color::Yellow);
        }
        let mut style = match self.in_heading {
            Some(HeadingLevel::H1) => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Some(HeadingLevel::H2) => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Some(HeadingLevel::H3) => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Some(_) => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            None => Style::default(),
        };
        if self.in_blockquote {
            style = style.fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        }
        if self.in_link {
            style = style.fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
        }
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        style
    }

    fn flush_line(&mut self) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if !self.indent.is_empty() {
            spans.push(Span::raw(self.indent.clone()));
        }
        if self.in_blockquote {
            spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
        }
        spans.append(&mut self.current_spans);
        self.lines.push(Line::from(spans));
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let style = self.current_style();
        let mut parts = text.split('\n');
        if let Some(first) = parts.next() {
            if !first.is_empty() {
                self.current_spans
                    .push(Span::styled(first.to_owned(), style));
            }
            for part in parts {
                self.flush_line();
                if !part.is_empty() {
                    self.current_spans
                        .push(Span::styled(part.to_owned(), style));
                }
            }
        }
    }

    fn blank_line(&mut self) {
        self.lines
            .push(Line::from(vec![Span::raw(self.indent.clone())]));
    }
}

pub fn markdown_to_lines(text: &str, indent: &str) -> Vec<Line<'static>> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(text, opts);
    let mut state = RenderState::new(indent);

    // Table state
    let mut in_table_header = false;
    let mut in_table_row = false;
    let mut table_cells: Vec<String> = Vec::new();
    let mut table_header_done = false;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                state.in_heading = Some(level);
            }
            Event::End(TagEnd::Heading(..)) => {
                state.flush_line();
                state.in_heading = None;
                state.blank_line();
            }

            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !state.current_spans.is_empty() {
                    state.flush_line();
                }
                if state.list_depth == 0 && !state.in_blockquote {
                    state.blank_line();
                }
            }

            Event::Start(Tag::Strong) => state.bold = true,
            Event::End(TagEnd::Strong) => state.bold = false,

            Event::Start(Tag::Emphasis) => state.italic = true,
            Event::End(TagEnd::Emphasis) => state.italic = false,

            Event::Start(Tag::CodeBlock(kind)) => {
                state.in_code_block = true;
                state.code_block_buffer.clear();
                if let CodeBlockKind::Fenced(lang) = kind {
                    let lang = lang.trim().to_owned();
                    if !lang.is_empty() {
                        state.code_block_lang = Some(lang.clone());
                        let label = format!("  {lang}");
                        let mut spans: Vec<Span<'static>> = Vec::new();
                        if !state.indent.is_empty() {
                            spans.push(Span::raw(state.indent.clone()));
                        }
                        spans.push(Span::styled(
                            label,
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ));
                        state.lines.push(Line::from(spans));
                    } else {
                        state.code_block_lang = None;
                    }
                } else {
                    state.code_block_lang = None;
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                let code = std::mem::take(&mut state.code_block_buffer);
                if let Some(ref lang) = state.code_block_lang.take() {
                    if let Some(highlighted) = highlight_code(&code, lang, &state.indent) {
                        state.lines.extend(highlighted);
                    } else {
                        // fallback: yellow
                        state.push_text(&code);
                        if !state.current_spans.is_empty() {
                            state.flush_line();
                        }
                    }
                } else {
                    // unfenced: yellow
                    state.push_text(&code);
                    if !state.current_spans.is_empty() {
                        state.flush_line();
                    }
                }
                state.in_code_block = false;
                state.blank_line();
            }

            Event::Start(Tag::BlockQuote(..)) => {
                state.in_blockquote = true;
            }
            Event::End(TagEnd::BlockQuote(..)) => {
                if !state.current_spans.is_empty() {
                    state.flush_line();
                }
                state.in_blockquote = false;
                state.blank_line();
            }

            Event::Start(Tag::List(start)) => {
                state.list_depth += 1;
                if let Some(n) = start {
                    state.ordered_counters.push(n);
                } else {
                    state.ordered_counters.push(0); // 0 = unordered sentinel
                }
            }
            Event::End(TagEnd::List(ordered)) => {
                state.list_depth = state.list_depth.saturating_sub(1);
                state.ordered_counters.pop();
                if !ordered && state.list_depth == 0 {
                    state.blank_line();
                }
            }

            Event::Start(Tag::Item) => {
                let item_indent = "  ".repeat(state.list_depth.saturating_sub(1) as usize);
                let is_ordered = state
                    .ordered_counters
                    .last()
                    .map(|&c| c > 0)
                    .unwrap_or(false);

                let bullet = if is_ordered {
                    let counter = state.ordered_counters.last_mut().unwrap();
                    let n = *counter;
                    *counter += 1;
                    format!("{item_indent}{n}. ")
                } else {
                    format!("{item_indent}• ")
                };
                state
                    .current_spans
                    .push(Span::styled(bullet, Style::default().fg(Color::Cyan)));
            }
            Event::End(TagEnd::Item) => {
                if !state.current_spans.is_empty() {
                    state.flush_line();
                }
            }

            Event::Start(Tag::Link { .. }) => {
                state.in_link = true;
            }
            Event::End(TagEnd::Link) => {
                state.in_link = false;
            }

            // Table handling
            Event::Start(Tag::Table(..)) => {
                table_cells.clear();
                table_header_done = false;
            }
            Event::End(TagEnd::Table) => {
                if !state.current_spans.is_empty() {
                    state.flush_line();
                }
                state.blank_line();
            }
            Event::Start(Tag::TableHead) => {
                in_table_header = true;
            }
            Event::End(TagEnd::TableHead) => {
                in_table_header = false;
                // Emit header row
                let row = table_cells.join(" | ");
                let mut spans: Vec<Span<'static>> = Vec::new();
                if !state.indent.is_empty() {
                    spans.push(Span::raw(state.indent.clone()));
                }
                spans.push(Span::raw(row));
                state.lines.push(Line::from(spans));
                // Separator
                let sep = table_cells
                    .iter()
                    .map(|c| "-".repeat(c.len()))
                    .collect::<Vec<_>>()
                    .join("-+-");
                let mut spans2: Vec<Span<'static>> = Vec::new();
                if !state.indent.is_empty() {
                    spans2.push(Span::raw(state.indent.clone()));
                }
                spans2.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
                state.lines.push(Line::from(spans2));
                table_cells.clear();
                table_header_done = true;
            }
            Event::Start(Tag::TableRow) => {
                in_table_row = true;
            }
            Event::End(TagEnd::TableRow) => {
                in_table_row = false;
                if table_header_done {
                    let row = table_cells.join(" | ");
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    if !state.indent.is_empty() {
                        spans.push(Span::raw(state.indent.clone()));
                    }
                    spans.push(Span::raw(row));
                    state.lines.push(Line::from(spans));
                    table_cells.clear();
                }
            }
            Event::Start(Tag::TableCell) => {}
            Event::End(TagEnd::TableCell) => {
                // Collect spans into a cell string
                let cell: String = state
                    .current_spans
                    .drain(..)
                    .map(|s| s.content.to_string())
                    .collect();
                table_cells.push(cell);
            }

            // Ignored structural tags
            Event::Start(Tag::HtmlBlock)
            | Event::Start(Tag::MetadataBlock(..))
            | Event::Start(Tag::DefinitionList)
            | Event::Start(Tag::DefinitionListTitle)
            | Event::Start(Tag::DefinitionListDefinition)
            | Event::Start(Tag::FootnoteDefinition(..))
            | Event::End(TagEnd::HtmlBlock)
            | Event::End(TagEnd::MetadataBlock(..))
            | Event::End(TagEnd::DefinitionList)
            | Event::End(TagEnd::DefinitionListTitle)
            | Event::End(TagEnd::DefinitionListDefinition)
            | Event::End(TagEnd::FootnoteDefinition) => {}

            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => {}
            Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough) => {}

            Event::Code(text) => {
                let s = text.to_string();
                state
                    .current_spans
                    .push(Span::styled(s, Style::default().fg(Color::Yellow)));
            }

            Event::Text(text) => {
                let s = text.to_string();
                if state.in_code_block {
                    state.code_block_buffer.push_str(&s);
                } else {
                    state.push_text(&s);
                }
            }

            Event::SoftBreak => {
                state.current_spans.push(Span::raw(" "));
            }
            Event::HardBreak => {
                state.flush_line();
            }

            Event::Rule => {
                let mut spans: Vec<Span<'static>> = Vec::new();
                if !state.indent.is_empty() {
                    spans.push(Span::raw(state.indent.clone()));
                }
                spans.push(Span::styled(
                    "─".repeat(32),
                    Style::default().fg(Color::DarkGray),
                ));
                state.lines.push(Line::from(spans));
            }

            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(_) => {}
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                state.current_spans.push(Span::raw(marker));
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                state.push_text(text.as_ref());
            }
            _ => {}
        }
        let _ = (in_table_header, in_table_row); // suppress unused warnings
    }

    // Streaming fallback: code block not yet closed
    if state.in_code_block && !state.code_block_buffer.is_empty() {
        let buf = state.code_block_buffer.clone();
        state.push_text(&buf);
    }

    if !state.current_spans.is_empty() {
        state.flush_line();
    }

    state.lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn spans_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn all_spans<'a>(lines: &'a [Line<'a>]) -> Vec<&'a Span<'a>> {
        lines.iter().flat_map(|l| l.spans.iter()).collect()
    }

    #[test]
    fn plain_text_single_line() {
        let lines = markdown_to_lines("Hello world", "  ");
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Hello world"), "text: {text}");
    }

    #[test]
    fn plain_text_multi_line() {
        // Two paragraphs
        let lines = markdown_to_lines("Line 1\n\nLine 2", "");
        let texts: Vec<String> = lines.iter().map(|l| spans_text(l)).collect();
        assert!(texts.iter().any(|t| t.contains("Line 1")));
        assert!(texts.iter().any(|t| t.contains("Line 2")));
    }

    #[test]
    fn heading_h1_bold_cyan() {
        let lines = markdown_to_lines("# Title", "");
        let spans = all_spans(&lines);
        let title_span = spans
            .iter()
            .find(|s| s.content.contains("Title"))
            .expect("Title span");
        assert_eq!(title_span.style.fg, Some(Color::Cyan));
        assert!(title_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn heading_h2_bold_green() {
        let lines = markdown_to_lines("## Sub", "");
        let spans = all_spans(&lines);
        let span = spans
            .iter()
            .find(|s| s.content.contains("Sub"))
            .expect("Sub span");
        assert_eq!(span.style.fg, Some(Color::Green));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn bold_text() {
        let lines = markdown_to_lines("Hello **bold** world", "");
        let spans = all_spans(&lines);
        let bold_span = spans
            .iter()
            .find(|s| s.content.contains("bold"))
            .expect("bold span");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
        // "Hello" should NOT be bold
        let hello = spans
            .iter()
            .find(|s| s.content.contains("Hello"))
            .expect("hello span");
        assert!(!hello.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn italic_text() {
        let lines = markdown_to_lines("Hello *italic* world", "");
        let spans = all_spans(&lines);
        let span = spans
            .iter()
            .find(|s| s.content.contains("italic"))
            .expect("italic span");
        assert!(span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_code() {
        let lines = markdown_to_lines("Use `foo()` here", "");
        let spans = all_spans(&lines);
        let code = spans
            .iter()
            .find(|s| s.content.contains("foo()"))
            .expect("code span");
        assert_eq!(code.style.fg, Some(Color::Yellow));
    }

    #[test]
    fn code_block() {
        let lines = markdown_to_lines("```\ncode\n```", "");
        let spans = all_spans(&lines);
        let code = spans
            .iter()
            .find(|s| s.content.contains("code"))
            .expect("code span");
        assert_eq!(code.style.fg, Some(Color::Yellow));
    }

    #[test]
    fn code_block_with_language_gets_rgb_colors() {
        let lines = markdown_to_lines("```rust\nlet x = 1;\n```", "");
        let spans = all_spans(&lines);
        // Should have at least one Rgb-colored span (not all yellow)
        assert!(spans
            .iter()
            .any(|s| matches!(s.style.fg, Some(Color::Rgb(_, _, _)))));
    }

    #[test]
    fn code_block_unknown_lang_falls_back_to_yellow() {
        let lines = markdown_to_lines("```xyzlang\nfoo bar\n```", "");
        let spans = all_spans(&lines);
        let code_span = spans.iter().find(|s| s.content.contains("foo bar"));
        assert!(code_span.is_some());
        assert_eq!(code_span.unwrap().style.fg, Some(Color::Yellow));
    }

    #[test]
    fn blockquote() {
        let lines = markdown_to_lines("> quoted", "");
        let spans = all_spans(&lines);
        let q = spans
            .iter()
            .find(|s| s.content.contains("quoted"))
            .expect("quoted span");
        assert_eq!(q.style.fg, Some(Color::DarkGray));
        assert!(q.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn unordered_list() {
        let lines = markdown_to_lines("- Item 1\n- Item 2", "");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains('•'));
    }

    #[test]
    fn ordered_list() {
        let lines = markdown_to_lines("1. First\n2. Second", "");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("1.") || text.contains("2."), "text: {text}");
    }

    #[test]
    fn horizontal_rule() {
        let lines = markdown_to_lines("---", "");
        let spans = all_spans(&lines);
        assert!(spans
            .iter()
            .any(|s| s.content.contains('─') || s.content.contains('-')));
    }

    #[test]
    fn link_styled() {
        let lines = markdown_to_lines("[text](http://x)", "");
        let spans = all_spans(&lines);
        let link = spans
            .iter()
            .find(|s| s.content.contains("text"))
            .expect("link span");
        assert_eq!(link.style.fg, Some(Color::Blue));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn empty_string() {
        let lines = markdown_to_lines("", "");
        assert!(lines.is_empty());
    }

    #[test]
    fn unclosed_bold() {
        // Should not panic, and "world" may or may not be bold depending on parser
        let lines = markdown_to_lines("Hello **world", "");
        let text: String = all_spans(&lines)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("world"), "text: {text}");
    }

    #[test]
    fn complex_mixed() {
        let doc = "# Heading\n\nSome **bold** and *italic* text.\n\n- item 1\n- item 2\n\n```rust\nlet x = 1;\n```\n\n> A blockquote\n\n[link](http://example.com)\n\n---\n";
        let lines = markdown_to_lines(doc, "  ");
        assert!(lines.len() > 5, "expected many lines, got {}", lines.len());
    }
}
