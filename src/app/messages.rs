use crate::types::{ChatMessage, MessageKind, Role, ToolCall, ToolResult};

use super::App;

impl App {
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
        if call.name != "delegate_task" {
            self.subtask_tool_calls += 1;
        }
    }

    pub fn add_tool_result(&mut self, result: &ToolResult) {
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

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.message_queue.clear();
        self.plan.clear();
        self.error_message = None;
        self.last_eval_count = None;
        self.last_eval_duration_ns = None;
        self.last_prompt_eval_count = None;
        self.context_used = 0;
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

    pub fn enqueue_message(&mut self, text: String, images: Vec<String>) {
        self.messages.push(ChatMessage {
            role: Role::User,
            content: text.clone(),
            kind: MessageKind::Queued,
        });
        self.message_queue.push_back((text, images));
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    pub fn dequeue_message(&mut self) -> Option<(String, Vec<String>)> {
        let item = self.message_queue.pop_front()?;
        for msg in &mut self.messages {
            if msg.role == Role::User && matches!(msg.kind, MessageKind::Queued) {
                msg.kind = MessageKind::Text;
                break;
            }
        }
        Some(item)
    }

    pub fn queue_len(&self) -> usize {
        self.message_queue.len()
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::types::{MessageKind, Role, ToolCall, ToolResult};

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn add_user_message_appends() {
        let mut app = make_app();
        app.add_user_message("hello".into());
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "hello");
        assert!(matches!(app.messages[0].role, Role::User));
    }

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
            images: vec![],
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

    #[test]
    fn add_tool_result_without_call_defaults_to_tool() {
        let mut app = make_app();
        let result = ToolResult {
            call_id: "c1".into(),
            output: "result".into(),
            is_error: false,
            images: vec![],
        };
        app.add_tool_result(&result);
        if let MessageKind::ToolResult { name, .. } = &app.messages[0].kind {
            assert_eq!(name, "tool");
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn add_tool_call_delegate_does_not_increment_counter() {
        let mut app = make_app();
        let call = ToolCall {
            id: "c1".into(),
            name: "delegate_task".into(),
            arguments: serde_json::json!({"prompt": "do stuff"}),
        };
        app.add_tool_call(&call);
        assert_eq!(app.subtask_tool_calls, 0);
    }

    #[test]
    fn clear_messages_empties_all() {
        let mut app = make_app();
        app.add_user_message("hello".into());
        app.set_error("oops".into());
        app.update_turn_stats(10, 500_000_000, 20, 0);
        app.clear_messages();
        assert!(app.messages.is_empty());
        assert!(app.error_message.is_none());
        assert!(app.last_eval_count.is_none());
        assert!(app.last_eval_duration_ns.is_none());
        assert!(app.last_prompt_eval_count.is_none());
    }

    #[test]
    fn clear_messages_idempotent_on_empty() {
        let mut app = make_app();
        app.clear_messages();
        assert!(app.messages.is_empty());
    }

    #[test]
    fn clear_messages_clears_queue() {
        let mut app = make_app();
        app.enqueue_message("msg".into(), vec![]);
        app.clear_messages();
        assert_eq!(app.queue_len(), 0);
        assert!(app.messages.is_empty());
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
    fn enqueue_message_adds_to_queue_and_messages() {
        let mut app = make_app();
        app.enqueue_message("queued msg".into(), vec![]);
        assert_eq!(app.queue_len(), 1);
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "queued msg");
        assert!(matches!(app.messages[0].kind, MessageKind::Queued));
        assert!(matches!(app.messages[0].role, Role::User));
    }

    #[test]
    fn enqueue_multiple_preserves_order() {
        let mut app = make_app();
        app.enqueue_message("first".into(), vec![]);
        app.enqueue_message("second".into(), vec![]);
        app.enqueue_message("third".into(), vec![]);
        assert_eq!(app.queue_len(), 3);
        assert_eq!(app.messages.len(), 3);
    }

    #[test]
    fn dequeue_returns_fifo_order() {
        let mut app = make_app();
        app.enqueue_message("first".into(), vec!["img1".into()]);
        app.enqueue_message("second".into(), vec![]);
        let (text, images) = app.dequeue_message().unwrap();
        assert_eq!(text, "first");
        assert_eq!(images, vec!["img1"]);
        assert_eq!(app.queue_len(), 1);
        let (text, _) = app.dequeue_message().unwrap();
        assert_eq!(text, "second");
        assert_eq!(app.queue_len(), 0);
    }

    #[test]
    fn dequeue_promotes_queued_to_text() {
        let mut app = make_app();
        app.enqueue_message("msg".into(), vec![]);
        assert!(matches!(app.messages[0].kind, MessageKind::Queued));
        app.dequeue_message();
        assert!(matches!(app.messages[0].kind, MessageKind::Text));
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let mut app = make_app();
        assert!(app.dequeue_message().is_none());
    }

    #[test]
    fn enqueue_with_images() {
        let mut app = make_app();
        app.enqueue_message("with img".into(), vec!["base64data".into()]);
        let (text, images) = app.dequeue_message().unwrap();
        assert_eq!(text, "with img");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0], "base64data");
    }

    #[test]
    fn dequeue_promotes_only_first_queued() {
        let mut app = make_app();
        app.enqueue_message("first".into(), vec![]);
        app.enqueue_message("second".into(), vec![]);
        app.dequeue_message();
        assert!(matches!(app.messages[0].kind, MessageKind::Text));
        assert!(matches!(app.messages[1].kind, MessageKind::Queued));
    }
}
