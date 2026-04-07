use std::path::PathBuf;
use std::time::Instant;

use crate::autocomplete::Autocomplete;
use crate::input::InputState;
use crate::types::{ChatMessage, MessageKind, Role, ToolCall, ToolResult};

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
    pub last_prompt_eval_count: Option<u64>,
    pub context_window_size: Option<u64>,
    pub total_tokens_up: u64,
    pub total_tokens_down: u64,
    pub pending_images: Vec<String>, // base64-encoded images to attach to next message
    pub autocomplete: Option<Autocomplete>,
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
            last_prompt_eval_count: None,
            context_window_size: None,
            total_tokens_up: 0,
            total_tokens_down: 0,
            pending_images: Vec::new(),
            autocomplete: None,
        }
    }

    pub fn add_user_message(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: Role::User,
            content: text,
            kind: MessageKind::Text,
        });
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    pub fn start_assistant_turn(&mut self) {
        self.streaming = true;
        self.current_streaming_text.clear();
        self.turn_started_at = Some(Instant::now());
        self.error_message = None;
    }

    pub fn append_streaming_text(&mut self, delta: &str) {
        self.current_streaming_text.push_str(delta);
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    pub fn append_thinking_text(&mut self, delta: &str) {
        self.current_thinking_text.push_str(delta);
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    pub fn set_thinking(&mut self, thinking: bool) {
        self.thinking = thinking;
    }

    pub fn finish_assistant_turn(&mut self) {
        let think = std::mem::take(&mut self.current_thinking_text);
        if !think.is_empty() {
            self.messages.push(ChatMessage {
                role: Role::Assistant,
                content: think,
                kind: MessageKind::Thinking,
            });
        }
        let text = std::mem::take(&mut self.current_streaming_text);
        if !text.is_empty() {
            self.messages.push(ChatMessage {
                role: Role::Assistant,
                content: text,
                kind: MessageKind::Text,
            });
        }
        self.streaming = false;
        self.thinking = false;
        self.turn_started_at = None;
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn update_turn_stats(&mut self, eval_count: u64, prompt_eval_count: u64) {
        self.last_eval_count = Some(eval_count);
        self.last_prompt_eval_count = Some(prompt_eval_count);
        self.total_tokens_down += eval_count;
        self.total_tokens_up += prompt_eval_count;
    }

    pub fn elapsed_secs(&self) -> Option<f64> {
        self.turn_started_at.map(|t| t.elapsed().as_secs_f64())
    }

    const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    pub fn spinner_char(&self) -> char {
        Self::SPINNER[(self.tick / 4 % 10) as usize]
    }

    pub fn add_tool_call(&mut self, call: &ToolCall) {
        let args_summary =
            serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());
        self.messages.push(ChatMessage {
            role: Role::Assistant,
            content: format!("{}({})", call.name, args_summary),
            kind: MessageKind::ToolCall {
                call_id: call.id.clone(),
                name: call.name.clone(),
                arguments: args_summary,
            },
        });
    }

    pub fn add_tool_result(&mut self, result: &ToolResult) {
        // Find the tool name by matching call_id
        let name = self
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                if let MessageKind::ToolCall { call_id, name, .. } = &m.kind {
                    if call_id == &result.call_id {
                        Some(name.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "tool".to_string());
        self.messages.push(ChatMessage {
            role: Role::Tool,
            content: result.output.clone(),
            kind: MessageKind::ToolResult {
                name,
                is_error: result.is_error,
            },
        });
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
        self.auto_scroll = false;
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
            if self.scroll_offset == 0 {
                self.auto_scroll = true;
            }
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.scroll_offset = 0;
    }

    pub fn scroll_page_up(&mut self) {
        let half = (self.viewport_height / 2).max(1);
        self.scroll_offset = self.scroll_offset.saturating_add(half);
        self.auto_scroll = false;
    }

    pub fn scroll_page_down(&mut self) {
        let half = (self.viewport_height / 2).max(1);
        self.scroll_offset = self.scroll_offset.saturating_sub(half);
        if self.scroll_offset == 0 {
            self.auto_scroll = true;
        }
    }

    pub fn tok_per_sec(&self) -> Option<f64> {
        let eval = self.last_eval_count?;
        let elapsed = self.elapsed_secs()?;
        if elapsed > 0.0 && eval > 0 {
            Some(eval as f64 / elapsed)
        } else {
            None
        }
    }

    pub fn add_pending_image(&mut self, base64_data: String) {
        self.pending_images.push(base64_data);
    }

    pub fn take_pending_images(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_images)
    }

    pub fn pending_image_count(&self) -> usize {
        self.pending_images.len()
    }

    pub fn help_text() -> &'static str {
        "Commands:\n  /clear, /new  \u{2014} clear conversation\n  /help         \u{2014} show this help\n\nKeybindings:\n  Enter          \u{2014} send message\n  Shift+Enter    \u{2014} newline\n  Tab            \u{2014} autocomplete /commands\n  Esc            \u{2014} cancel streaming\n  Ctrl+C         \u{2014} quit\n  Ctrl+U         \u{2014} clear input\n  Ctrl+W         \u{2014} delete word\n  Ctrl+A         \u{2014} start of line\n  Ctrl+E         \u{2014} end of line\n  Ctrl+V         \u{2014} paste image from clipboard\n  Up/Down        \u{2014} input history\n  Shift+Up/Down  \u{2014} scroll chat\n  PageUp/PageDn  \u{2014} scroll page\n  End            \u{2014} scroll to bottom\n  Mouse wheel    \u{2014} scroll chat"
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.error_message = None;
        self.last_eval_count = None;
        self.last_prompt_eval_count = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

    // --- user message ---
    #[test]
    fn add_user_message_appends() {
        let mut app = make_app();
        app.add_user_message("hello".into());
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "hello");
        assert!(matches!(app.messages[0].role, Role::User));
    }

    // --- streaming ---
    #[test]
    fn start_assistant_turn_sets_streaming() {
        let mut app = make_app();
        app.start_assistant_turn();
        assert!(app.streaming);
        assert!(app.current_streaming_text.is_empty());
    }

    #[test]
    fn append_streaming_text_accumulates() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.append_streaming_text("foo");
        app.append_streaming_text("bar");
        assert_eq!(app.current_streaming_text, "foobar");
    }

    #[test]
    fn finish_assistant_turn_moves_text_to_messages() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.append_streaming_text("response");
        app.finish_assistant_turn();
        assert!(!app.streaming);
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "response");
        assert!(app.current_streaming_text.is_empty());
    }

    #[test]
    fn finish_assistant_turn_empty_no_message() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.finish_assistant_turn();
        assert!(app.messages.is_empty());
    }

    #[test]
    fn set_thinking_toggles() {
        let mut app = make_app();
        app.set_thinking(true);
        assert!(app.thinking);
        app.set_thinking(false);
        assert!(!app.thinking);
    }

    // --- tool messages ---
    #[test]
    fn add_tool_call_adds_message() {
        let mut app = make_app();
        let call = ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "foo.rs"}),
        };
        app.add_tool_call(&call);
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(app.messages[0].kind, MessageKind::ToolCall { .. }));
    }

    #[test]
    fn add_tool_result_adds_message() {
        let mut app = make_app();
        let call = ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({}),
        };
        app.add_tool_call(&call);
        let result = ToolResult {
            call_id: "c1".into(),
            output: "content".into(),
            is_error: false,
        };
        app.add_tool_result(&result);
        assert_eq!(app.messages.len(), 2);
        assert!(matches!(
            app.messages[1].kind,
            MessageKind::ToolResult {
                is_error: false,
                ..
            }
        ));
    }

    // --- scroll ---
    #[test]
    fn scroll_up_increments_disables_auto_scroll() {
        let mut app = make_app();
        app.scroll_up();
        assert_eq!(app.scroll_offset, 1);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn scroll_down_decrements() {
        let mut app = make_app();
        app.scroll_offset = 5;
        app.scroll_down();
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn scroll_down_at_zero_stays_zero() {
        let mut app = make_app();
        app.scroll_down();
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn scroll_to_bottom_enables_auto_scroll() {
        let mut app = make_app();
        app.scroll_up();
        app.scroll_to_bottom();
        assert!(app.auto_scroll);
        assert_eq!(app.scroll_offset, 0);
    }

    // --- error ---
    #[test]
    fn set_error_stores_message() {
        let mut app = make_app();
        app.set_error("oops".into());
        assert_eq!(app.error_message, Some("oops".into()));
    }

    // --- tick / spinner ---
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
        assert_eq!(app.spinner_char(), '⠋'); // tick=0
        app.tick = 4;
        assert_eq!(app.spinner_char(), '⠙'); // tick=4 → idx 1
        app.tick = 40;
        assert_eq!(app.spinner_char(), '⠋'); // wraps back
    }

    // --- turn timer ---
    #[test]
    fn start_assistant_turn_records_instant() {
        let mut app = make_app();
        app.start_assistant_turn();
        assert!(app.turn_started_at.is_some());
    }

    #[test]
    fn finish_assistant_turn_clears_instant() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.append_streaming_text("x");
        app.finish_assistant_turn();
        assert!(app.turn_started_at.is_none());
    }

    #[test]
    fn elapsed_secs_none_when_idle() {
        let app = make_app();
        assert!(app.elapsed_secs().is_none());
    }

    #[test]
    fn elapsed_secs_some_when_streaming() {
        let mut app = make_app();
        app.start_assistant_turn();
        assert!(app.elapsed_secs().is_some());
    }

    // --- turn stats ---
    #[test]
    fn update_turn_stats_stores_values() {
        let mut app = make_app();
        app.update_turn_stats(100, 200);
        assert_eq!(app.last_eval_count, Some(100));
        assert_eq!(app.last_prompt_eval_count, Some(200));
    }

    // --- page scrolling ---
    #[test]
    fn viewport_height_default_zero() {
        let app = make_app();
        assert_eq!(app.viewport_height, 0);
    }

    #[test]
    fn scroll_page_up_moves_half_viewport() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn scroll_page_up_disables_auto_scroll() {
        let mut app = make_app();
        app.viewport_height = 20;
        assert!(app.auto_scroll);
        app.scroll_page_up();
        assert!(!app.auto_scroll);
    }

    #[test]
    fn scroll_page_down_moves_half_viewport() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = 20;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn scroll_page_down_to_zero_enables_auto_scroll() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = 5;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.auto_scroll);
    }

    #[test]
    fn scroll_page_up_saturates_at_u32_max() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = u32::MAX - 5;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, u32::MAX);
    }

    #[test]
    fn scroll_page_down_clamps_to_zero() {
        let mut app = make_app();
        app.viewport_height = 40;
        app.scroll_offset = 3;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.auto_scroll);
    }

    #[test]
    fn scroll_page_up_zero_viewport_moves_by_one() {
        let mut app = make_app();
        app.viewport_height = 0;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn scroll_page_down_zero_viewport_moves_by_one() {
        let mut app = make_app();
        app.viewport_height = 0;
        app.scroll_offset = 5;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn scroll_page_up_odd_viewport() {
        let mut app = make_app();
        app.viewport_height = 21;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 10); // 21/2 = 10 (integer division)
    }

    #[test]
    fn scroll_page_up_viewport_one() {
        let mut app = make_app();
        app.viewport_height = 1;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 1); // max(1/2, 1) = max(0, 1) = 1
    }

    #[test]
    fn multiple_page_ups_accumulate() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_page_up();
        app.scroll_page_up();
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 30);
    }

    #[test]
    fn page_up_then_page_down_returns_to_original() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = 10;
        app.auto_scroll = false;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 20);
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 10);
    }

    // --- clear_messages ---
    #[test]
    fn clear_messages_empties_all() {
        let mut app = make_app();
        app.add_user_message("hello".into());
        app.set_error("oops".into());
        app.update_turn_stats(10, 20);
        app.clear_messages();
        assert!(app.messages.is_empty());
        assert!(app.error_message.is_none());
        assert!(app.last_eval_count.is_none());
        assert!(app.last_prompt_eval_count.is_none());
    }

    #[test]
    fn clear_messages_idempotent_on_empty() {
        let mut app = make_app();
        app.clear_messages();
        assert!(app.messages.is_empty());
    }

    #[test]
    fn scroll_down_to_zero_enables_auto_scroll() {
        let mut app = make_app();
        app.scroll_offset = 1;
        app.auto_scroll = false;
        app.scroll_down();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.auto_scroll);
    }

    #[test]
    fn tok_per_sec_none_when_idle() {
        let app = make_app();
        assert!(app.tok_per_sec().is_none());
    }

    #[test]
    fn tok_per_sec_some_when_streaming() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.update_turn_stats(100, 50);
        // Can't easily test exact value since elapsed depends on time
        // Just verify it returns Some
        assert!(app.tok_per_sec().is_some());
    }

    #[test]
    fn pending_images_lifecycle() {
        let mut app = make_app();
        assert_eq!(app.pending_image_count(), 0);
        app.add_pending_image("base64data".into());
        assert_eq!(app.pending_image_count(), 1);
        let images = app.take_pending_images();
        assert_eq!(images.len(), 1);
        assert_eq!(app.pending_image_count(), 0);
    }

    #[test]
    fn help_text_not_empty() {
        assert!(!App::help_text().is_empty());
    }

    // --- context window size ---
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

    // --- add_tool_result without prior call ---
    #[test]
    fn add_tool_result_without_call_defaults_to_tool() {
        let mut app = make_app();
        let result = ToolResult {
            call_id: "c1".into(),
            output: "result".into(),
            is_error: false,
        };
        app.add_tool_result(&result);
        if let MessageKind::ToolResult { name, .. } = &app.messages[0].kind {
            assert_eq!(name, "tool");
        } else {
            panic!("expected ToolResult");
        }
    }

    // --- multiple streaming turns ---
    #[test]
    fn multiple_turns_accumulate_messages() {
        let mut app = make_app();
        app.add_user_message("q1".into());
        app.start_assistant_turn();
        app.append_streaming_text("a1");
        app.finish_assistant_turn();
        app.add_user_message("q2".into());
        app.start_assistant_turn();
        app.append_streaming_text("a2");
        app.finish_assistant_turn();
        assert_eq!(app.messages.len(), 4);
        assert_eq!(app.messages[0].content, "q1");
        assert_eq!(app.messages[1].content, "a1");
        assert_eq!(app.messages[2].content, "q2");
        assert_eq!(app.messages[3].content, "a2");
    }

    #[test]
    fn update_turn_stats_accumulates_across_turns() {
        let mut app = make_app();
        app.update_turn_stats(100, 200);
        app.update_turn_stats(50, 75);
        assert_eq!(app.total_tokens_down, 150);
        assert_eq!(app.total_tokens_up, 275);
    }

    #[test]
    fn clear_messages_does_not_reset_cumulative_tokens() {
        let mut app = make_app();
        app.update_turn_stats(100, 200);
        app.clear_messages();
        assert_eq!(app.total_tokens_down, 100);
        assert_eq!(app.total_tokens_up, 200);
    }
}
