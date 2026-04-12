use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub images: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn new(role: Role, content: String) -> Self {
        Self {
            role,
            content,
            images: vec![],
            tool_calls: vec![],
        }
    }

    pub fn tool_request(content: String, calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content,
            images: vec![],
            tool_calls: calls,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub source: ToolSource,
}

#[derive(Debug, Clone)]
pub enum ToolSource {
    BuiltIn,
    Mcp,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
    pub images: Vec<String>,
}

/// Wrapper for oneshot::Sender that implements Debug.
pub struct OneshotTx(pub oneshot::Sender<String>);

impl std::fmt::Debug for OneshotTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<oneshot::Sender>")
    }
}

#[derive(Debug)]
pub enum AgentEvent {
    ThinkingStarted,
    ThinkingDelta(String),
    ThinkingDone,
    TextDelta(String),
    ToolRequested(ToolCall),
    ToolCompleted(ToolResult),
    TurnStats {
        eval_count: u64,
        eval_duration_ns: u64,
        prompt_eval_count: u64,
    },
    TurnDone,
    Error(String),
    LoopDetected,
    SubtaskEnter {
        depth: usize,
        label: String,
    },
    SubtaskExit {
        depth: usize,
    },
    InterviewQuestion {
        question: String,
        suggestions: Vec<String>,
        answer_tx: OneshotTx,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    pub depth: usize,
    pub label: String,
    pub status: NodeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeStatus {
    Active,
    Suspended,
    Done,
    Failed,
}

#[derive(Debug)]
pub enum TurnOutcome {
    Text(String),
    ToolCalls(String, Vec<ToolCall>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_new_sets_role_and_content() {
        let msg = Message::new(Role::User, "hello".into());
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "hello");
        assert!(msg.images.is_empty());
        assert!(msg.tool_calls.is_empty());
    }

    #[test]
    fn message_new_assistant_role() {
        let msg = Message::new(Role::Assistant, "reply".into());
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "reply");
    }

    #[test]
    fn message_tool_request_sets_role_and_calls() {
        let calls = vec![ToolCall {
            id: "1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "foo.txt"}),
        }];
        let msg = Message::tool_request("calling tool".into(), calls);
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "calling tool");
        assert_eq!(msg.tool_calls.len(), 1);
        assert_eq!(msg.tool_calls[0].name, "read_file");
        assert!(msg.images.is_empty());
    }

    #[test]
    fn message_tool_request_empty_calls() {
        let msg = Message::tool_request(String::new(), vec![]);
        assert_eq!(msg.role, Role::Assistant);
        assert!(msg.tool_calls.is_empty());
        assert!(msg.content.is_empty());
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone)]
pub enum MessageKind {
    Text,
    Queued,
    Thinking,
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        is_error: bool,
    },
    SubtaskEnter {
        depth: usize,
        label: String,
    },
    SubtaskExit {
        depth: usize,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    #[default]
    Plan,
    Thorough,
    Oneshot,
}

impl AgentMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Plan => Self::Thorough,
            Self::Thorough => Self::Oneshot,
            Self::Oneshot => Self::Plan,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Thorough => "thorough",
            Self::Oneshot => "oneshot",
        }
    }
}
