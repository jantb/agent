use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::types::PlanItem;

pub(crate) mod convert;
pub(crate) mod gitignore;
pub(crate) mod history;
pub(crate) mod persist;

pub use gitignore::ensure_gitignore;

pub(super) const SESSION_DIR: &str = ".agent";
pub(super) const SESSION_FILE: &str = "session.json";
pub(super) const VERSION: u32 = 1;

/// Persisted conversation state. Invariant: only data needed to resume the
/// conversation belongs here (messages, plan). Ephemeral UI state (tree,
/// scroll, streaming) lives on `App` and is not persisted.
#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    pub version: u32,
    pub model: String,
    pub working_dir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<SessionMessage>,
    #[serde(default, deserialize_with = "deserialize_null_as_empty")]
    pub plan: Vec<PlanItem>,
}

pub(super) fn deserialize_null_as_empty<'de, D, T: Default + Deserialize<'de>>(
    d: D,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(|v| v.unwrap_or_default())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum SessionMessage {
    #[serde(rename = "text")]
    Text {
        role: crate::types::Role,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
    },
}

impl Session {
    pub fn new(model: &str, working_dir: &Path) -> Self {
        let now = chrono::Utc::now();
        Self {
            version: VERSION,
            model: model.to_string(),
            working_dir: working_dir.to_string_lossy().into_owned(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            plan: Vec::new(),
        }
    }

    pub fn append_message(&mut self, msg: SessionMessage) {
        self.messages.push(msg);
    }

    pub fn session_path(working_dir: &Path) -> PathBuf {
        working_dir.join(SESSION_DIR).join(SESSION_FILE)
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
    fn new_session_has_empty_messages() {
        let dir = tmp();
        let session = Session::new("gpt-4", dir.path());
        assert!(session.messages.is_empty());
    }

    #[test]
    fn append_message_adds_to_vec() {
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.append_message(SessionMessage::Text {
            role: crate::types::Role::User,
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
}
