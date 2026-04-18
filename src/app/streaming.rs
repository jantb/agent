use std::time::Instant;

use crate::types::{ChatMessage, MessageKind, NodeInfo, NodeStatus, Role};

use super::App;

impl App {
    pub fn start_assistant_turn(&mut self) {
        self.streaming = true;
        self.current_streaming_text.clear();
        self.turn_started_at = Some(Instant::now());
        self.error_message = None;
        self.tree.clear();
        self.subtask_tool_calls = 0;
        self.tree.push(NodeInfo {
            depth: 0,
            label: "orchestrator".into(),
            status: NodeStatus::Active,
            context_used: 0,
        });
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

    pub fn flush_thinking(&mut self) {
        let think = std::mem::take(&mut self.current_thinking_text);
        if !think.is_empty() {
            self.messages.push(ChatMessage {
                role: Role::Assistant,
                content: think,
                kind: MessageKind::Thinking,
            });
        }
        self.thinking = false;
    }

    pub fn finish_assistant_turn(&mut self) {
        self.flush_thinking();
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
        self.tree.clear();
        self.subtask_tool_calls = 0;
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    pub fn update_turn_stats(
        &mut self,
        eval_count: u64,
        eval_duration_ns: u64,
        prompt_eval_count: u64,
        depth: usize,
    ) {
        self.last_eval_count = Some(eval_count);
        self.last_eval_duration_ns = Some(eval_duration_ns);
        if depth == 0 {
            self.last_prompt_eval_count = Some(prompt_eval_count);
            self.context_used = prompt_eval_count;
        }
        self.total_tokens_down += eval_count;
        self.total_tokens_up += prompt_eval_count;
        // Update per-node context bar: find last node at this depth.
        if let Some(node) = self.tree.iter_mut().rfind(|n| n.depth == depth) {
            node.context_used = prompt_eval_count;
        }
    }

    pub fn elapsed_secs(&self) -> Option<f64> {
        self.turn_started_at.map(|t| t.elapsed().as_secs_f64())
    }

    pub fn tok_per_sec(&self) -> Option<f64> {
        let eval = self.last_eval_count?;
        let dur_ns = self.last_eval_duration_ns?;
        if dur_ns > 0 && eval > 0 {
            Some(eval as f64 / (dur_ns as f64 / 1_000_000_000.0))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::types::MessageKind;

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

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

    #[test]
    fn update_turn_stats_stores_values() {
        let mut app = make_app();
        app.update_turn_stats(100, 2_000_000_000, 200, 0);
        assert_eq!(app.last_eval_count, Some(100));
        assert_eq!(app.last_eval_duration_ns, Some(2_000_000_000));
        assert_eq!(app.last_prompt_eval_count, Some(200));
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
        app.update_turn_stats(100, 2_000_000_000, 50, 0);
        assert!(app.tok_per_sec().is_some());
    }

    #[test]
    fn tok_per_sec_uses_ollama_duration() {
        let mut app = make_app();
        app.update_turn_stats(100, 2_000_000_000, 50, 0);
        let rate = app.tok_per_sec().unwrap();
        assert!((rate - 50.0).abs() < 0.01, "expected ~50, got {rate}");
    }

    #[test]
    fn tok_per_sec_stable_over_time() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.update_turn_stats(1000, 1_000_000_000, 50, 0);
        let rate1 = app.tok_per_sec().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let rate2 = app.tok_per_sec().unwrap();
        assert!(
            (rate1 - rate2).abs() < 0.01,
            "tok/s drifted: {rate1:.0} -> {rate2:.0}"
        );
    }

    #[test]
    fn tok_per_sec_none_without_duration() {
        let mut app = make_app();
        app.update_turn_stats(100, 0, 50, 0);
        assert!(app.tok_per_sec().is_none());
    }

    #[test]
    fn tok_per_sec_none_before_stats() {
        let mut app = make_app();
        app.start_assistant_turn();
        assert!(app.tok_per_sec().is_none());
    }

    #[test]
    fn update_turn_stats_accumulates_across_turns() {
        let mut app = make_app();
        app.update_turn_stats(100, 1_000_000_000, 200, 0);
        app.update_turn_stats(50, 500_000_000, 75, 0);
        assert_eq!(app.total_tokens_down, 150);
        assert_eq!(app.total_tokens_up, 275);
    }

    #[test]
    fn clear_messages_does_not_reset_cumulative_tokens() {
        let mut app = make_app();
        app.update_turn_stats(100, 1_000_000_000, 200, 0);
        app.clear_messages();
        assert_eq!(app.total_tokens_down, 100);
        assert_eq!(app.total_tokens_up, 200);
    }

    #[test]
    fn flush_thinking_moves_to_messages() {
        let mut app = make_app();
        app.current_thinking_text = "deep thought".into();
        app.thinking = true;
        app.flush_thinking();
        assert!(!app.thinking);
        assert!(app.current_thinking_text.is_empty());
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "deep thought");
        assert!(matches!(app.messages[0].kind, MessageKind::Thinking));
    }

    #[test]
    fn flush_thinking_empty_noop() {
        let mut app = make_app();
        app.flush_thinking();
        assert!(app.messages.is_empty());
    }

    #[test]
    fn finish_assistant_turn_flushes_thinking_and_text() {
        let mut app = make_app();
        app.current_thinking_text = "reason".into();
        app.current_streaming_text = "response".into();
        app.streaming = true;
        app.finish_assistant_turn();
        assert_eq!(app.messages.len(), 2);
        assert!(matches!(app.messages[0].kind, MessageKind::Thinking));
        assert_eq!(app.messages[0].content, "reason");
        assert!(matches!(app.messages[1].kind, MessageKind::Text));
        assert_eq!(app.messages[1].content, "response");
        assert!(!app.streaming);
        assert!(app.current_thinking_text.is_empty());
        assert!(app.current_streaming_text.is_empty());
    }

    #[test]
    fn finish_assistant_turn_with_only_thinking() {
        let mut app = make_app();
        app.current_thinking_text = "thought".into();
        app.streaming = true;
        app.finish_assistant_turn();
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(app.messages[0].kind, MessageKind::Thinking));
        assert!(!app.streaming);
    }

    #[test]
    fn update_turn_stats_depth0_updates_prompt_eval() {
        let mut app = make_app();
        app.update_turn_stats(100, 2_000_000_000, 500, 0);
        assert_eq!(app.last_prompt_eval_count, Some(500));
    }

    #[test]
    fn update_turn_stats_depth1_skips_prompt_eval() {
        let mut app = make_app();
        app.update_turn_stats(100, 2_000_000_000, 500, 0);
        app.update_turn_stats(50, 1_000_000_000, 200, 1);
        assert_eq!(app.last_prompt_eval_count, Some(500));
        assert_eq!(app.total_tokens_down, 150);
        assert_eq!(app.total_tokens_up, 700);
    }

    #[test]
    fn context_used_reflects_latest_prompt_not_sum() {
        let mut app = make_app();
        app.update_turn_stats(100, 1_000_000_000, 200, 0);
        assert_eq!(app.context_used, 200);
        app.update_turn_stats(50, 500_000_000, 275, 0);
        // prompt_eval_count 275 already includes prior history — counter must SET, not sum
        assert_eq!(app.context_used, 275);
    }

    #[test]
    fn context_used_not_polluted_by_subagent_turns() {
        let mut app = make_app();
        app.update_turn_stats(100, 1_000_000_000, 200, 0);
        // subagent runs many turns with its own growing prompt — must NOT affect main counter
        app.update_turn_stats(50, 500_000_000, 5000, 1);
        app.update_turn_stats(50, 500_000_000, 8000, 2);
        assert_eq!(app.context_used, 200);
        // next main-thread turn: counter reflects its own prompt size only
        app.update_turn_stats(100, 1_000_000_000, 350, 0);
        assert_eq!(app.context_used, 350);
    }

    #[test]
    fn subtask_node_context_used_updates_independently() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.update_turn_stats(10, 1_000_000_000, 200, 0);
        app.enter_subtask(1, "worker_a".into());
        app.update_turn_stats(20, 1_000_000_000, 500, 1);
        assert_eq!(app.tree[0].context_used, 200, "orchestrator node");
        assert_eq!(app.tree[1].context_used, 500, "worker_a node");
    }

    #[test]
    fn new_subtask_resets_context_bar() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "worker_a".into());
        app.update_turn_stats(20, 1_000_000_000, 900, 1);
        assert_eq!(app.tree[1].context_used, 900);
        app.exit_subtask(1);
        app.enter_subtask(1, "worker_b".into());
        // fresh subtask — counter starts at 0
        assert_eq!(app.tree.last().unwrap().context_used, 0);
    }

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
}
