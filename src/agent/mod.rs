mod loop_detect;
mod subtask;
mod turn;

use std::{path::PathBuf, sync::Arc};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    mcp::McpRegistry,
    ollama::OllamaClient,
    prompts::{PLAN_PROMPT_APPENDIX, THOROUGH_PROMPT_APPENDIX},
    session::{Session, SessionMessage},
    tools::selection::tools_for_depth,
    types::{AgentEvent, AgentMode, Role, ToolDefinition},
};

pub use crate::tools::selection::is_flat_model;

pub(super) const MAX_TOOL_ROUNDS: u32 = 25;

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
}

pub struct AgentTask {
    pub(super) ollama: Arc<OllamaClient>,
    pub(super) mcp: Arc<McpRegistry>,
    pub(super) working_dir: PathBuf,
    /// Depth-filtered tools for this node.
    pub(super) tools: Vec<ToolDefinition>,
    /// Full unfiltered tool set (built-ins + MCP), used to compute child tool sets.
    pub(super) all_tools: Vec<ToolDefinition>,
    pub(super) event_tx: mpsc::Sender<AgentEvent>,
    /// None for subtasks (no interactive cancel).
    pub(super) action_rx: Option<mpsc::Receiver<UserAction>>,
    /// Actions received mid-turn that weren't Cancel/Quit — processed after the turn ends.
    pub(super) pending_actions: std::collections::VecDeque<UserAction>,
    pub(super) session: Session,
    pub(super) system_prompt: String,
    pub(super) depth: usize,
    pub(super) flat: bool,
    pub(super) mode: AgentMode,
    pub(super) mcp_tools_context: String,
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
        Self {
            ollama: cfg.ollama,
            mcp: cfg.mcp,
            working_dir,
            tools,
            all_tools: cfg.all_tools,
            event_tx: cfg.event_tx,
            action_rx: Some(cfg.action_rx),
            pending_actions: std::collections::VecDeque::new(),
            session: cfg.session,
            system_prompt,
            depth: 0,
            flat: cfg.flat,
            mode: cfg.mode,
            mcp_tools_context: cfg.mcp_tools_context,
        }
    }

    pub async fn run(mut self) {
        loop {
            let action = if let Some(a) = self.pending_actions.pop_front() {
                a
            } else {
                let rx = self
                    .action_rx
                    .as_mut()
                    .expect("root AgentTask must have action_rx");
                match rx.recv().await {
                    Some(a) => a,
                    None => return,
                }
            };

            match action {
                UserAction::Quit => return,
                UserAction::Cancel => {
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::ClearHistory => {
                    self.session.messages.clear();
                    self.session.plan.clear();
                    self.save_or_emit_error().await;
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::ChangeModel(model) => {
                    self.flat = is_flat_model(&model);
                    self.refresh_tools_and_prompt();
                    self.ollama.set_model(model);
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::ToggleFlat(new_flat) => {
                    self.flat = new_flat;
                    self.refresh_tools_and_prompt();
                    self.emit(AgentEvent::TurnDone).await;
                }
                UserAction::SendMessage { text, images, mode } => {
                    if mode != self.mode {
                        self.mode = mode;
                        self.refresh_tools_and_prompt();
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
                    let mut round: u32 = 0;
                    let mut recent_text_fps: std::collections::VecDeque<u64> =
                        std::collections::VecDeque::with_capacity(20);
                    let mut nudged = false;
                    let mut nudge_msg_idx: usize = 0;
                    'turn: loop {
                        match self.execute_turn().await {
                            turn::TurnPhaseResult::Text(content) => {
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
                            turn::TurnPhaseResult::ToolCalls(text, calls) => {
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
                            turn::TurnPhaseResult::Cancelled => {
                                self.emit(AgentEvent::TurnDone).await;
                                break 'turn;
                            }
                            turn::TurnPhaseResult::Error(e) => {
                                warn!(error = %e, "turn error");
                                self.emit(AgentEvent::Error(e.to_string())).await;
                                break 'turn;
                            }
                            turn::TurnPhaseResult::Quit => return,
                        }
                    }
                }
            }
        }
    }
}
