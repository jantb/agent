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

/// Returns the system prompt for a given depth level.
pub fn system_prompt_for_depth(
    depth: usize,
    working_dir: &std::path::Path,
    memory_index: &str,
) -> String {
    match depth {
        0 => orchestrator_system_prompt(working_dir, memory_index),
        1 => coordinator_system_prompt(working_dir),
        _ => worker_system_prompt(working_dir),
    }
}

fn orchestrator_system_prompt(working_dir: &std::path::Path, memory_index: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the orchestration layer of a multi-level AI agent. Working directory: {dir}
Primary tool: `delegate_task`. You may also use `write_file`, `append_file`, `edit_file` to write final output directly.

## Rules
1. Decompose the user's goal into focused, atomic subtasks.
2. Delegate each via `delegate_task` — one at a time, sequentially.
3. Every prompt must be fully self-contained (file paths, line ranges, full context). Sub-agents see nothing else.
4. Never send vague prompts. Be specific: \"Read src/foo.rs lines 10-30 and summarize the validation logic\".
5. Synthesize results into a concise final answer (1-3 paragraphs max). For lengthy output, write it directly with `write_file` and include the path in your summary."
    );
    if !memory_index.is_empty() {
        prompt.push_str("\n\n## Stored memories\n");
        prompt.push_str(memory_index);
    }
    prompt
}

fn coordinator_system_prompt(working_dir: &std::path::Path) -> String {
    let dir = working_dir.display();
    format!(
        "\
You are the coordination layer of a multi-level AI agent. Working directory: {dir}
Tools: glob_files, search_files, list_dir, delegate_task.

## Rules
1. Search FIRST to locate relevant files before delegating.
2. If search tools answer the question directly, return the answer — don't delegate trivially.
3. Delegate specific file operations (read/write/edit) to sub-agents — you lack file I/O tools.
4. Every delegate_task prompt must be self-contained. Sub-agents see nothing else.
5. Return a concise synthesis (2 paragraphs max). For lengthy results, delegate writing to a .md file and report the path."
    )
}

fn worker_system_prompt(working_dir: &std::path::Path) -> String {
    let dir = working_dir.display();
    format!(
        "\
You are the execution layer of a multi-level AI agent. Working directory: {dir}
You have full file-tool access. No delegation — complete everything yourself.

## Rules
1. Execute the task completely. Use read_file, write_file, edit_file, search_files as needed.
2. Read only what is necessary; write/edit only what is asked.
3. Sandboxed to {dir}.
4. On error, report clearly rather than looping.
5. Return a concise summary (under 500 words). Use file:line references for findings.
6. For any output longer than ~20 lines (reports, analysis, code, documentation), write it to a .md file under the working directory and return only the file path + a 1-sentence summary. Reserve inline output for short answers only."
    )
}

/// Filter the full tool set to the subset appropriate for a given depth.
/// depth 0 (orchestrator): only delegate_task
/// depth 1 (coordinator): delegate_task + navigation tools
/// depth 2+ (worker):     all tools except delegate_task
pub fn tools_for_depth(all_tools: &[ToolDefinition], depth: usize) -> Vec<ToolDefinition> {
    const COORDINATOR_TOOLS: &[&str] = &["delegate_task", "glob_files", "search_files", "list_dir"];
    const ORCHESTRATOR_WRITE_TOOLS: &[&str] = &["write_file", "append_file", "edit_file"];
    match depth {
        0 => all_tools
            .iter()
            .filter(|t| {
                t.name == "delegate_task" || ORCHESTRATOR_WRITE_TOOLS.contains(&t.name.as_str())
            })
            .cloned()
            .collect(),
        1 => all_tools
            .iter()
            .filter(|t| COORDINATOR_TOOLS.contains(&t.name.as_str()))
            .cloned()
            .collect(),
        _ => all_tools
            .iter()
            .filter(|t| t.name != "delegate_task")
            .cloned()
            .collect(),
    }
}

static SUBTASK_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// RAII guard: emits SubtaskExit via try_send in Drop, ensuring the event
/// is always delivered even if the subtask future is cancelled or panics.
/// Disarm before the normal explicit emit to avoid double-emission.
struct SubtaskExitGuard {
    tx: mpsc::Sender<crate::types::AgentEvent>,
    depth: usize,
    armed: bool,
}

impl SubtaskExitGuard {
    fn new(tx: mpsc::Sender<crate::types::AgentEvent>, depth: usize) -> Self {
        Self {
            tx,
            depth,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SubtaskExitGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self
                .tx
                .try_send(crate::types::AgentEvent::SubtaskExit { depth: self.depth });
        }
    }
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

fn truncate_subtask_result(s: String, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s;
    }
    let truncated: String = s.chars().take(max_chars).collect();
    let total = s.chars().count();
    format!("{truncated}\n\n[truncated: {total} chars total, showing first {max_chars}]")
}

pub enum UserAction {
    SendMessage(String, Vec<String>), // text, base64 images
    Cancel,
    Quit,
    ClearHistory,
    ChangeModel(String),
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
    /// Depth-filtered tools for this node.
    tools: Vec<ToolDefinition>,
    /// Full unfiltered tool set (built-ins + MCP), used to compute child tool sets.
    all_tools: Vec<ToolDefinition>,
    event_tx: mpsc::Sender<AgentEvent>,
    /// None for subtasks (no interactive cancel).
    action_rx: Option<mpsc::Receiver<UserAction>>,
    session: Session,
    system_prompt: String,
    depth: usize,
}

pub struct AgentTaskConfig {
    pub ollama: Arc<OllamaClient>,
    pub mcp: Arc<McpRegistry>,
    pub working_dir: PathBuf,
    /// Full unfiltered tool set; tools_for_depth(0) is applied in new().
    pub all_tools: Vec<ToolDefinition>,
    pub event_tx: mpsc::Sender<AgentEvent>,
    pub action_rx: mpsc::Receiver<UserAction>,
    pub session: Session,
    /// Pass the orchestrator system prompt (system_prompt_for_depth(0, ...)).
    pub system_prompt: String,
}

impl AgentTask {
    pub fn new(cfg: AgentTaskConfig) -> Self {
        let working_dir = match cfg.working_dir.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                warn!("failed to canonicalize working_dir: {e}; using as-is (sandboxing may be unreliable)");
                cfg.working_dir
            }
        };
        let tools = tools_for_depth(&cfg.all_tools, 0);
        Self {
            ollama: cfg.ollama,
            mcp: cfg.mcp,
            working_dir,
            tools,
            all_tools: cfg.all_tools,
            event_tx: cfg.event_tx,
            action_rx: Some(cfg.action_rx),
            session: cfg.session,
            system_prompt: cfg.system_prompt,
            depth: 0,
        }
    }

    fn history(&self) -> Vec<crate::types::Message> {
        if self.depth == 0 {
            self.session.to_compressed_history(&self.system_prompt, 2)
        } else {
            self.session.to_ollama_history(&self.system_prompt)
        }
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
                // Only emit protocol-level events for the root task; subtasks
                // return a string result — emitting TurnDone from a subtask
                // would be received by the headless/TUI loop and cause early
                // termination before the parent orchestrator finishes.
                if self.depth == 0 {
                    self.emit(AgentEvent::LoopDetected).await;
                    self.emit(AgentEvent::TurnDone).await;
                }
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
        if self.depth > 0 {
            return; // subtasks don't persist their isolated sessions
        }
        match self.session.save(&self.working_dir) {
            Ok(()) => debug!("session saved"),
            Err(e) => self.emit(AgentEvent::Error(e.to_string())).await,
        }
    }

    async fn execute_turn(&mut self) -> TurnPhaseResult {
        let history = self.history();
        if let Some(rx) = &mut self.action_rx {
            tokio::select! {
                action = rx.recv() => {
                    match action {
                        Some(UserAction::Cancel) | None => TurnPhaseResult::Cancelled,
                        Some(UserAction::Quit) => TurnPhaseResult::Quit,
                        _ => TurnPhaseResult::Cancelled,
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
        } else {
            match self
                .ollama
                .stream_turn(&history, &self.tools, self.event_tx.clone())
                .await
            {
                Err(e) => TurnPhaseResult::Error(e),
                Ok(TurnOutcome::Text(content)) => TurnPhaseResult::Text(content),
                Ok(TurnOutcome::ToolCalls(text, calls)) => TurnPhaseResult::ToolCalls(text, calls),
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
            let result = if call.name == "delegate_task" {
                let prompt = call
                    .arguments
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let answer = self.run_subtask(prompt).await;
                ToolResult {
                    call_id: call.id.clone(),
                    output: answer,
                    is_error: false,
                    images: vec![],
                }
            } else {
                Self::dispatch_tool(&call, &self.tools, &self.working_dir, &self.mcp).await
            };
            debug!(tool = %call.name, is_error = result.is_error, "tool dispatch done");

            self.emit(AgentEvent::ToolCompleted(result.clone())).await;
            // For delegate_task results at the orchestrator level, store a compact
            // summary so orchestrator context stays bounded across turns.
            // The full result has already been emitted to the TUI above.
            let stored_content = if call.name == "delegate_task" && self.depth == 0 {
                truncate_subtask_result(result.output.clone(), 2000)
            } else {
                result.output.clone()
            };
            self.session.append_message(SessionMessage::ToolResult {
                name: call.name.clone(),
                content: stored_content,
                is_error: result.is_error,
                images: result.images.clone(),
            });
            if matches!(call.name.as_str(), "remember" | "forget") {
                let idx = memory::build_memory_index(&self.working_dir);
                self.system_prompt = system_prompt_for_depth(self.depth, &self.working_dir, &idx);
            }
        }
        self.save_or_emit_error().await;
    }

    /// Run an isolated subtask: fresh session, depth-filtered tools, depth+1.
    /// The parent session is never touched during child execution.
    async fn run_subtask(&self, prompt: String) -> String {
        const MAX_DEPTH: usize = 3;
        if self.depth + 1 >= MAX_DEPTH {
            return format!(
                "error: max delegation depth ({MAX_DEPTH}) reached — execute the task directly"
            );
        }
        let child_depth = self.depth + 1;
        let label: String = prompt.chars().take(40).collect();
        self.emit(AgentEvent::SubtaskEnter {
            depth: child_depth,
            label: label.clone(),
        })
        .await;

        let child_tools = tools_for_depth(&self.all_tools, child_depth);
        let child_system = system_prompt_for_depth(child_depth, &self.working_dir, "");
        let mut child_session = Session::new("subtask", &self.working_dir);
        child_session.append_message(SessionMessage::Text {
            role: Role::User,
            content: prompt,
            images: vec![],
        });

        let mut child = AgentTask {
            ollama: Arc::clone(&self.ollama),
            mcp: Arc::clone(&self.mcp),
            working_dir: self.working_dir.clone(),
            tools: child_tools,
            all_tools: self.all_tools.clone(),
            event_tx: self.event_tx.clone(),
            action_rx: None,
            session: child_session,
            system_prompt: child_system,
            depth: child_depth,
        };

        let n = SUBTASK_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Guard ensures SubtaskExit fires via try_send even on panic/cancellation.
        let mut exit_guard = SubtaskExitGuard::new(self.event_tx.clone(), child_depth);

        let result = child.run_single_task().await;
        let result = truncate_subtask_result(result, 8000);
        if let Err(e) = child.session.save_subtask(
            &self.working_dir,
            child_depth,
            &label,
            &child.system_prompt,
            n,
        ) {
            warn!("failed to save subtask context: {e}");
        }

        // Disarm before the awaited emit to avoid double-emission on normal exit.
        exit_guard.disarm();
        self.emit(AgentEvent::SubtaskExit { depth: child_depth })
            .await;
        result
    }

    /// Run one complete task (turn loop) to completion and return the final text.
    /// Used by subtasks; does not wait for user messages.
    /// Boxed to break the async recursion cycle (run_single_task → handle_tool_calls → run_subtask → run_single_task).
    fn run_single_task(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send + '_>> {
        Box::pin(async move {
            const MAX_TOOL_ROUNDS: u32 = 25;
            let mut round = 0u32;
            let mut recent_fps: std::collections::VecDeque<u64> =
                std::collections::VecDeque::with_capacity(20);
            let mut nudged = false;
            let mut nudge_msg_idx: usize = 0;

            loop {
                match self.execute_turn().await {
                    TurnPhaseResult::Text(content) => {
                        round += 1;
                        if round > MAX_TOOL_ROUNDS {
                            return "subtask: hard tool-round cap reached".into();
                        }
                        if content.trim().is_empty() {
                            if round >= 5 {
                                return "subtask: model returned only thinking content".into();
                            }
                            continue;
                        }
                        match self
                            .handle_loop_check(
                                &content,
                                &mut recent_fps,
                                &mut nudged,
                                &mut nudge_msg_idx,
                                "subtask_text",
                            )
                            .await
                        {
                            Some(true) => continue,
                            Some(false) => return "subtask: loop detected".into(),
                            None => {}
                        }
                        self.session.append_message(SessionMessage::Text {
                            role: Role::Assistant,
                            content: content.clone(),
                            images: vec![],
                        });
                        return content;
                    }
                    TurnPhaseResult::ToolCalls(text, calls) => {
                        round += 1;
                        if round > MAX_TOOL_ROUNDS {
                            return "subtask: hard tool-round cap reached".into();
                        }
                        match self
                            .handle_loop_check(
                                &text,
                                &mut recent_fps,
                                &mut nudged,
                                &mut nudge_msg_idx,
                                "subtask_tool",
                            )
                            .await
                        {
                            Some(true) => continue,
                            Some(false) => return "subtask: loop detected".into(),
                            None => {}
                        }
                        self.handle_tool_calls(text, calls).await;
                    }
                    TurnPhaseResult::Cancelled => return "subtask cancelled".into(),
                    TurnPhaseResult::Error(e) => return format!("subtask error: {e}"),
                    TurnPhaseResult::Quit => return "subtask quit".into(),
                }
            }
        }) // Box::pin
    }

    pub async fn run(mut self) {
        loop {
            let rx = self
                .action_rx
                .as_mut()
                .expect("root AgentTask must have action_rx");
            let action = match rx.recv().await {
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
                UserAction::ChangeModel(model) => {
                    self.ollama.set_model(model);
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
                output: format!(
                    "'{}' is not available at this agent level. \
                     Use delegate_task to have a sub-agent perform this operation.",
                    call.name
                ),
                is_error: true,
                images: vec![],
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
    fn test_orchestrator_system_prompt_empty_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "");
        assert!(prompt.contains("orchestration layer"));
        assert!(prompt.contains("/tmp/test"));
        assert!(!prompt.contains("Stored memories"));
    }

    #[test]
    fn test_orchestrator_system_prompt_with_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "Memory 1: hello");
        assert!(prompt.contains("Stored memories"));
        assert!(prompt.contains("Memory 1: hello"));
    }

    #[test]
    fn test_coordinator_system_prompt() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(1, dir, "");
        assert!(prompt.contains("coordination layer"));
        assert!(prompt.contains("/tmp/test"));
    }

    #[test]
    fn test_worker_system_prompt() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(2, dir, "");
        assert!(prompt.contains("execution layer"));
        assert!(prompt.contains("/tmp/test"));
    }

    #[test]
    fn tools_for_depth_orchestrator_has_delegate_and_write_tools() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"append_file"));
        assert!(names.contains(&"edit_file"));
        assert!(!names.contains(&"read_file"));
        assert!(!names.contains(&"glob_files"));
        assert_eq!(tools.len(), 4);
    }

    #[test]
    fn tools_for_depth_coordinator_has_search_and_delegate() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 1);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"glob_files"));
        assert!(names.contains(&"search_files"));
        assert!(names.contains(&"list_dir"));
        assert!(!names.contains(&"read_file"));
        assert!(!names.contains(&"write_file"));
    }

    #[test]
    fn tools_for_depth_worker_has_file_tools_no_delegate() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"edit_file"));
        assert!(!names.contains(&"delegate_task"));
    }

    #[test]
    fn tools_for_depth_coordinator_exact_set() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 1);
        let names: std::collections::HashSet<_> = tools.iter().map(|t| t.name.as_str()).collect();
        let expected: std::collections::HashSet<&str> =
            ["delegate_task", "glob_files", "search_files", "list_dir"].into();
        assert_eq!(names, expected);
    }

    #[test]
    fn tools_for_depth_worker_excludes_only_delegate() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let worker_tools = tools_for_depth(&all, 2);
        let mut expected: std::collections::HashSet<_> =
            all.iter().map(|t| t.name.as_str()).collect();
        expected.remove("delegate_task");
        let worker_names: std::collections::HashSet<_> =
            worker_tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(worker_names, expected);
    }

    #[test]
    fn tools_for_depth_3_same_as_depth_2() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let mut d2: Vec<_> = tools_for_depth(&all, 2)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let mut d3: Vec<_> = tools_for_depth(&all, 3)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        d2.sort();
        d3.sort();
        assert_eq!(d2, d3);
    }

    #[test]
    fn truncate_subtask_result_short_unchanged() {
        let s = "hello".to_string();
        assert_eq!(truncate_subtask_result(s, 100), "hello");
    }

    #[test]
    fn truncate_subtask_result_long_gets_suffix() {
        let s = "a".repeat(5000);
        let result = truncate_subtask_result(s, 100);
        assert!(result.starts_with("aaaa"));
        assert!(result.contains("[truncated:"));
        assert!(result.contains("5000 chars total"));
    }
}
