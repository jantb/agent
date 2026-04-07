pub struct InputState {
    pub text: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
    pub draft: String,
    pub pasted: Option<String>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
            pasted: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pasted.is_none() && self.text.is_empty()
    }

    pub fn insert_paste(&mut self, content: String) {
        if content.contains('\n') {
            self.pasted = Some(content);
            self.text.clear();
            self.cursor_pos = 0;
            self.history_pos = None;
        } else {
            self.text.insert_str(self.cursor_pos, &content);
            self.cursor_pos += content.len();
            self.history_pos = None;
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.text.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        self.history_pos = None;
    }

    pub fn pop_char(&mut self) {
        if self.cursor_pos == 0 {
            if self.text.is_empty() {
                self.pasted = None;
            }
            return;
        }
        let mut pos = self.cursor_pos - 1;
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        self.text.remove(pos);
        self.cursor_pos = pos;
        self.history_pos = None;
    }

    pub fn take(&mut self) -> String {
        let combined = match self.pasted.take() {
            Some(pasted) => {
                let suffix = std::mem::take(&mut self.text);
                if suffix.is_empty() {
                    pasted
                } else {
                    format!("{pasted}\n{suffix}")
                }
            }
            None => std::mem::take(&mut self.text),
        };
        self.cursor_pos = 0;
        self.history_pos = None;
        self.draft.clear();
        if !combined.is_empty() {
            self.history.push(combined.clone());
        }
        combined
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_pos {
            None => {
                self.pasted = None;
                self.draft = self.text.clone();
                let new_pos = self.history.len() - 1;
                self.history_pos = Some(new_pos);
                self.text = self.history[new_pos].clone();
                self.cursor_pos = self.text.len();
            }
            Some(i) if i > 0 => {
                let new_pos = i - 1;
                self.history_pos = Some(new_pos);
                self.text = self.history[new_pos].clone();
                self.cursor_pos = self.text.len();
            }
            _ => {}
        }
    }

    pub fn history_next(&mut self) {
        match self.history_pos {
            Some(i) if i < self.history.len().saturating_sub(1) => {
                let new_pos = i + 1;
                self.history_pos = Some(new_pos);
                self.text = self.history[new_pos].clone();
                self.cursor_pos = self.text.len();
            }
            Some(_) => {
                self.text = std::mem::take(&mut self.draft);
                self.cursor_pos = self.text.len();
                self.history_pos = None;
            }
            None => {}
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let mut pos = self.cursor_pos - 1;
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        self.cursor_pos = pos;
    }

    pub fn move_right(&mut self) {
        if self.cursor_pos >= self.text.len() {
            return;
        }
        let mut pos = self.cursor_pos + 1;
        while pos <= self.text.len() && !self.text.is_char_boundary(pos) {
            pos += 1;
        }
        self.cursor_pos = pos;
    }

    pub fn line_count(&self) -> usize {
        if self.pasted.is_some() {
            1
        } else {
            self.text.split('\n').count().max(1)
        }
    }

    pub fn delete_word(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let before = &self.text[..self.cursor_pos];
        let trimmed = before.trim_end_matches(' ');
        let word_start = trimmed.rfind(' ').map(|i| i + 1).unwrap_or(0);
        self.text.drain(word_start..self.cursor_pos);
        self.cursor_pos = word_start;
        self.history_pos = None;
    }

    pub fn clear_line(&mut self) {
        self.pasted = None;
        self.text.clear();
        self.cursor_pos = 0;
        self.history_pos = None;
    }

    pub fn move_to_start(&mut self) {
        self.cursor_pos = self.text[..self.cursor_pos]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
    }

    pub fn move_to_end(&mut self) {
        self.cursor_pos = self.text[self.cursor_pos..]
            .find('\n')
            .map(|i| self.cursor_pos + i)
            .unwrap_or(self.text.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> InputState {
        InputState::new()
    }

    #[test]
    fn push_input_char_inserts_at_cursor() {
        let mut s = make();
        s.push_char('a');
        s.push_char('b');
        assert_eq!(s.text, "ab");
        assert_eq!(s.cursor_pos, 2);
    }

    #[test]
    fn pop_input_char_removes_before_cursor() {
        let mut s = make();
        s.push_char('a');
        s.push_char('b');
        s.pop_char();
        assert_eq!(s.text, "a");
        assert_eq!(s.cursor_pos, 1);
    }

    #[test]
    fn pop_input_char_at_zero_does_nothing() {
        let mut s = make();
        s.pop_char();
        assert!(s.text.is_empty());
    }

    #[test]
    fn take_input_clears_and_returns() {
        let mut s = make();
        s.push_char('h');
        s.push_char('i');
        let taken = s.take();
        assert_eq!(taken, "hi");
        assert!(s.text.is_empty());
        assert_eq!(s.cursor_pos, 0);
        assert_eq!(s.history.len(), 1);
    }

    #[test]
    fn take_input_empty_not_added_to_history() {
        let mut s = make();
        let taken = s.take();
        assert!(taken.is_empty());
        assert!(s.history.is_empty());
    }

    #[test]
    fn history_prev_saves_draft_and_loads() {
        let mut s = make();
        s.history.push("old msg".into());
        s.text = "current draft".into();
        s.cursor_pos = s.text.len();
        s.history_prev();
        assert_eq!(s.text, "old msg");
        assert_eq!(s.draft, "current draft");
        assert_eq!(s.history_pos, Some(0));
    }

    #[test]
    fn history_next_restores_draft() {
        let mut s = make();
        s.history.push("old msg".into());
        s.text = "draft".into();
        s.cursor_pos = s.text.len();
        s.history_prev();
        s.history_next();
        assert_eq!(s.text, "draft");
        assert_eq!(s.history_pos, None);
    }

    #[test]
    fn history_cycle_multiple_items() {
        let mut s = make();
        s.history.push("first".into());
        s.history.push("second".into());
        s.history_prev();
        assert_eq!(s.text, "second");
        s.history_prev();
        assert_eq!(s.text, "first");
        s.history_next();
        assert_eq!(s.text, "second");
    }

    #[test]
    fn history_prev_empty_history_noop() {
        let mut s = make();
        s.history_prev();
        assert!(s.text.is_empty());
        assert!(s.history_pos.is_none());
    }

    #[test]
    fn move_cursor_left_right() {
        let mut s = make();
        s.push_char('a');
        s.push_char('b');
        assert_eq!(s.cursor_pos, 2);
        s.move_left();
        assert_eq!(s.cursor_pos, 1);
        s.move_right();
        assert_eq!(s.cursor_pos, 2);
    }

    #[test]
    fn move_cursor_left_at_zero_stays() {
        let mut s = make();
        s.move_left();
        assert_eq!(s.cursor_pos, 0);
    }

    #[test]
    fn move_cursor_right_at_end_stays() {
        let mut s = make();
        s.push_char('x');
        s.move_right();
        assert_eq!(s.cursor_pos, 1);
    }

    #[test]
    fn delete_word_removes_last_word() {
        let mut s = make();
        for c in "hello world".chars() {
            s.push_char(c);
        }
        s.delete_word();
        assert_eq!(s.text, "hello ");
    }

    #[test]
    fn delete_word_removes_only_word() {
        let mut s = make();
        for c in "hello".chars() {
            s.push_char(c);
        }
        s.delete_word();
        assert!(s.text.is_empty());
    }

    #[test]
    fn delete_word_at_start_does_nothing() {
        let mut s = make();
        s.delete_word();
        assert!(s.text.is_empty());
    }

    #[test]
    fn push_input_char_breaks_history_cycle() {
        let mut s = make();
        s.history.push("old".into());
        s.history_prev();
        assert!(s.history_pos.is_some());
        s.push_char('x');
        assert!(s.history_pos.is_none());
    }

    #[test]
    fn push_multibyte_char() {
        let mut s = make();
        s.push_char('å');
        assert_eq!(s.text, "å");
        assert_eq!(s.cursor_pos, 2);
    }

    #[test]
    fn pop_multibyte_char() {
        let mut s = make();
        s.push_char('å');
        s.pop_char();
        assert!(s.text.is_empty());
        assert_eq!(s.cursor_pos, 0);
    }

    #[test]
    fn move_cursor_over_multibyte() {
        let mut s = make();
        s.push_char('a');
        s.push_char('å');
        s.push_char('b');
        assert_eq!(s.cursor_pos, 4);
        s.move_left();
        assert_eq!(s.cursor_pos, 3);
        s.move_left();
        assert_eq!(s.cursor_pos, 1);
        s.move_right();
        assert_eq!(s.cursor_pos, 3);
    }

    #[test]
    fn line_count_empty_is_one() {
        let s = make();
        assert_eq!(s.line_count(), 1);
    }

    #[test]
    fn line_count_single_line() {
        let mut s = make();
        for c in "hello".chars() {
            s.push_char(c);
        }
        assert_eq!(s.line_count(), 1);
    }

    #[test]
    fn line_count_with_newlines() {
        let mut s = make();
        for c in "line1\nline2\nline3".chars() {
            s.push_char(c);
        }
        assert_eq!(s.line_count(), 3);
    }

    #[test]
    fn history_next_at_none_is_noop() {
        let mut s = make();
        s.text = "draft".into();
        s.history_next();
        assert_eq!(s.text, "draft");
    }

    #[test]
    fn history_prev_at_oldest_is_noop() {
        let mut s = make();
        s.history.push("only".into());
        s.history_prev();
        assert_eq!(s.text, "only");
        s.history_prev();
        assert_eq!(s.text, "only");
        assert_eq!(s.history_pos, Some(0));
    }

    #[test]
    fn clear_line_empties_text() {
        let mut s = make();
        s.push_char('a');
        s.push_char('b');
        s.clear_line();
        assert!(s.text.is_empty());
        assert_eq!(s.cursor_pos, 0);
    }

    #[test]
    fn clear_line_resets_history_pos() {
        let mut s = make();
        s.history.push("old".into());
        s.history_prev();
        assert!(s.history_pos.is_some());
        s.clear_line();
        assert!(s.history_pos.is_none());
    }

    #[test]
    fn move_to_start_single_line() {
        let mut s = make();
        for c in "hello".chars() {
            s.push_char(c);
        }
        s.move_to_start();
        assert_eq!(s.cursor_pos, 0);
    }

    #[test]
    fn move_to_end_single_line() {
        let mut s = make();
        for c in "hello".chars() {
            s.push_char(c);
        }
        s.move_to_start();
        s.move_to_end();
        assert_eq!(s.cursor_pos, 5);
    }

    #[test]
    fn move_to_start_multiline() {
        let mut s = make();
        for c in "line1\nline2".chars() {
            s.push_char(c);
        }
        // cursor at end of "line2" (pos 11)
        s.move_to_start();
        // should go to start of "line2" (pos 6, after the \n)
        assert_eq!(s.cursor_pos, 6);
    }

    #[test]
    fn move_to_end_multiline() {
        let mut s = make();
        for c in "line1\nline2".chars() {
            s.push_char(c);
        }
        s.cursor_pos = 2; // mid "line1"
        s.move_to_end();
        assert_eq!(s.cursor_pos, 5); // end of "line1" (before \n)
    }

    // --- new paste tests ---

    #[test]
    fn insert_paste_multiline_sets_pasted_clears_text() {
        let mut s = make();
        s.push_char('x');
        s.insert_paste("hello\nworld".to_string());
        assert_eq!(s.pasted.as_deref(), Some("hello\nworld"));
        assert!(s.text.is_empty());
        assert_eq!(s.cursor_pos, 0);
        assert!(s.history_pos.is_none());
    }

    #[test]
    fn insert_paste_single_line_inserts_into_text() {
        let mut s = make();
        s.push_char('a');
        s.push_char('b');
        s.insert_paste("XY".to_string());
        assert_eq!(s.text, "abXY");
        assert_eq!(s.cursor_pos, 4);
        assert!(s.pasted.is_none());
    }

    #[test]
    fn take_combines_pasted_and_suffix() {
        let mut s = make();
        s.insert_paste("line1\nline2".to_string());
        s.push_char('s');
        s.push_char('u');
        s.push_char('f');
        let result = s.take();
        assert_eq!(result, "line1\nline2\nsuf");
        assert!(s.pasted.is_none());
        assert!(s.text.is_empty());
    }

    #[test]
    fn take_pasted_only_no_suffix() {
        let mut s = make();
        s.insert_paste("a\nb".to_string());
        let result = s.take();
        assert_eq!(result, "a\nb");
        assert!(s.pasted.is_none());
    }

    #[test]
    fn is_empty_false_when_pasted() {
        let mut s = make();
        s.insert_paste("a\nb".to_string());
        assert!(!s.is_empty());
    }

    #[test]
    fn pop_char_clears_pasted_when_text_empty() {
        let mut s = make();
        s.insert_paste("a\nb".to_string());
        assert!(s.pasted.is_some());
        s.pop_char(); // cursor_pos == 0, text empty → clear pasted
        assert!(s.pasted.is_none());
    }

    #[test]
    fn clear_line_clears_pasted() {
        let mut s = make();
        s.insert_paste("a\nb".to_string());
        s.clear_line();
        assert!(s.pasted.is_none());
        assert!(s.text.is_empty());
    }

    #[test]
    fn line_count_is_one_when_pasted() {
        let mut s = make();
        s.insert_paste("a\nb\nc".to_string());
        assert_eq!(s.line_count(), 1);
    }
}
