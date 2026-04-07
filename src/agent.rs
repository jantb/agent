use std::{path::PathBuf, sync::Arc};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    mcp::McpRegistry,
    memory,
    ollama::OllamaClient,
    session::{Session, SessionMessage},
    tools::execute_built_in,
    types::{AgentEvent, Role, ToolCall, ToolDefinition, ToolResult, ToolSource, TurnOutcome},
};

pub fn system_prompt(memory_index: &str) -> String {
    let mut prompt = "You are a helpful local AI agent running in the terminal.\n\
You have access to tools to read, write, search, and edit files in the current working directory.\n\
You also have access to tools from connected MCP servers.\n\
\n\
Built-in file tools:\n\
- read_file: read with optional line ranges\n\
- write_file: write/overwrite a file\n\
- append_file: append to a file (creates if missing)\n\
- list_dir: list directory; use depth>0 to recurse (max 10)\n\
- edit_file: replace exact substring; use replace_all=true for all occurrences\n\
- replace_lines: replace a 1-based line range with new content\n\
- search_files: grep recursively; use is_regex=true for regex patterns\n\
- glob_files: find files by glob pattern (e.g. '**/*.rs')\n\
- delete_path: delete a file or empty directory\n\
\n\
Memory tools:\n\
- remember: store persistent knowledge across sessions\n\
- recall: search stored memories by keyword\n\
- forget: delete a stored memory\n\
- list_memories: list all stored memories\n\
\n\
Guidelines:\n\
- Always think through what the user wants before acting.\n\
- Use tools when you need to inspect or modify files — don't guess at file contents.\n\
- Be concise. This is a terminal interface.\n\
- read_file returns numbered lines. For large files, use start_line/end_line to read specific ranges.\n\
- Use search_files or glob_files first to find relevant code, then read_file with line ranges.\n\
- Use edit_file to make precise edits by matching exact substrings. Read the file first to get the exact text.\n\
- If a tool fails, explain what went wrong and suggest alternatives."
        .to_string();
    if !memory_index.is_empty() {
        prompt.push_str("\n\nStored memories:\n");
        prompt.push_str(memory_index);
    }
    prompt
}

pub enum UserAction {
    SendMessage(String, Vec<String>), // text, base64 images
    Cancel,
    Quit,
    ClearHistory,
}

enum TurnPhaseResult {
    Text(String),
    ToolCalls(String, Vec<ToolCall>),
    Cancelled,
    Error(anyhow::Error),
    Quit,
}

pub struct AgentTask {
    ollama: Arc<OllamaClient>,
    mcp: Arc<McpRegistry>,
    working_dir: PathBuf,
    tools: Vec<ToolDefinition>,
    event_tx: mpsc::Sender<AgentEvent>,
    action_rx: mpsc::Receiver<UserAction>,
    session: Session,
    system_prompt: String,
}

pub struct AgentTaskConfig {
    pub ollama: Arc<OllamaClient>,
    pub mcp: Arc<McpRegistry>,
    pub working_dir: PathBuf,
    pub tools: Vec<ToolDefinition>,
    pub event_tx: mpsc::Sender<AgentEvent>,
    pub action_rx: mpsc::Receiver<UserAction>,
    pub session: Session,
    pub system_prompt: String,
}

impl AgentTask {
    pub fn new(cfg: AgentTaskConfig) -> Self {
        let working_dir = cfg.working_dir.canonicalize().unwrap_or(cfg.working_dir);
        Self {
            ollama: cfg.ollama,
            mcp: cfg.mcp,
            working_dir,
            tools: cfg.tools,
            event_tx: cfg.event_tx,
            action_rx: cfg.action_rx,
            session: cfg.session,
            system_prompt: cfg.system_prompt,
        }
    }

    fn history(&self) -> Vec<crate::types::Message> {
        self.session.to_ollama_history(&self.system_prompt)
    }

    async fn emit(&self, event: AgentEvent) {
        if let Err(e) = self.event_tx.send(event).await {
            tracing::error!("failed to send agent event: {e}");
        }
    }

    async fn save_or_emit_error(&mut self) {
        match self.session.save(&self.working_dir) {
            Ok(()) => debug!("session saved"),
            Err(e) => self.emit(AgentEvent::Error(e.to_string())).await,
        }
    }

    async fn execute_turn(&mut self) -> TurnPhaseResult {
        let history = self.history();
        tokio::select! {
            action = self.action_rx.recv() => {
                match action {
                    Some(UserAction::Cancel) | None => TurnPhaseResult::Cancelled,
                    Some(UserAction::Quit) => TurnPhaseResult::Quit,
                    _ => TurnPhaseResult::Cancelled, // ignore other actions during turn
                }
            }
            outcome = self.ollama.stream_turn(&history, &self.tools, self.event_tx.clone()) => {
                match outcome {
                    Err(e) => TurnPhaseResult::Error(e),
                    Ok(TurnOutcome::Text(content)) => TurnPhaseResult::Text(content),
                    Ok(TurnOutcome::ToolCalls(text, calls)) => TurnPhaseResult::ToolCalls(text, calls),
                }
            }
        }
    }

    async fn handle_text_turn(&mut self, content: String) {
        self.session.append_message(SessionMessage::Text {
            role: Role::Assistant,
            content,
            images: vec![],
        });
        self.save_or_emit_error().await;
        self.emit(AgentEvent::TurnDone).await;
    }

    async fn handle_tool_calls(&mut self, text: String, calls: Vec<ToolCall>) {
        if !text.is_empty() {
            self.session.append_message(SessionMessage::Text {
                role: Role::Assistant,
                content: text,
                images: vec![],
            });
        }
        for call in calls {
            self.emit(AgentEvent::ToolRequested(call.clone())).await;
            self.session.append_message(SessionMessage::ToolCall {
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
            });

            debug!(tool = %call.name, id = %call.id, "tool dispatch start");
            let result =
                Self::dispatch_tool(&call, &self.tools, &self.working_dir, &self.mcp).await;
            debug!(tool = %call.name, is_error = result.is_error, "tool dispatch done");

            self.emit(AgentEvent::ToolCompleted(result.clone())).await;
            self.session.append_message(SessionMessage::ToolResult {
                name: call.name.clone(),
                content: result.output.clone(),
                is_error: result.is_error,
            });
            if matches!(call.name.as_str(), "remember" | "forget") {
                let idx = memory::build_memory_index(&self.working_dir);
                self.system_prompt = system_prompt(&idx);
            }
        }
        self.save_or_emit_error().await;
    }

    pub async fn run(mut self) {
        loop {
            let action = match self.action_rx.recv().await {
                Some(a) => a,
                None => return,
            };

            match action {
                UserAction::Quit => return,
                UserAction::Cancel => {
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::ClearHistory => {
                    self.session.messages.clear();
                    self.save_or_emit_error().await;
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::SendMessage(text, images) => {
                    self.session.append_message(SessionMessage::Text {
                        role: Role::User,
                        content: text,
                        images,
                    });
                    self.save_or_emit_error().await;

                    info!(
                        session_messages = self.session.messages.len(),
                        tools = self.tools.len(),
                        "turn start"
                    );
                    'turn: loop {
                        match self.execute_turn().await {
                            TurnPhaseResult::Text(content) => {
                                debug!(chars = content.len(), "turn result: text");
                                self.handle_text_turn(content).await;
                                break 'turn;
                            }
                            TurnPhaseResult::ToolCalls(text, calls) => {
                                debug!(count = calls.len(), "turn result: tool calls");
                                self.handle_tool_calls(text, calls).await;
                            }
                            TurnPhaseResult::Cancelled => {
                                self.emit(AgentEvent::TurnDone).await;
                                break 'turn;
                            }
                            TurnPhaseResult::Error(e) => {
                                warn!(error = %e, "turn error");
                                self.emit(AgentEvent::Error(e.to_string())).await;
                                break 'turn;
                            }
                            TurnPhaseResult::Quit => return,
                        }
                    }
                }
            }
        }
    }

    async fn dispatch_tool(
        call: &ToolCall,
        tools: &[ToolDefinition],
        working_dir: &std::path::Path,
        mcp: &McpRegistry,
    ) -> ToolResult {
        match tools.iter().find(|t| t.name == call.name) {
            None => ToolResult {
                call_id: call.id.clone(),
                output: format!("unknown tool: {}", call.name),
                is_error: true,
            },
            Some(def) => match &def.source {
                ToolSource::BuiltIn => execute_built_in(call, working_dir).await,
                ToolSource::Mcp => mcp.execute(call).await,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_empty_memory() {
        let prompt = system_prompt("");
        assert!(prompt.contains("You are a helpful local AI agent"));
        assert!(!prompt.contains("Stored memories:"));
    }

    #[test]
    fn test_system_prompt_with_memory() {
        let prompt = system_prompt("Memory 1: hello");
        assert!(prompt.contains("Stored memories:"));
        assert!(prompt.contains("Memory 1: hello"));
    }
}
