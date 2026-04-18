use crate::types::{Message, Role, ToolCall};

use super::{Session, SessionMessage};

impl Session {
    /// Build the Ollama history from session messages, prepending the system prompt.
    pub fn to_ollama_history(&self, system_prompt: &str) -> Vec<Message> {
        let mut history = vec![Message::new(Role::System, system_prompt.to_string())];
        let mut i = 0;
        while i < self.messages.len() {
            match &self.messages[i] {
                SessionMessage::Text {
                    role,
                    content,
                    images,
                } => {
                    let mut msg = Message::new(role.clone(), content.clone());
                    msg.images = images.clone();
                    history.push(msg);
                    i += 1;
                }
                SessionMessage::Thinking { .. } => {
                    i += 1;
                }
                SessionMessage::ToolCall { .. } => {
                    let mut calls = Vec::new();
                    while i < self.messages.len() {
                        if let SessionMessage::ToolCall {
                            id,
                            name,
                            arguments,
                        } = &self.messages[i]
                        {
                            calls.push(ToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                arguments: serde_json::from_str(arguments)
                                    .unwrap_or(serde_json::Value::Null),
                            });
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    history.push(Message::tool_request(String::new(), calls));
                }
                SessionMessage::ToolResult {
                    content, images, ..
                } => {
                    let mut msg = Message::new(Role::Tool, content.clone());
                    msg.images = images.clone();
                    history.push(msg);
                    i += 1;
                }
            }
        }
        history
    }

    /// Build compressed Ollama history for the orchestrator.
    /// Keeps the system prompt and all user messages intact.
    /// For tool results older than `keep_recent` exchanges, replaces the full
    /// content with a one-line summary to save context tokens.
    pub fn to_compressed_history(&self, system_prompt: &str, keep_recent: usize) -> Vec<Message> {
        let mut history = vec![Message::new(Role::System, system_prompt.to_string())];

        let user_count = self
            .messages
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    SessionMessage::Text {
                        role: Role::User,
                        ..
                    }
                )
            })
            .count();
        let compress_before = user_count.saturating_sub(keep_recent);
        let mut user_seen = 0usize;

        let mut i = 0;
        while i < self.messages.len() {
            match &self.messages[i] {
                SessionMessage::Text {
                    role,
                    content,
                    images,
                } => {
                    if *role == Role::User {
                        user_seen += 1;
                    }
                    let mut msg = Message::new(role.clone(), content.clone());
                    msg.images = images.clone();
                    history.push(msg);
                    i += 1;
                }
                SessionMessage::Thinking { .. } => {
                    i += 1;
                }
                SessionMessage::ToolCall { .. } => {
                    let mut calls = Vec::new();
                    while i < self.messages.len() {
                        if let SessionMessage::ToolCall {
                            id,
                            name,
                            arguments,
                        } = &self.messages[i]
                        {
                            calls.push(ToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                arguments: serde_json::from_str(arguments)
                                    .unwrap_or(serde_json::Value::Null),
                            });
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    history.push(Message::tool_request(String::new(), calls));
                }
                SessionMessage::ToolResult { content, .. } => {
                    let compressed = if user_seen <= compress_before {
                        let first_line = content.lines().next().unwrap_or("");
                        let total_chars = content.len();
                        if total_chars > 200 {
                            format!(
                                "{} [{} chars, summarized]",
                                &first_line[..first_line.len().min(150)],
                                total_chars
                            )
                        } else {
                            content.clone()
                        }
                    } else {
                        content.clone()
                    };
                    history.push(Message::new(Role::Tool, compressed));
                    i += 1;
                }
            }
        }
        history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn to_ollama_history_includes_images() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "look at this".into(),
            images: vec!["img1".into(), "img2".into()],
        });
        let history = session.to_ollama_history("system");
        assert_eq!(history[1].images, vec!["img1", "img2"]);
    }

    #[test]
    fn to_ollama_history_empty_images_by_default() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "hello".into(),
            images: vec![],
        });
        let history = session.to_ollama_history("system");
        assert!(history[1].images.is_empty());
    }

    #[test]
    fn to_ollama_history_skips_thinking() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "hello".into(),
            images: vec![],
        });
        session.append_message(SessionMessage::Thinking {
            content: "internal".into(),
        });
        session.append_message(SessionMessage::Text {
            role: Role::Assistant,
            content: "hi".into(),
            images: vec![],
        });
        let history = session.to_ollama_history("sys");
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(|m| m.role != Role::Tool));
    }

    #[test]
    fn to_ollama_history_preserves_tool_call_id() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::ToolCall {
            id: "call-7".into(),
            name: "shell".into(),
            arguments: "{}".into(),
        });
        let history = session.to_ollama_history("sys");
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].tool_calls[0].id, "call-7");
    }

    #[test]
    fn compressed_history_truncates_old_tool_results() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        // Exchange 1 (old — will be compressed)
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "task 1".into(),
            images: vec![],
        });
        session.append_message(SessionMessage::ToolCall {
            id: "c1".into(),
            name: "delegate_task".into(),
            arguments: "{}".into(),
        });
        session.append_message(SessionMessage::ToolResult {
            name: "delegate_task".into(),
            content: "a]".repeat(500),
            is_error: false,
            images: vec![],
        });
        session.append_message(SessionMessage::Text {
            role: Role::Assistant,
            content: "done 1".into(),
            images: vec![],
        });
        // Exchange 2 (recent — kept intact)
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "task 2".into(),
            images: vec![],
        });
        session.append_message(SessionMessage::ToolCall {
            id: "c2".into(),
            name: "delegate_task".into(),
            arguments: "{}".into(),
        });
        session.append_message(SessionMessage::ToolResult {
            name: "delegate_task".into(),
            content: "b".repeat(500),
            is_error: false,
            images: vec![],
        });
        session.append_message(SessionMessage::Text {
            role: Role::Assistant,
            content: "done 2".into(),
            images: vec![],
        });

        let full = session.to_ollama_history("sys");
        let compressed = session.to_compressed_history("sys", 1);

        let full_tool_result = &full[3].content;
        assert!(full_tool_result.len() >= 1000);

        let comp_tool_result = &compressed[3].content;
        assert!(
            comp_tool_result.len() < 200,
            "old tool result should be compressed, got {} chars",
            comp_tool_result.len()
        );
        assert!(comp_tool_result.contains("summarized"));

        let comp_recent = &compressed[7].content;
        assert_eq!(comp_recent.len(), 500);
    }
}
