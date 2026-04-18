use std::fs;

use anyhow::Context;
use chrono::Utc;
use serde::Serialize;

use super::{Session, SESSION_DIR};

impl Session {
    pub fn load(working_dir: &std::path::Path) -> anyhow::Result<Option<Session>> {
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

    pub fn save(&self, working_dir: &std::path::Path) -> anyhow::Result<()> {
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

    /// Save an isolated subtask context to `.agent/subtasks/{counter:03}_d{depth}.json`.
    pub fn save_subtask(
        &self,
        working_dir: &std::path::Path,
        depth: usize,
        label: &str,
        system_prompt: &str,
        counter: usize,
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct SubtaskLog<'a> {
            depth: usize,
            label: &'a str,
            system_prompt: &'a str,
            session: &'a Session,
        }
        let mut session_snapshot = self.clone();
        session_snapshot.updated_at = Utc::now();
        let log = SubtaskLog {
            depth,
            label,
            system_prompt,
            session: &session_snapshot,
        };
        let json = serde_json::to_string_pretty(&log)?;
        let dir = working_dir.join(SESSION_DIR).join("subtasks");
        fs::create_dir_all(&dir)?;
        let fname = format!("{counter:03}_d{depth}.json");
        fs::write(dir.join(fname), json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionMessage;
    use crate::types::Role;
    use std::fs;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
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
    fn plan_survives_save_and_load() {
        use crate::types::{PlanItem, PlanStatus};
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.plan = vec![
            PlanItem {
                content: "first".into(),
                status: PlanStatus::InProgress,
            },
            PlanItem {
                content: "second".into(),
                status: PlanStatus::Pending,
            },
        ];
        session.save(dir.path()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().expect("session present");
        assert_eq!(loaded.plan.len(), 2);
        assert_eq!(loaded.plan[0].content, "first");
        assert_eq!(loaded.plan[0].status, PlanStatus::InProgress);
        assert_eq!(loaded.plan[1].status, PlanStatus::Pending);
    }

    #[test]
    fn plan_defaults_to_empty_on_old_session_json() {
        let dir = tmp();
        let path = dir.path().join(".agent").join("session.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let json = serde_json::json!({
            "version": 1,
            "model": "gpt-4",
            "working_dir": dir.path().to_string_lossy(),
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "messages": [],
        });
        fs::write(&path, json.to_string()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().expect("session present");
        assert!(loaded.plan.is_empty());
    }

    #[test]
    fn plan_null_in_json_deserializes_as_empty() {
        let dir = tmp();
        let path = dir.path().join(".agent").join("session.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let json = serde_json::json!({
            "version": 1,
            "model": "gpt-4",
            "working_dir": dir.path().to_string_lossy(),
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "messages": [],
            "plan": null,
        });
        fs::write(&path, json.to_string()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().expect("session present");
        assert!(loaded.plan.is_empty());
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = tmp();
        let result = Session::load(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn clear_history_leaves_plan_empty_after_save_load() {
        use crate::types::{PlanItem, PlanStatus};
        let dir = tmp();
        let mut session = Session::new("gpt-4", dir.path());
        session.plan = vec![PlanItem {
            content: "step 1".into(),
            status: PlanStatus::Pending,
        }];
        session.messages.push(SessionMessage::Text {
            role: Role::User,
            content: "do stuff".into(),
            images: vec![],
        });
        session.save(dir.path()).unwrap();
        session.messages.clear();
        session.plan.clear();
        session.save(dir.path()).unwrap();
        let loaded = Session::load(dir.path()).unwrap().expect("session present");
        assert!(loaded.plan.is_empty());
        assert!(loaded.messages.is_empty());
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
}
