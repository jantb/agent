use std::{path::PathBuf, sync::Arc};

use tokio::sync::mpsc;

use crate::{
    mcp::McpRegistry,
    memory,
    ollama::OllamaClient,
    session::{Session, SessionMessage},
    tools::execute_built_in,
    types::{
        AgentEvent, Message, Role, ToolCall, ToolDefinition, ToolResult, ToolSource, TurnOutcome,
    },
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
- diff_files: unified diff between two files\n\
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
- Never reference files outside the working directory.\n\
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
    history: Vec<Message>,
    tools: Vec<ToolDefinition>,
    event_tx: mpsc::Sender<AgentEvent>,
    action_rx: mpsc::Receiver<UserAction>,
    session: Session,
}

pub struct AgentTaskConfig {
    pub ollama: Arc<OllamaClient>,
    pub mcp: Arc<McpRegistry>,
    pub working_dir: PathBuf,
    pub tools: Vec<ToolDefinition>,
    pub event_tx: mpsc::Sender<AgentEvent>,
    pub action_rx: mpsc::Receiver<UserAction>,
    pub history: Vec<Message>,
    pub session: Session,
}

impl AgentTask {
    pub fn new(cfg: AgentTaskConfig) -> Self {
        let working_dir = cfg.working_dir.canonicalize().unwrap_or(cfg.working_dir);
        Self {
            ollama: cfg.ollama,
            mcp: cfg.mcp,
            working_dir,
            history: cfg.history,
            tools: cfg.tools,
            event_tx: cfg.event_tx,
            action_rx: cfg.action_rx,
            session: cfg.session,
        }
    }

    async fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event).await;
    }

    async fn save_or_emit_error(&mut self) {
        if let Err(e) = self.session.save(&self.working_dir) {
            self.emit(AgentEvent::Error(e.to_string())).await;
        }
    }

    async fn execute_turn(&mut self, use_tool_model: bool) -> TurnPhaseResult {
        tokio::select! {
            action = self.action_rx.recv() => {
                match action {
                    Some(UserAction::Cancel) | None => TurnPhaseResult::Cancelled,
                    Some(UserAction::Quit) => TurnPhaseResult::Quit,
                    _ => TurnPhaseResult::Cancelled, // ignore other actions during turn
                }
            }
            outcome = self.ollama.stream_turn(&self.history, &self.tools, self.event_tx.clone(), use_tool_model) => {
                match outcome {
                    Err(e) => TurnPhaseResult::Error(e),
                    Ok(TurnOutcome::Text(content)) => TurnPhaseResult::Text(content),
                    Ok(TurnOutcome::ToolCalls(text, calls)) => TurnPhaseResult::ToolCalls(text, calls),
                }
            }
        }
    }

    async fn handle_text_turn(&mut self, content: String) {
        self.history
            .push(Message::new(Role::Assistant, content.clone()));
        self.session.append_message(SessionMessage::Text {
            role: Role::Assistant,
            content,
        });
        self.save_or_emit_error().await;
        self.emit(AgentEvent::TurnDone).await;
    }

    async fn handle_tool_calls(&mut self, text: String, calls: Vec<ToolCall>) {
        self.history
            .push(Message::tool_request(text, calls.clone()));
        for call in calls {
            self.emit(AgentEvent::ToolRequested(call.clone())).await;
            self.session.append_message(SessionMessage::ToolCall {
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
            });

            let result =
                Self::dispatch_tool(&call, &self.tools, &self.working_dir, &self.mcp).await;

            self.emit(AgentEvent::ToolCompleted(result.clone())).await;
            self.session.append_message(SessionMessage::ToolResult {
                name: call.name.clone(),
                content: result.output.clone(),
                is_error: result.is_error,
            });
            self.history.push(Message::new(Role::Tool, result.output));
            if matches!(call.name.as_str(), "remember" | "forget") {
                let idx = memory::build_memory_index(&self.working_dir);
                if let Some(sys) = self.history.iter_mut().find(|m| m.role == Role::System) {
                    sys.content = system_prompt(&idx);
                }
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
                    self.history.retain(|m| m.role == Role::System);
                    self.session.messages.clear();
                    self.save_or_emit_error().await;
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::SendMessage(text, images) => {
                    self.history
                        .push(Message::with_images(Role::User, text.clone(), images));
                    self.session.append_message(SessionMessage::Text {
                        role: Role::User,
                        content: text,
                    });
                    self.save_or_emit_error().await;

                    let mut tool_followup = false;
                    'turn: loop {
                        match self.execute_turn(tool_followup).await {
                            TurnPhaseResult::Text(content) => {
                                self.handle_text_turn(content).await;
                                break 'turn;
                            }
                            TurnPhaseResult::ToolCalls(text, calls) => {
                                self.handle_tool_calls(text, calls).await;
                                tool_followup = true;
                            }
                            TurnPhaseResult::Cancelled => {
                                self.emit(AgentEvent::TurnDone).await;
                                break 'turn;
                            }
                            TurnPhaseResult::Error(e) => {
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
                ToolSource::Mcp { .. } => mcp.execute(call).await,
            },
        }
    }
}
