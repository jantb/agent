use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use crate::autocomplete::Autocomplete;
use crate::input::InputState;
use crate::types::{AgentMode, ChatMessage, NodeInfo, PlanItem};

mod messages;
mod pickers;
mod plan;
mod scroll;
mod streaming;
mod tree;

pub use pickers::{InterviewPickerState, ModelPickerState};

pub struct App {
    pub messages: Vec<ChatMessage>,
    pub input: InputState,
    pub streaming: bool,
    pub thinking: bool,
    pub current_streaming_text: String,
    pub current_thinking_text: String,
    pub scroll_offset: u32,
    pub auto_scroll: bool,
    pub viewport_height: u32,
    pub model_name: String,
    pub working_dir: PathBuf,
    pub mcp_connected: Vec<String>,
    pub mcp_failed: Vec<(String, String)>,
    pub resumed_session: Option<String>,
    pub running: bool,
    pub error_message: Option<String>,
    pub tick: u64,
    pub turn_started_at: Option<Instant>,
    pub last_eval_count: Option<u64>,
    pub last_eval_duration_ns: Option<u64>,
    pub last_prompt_eval_count: Option<u64>,
    pub context_used: u64,
    pub context_window_size: Option<u64>,
    pub total_tokens_up: u64,
    pub total_tokens_down: u64,
    pub pending_images: Vec<String>,
    pub message_queue: VecDeque<(String, Vec<String>)>,
    pub autocomplete: Option<Autocomplete>,
    pub available_models: Vec<String>,
    pub model_picker: Option<ModelPickerState>,
    pub interview_picker: Option<InterviewPickerState>,
    pub mode: AgentMode,
    pub flat: bool,
    /// Current plan (replaced atomically by update_plan tool).
    pub plan: Vec<PlanItem>,
    /// Live agent tree: nodes in enter order, with depth-based hierarchy.
    pub tree: Vec<NodeInfo>,
    /// Tool call counter for the currently active subtask node.
    pub subtask_tool_calls: usize,
}

impl App {
    pub fn new(model_name: String, working_dir: PathBuf) -> Self {
        App {
            messages: Vec::new(),
            input: InputState::new(),
            streaming: false,
            thinking: false,
            current_streaming_text: String::new(),
            current_thinking_text: String::new(),
            scroll_offset: 0,
            auto_scroll: true,
            viewport_height: 0,
            model_name,
            working_dir,
            mcp_connected: Vec::new(),
            mcp_failed: Vec::new(),
            resumed_session: None,
            running: true,
            error_message: None,
            tick: 0,
            turn_started_at: None,
            last_eval_count: None,
            last_eval_duration_ns: None,
            last_prompt_eval_count: None,
            context_used: 0,
            context_window_size: None,
            total_tokens_up: 0,
            total_tokens_down: 0,
            pending_images: Vec::new(),
            message_queue: VecDeque::new(),
            autocomplete: None,
            available_models: Vec::new(),
            model_picker: None,
            interview_picker: None,
            mode: AgentMode::default(),
            flat: false,
            plan: Vec::new(),
            tree: Vec::new(),
            subtask_tool_calls: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    pub fn spinner_char(&self) -> char {
        Self::SPINNER[(self.tick / 4 % 10) as usize]
    }

    pub fn help_text() -> &'static str {
        "Commands:
         — /clear, /new: clear conversation
         — /help: show this help
         Keybindings:
         — Enter: send message
         — Shift+Enter: newline
         — Tab: autocomplete /commands
         — Esc: cancel streaming
         — Ctrl+C: quit
         — Ctrl+U: clear input
         — Ctrl+W: delete word
         — Ctrl+A: start of line
         — Ctrl+E: end of line
         — Shift+Tab: cycle mode (plan → thorough → oneshot)
         — /review <scope>: verify the task is solved + check test coverage
         — /simplify <scope>: refactor for clarity, reuse, and idiomatic style
         — /flat: toggle flat mode (single-level, no delegation)
         — Ctrl+V: paste image from clipboard
         — Up/Down: input history
         — Shift+Up/Down: scroll chat
         — PageUp/PageDn: scroll page
         — End: scroll to bottom
         — Mouse wheel: scroll chat"
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::App;

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn tick_increments() {
        let mut app = make_app();
        app.tick();
        app.tick();
        app.tick();
        assert_eq!(app.tick, 3);
    }

    #[test]
    fn spinner_char_cycles() {
        let mut app = make_app();
        assert_eq!(app.spinner_char(), '⠋');
        app.tick = 4;
        assert_eq!(app.spinner_char(), '⠙');
        app.tick = 40;
        assert_eq!(app.spinner_char(), '⠋');
    }

    #[test]
    fn set_error_stores_message() {
        let mut app = make_app();
        app.set_error("oops".into());
        assert_eq!(app.error_message, Some("oops".into()));
    }

    #[test]
    fn help_text_not_empty() {
        assert!(!App::help_text().is_empty());
    }

    #[test]
    fn context_window_size_default_none() {
        let app = make_app();
        assert!(app.context_window_size.is_none());
    }

    #[test]
    fn context_window_size_set() {
        let mut app = make_app();
        app.context_window_size = Some(32768);
        assert_eq!(app.context_window_size, Some(32768));
    }

    #[test]
    fn interview_picker_default_none_and_mode_default() {
        use crate::types::AgentMode;
        let app = make_app();
        assert!(app.interview_picker.is_none());
        assert_eq!(app.mode, AgentMode::default());
    }
}
