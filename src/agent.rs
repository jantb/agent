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

## CRITICAL: Always use tools for file operations

NEVER show file content in your response as a code block when asked to create or edit a file. \
ALWAYS use the write_file or edit_file tool. If the user asks you to create a file, call write_file immediately. \
Do not display the code and then say you'll create it — create it first, then briefly confirm.

## Core workflow

1. **Understand first.** Read relevant files before making changes.
2. **Locate then act.** Use search_files, glob_files, or line_count to find code, then read_file with line ranges.
3. **Edit precisely.** edit_file matches an exact substring — copy old_string verbatim from read_file output, preserving whitespace and indentation. If ambiguous, include more surrounding context to make it unique.
4. **Verify after editing.** If an edit_file call fails, re-read the file before retrying.
5. **One step at a time.** For multi-step tasks, explain your plan, then execute step by step.

## Responding

- Be concise. This is a terminal — short, direct answers.
- Use markdown for structure when it helps, but don't over-format.
- If a tool fails, read the error, diagnose, and fix it. Don't retry blindly.

## Code style

- Write idiomatic, compact code. No boilerplate, no unnecessary abstractions.
- Keep functions short and clear. Only comment the non-obvious why.

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

fn text_fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    text.split_whitespace().for_each(|w| w.hash(&mut hasher));
    hasher.finish()
}

/// Returns Some(fingerprint) if the text was already seen in the window.
fn check_repeated_text(text: &str, window: &std::collections::VecDeque<u64>) -> Option<u64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let fp = text_fingerprint(trimmed);
    if window.contains(&fp) {
        Some(fp)
    } else {
        None
    }
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

    /// Check for a repeated-text loop.
    /// Returns `Some(true)` → `continue 'turn` (nudge injected),
    ///         `Some(false)` → `break 'turn` (loop persists after nudge),
    ///         `None` → no repetition, proceed normally.
    async fn handle_loop_check(
        &mut self,
        text: &str,
        recent_fps: &mut std::collections::VecDeque<u64>,
        nudged: &mut bool,
        nudge_msg_idx: &mut usize,
        context: &str,
    ) -> Option<bool> {
        if let Some(fp) = check_repeated_text(text, recent_fps) {
            if *nudged {
                warn!("loop persists after nudge, breaking");
                self.session.messages.truncate(*nudge_msg_idx);
                self.save_or_emit_error().await;
                self.emit(AgentEvent::LoopDetected).await;
                self.emit(AgentEvent::TurnDone).await;
                return Some(false);
            }
            warn!(fp = %fp, context, "repeated text detected, injecting nudge");
            *nudged = true;
            *nudge_msg_idx = self.session.messages.len();
            self.session.append_message(SessionMessage::Text {
                role: Role::User,
                content: "You are repeating yourself. Provide a different, complete response."
                    .into(),
                images: vec![],
            });
            self.save_or_emit_error().await;
            return Some(true);
        }
        let fp = text_fingerprint(text.trim());
        if !text.trim().is_empty() {
            recent_fps.push_back(fp);
            if recent_fps.len() > 20 {
                recent_fps.pop_front();
            }
        }
        None
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
                id: call.id.clone(),
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
                    const MAX_TOOL_ROUNDS: u32 = 25;
                    let mut round: u32 = 0;
                    let mut recent_text_fps: std::collections::VecDeque<u64> =
                        std::collections::VecDeque::with_capacity(20);
                    let mut nudged = false;
                    let mut nudge_msg_idx: usize = 0;
                    'turn: loop {
                        match self.execute_turn().await {
                            TurnPhaseResult::Text(content) => {
                                debug!(chars = content.len(), "turn result: text");
                                round += 1;
                                if round > MAX_TOOL_ROUNDS {
                                    warn!(rounds = round, "hard cap reached");
                                    self.emit(AgentEvent::LoopDetected).await;
                                    self.emit(AgentEvent::TurnDone).await;
                                    break 'turn;
                                }
                                if content.trim().is_empty() {
                                    if round >= 5 {
                                        warn!("empty text after {round} retries, giving up");
                                        self.emit(AgentEvent::Error(
                                            "Model returned only thinking content with no visible response after multiple retries".into()
                                        )).await;
                                        break 'turn;
                                    } else if round >= 3 {
                                        warn!("empty text after {round} retries, injecting nudge");
                                        self.session.append_message(SessionMessage::Text {
                                            role: Role::User,
                                            content: "Your previous responses contained only thinking with no visible text. You MUST respond with either a tool call or visible text. Do not only think — take action.".into(),
                                            images: vec![],
                                        });
                                        self.save_or_emit_error().await;
                                    } else {
                                        debug!("empty text turn (thinking-only), retrying");
                                    }
                                    continue 'turn;
                                }
                                match self
                                    .handle_loop_check(
                                        &content,
                                        &mut recent_text_fps,
                                        &mut nudged,
                                        &mut nudge_msg_idx,
                                        "text",
                                    )
                                    .await
                                {
                                    Some(true) => continue 'turn,
                                    Some(false) => break 'turn,
                                    None => {}
                                }
                                self.handle_text_turn(content).await;
                                break 'turn;
                            }
                            TurnPhaseResult::ToolCalls(text, calls) => {
                                debug!(count = calls.len(), "turn result: tool calls");
                                round += 1;
                                if round > MAX_TOOL_ROUNDS {
                                    warn!(rounds = round, "hard cap reached");
                                    self.emit(AgentEvent::LoopDetected).await;
                                    self.emit(AgentEvent::TurnDone).await;
                                    break 'turn;
                                }
                                match self
                                    .handle_loop_check(
                                        &text,
                                        &mut recent_text_fps,
                                        &mut nudged,
                                        &mut nudge_msg_idx,
                                        "tool_round",
                                    )
                                    .await
                                {
                                    Some(true) => continue 'turn,
                                    Some(false) => break 'turn,
                                    None => {}
                                }
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
    fn text_fingerprint_same_text() {
        let a = text_fingerprint("hello world");
        let b = text_fingerprint("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn text_fingerprint_whitespace_normalized() {
        let a = text_fingerprint("hello   world");
        let b = text_fingerprint("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn text_fingerprint_different_text() {
        let a = text_fingerprint("hello world");
        let b = text_fingerprint("goodbye world");
        assert_ne!(a, b);
    }

    #[test]
    fn check_repeated_text_empty_returns_none() {
        let window = std::collections::VecDeque::new();
        assert!(check_repeated_text("", &window).is_none());
        assert!(check_repeated_text("   ", &window).is_none());
    }

    #[test]
    fn check_repeated_text_detects_repeat() {
        let mut window = std::collections::VecDeque::new();
        window.push_back(text_fingerprint("hello world"));
        assert!(check_repeated_text("hello world", &window).is_some());
    }

    #[test]
    fn check_repeated_text_no_false_positive() {
        let mut window = std::collections::VecDeque::new();
        window.push_back(text_fingerprint("hello world"));
        assert!(check_repeated_text("different text", &window).is_none());
    }

    #[test]
    fn check_repeated_text_window_catches_cycle() {
        let mut window = std::collections::VecDeque::new();
        window.push_back(text_fingerprint("message A"));
        window.push_back(text_fingerprint("message B"));
        window.push_back(text_fingerprint("message C"));
        // A appears again — should detect
        assert!(check_repeated_text("message A", &window).is_some());
    }

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
