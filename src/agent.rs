use std::{path::PathBuf, sync::Arc};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    mcp::McpRegistry,
    memory,
    ollama::OllamaClient,
    session::{Session, SessionMessage},
    tools::execute_built_in,
    types::{
        AgentEvent, AgentMode, CavemanLevel, OneshotTx, Role, ToolCall, ToolDefinition, ToolResult,
        ToolSource, TurnOutcome,
    },
};

/// Returns true if the model should use flat (single-level) mode.
/// Dense models are too slow for multi-level delegation.
pub fn is_flat_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("31b") || m.contains("dense")
}

/// Build a system-prompt section that lists all MCP-provided tools by name and description.
pub fn mcp_tools_prompt_section(mcp_tools: &[ToolDefinition]) -> String {
    if mcp_tools.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## MCP tools\n\
         These tools are provided by connected servers. Use them instead of manual alternatives when applicable.\n",
    );
    for t in mcp_tools {
        s.push_str(&format!("- `{}`: {}\n", t.name, t.description));
    }
    s
}

/// Returns the system prompt for a given depth level.
/// When `flat` is true, all depths use the worker prompt (single-level agent).
pub fn system_prompt_for_depth(
    depth: usize,
    working_dir: &std::path::Path,
    memory_index: &str,
    mcp_tools_context: &str,
    flat: bool,
    caveman: CavemanLevel,
) -> String {
    let mut prompt = if flat {
        flat_system_prompt(working_dir, memory_index, mcp_tools_context)
    } else {
        match depth {
            0 => orchestrator_system_prompt(working_dir, memory_index, mcp_tools_context),
            1 => coordinator_system_prompt(working_dir, mcp_tools_context),
            _ => worker_system_prompt(working_dir, mcp_tools_context),
        }
    };
    if caveman.is_active() {
        prompt = compress_prompt(&prompt, caveman);
        prompt.push_str(caveman_appendix(caveman));
    }
    prompt
}

fn flat_system_prompt(
    working_dir: &std::path::Path,
    memory_index: &str,
    mcp_tools_context: &str,
) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are a coding AI agent. Working directory: {dir}
You help users write, edit, debug, and understand code. Prefer idiomatic, compact solutions.
You have full tool access. Complete everything yourself — no delegation needed.

## Rules
1. Use read_file, write_file, edit_file, search_files, glob_files, list_dir as needed.
2. For long output (>20 lines), write to a file and return the path + 1-sentence summary.
3. Sandboxed to {dir}.
4. On error, report clearly rather than looping.
5. Return concise summaries. Use file:line references.
6. Think briefly. Act fast.
7. TDD: when fixing bugs or adding features, write a failing test first, run it to confirm failure, implement the fix, then run the test again to verify it passes."
    );
    if !memory_index.is_empty() {
        prompt.push_str("\n\n## Stored memories\n");
        prompt.push_str(memory_index);
    }
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

fn orchestrator_system_prompt(
    working_dir: &std::path::Path,
    memory_index: &str,
    mcp_tools_context: &str,
) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the orchestration layer of a coding AI agent. Working directory: {dir}
You help users write, edit, debug, and understand code. Prefer idiomatic, compact solutions.
Your ONLY tool is `delegate_task`. You MUST call it for EVERY request — no exceptions.
You do NOT have bash, shell, or command execution access. Use only the provided tools.

## Rules
1. Break the goal into focused subtasks. Delegate each via `delegate_task` sequentially.
2. Prompts must be self-contained — sub-agents have no context. Include file paths, content, and exact instructions.
3. For follow-up questions, delegate a fresh task — do NOT answer from memory.
4. For long output (reports, listings, code), tell sub-agents to write results to a file rather than returning inline.
5. After delegation, synthesize a concise answer (1-3 paragraphs).
6. Think briefly. Act fast.
7. TDD: when fixing bugs or adding features, instruct sub-agents to write a failing test first, run it, implement the fix, then run the test again.

## Examples
- User: \"Write hello to out.txt\" → delegate_task(\"Write the text 'hello' to the file out.txt\")
- User: \"Search for pub fn in src/\" → delegate_task(\"Search .rs files under src/ for 'pub fn'. List each function name and file.\")
- User: \"Now count them\" → delegate_task(\"Search .rs files under src/ for 'pub fn'. Return a total count and the top 5 files by count.\")"
    );
    if !memory_index.is_empty() {
        prompt.push_str("\n\n## Stored memories\n");
        prompt.push_str(memory_index);
    }
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

fn coordinator_system_prompt(working_dir: &std::path::Path, mcp_tools_context: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the coordination layer of a coding AI agent. Working directory: {dir}
Tools: glob_files, search_files, list_dir, delegate_task.
You do NOT have bash, shell, or command execution access. Use only the provided tools.

## Rules
1. Search FIRST with your own tools. NEVER delegate search/analysis — do it yourself.
2. Only delegate_task for file I/O (read_file, write_file, edit_file) — you lack those tools.
3. After search_files returns, format and return results immediately. No re-searching or over-thinking.
4. delegate_task prompts must be self-contained with full file paths.
5. Think briefly. Act fast."
    );
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

fn worker_system_prompt(working_dir: &std::path::Path, mcp_tools_context: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the execution layer of a coding AI agent. Working directory: {dir}
You have full file-tool access. No delegation — complete everything yourself.
Write idiomatic, compact code. Fix root causes, not symptoms.
You do NOT have bash, shell, or command execution access. Use only the provided tools.

## Rules
1. Execute the task completely. Use read_file, write_file, edit_file, search_files as needed.
2. Read only what is necessary; write/edit only what is asked.
3. Sandboxed to {dir}.
4. On error, report clearly rather than looping.
5. For long output (>20 lines), write to a file and return the path + 1-sentence summary.
6. Return concise summaries (under 500 words). Use file:line references.
7. TDD: when fixing bugs or adding features, write a failing test first, run it to confirm failure, implement the fix, then run the test again to verify it passes."
    );
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

const PLAN_PROMPT_APPENDIX: &str = "\n\n## Plan mode\n\
You have the `interview_question` tool and you MUST use it before taking any action.\n\
Your workflow has three mandatory phases:\n\
1. **Clarify**: Ask probing questions via `interview_question` to fully understand scope, constraints, \
edge cases, and preferences. Do not skip this even if the request seems clear — surface hidden assumptions. \
Ask at least one question.\n\
2. **Plan**: After the user answers, present a concrete numbered plan of what you will do. \
Ask for approval or changes before you proceed.\n\
3. **Execute**: Only after explicit user approval, carry out the plan.\n\
If the user answers \"[DONE]\", move to the next phase immediately.";

const THOROUGH_PROMPT_APPENDIX: &str = "\n\n## Thorough mode\n\
You have the `interview_question` tool. You MUST call it at least once before doing any work — \
ask about any ambiguity, unclear requirements, or choices with significant implementation impact. \
Do not default to assumptions when you could ask. Prefer asking over guessing. \
If the user answers \"[DONE]\", stop asking and proceed.";

fn caveman_appendix(level: CavemanLevel) -> &'static str {
    match level {
        CavemanLevel::Off => "",
        CavemanLevel::Lite => {
            "\n\n## Output style: concise\n\
Remove filler words, pleasantries, and hedging. Keep grammar intact. \
No \"just\", \"really\", \"basically\", \"I think\". \
Be direct. Code unchanged."
        }
        CavemanLevel::Full => {
            "\n\n## Output style: caveman\n\
Terse. Fragments OK. Drop articles (the/a/an), filler, pleasantries, hedging. \
Short synonyms. Code/URLs/commands unchanged. \
Pattern: [thing] [action] [reason]. [next step]."
        }
        CavemanLevel::Ultra => {
            "\n\n## Output style: ultra-terse\n\
Max compress. Telegraphic. No articles/filler/hedge. \
Abbrev when unambiguous. Code unchanged. \
Pattern: [thing] [verb] [why]. [next]."
        }
    }
}

/// Compress a system prompt by removing filler based on caveman level.
fn compress_prompt(prompt: &str, level: CavemanLevel) -> String {
    match level {
        CavemanLevel::Off => prompt.to_string(),
        CavemanLevel::Lite => {
            // Collapse multiple blank lines into a single blank line
            let mut result = String::with_capacity(prompt.len());
            let mut prev_blank = false;
            for line in prompt.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    if !prev_blank {
                        result.push('\n');
                        prev_blank = true;
                    }
                } else {
                    prev_blank = false;
                    result.push_str(line);
                    result.push('\n');
                }
            }
            result
        }
        CavemanLevel::Full => {
            // Strip articles + filler words, collapse whitespace
            let mut result = String::with_capacity(prompt.len());
            let mut prev_blank = false;
            for line in prompt.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    if !prev_blank {
                        result.push('\n');
                        prev_blank = true;
                    }
                    continue;
                }
                if prev_blank {
                    result.push('\n');
                }
                prev_blank = false;
                let stripped = strip_filler_words(line);
                result.push_str(&stripped);
                result.push('\n');
            }
            result
        }
        CavemanLevel::Ultra => {
            // Aggressive: strip articles + filler + compress sentences
            let mut result = String::with_capacity(prompt.len());
            for line in prompt.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let stripped = strip_filler_words(trimmed);
                if !stripped.trim().is_empty() {
                    result.push_str(stripped.trim());
                    result.push('\n');
                }
            }
            result
        }
    }
}

fn strip_filler_words(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    // Preserve leading whitespace
    let leading: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    result.push_str(&leading);
    let content = &line[leading.len()..];
    let mut first = true;
    for word in content.split_whitespace() {
        let lower = word.to_lowercase();
        // Strip pure filler; keep articles at sentence start for readability
        let is_filler = matches!(
            lower.trim_matches(|c: char| !c.is_alphanumeric()),
            "the"
                | "a"
                | "an"
                | "just"
                | "really"
                | "basically"
                | "simply"
                | "actually"
                | "very"
                | "quite"
                | "perhaps"
                | "please"
        );
        if is_filler {
            continue;
        }
        if !first {
            result.push(' ');
        }
        first = false;
        result.push_str(word);
    }
    result
}

/// Filter the full tool set to the subset appropriate for a given depth.
/// When `flat` is true, all depths get worker tools (no delegate_task).
/// depth 0 (orchestrator): only delegate_task
/// depth 1 (coordinator): delegate_task + navigation tools
/// depth 2+ (worker):     all tools except delegate_task
pub fn tools_for_depth(
    all_tools: &[ToolDefinition],
    depth: usize,
    flat: bool,
    mode: AgentMode,
) -> Vec<ToolDefinition> {
    let mut tools: Vec<ToolDefinition> = if flat {
        all_tools
            .iter()
            .filter(|t| t.name != "delegate_task")
            .cloned()
            .collect()
    } else {
        const COORDINATOR_TOOLS: &[&str] =
            &["delegate_task", "glob_files", "search_files", "list_dir"];
        match depth {
            0 => all_tools
                .iter()
                .filter(|t| t.name == "delegate_task")
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
    };
    if matches!(mode, AgentMode::Plan | AgentMode::Thorough) && depth == 0 {
        tools.push(crate::tools::interview_question_def());
    }
    tools
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
    SendMessage {
        text: String,
        images: Vec<String>,
        mode: AgentMode,
    },
    Cancel,
    Quit,
    ClearHistory,
    ChangeModel(String),
    ToggleFlat(bool),
    SetCaveman(CavemanLevel),
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
    flat: bool,
    caveman: CavemanLevel,
    mode: AgentMode,
    mcp_tools_context: String,
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
    /// Single-level mode: no delegation hierarchy.
    pub flat: bool,
    pub caveman: CavemanLevel,
    /// Agent mode: controls which tools are available and how the agent behaves.
    pub mode: AgentMode,
    pub mcp_tools_context: String,
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
        let tools = tools_for_depth(&cfg.all_tools, 0, cfg.flat, cfg.mode);
        let mut system_prompt = cfg.system_prompt;
        match cfg.mode {
            AgentMode::Plan => system_prompt.push_str(PLAN_PROMPT_APPENDIX),
            AgentMode::Thorough => system_prompt.push_str(THOROUGH_PROMPT_APPENDIX),
            AgentMode::Oneshot => {}
        }
        if cfg.caveman.is_active() {
            system_prompt = compress_prompt(&system_prompt, cfg.caveman);
            system_prompt.push_str(caveman_appendix(cfg.caveman));
        }
        Self {
            ollama: cfg.ollama,
            mcp: cfg.mcp,
            working_dir,
            tools,
            all_tools: cfg.all_tools,
            event_tx: cfg.event_tx,
            action_rx: Some(cfg.action_rx),
            session: cfg.session,
            system_prompt,
            depth: 0,
            flat: cfg.flat,
            caveman: cfg.caveman,
            mode: cfg.mode,
            mcp_tools_context: cfg.mcp_tools_context,
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
                let custom_system = call
                    .arguments
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let answer = self.run_subtask(prompt, custom_system).await;
                ToolResult {
                    call_id: call.id.clone(),
                    output: answer,
                    is_error: false,
                    images: vec![],
                }
            } else if call.name == "interview_question" {
                let question = call
                    .arguments
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let suggestions: Vec<String> = call
                    .arguments
                    .get("suggestions")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let (tx, rx) = tokio::sync::oneshot::channel::<String>();
                self.emit(AgentEvent::InterviewQuestion {
                    question,
                    suggestions,
                    answer_tx: OneshotTx(tx),
                })
                .await;
                let answer = rx.await.unwrap_or_else(|_| "cancelled".into());
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
                self.system_prompt = system_prompt_for_depth(
                    self.depth,
                    &self.working_dir,
                    &idx,
                    &self.mcp_tools_context,
                    self.flat,
                    self.caveman,
                );
            }
        }
        self.save_or_emit_error().await;
    }

    /// Run an isolated subtask: fresh session, depth-filtered tools, depth+1.
    /// The parent session is never touched during child execution.
    async fn run_subtask(&self, prompt: String, custom_system: Option<String>) -> String {
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

        let child_tools =
            tools_for_depth(&self.all_tools, child_depth, self.flat, AgentMode::Oneshot);
        let mut child_system = system_prompt_for_depth(
            child_depth,
            &self.working_dir,
            "",
            &self.mcp_tools_context,
            self.flat,
            self.caveman,
        );
        if let Some(extra) = custom_system {
            child_system.push_str("\n\n## Instructions from orchestrator\n");
            child_system.push_str(&extra);
        }
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
            flat: self.flat,
            caveman: self.caveman,
            mode: AgentMode::Oneshot,
            mcp_tools_context: self.mcp_tools_context.clone(),
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
                    self.flat = is_flat_model(&model);
                    self.tools = tools_for_depth(&self.all_tools, self.depth, self.flat, self.mode);
                    let idx = memory::build_memory_index(&self.working_dir);
                    self.system_prompt = system_prompt_for_depth(
                        self.depth,
                        &self.working_dir,
                        &idx,
                        &self.mcp_tools_context,
                        self.flat,
                        self.caveman,
                    );
                    match self.mode {
                        AgentMode::Plan => self.system_prompt.push_str(PLAN_PROMPT_APPENDIX),
                        AgentMode::Thorough => {
                            self.system_prompt.push_str(THOROUGH_PROMPT_APPENDIX)
                        }
                        AgentMode::Oneshot => {}
                    }
                    if self.caveman.is_active() {
                        self.system_prompt = compress_prompt(&self.system_prompt, self.caveman);
                        self.system_prompt.push_str(caveman_appendix(self.caveman));
                    }
                    self.ollama.set_model(model);
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::ToggleFlat(new_flat) => {
                    self.flat = new_flat;
                    self.tools = tools_for_depth(&self.all_tools, self.depth, self.flat, self.mode);
                    let idx = memory::build_memory_index(&self.working_dir);
                    self.system_prompt = system_prompt_for_depth(
                        self.depth,
                        &self.working_dir,
                        &idx,
                        &self.mcp_tools_context,
                        self.flat,
                        self.caveman,
                    );
                    match self.mode {
                        AgentMode::Plan => self.system_prompt.push_str(PLAN_PROMPT_APPENDIX),
                        AgentMode::Thorough => {
                            self.system_prompt.push_str(THOROUGH_PROMPT_APPENDIX)
                        }
                        AgentMode::Oneshot => {}
                    }
                    if self.caveman.is_active() {
                        self.system_prompt = compress_prompt(&self.system_prompt, self.caveman);
                        self.system_prompt.push_str(caveman_appendix(self.caveman));
                    }
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::SetCaveman(level) => {
                    self.caveman = level;
                    let idx = memory::build_memory_index(&self.working_dir);
                    self.system_prompt = system_prompt_for_depth(
                        self.depth,
                        &self.working_dir,
                        &idx,
                        &self.mcp_tools_context,
                        self.flat,
                        self.caveman,
                    );
                    match self.mode {
                        AgentMode::Plan => self.system_prompt.push_str(PLAN_PROMPT_APPENDIX),
                        AgentMode::Thorough => {
                            self.system_prompt.push_str(THOROUGH_PROMPT_APPENDIX)
                        }
                        AgentMode::Oneshot => {}
                    }
                    if self.caveman.is_active() {
                        self.system_prompt = compress_prompt(&self.system_prompt, self.caveman);
                        self.system_prompt.push_str(caveman_appendix(self.caveman));
                    }
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::SendMessage { text, images, mode } => {
                    if mode != self.mode {
                        self.mode = mode;
                        self.tools =
                            tools_for_depth(&self.all_tools, self.depth, self.flat, self.mode);
                        let idx = memory::build_memory_index(&self.working_dir);
                        self.system_prompt = system_prompt_for_depth(
                            self.depth,
                            &self.working_dir,
                            &idx,
                            &self.mcp_tools_context,
                            self.flat,
                            self.caveman,
                        );
                        match self.mode {
                            AgentMode::Plan => self.system_prompt.push_str(PLAN_PROMPT_APPENDIX),
                            AgentMode::Thorough => {
                                self.system_prompt.push_str(THOROUGH_PROMPT_APPENDIX)
                            }
                            AgentMode::Oneshot => {}
                        }
                        if self.caveman.is_active() {
                            self.system_prompt = compress_prompt(&self.system_prompt, self.caveman);
                            self.system_prompt.push_str(caveman_appendix(self.caveman));
                        }
                    }
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
        let prompt = system_prompt_for_depth(0, dir, "", "", false, CavemanLevel::Off);
        assert!(prompt.contains("orchestration layer"));
        assert!(prompt.contains("/tmp/test"));
        assert!(!prompt.contains("Stored memories"));
    }

    #[test]
    fn test_orchestrator_system_prompt_with_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt =
            system_prompt_for_depth(0, dir, "Memory 1: hello", "", false, CavemanLevel::Off);
        assert!(prompt.contains("Stored memories"));
        assert!(prompt.contains("Memory 1: hello"));
    }

    #[test]
    fn test_coordinator_system_prompt() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(1, dir, "", "", false, CavemanLevel::Off);
        assert!(prompt.contains("coordination layer"));
        assert!(prompt.contains("/tmp/test"));
    }

    #[test]
    fn test_worker_system_prompt() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(2, dir, "", "", false, CavemanLevel::Off);
        assert!(prompt.contains("execution layer"));
        assert!(prompt.contains("/tmp/test"));
    }

    #[test]
    fn tools_for_depth_orchestrator_has_only_delegate() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(!names.contains(&"write_file"));
        assert!(!names.contains(&"read_file"));
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn tools_for_depth_coordinator_has_search_and_delegate() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 1, false, AgentMode::Oneshot);
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
        let tools = tools_for_depth(&all, 2, false, AgentMode::Oneshot);
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
        let tools = tools_for_depth(&all, 1, false, AgentMode::Oneshot);
        let names: std::collections::HashSet<_> = tools.iter().map(|t| t.name.as_str()).collect();
        let expected: std::collections::HashSet<&str> =
            ["delegate_task", "glob_files", "search_files", "list_dir"].into();
        assert_eq!(names, expected);
    }

    #[test]
    fn tools_for_depth_worker_excludes_only_delegate() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let worker_tools = tools_for_depth(&all, 2, false, AgentMode::Oneshot);
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
        let mut d2: Vec<_> = tools_for_depth(&all, 2, false, AgentMode::Oneshot)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let mut d3: Vec<_> = tools_for_depth(&all, 3, false, AgentMode::Oneshot)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        d2.sort();
        d3.sort();
        assert_eq!(d2, d3);
    }

    #[test]
    fn flat_mode_all_depths_get_worker_tools() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let d0 = tools_for_depth(&all, 0, true, AgentMode::Oneshot);
        let d1 = tools_for_depth(&all, 1, true, AgentMode::Oneshot);
        let d2 = tools_for_depth(&all, 2, true, AgentMode::Oneshot);
        assert_eq!(d0.len(), d2.len());
        assert_eq!(d1.len(), d2.len());
        assert!(d0.iter().all(|t| t.name != "delegate_task"));
    }

    #[test]
    fn flat_mode_prompt_has_no_delegation() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Off);
        assert!(!prompt.contains("delegate_task"));
        assert!(prompt.contains("coding AI agent"));
    }

    #[test]
    fn thorough_mode_adds_interview_question_at_depth_0() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Thorough);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"interview_question"));
    }

    #[test]
    fn thorough_mode_does_not_add_interview_question_at_depth_2() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Thorough);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"interview_question"));
    }

    #[test]
    fn plan_mode_adds_interview_question_at_depth_0() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"interview_question"));
    }

    #[test]
    fn plan_mode_no_interview_at_depth_2() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"interview_question"));
    }

    #[test]
    fn oneshot_no_interview_any_depth() {
        use crate::tools::built_in_tool_definitions;
        let all = built_in_tool_definitions();
        for depth in 0..3 {
            let tools = tools_for_depth(&all, depth, false, AgentMode::Oneshot);
            let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
            assert!(
                !names.contains(&"interview_question"),
                "depth {depth} should not have interview_question in Oneshot"
            );
        }
    }

    #[test]
    fn plan_prompt_contains_three_phases() {
        assert!(PLAN_PROMPT_APPENDIX.contains("interview_question"));
        assert!(PLAN_PROMPT_APPENDIX.contains("Clarify"));
        assert!(PLAN_PROMPT_APPENDIX.contains("Plan"));
        assert!(PLAN_PROMPT_APPENDIX.contains("Execute"));
    }

    #[test]
    fn thorough_prompt_contains_must_call() {
        assert!(THOROUGH_PROMPT_APPENDIX.contains("interview_question"));
        assert!(THOROUGH_PROMPT_APPENDIX.contains("MUST call it"));
    }

    #[test]
    fn is_flat_model_detects_dense() {
        assert!(is_flat_model("gemma4:31b"));
        assert!(is_flat_model("gemma4:31b-cloud"));
        assert!(!is_flat_model("gemma4:26b"));
        assert!(!is_flat_model("gemma4:e4b"));
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

    #[test]
    fn compress_prompt_off_is_identity() {
        let input = "You are a coding AI agent.\nYou help the users.";
        assert_eq!(compress_prompt(input, CavemanLevel::Off), input);
    }

    #[test]
    fn compress_prompt_lite_collapses_blank_lines() {
        let input = "Line one.\n\n\n\nLine two.\n\n\nLine three.";
        let result = compress_prompt(input, CavemanLevel::Lite);
        // Should collapse multiple blank lines into one
        assert!(!result.contains("\n\n\n"));
        assert!(result.contains("Line one."));
        assert!(result.contains("Line two."));
        assert!(result.contains("Line three."));
    }

    #[test]
    fn compress_prompt_full_strips_articles() {
        let input = "Return the result of a computation.";
        let result = compress_prompt(input, CavemanLevel::Full);
        assert!(!result.contains(" the "));
        assert!(!result.contains(" a "));
        assert!(result.contains("result"));
        assert!(result.contains("computation"));
    }

    #[test]
    fn compress_prompt_full_strips_filler_words() {
        let input = "You should just basically really try to simply do it.";
        let result = compress_prompt(input, CavemanLevel::Full);
        assert!(!result.contains("just"));
        assert!(!result.contains("basically"));
        assert!(!result.contains("really"));
        assert!(!result.contains("simply"));
        assert!(result.contains("try"));
        assert!(result.contains("do"));
    }

    #[test]
    fn compress_prompt_ultra_removes_blank_lines() {
        let input = "Line one.\n\n\nLine two.\n\n";
        let result = compress_prompt(input, CavemanLevel::Ultra);
        assert!(!result.contains("\n\n"));
        assert!(result.contains("Line one."));
        assert!(result.contains("Line two."));
    }

    #[test]
    fn compress_prompt_preserves_code_tokens() {
        // Code-like content should survive compression
        let input = "Use `read_file` and `write_file` as needed.";
        let result = compress_prompt(input, CavemanLevel::Full);
        assert!(result.contains("`read_file`"));
        assert!(result.contains("`write_file`"));
    }

    #[test]
    fn caveman_appendix_off_is_empty() {
        assert!(caveman_appendix(CavemanLevel::Off).is_empty());
    }

    #[test]
    fn caveman_appendix_levels_non_empty() {
        assert!(!caveman_appendix(CavemanLevel::Lite).is_empty());
        assert!(!caveman_appendix(CavemanLevel::Full).is_empty());
        assert!(!caveman_appendix(CavemanLevel::Ultra).is_empty());
    }

    #[test]
    fn caveman_appendix_increasing_terseness() {
        // Each level should be shorter or equal to the previous
        let lite = caveman_appendix(CavemanLevel::Lite);
        let full = caveman_appendix(CavemanLevel::Full);
        let ultra = caveman_appendix(CavemanLevel::Ultra);
        // All should mention code being unchanged
        assert!(lite.contains("Code unchanged") || lite.contains("code unchanged"));
        assert!(full.contains("Code") || full.contains("code"));
        assert!(ultra.contains("Code") || ultra.contains("code"));
    }

    #[test]
    fn compress_prompt_full_reduces_length() {
        let input = "You are a coding AI agent. You help the users write, edit, debug, and understand code. \
            Prefer idiomatic, compact solutions. You have the full tool access. Just basically complete \
            everything yourself — no delegation needed.";
        let original_len = input.len();
        let compressed = compress_prompt(input, CavemanLevel::Full);
        assert!(
            compressed.len() < original_len,
            "Full compression should reduce token count"
        );
    }

    #[test]
    fn system_prompt_caveman_off_unchanged() {
        let dir = std::path::Path::new("/tmp/test");
        let without = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Off);
        assert!(without.contains("You are a coding AI agent"));
        assert!(!without.contains("Output style"));
    }

    #[test]
    fn system_prompt_caveman_lite_adds_appendix() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Lite);
        assert!(prompt.contains("Output style: concise"));
        assert!(prompt.contains("Code unchanged"));
    }

    #[test]
    fn system_prompt_caveman_full_strips_and_appends() {
        let dir = std::path::Path::new("/tmp/test");
        let off = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Off);
        let full = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Full);
        assert!(full.contains("Output style: caveman"));
        assert!(
            full.len() < off.len() + 200,
            "Full prompt should not be much longer than off after compression removes filler"
        );
        let base_portion = full.split("## Output style").next().unwrap();
        assert!(
            !base_portion
                .split_whitespace()
                .any(|w| w == "the" || w == "a" || w == "an"),
            "Articles should be stripped from base prompt in Full mode"
        );
    }

    #[test]
    fn system_prompt_caveman_ultra_most_compressed() {
        let dir = std::path::Path::new("/tmp/test");
        let off = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Off);
        let ultra = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Ultra);
        assert!(ultra.contains("Output style: ultra-terse"));
        let off_base = off.split("## Output style").next().unwrap_or(&off);
        let ultra_base = ultra.split("## Output style").next().unwrap_or(&ultra);
        assert!(
            ultra_base.len() < off_base.len(),
            "Ultra base prompt ({}) should be shorter than Off base ({})",
            ultra_base.len(),
            off_base.len()
        );
    }

    #[test]
    fn system_prompt_caveman_preserves_code_references() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Ultra);
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("write_file"));
        assert!(prompt.contains("edit_file"));
    }

    #[test]
    fn system_prompt_caveman_levels_produce_different_prompts() {
        let dir = std::path::Path::new("/tmp/test");
        let off = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Off);
        let lite = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Lite);
        let full = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Full);
        let ultra = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Ultra);
        assert_ne!(off, lite, "Off and Lite should differ");
        assert_ne!(lite, full, "Lite and Full should differ");
        assert_ne!(full, ultra, "Full and Ultra should differ");
        assert_ne!(off, ultra, "Off and Ultra should differ");
    }

    #[test]
    fn mcp_tools_prompt_section_with_tools() {
        let tools = vec![
            ToolDefinition {
                name: "gradle_build".into(),
                description: "Run gradle build".into(),
                parameters: serde_json::json!({}),
                source: ToolSource::Mcp,
            },
            ToolDefinition {
                name: "gradle_test".into(),
                description: "Run gradle test".into(),
                parameters: serde_json::json!({}),
                source: ToolSource::Mcp,
            },
        ];
        let section = mcp_tools_prompt_section(&tools);
        assert!(section.contains("## MCP tools"));
        assert!(section.contains("`gradle_build`"));
        assert!(section.contains("`gradle_test`"));
        assert!(section.contains("Run gradle build"));
    }

    #[test]
    fn mcp_tools_prompt_section_empty() {
        let section = mcp_tools_prompt_section(&[]);
        assert!(section.is_empty());
    }

    #[test]
    fn system_prompt_includes_mcp_tools_context() {
        let dir = std::path::Path::new("/tmp/test");
        let ctx = "## MCP tools\n- `gradle_build`: Run gradle build\n";
        let prompt = system_prompt_for_depth(0, dir, "", ctx, true, CavemanLevel::Off);
        assert!(prompt.contains("## MCP tools"));
        assert!(prompt.contains("`gradle_build`"));
    }

    #[test]
    fn system_prompt_omits_empty_mcp_context() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", true, CavemanLevel::Off);
        assert!(!prompt.contains("## MCP tools"));
    }
}
