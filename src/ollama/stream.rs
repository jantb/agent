pub(super) struct LineParser {
    buf: Vec<u8>,
}

impl LineParser {
    pub(super) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<serde_json::Value> {
        self.buf.extend_from_slice(bytes);
        let last_newline = match self.buf.iter().rposition(|&b| b == b'\n') {
            Some(pos) => pos,
            None => return vec![],
        };
        let complete: Vec<u8> = self.buf.drain(..=last_newline).collect();
        complete
            .split(|&b| b == b'\n')
            .filter(|l| !l.iter().all(|b| b.is_ascii_whitespace()))
            .filter_map(|l| {
                let s = match std::str::from_utf8(l) {
                    Ok(s) => s.trim(),
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping line with invalid UTF-8");
                        return None;
                    }
                };
                if s.is_empty() {
                    return None;
                }
                match serde_json::from_str(s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(line = s, error = %e, "skipping malformed JSON line");
                        None
                    }
                }
            })
            .collect()
    }
}

const ALL_TAGS: &[&str] = &[
    "<|channel>thought",
    "<|channel>text",
    "<think>",
    "<channel|>",
    "</think>",
];

fn tag_open(s: &str) -> Option<FilterState> {
    match s {
        "<|channel>thought" => Some(FilterState::InThinkTag),
        "<|channel>text" => Some(FilterState::InTextTag),
        "<think>" => Some(FilterState::InThinkTag),
        _ => None,
    }
}

fn is_close_tag(s: &str, prior: &PriorState) -> bool {
    match prior {
        PriorState::InThinkTag => s == "<channel|>" || s == "</think>",
        PriorState::InTextTag => s == "<channel|>",
        PriorState::Text => false,
    }
}

#[derive(Clone)]
enum PriorState {
    Text,
    InThinkTag,
    InTextTag,
}

enum FilterState {
    Text,
    InThinkTag,
    InTextTag,
    Pending { buf: String, prior: PriorState },
}

#[derive(Default)]
pub struct FilterOutput {
    pub text: String,
    pub thinking: String,
}

pub struct ThinkTagFilter {
    state: FilterState,
}

impl Default for ThinkTagFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkTagFilter {
    pub fn new() -> Self {
        Self {
            state: FilterState::Text,
        }
    }

    pub fn feed(&mut self, delta: &str) -> FilterOutput {
        let mut out = FilterOutput::default();
        for ch in delta.chars() {
            self.push_char(ch, &mut out);
        }
        out
    }

    pub fn flush(&mut self) -> FilterOutput {
        let mut out = FilterOutput::default();
        if let FilterState::Pending { buf, prior } = &self.state {
            let buf = buf.clone();
            let prior = prior.clone();
            match &prior {
                PriorState::InThinkTag => out.thinking.push_str(&buf),
                _ => out.text.push_str(&buf),
            }
            self.state = match prior {
                PriorState::Text => FilterState::Text,
                PriorState::InThinkTag => FilterState::InThinkTag,
                PriorState::InTextTag => FilterState::InTextTag,
            };
        }
        out
    }

    fn push_char(&mut self, ch: char, out: &mut FilterOutput) {
        match &mut self.state {
            FilterState::Text => {
                if ch == '<' {
                    self.state = FilterState::Pending {
                        buf: String::from('<'),
                        prior: PriorState::Text,
                    };
                } else {
                    out.text.push(ch);
                }
            }
            FilterState::InThinkTag => {
                if ch == '<' {
                    self.state = FilterState::Pending {
                        buf: String::from('<'),
                        prior: PriorState::InThinkTag,
                    };
                } else {
                    out.thinking.push(ch);
                }
            }
            FilterState::InTextTag => {
                if ch == '<' {
                    self.state = FilterState::Pending {
                        buf: String::from('<'),
                        prior: PriorState::InTextTag,
                    };
                } else {
                    out.text.push(ch);
                }
            }
            FilterState::Pending { buf, prior } => {
                buf.push(ch);
                let buf_str = buf.as_str();
                if let Some(new_state) = tag_open(buf_str) {
                    self.state = new_state;
                } else if is_close_tag(buf_str, prior) {
                    self.state = FilterState::Text;
                } else if ALL_TAGS.iter().any(|t| t.starts_with(buf_str)) {
                    // still a valid prefix, stay pending
                } else {
                    // no match — flush buf to appropriate output and return to prior
                    let buf = buf.clone();
                    let prior = prior.clone();
                    match &prior {
                        PriorState::InThinkTag => out.thinking.push_str(&buf),
                        _ => out.text.push_str(&buf),
                    }
                    self.state = match prior {
                        PriorState::Text => FilterState::Text,
                        PriorState::InThinkTag => FilterState::InThinkTag,
                        PriorState::InTextTag => FilterState::InTextTag,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- LineParser tests ---

    #[test]
    fn line_parser_complete_line() {
        let mut p = LineParser::new();
        let vals = p.feed(b"{\"done\":true}\n");
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0]["done"], true);
    }

    #[test]
    fn line_parser_partial_buffered() {
        let mut p = LineParser::new();
        let vals = p.feed(b"{\"done\":");
        assert!(vals.is_empty());
        let vals = p.feed(b"true}\n");
        assert_eq!(vals.len(), 1);
    }

    #[test]
    fn line_parser_multiple_lines() {
        let mut p = LineParser::new();
        let vals = p.feed(b"{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(vals.len(), 2);
    }

    #[test]
    fn line_parser_invalid_json_skipped() {
        let mut p = LineParser::new();
        let vals = p.feed(b"not json\n{\"ok\":1}\n");
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0]["ok"], 1);
    }

    #[test]
    fn line_parser_multibyte_utf8_split_across_feeds() {
        // ü = [0xc3, 0xbc]; full line: {"content":"über"}\n
        let full = b"{\"content\":\"\xc3\xbcber\"}\n";
        // split between the two bytes of ü (after the opening quote)
        let split = full.iter().position(|&b| b == 0xc3).unwrap();
        let mut p = LineParser::new();
        let v1 = p.feed(&full[..split + 1]); // includes 0xc3
        assert!(v1.is_empty());
        let v2 = p.feed(&full[split + 1..]); // 0xbc + rest + \n
        assert_eq!(v2.len(), 1);
        assert_eq!(v2[0]["content"].as_str().unwrap(), "über");
    }

    #[test]
    fn line_parser_invalid_utf8_skipped() {
        let mut p = LineParser::new();
        // invalid UTF-8 bytes followed by newline, then a valid JSON line
        let mut bytes = vec![0xff, 0xfe, b'\n'];
        bytes.extend_from_slice(b"{\"ok\":1}\n");
        let vals = p.feed(&bytes);
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0]["ok"], 1);
    }

    // --- ThinkTagFilter tests ---

    #[test]
    fn filter_strips_channel_think_tag() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>thoughtsome thinking<channel|>real text");
        assert_eq!(out.thinking, "some thinking");
        assert_eq!(out.text, "real text");
    }

    #[test]
    fn filter_handles_split_open_tag() {
        let mut f = ThinkTagFilter::new();
        let out1 = f.feed("<|channel>th");
        assert!(out1.text.is_empty() && out1.thinking.is_empty());
        let out2 = f.feed("oughtsome thought<channel|>");
        assert_eq!(out2.thinking, "some thought");
    }

    #[test]
    fn filter_handles_split_close_tag() {
        let mut f = ThinkTagFilter::new();
        f.feed("<|channel>thoughtthinking<chan");
        let out = f.feed("nel|>after");
        assert!(out.text.contains("after"));
    }

    #[test]
    fn filter_strips_legacy_think_tags() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<think>internal</think>visible");
        assert_eq!(out.thinking, "internal");
        assert_eq!(out.text, "visible");
    }

    #[test]
    fn filter_passes_non_tag_angle_bracket() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("x < y is true");
        assert_eq!(out.text, "x < y is true");
    }

    #[test]
    fn filter_passthrough_no_tags() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("hello world");
        assert_eq!(out.text, "hello world");
        assert_eq!(out.thinking, "");
    }

    #[test]
    fn filter_text_channel_transparent() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>textvisible content<channel|>");
        assert_eq!(out.text, "visible content");
    }

    #[test]
    fn filter_text_channel_with_trailing() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>textvisible<channel|> more text");
        assert_eq!(out.text, "visible more text");
    }

    #[test]
    fn filter_think_then_text_sequence() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>thoughtmy plan<channel|><|channel>textmy response<channel|>");
        assert_eq!(out.thinking, "my plan");
        assert_eq!(out.text, "my response");
    }

    #[test]
    fn filter_flush_pending() {
        let mut f = ThinkTagFilter::new();
        let out1 = f.feed("hello<|ch");
        assert_eq!(out1.text, "hello");
        let out2 = f.flush();
        assert_eq!(out2.text, "<|ch");
    }

    #[test]
    fn filter_flush_returns_thinking_from_pending_think_state() {
        // Pending buffer accumulated while in InThinkTag state should flush as thinking
        let mut f = ThinkTagFilter::new();
        f.feed("<think>partial thought</think>");
        // Now feed content that starts a new pending inside think (after re-entering via another <think>)
        let mut f2 = ThinkTagFilter::new();
        let _ = f2.feed("<think>deep<");
        // <  puts us in Pending{prior: InThinkTag}; flush should emit "<" as thinking
        let out = f2.flush();
        assert_eq!(out.thinking, "<");
    }

    #[test]
    fn filter_flush_returns_thinking_no_close_tag() {
        let mut f = ThinkTagFilter::new();
        let _ = f.feed("<think>my reasoning");
        // state is now InThinkTag (no pending, just mid-think)
        // flush should not panic and thinking already emitted via feed
        // test that a subsequent flush is safe (no content)
        let out = f.flush();
        assert_eq!(out.thinking, "");
        assert_eq!(out.text, "");
    }

    #[test]
    fn filter_think_no_close_tag_content_via_feed() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<think>my reasoning");
        assert_eq!(out.thinking, "my reasoning");
        assert_eq!(out.text, "");
    }

    #[test]
    fn filter_nested_angle_bracket_in_think_block() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<think>if x < 10 then y</think>visible");
        assert_eq!(out.thinking, "if x < 10 then y");
        assert_eq!(out.text, "visible");
    }
}
