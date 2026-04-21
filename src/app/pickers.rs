pub struct ModelPickerState {
    pub models: Vec<String>,
    pub selected: usize,
    pub filter: String,
    pub filtered: Vec<usize>,
}

impl ModelPickerState {
    pub fn new(models: Vec<String>, initial_selected: usize) -> Self {
        let filtered: Vec<usize> = (0..models.len()).collect();
        Self {
            models,
            selected: initial_selected.min(filtered.len().saturating_sub(1)),
            filter: String::new(),
            filtered,
        }
    }

    pub fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .models
            .iter()
            .enumerate()
            .filter(|(_, m)| needle.is_empty() || m.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
        self.refilter();
    }

    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.selected = 0;
        self.refilter();
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }
    pub fn selected(&self) -> Option<&str> {
        let &idx = self.filtered.get(self.selected)?;
        self.models.get(idx).map(String::as_str)
    }
}

use crate::input::InputState;

pub struct InterviewPickerState {
    pub question: String,
    pub suggestions: Vec<String>,
    pub selected: usize,
    pub custom_input: InputState,
    pub custom_mode: bool,
    pub answer_tx: Option<tokio::sync::oneshot::Sender<String>>,
}

impl InterviewPickerState {
    pub fn move_up(&mut self) {
        if !self.custom_mode {
            self.selected = self.selected.saturating_sub(1);
        }
    }
    pub fn move_down(&mut self) {
        if !self.custom_mode && self.selected + 1 < self.suggestions.len() {
            self.selected += 1;
        }
    }
    pub fn submit(&mut self) -> Option<String> {
        let custom_text = self.custom_input.text.trim().to_string();
        let answer = if self.custom_mode && !custom_text.is_empty() {
            custom_text
        } else {
            self.suggestions.get(self.selected)?.to_string()
        };
        if let Some(tx) = self.answer_tx.take() {
            let _ = tx.send(answer.clone());
        }
        Some(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interview_picker_move_up_down() {
        let mut picker = InterviewPickerState {
            question: "test?".into(),
            suggestions: vec!["a".into(), "b".into(), "c".into()],
            selected: 0,
            custom_input: InputState::new(),
            custom_mode: false,
            answer_tx: None,
        };
        picker.move_down();
        assert_eq!(picker.selected, 1);
        picker.move_down();
        assert_eq!(picker.selected, 2);
        picker.move_down();
        assert_eq!(picker.selected, 2);
        picker.move_up();
        assert_eq!(picker.selected, 1);
        picker.move_up();
        assert_eq!(picker.selected, 0);
        picker.move_up();
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn interview_picker_custom_mode_ignores_nav() {
        let mut picker = InterviewPickerState {
            question: "test?".into(),
            suggestions: vec!["a".into(), "b".into()],
            selected: 0,
            custom_input: InputState::new(),
            custom_mode: true,
            answer_tx: None,
        };
        picker.move_down();
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn interview_picker_submit_suggestion() {
        let mut picker = InterviewPickerState {
            question: "test?".into(),
            suggestions: vec!["alpha".into(), "beta".into()],
            selected: 1,
            custom_input: InputState::new(),
            custom_mode: false,
            answer_tx: None,
        };
        let answer = picker.submit();
        assert_eq!(answer, Some("beta".into()));
    }

    #[test]
    fn interview_picker_submit_custom() {
        let mut picker = InterviewPickerState {
            question: "test?".into(),
            suggestions: vec!["a".into()],
            selected: 0,
            custom_input: {
                let mut s = InputState::new();
                for c in "my answer".chars() {
                    s.push_char(c);
                }
                s
            },
            custom_mode: true,
            answer_tx: None,
        };
        let answer = picker.submit();
        assert_eq!(answer, Some("my answer".into()));
    }

    #[test]
    fn interview_picker_submit_empty_custom_falls_back() {
        let mut picker = InterviewPickerState {
            question: "test?".into(),
            suggestions: vec!["fallback".into()],
            selected: 0,
            custom_input: InputState::new(),
            custom_mode: true,
            answer_tx: None,
        };
        let answer = picker.submit();
        assert_eq!(answer, Some("fallback".into()));
    }

    #[test]
    fn interview_picker_submit_sends_via_oneshot() {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let mut picker = InterviewPickerState {
            question: "test?".into(),
            suggestions: vec!["yes".into()],
            selected: 0,
            custom_input: InputState::new(),
            custom_mode: false,
            answer_tx: Some(tx),
        };
        let answer = picker.submit();
        assert_eq!(answer, Some("yes".into()));
        assert!(picker.answer_tx.is_none());
        let received = rx.blocking_recv().unwrap();
        assert_eq!(received, "yes");
    }
}
