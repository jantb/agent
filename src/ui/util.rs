pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
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

pub(super) fn compute_scroll(
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
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_gets_ellipsis() {
        assert_eq!(truncate("hello world", 5), "hello...");
    }

    #[test]
    fn truncate_multibyte_chars() {
        let s = "😀😁😂😃😄";
        assert_eq!(truncate(s, 3), "😀😁😂...");
        let cjk = "你好世界测试";
        assert_eq!(truncate(cjk, 4), "你好世界...");
    }

    #[test]
    fn compute_scroll_auto_scroll_bottom() {
        assert_eq!(compute_scroll(100, 20, true, 0), 80);
    }

    #[test]
    fn compute_scroll_auto_scroll_content_fits() {
        assert_eq!(compute_scroll(10, 20, true, 0), 0);
    }

    #[test]
    fn compute_scroll_manual_at_bottom() {
        assert_eq!(compute_scroll(100, 20, false, 0), 80);
    }

    #[test]
    fn compute_scroll_manual_scrolled_up() {
        assert_eq!(compute_scroll(100, 20, false, 30), 50);
    }

    #[test]
    fn compute_scroll_manual_offset_clamped() {
        assert_eq!(compute_scroll(100, 20, false, 999), 0);
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
}
