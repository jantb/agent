use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{ChatMessage, Message, MessageKind, Role, ToolCall};

const SESSION_DIR: &str = ".agent";
const SESSION_FILE: &str = "session.json";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    pub version: u32,
    pub model: String,
    pub working_dir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<SessionMessage>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum SessionMessage {
    #[serde(rename = "text")]
    Text {
        role: Role,
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
    },
    #[serde(rename = "thinking")]
    Thinking { content: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(default)]
        id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        name: String,
        content: String,
        is_error: bool,
    },
}

impl Session {
    pub fn new(model: &str, working_dir: &Path) -> Self {
        let now = Utc::now();
        Self {
            version: VERSION,
            model: model.to_string(),
            working_dir: working_dir.to_string_lossy().into_owned(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        }
    }

    pub fn load(working_dir: &Path) -> anyhow::Result<Option<Session>> {
        let path = Self::session_path(working_dir);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("reading session file {}", path.display()))?;
        let session = serde_json::from_str(&data)
            .with_context(|| format!("parsing session file {}", path.display()))?;
        Ok(Some(session))
    }

    pub fn save(&self, working_dir: &Path) -> anyhow::Result<()> {
        let mut session = self.clone();
        session.updated_at = Utc::now();
        let json = serde_json::to_string_pretty(&session).context("serializing session")?;
        let path = Self::session_path(working_dir);
        let tmp_path = path.with_extension("json.tmp");
        fs::create_dir_all(path.parent().unwrap_or(working_dir))?;
        fs::write(&tmp_path, &json).context("writing session temp file")?;
        fs::rename(&tmp_path, &path).context("renaming session temp file")?;
        Ok(())
    }

    pub fn append_message(&mut self, msg: SessionMessage) {
        self.messages.push(msg);
    }

    pub fn session_path(working_dir: &Path) -> PathBuf {
        working_dir.join(SESSION_DIR).join(SESSION_FILE)
    }

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
                    // Collect consecutive ToolCall entries into one tool_request Message
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
                    history.push(Message::new(Role::Tool, content.clone()));
                    i += 1;
                }
            }
        }
        history
    }
}

// --- Conversions ---

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
            },
            MessageKind::Thinking => SessionMessage::Thinking {
                content: msg.content.clone(),
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
            } => ChatMessage {
                role: Role::Tool,
                content,
                kind: MessageKind::ToolResult { name, is_error },
            },
        }
    }
}

// --- .gitignore helper ---

pub fn ensure_gitignore(working_dir: &Path) -> anyhow::Result<()> {
    let path = working_dir.join(".gitignore");
    let entry = ".agent/\n";
    if path.exists() {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        if !contents.lines().any(|l| l.trim() == ".agent/") {
            let mut updated = contents;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(entry);
            fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
        }
    } else {
        fs::write(&path, entry).with_context(|| format!("creating {}", path.display()))?;
    }
    Ok(())
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn new_session_has_empty_messages() {
        let dir = tmp();
        let session = Session::new("gpt-4", dir.path());
        assert!(session.messages.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "hello".into(),
            images: vec![],
        });
        session.save(dir.path()).unwrap();

        let loaded = Session::load(dir.path()).unwrap().expect("session present");
        assert_eq!(loaded.messages.len(), 1);
        assert!(
            matches!(&loaded.messages[0], SessionMessage::Text { content, .. } if content == "hello")
        );
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = tmp();
        let result = Session::load(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn append_message_adds_to_vec() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "hi".into(),
            images: vec![],
        });
        session.append_message(SessionMessage::ToolCall {
            id: "call-0".into(),
            name: "shell".into(),
            arguments: "{}".into(),
        });
        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn ensure_gitignore_creates_file() {
        let dir = tmp();
        ensure_gitignore(dir.path()).unwrap();
        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(contents.contains(".agent/"));
    }

    #[test]
    fn ensure_gitignore_appends_if_missing() {
        let dir = tmp();
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        ensure_gitignore(dir.path()).unwrap();
        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(contents.contains("target/"));
        assert!(contents.contains(".agent/"));
    }

    #[test]
    fn ensure_gitignore_no_duplicate() {
        let dir = tmp();
        fs::write(dir.path().join(".gitignore"), ".agent/\n").unwrap();
        ensure_gitignore(dir.path()).unwrap();
        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(contents.matches(".agent/").count(), 1);
    }

    #[test]
    fn save_and_load_preserves_images() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: Role::User,
            content: "describe this".into(),
            images: vec!["base64data".into()],
        });
        session.save(dir.path()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().unwrap();
        match &loaded.messages[0] {
            SessionMessage::Text { images, .. } => assert_eq!(images, &["base64data"]),
            _ => panic!("expected Text"),
        }
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
        // history[0] = system, history[1] = user message
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
    fn thinking_content_preserved_through_save_load() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Thinking {
            content: "reasoning about X".into(),
        });
        session.save(dir.path()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().unwrap();
        assert!(
            matches!(&loaded.messages[0], SessionMessage::Thinking { content } if content == "reasoning about X")
        );
    }

    #[test]
    fn tool_call_id_preserved_through_save_load() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::ToolCall {
            id: "call-42".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        });
        session.save(dir.path()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().unwrap();
        assert!(
            matches!(&loaded.messages[0], SessionMessage::ToolCall { id, .. } if id == "call-42")
        );
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
        // system + user + assistant = 3; thinking skipped
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(|m| m.role != Role::Tool));
    }

    #[test]
    fn load_tolerates_missing_tool_call_id() {
        let dir = tmp();
        let json = r#"{
            "version": 1,
            "model": "test",
            "working_dir": "/tmp",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "messages": [
                {"kind": "tool_call", "name": "list_dir", "arguments": "{\"path\":\".\"}" }
            ]
        }"#;
        let session_dir = dir.path().join(".agent");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("session.json"), json).unwrap();
        let loaded = Session::load(dir.path()).unwrap().unwrap();
        assert!(matches!(
            &loaded.messages[0],
            SessionMessage::ToolCall { id, name, .. } if id.is_empty() && name == "list_dir"
        ));
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
        // system + tool_request = 2
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].tool_calls[0].id, "call-7");
    }
}
