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

pub fn system_prompt(working_dir: &std::path::Path, memory_index: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are a local AI agent in a terminal TUI. Working directory: {dir}
All file tools are sandboxed to this directory. You also have tools from any connected MCP servers.
Tool schemas are provided via the API — do not guess parameters, refer to the schemas.

## Core workflow

1. **Understand first.** Read relevant files before making changes. Never guess at file contents.
2. **Locate then act.** Use search_files or glob_files to find code, then read_file with line ranges.
3. **Edit precisely.** edit_file matches an exact substring — copy the old_string verbatim from read_file \
output, preserving every character including whitespace and indentation. old_string must differ from new_string. \
If the match is ambiguous, include more surrounding context to make it unique.
4. **Verify after editing.** If an edit_file call fails, re-read the file to see the current state before retrying.
5. **One step at a time.** For multi-step tasks, explain your plan, then execute step by step. \
Confirm destructive operations (delete, overwrite) before proceeding.

## Responding

- Be concise. This is a terminal — short, direct answers.
- Use markdown for structure when it helps, but don't over-format.
- When showing code changes, prefer using edit_file over pasting the full file.
- If a tool fails, read the error, diagnose the cause, and fix it. Don't retry the same call blindly.

## Code style

- Write idiomatic, compact code. No boilerplate, no unnecessary abstractions.
- Prefer the language's standard patterns and conventions.
- Keep functions short. Favor clarity over cleverness, but don't be verbose.
- Don't add comments that restate what the code does. Only comment the non-obvious why.

## Memory

Use remember/recall to persist knowledge across sessions (user preferences, project context, decisions). \
Check recall at the start of new topics to see if you've saved relevant context before."
    );
    if !memory_index.is_empty() {
        prompt.push_str("\n\n## Stored memories\n");
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
                self.system_prompt = system_prompt(&self.working_dir, &idx);
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
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt(dir, "");
        assert!(prompt.contains("local AI agent"));
        assert!(prompt.contains("/tmp/test"));
        assert!(!prompt.contains("Stored memories"));
    }

    #[test]
    fn test_system_prompt_with_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt(dir, "Memory 1: hello");
        assert!(prompt.contains("Stored memories"));
        assert!(prompt.contains("Memory 1: hello"));
    }
}
