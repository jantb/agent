use crate::types::{ChatMessage, MessageKind, Role};

use super::SessionMessage;

impl From<&ChatMessage> for SessionMessage {
    fn from(msg: &ChatMessage) -> Self {
        match &msg.kind {
            MessageKind::Text => SessionMessage::Text {
                role: msg.role.clone(),
                content: msg.content.clone(),
                images: vec![],
            },
            MessageKind::ToolCall {
                call_id,
                name,
                arguments,
            } => SessionMessage::ToolCall {
                id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            },
            MessageKind::ToolResult { name, is_error } => SessionMessage::ToolResult {
                name: name.clone(),
                content: msg.content.clone(),
                is_error: *is_error,
                images: vec![],
            },
            MessageKind::Thinking => SessionMessage::Thinking {
                content: msg.content.clone(),
            },
            MessageKind::Queued
            | MessageKind::SubtaskEnter { .. }
            | MessageKind::SubtaskExit { .. }
            | MessageKind::PlanUpdate { .. } => SessionMessage::Text {
                role: msg.role.clone(),
                content: msg.content.clone(),
                images: vec![],
            },
        }
    }
}

impl From<ChatMessage> for SessionMessage {
    fn from(msg: ChatMessage) -> Self {
        SessionMessage::from(&msg)
    }
}

impl From<SessionMessage> for ChatMessage {
    fn from(msg: SessionMessage) -> Self {
        match msg {
            SessionMessage::Text { role, content, .. } => ChatMessage {
                role,
                content,
                kind: MessageKind::Text,
            },
            SessionMessage::Thinking { content } => ChatMessage {
                role: Role::Assistant,
                content,
                kind: MessageKind::Thinking,
            },
            SessionMessage::ToolCall {
                id,
                name,
                arguments,
            } => ChatMessage {
                role: Role::Assistant,
                content: String::new(),
                kind: MessageKind::ToolCall {
                    call_id: id,
                    name,
                    arguments,
                },
            },
            SessionMessage::ToolResult {
                name,
                content,
                is_error,
                ..
            } => ChatMessage {
                role: Role::Tool,
                content,
                kind: MessageKind::ToolResult { name, is_error },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_message_to_chat_message_roundtrip() {
        let original = SessionMessage::Text {
            role: Role::Assistant,
            content: "hello there".into(),
            images: vec![],
        };
        let chat: ChatMessage = original.into();
        assert_eq!(chat.role, Role::Assistant);
        assert_eq!(chat.content, "hello there");
        let back: SessionMessage = SessionMessage::from(&chat);
        assert!(matches!(back, SessionMessage::Text { role, .. } if role == Role::Assistant));
    }

    #[test]
    fn thinking_roundtrip_through_chat_message() {
        let chat = ChatMessage {
            role: Role::Assistant,
            content: "deep thoughts".into(),
            kind: MessageKind::Thinking,
        };
        let session: SessionMessage = SessionMessage::from(&chat);
        let back: ChatMessage = session.into();
        assert_eq!(back.content, "deep thoughts");
        assert!(matches!(back.kind, MessageKind::Thinking));
    }

    #[test]
    fn tool_call_id_roundtrip_through_chat_message() {
        let chat = ChatMessage {
            role: Role::Assistant,
            content: String::new(),
            kind: MessageKind::ToolCall {
                call_id: "my-id".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            },
        };
        let session: SessionMessage = SessionMessage::from(&chat);
        let back: ChatMessage = session.into();
        assert!(
            matches!(back.kind, MessageKind::ToolCall { ref call_id, .. } if call_id == "my-id")
        );
    }
}
