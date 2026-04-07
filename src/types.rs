use serde::{Deserialize, Serialize};

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

    pub fn with_images(role: Role, content: String, images: Vec<String>) -> Self {
        Self {
            role,
            content,
            images,
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
    Mcp {
        #[allow(dead_code)]
        server_name: String,
    },
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    #[allow(dead_code)]
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug)]
pub enum AgentEvent {
    ThinkingStarted,
    ThinkingDelta(#[allow(dead_code)] String),
    ThinkingDone,
    TextDelta(String),
    ToolRequested(ToolCall),
    ToolCompleted(ToolResult),
    TurnStats {
        eval_count: u64,
        prompt_eval_count: u64,
    },
    TurnDone,
    Error(String),
}

#[derive(Debug)]
pub enum TurnOutcome {
    Text(String),
    ToolCalls(String, Vec<ToolCall>),
}
