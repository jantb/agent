use tracing::{debug, warn};

use crate::{
    mcp::McpRegistry,
    memory,
    prompts::{system_prompt_for_depth, PLAN_PROMPT_APPENDIX, THOROUGH_PROMPT_APPENDIX},
    session::SessionMessage,
    tools::{execute_built_in_with_mode, selection::tools_for_depth},
    types::{
        AgentEvent, AgentMode, OneshotTx, PlanItem, Role, ToolCall, ToolDefinition, ToolResult,
        ToolSource, TurnOutcome,
    },
};

use super::{
    loop_detect::{check_repeated_text, text_fingerprint, truncate_subtask_result},
    AgentTask,
};

pub(super) enum TurnPhaseResult {
    Text(String),
    ToolCalls(String, Vec<ToolCall>),
    Cancelled,
    Error(anyhow::Error),
    Quit,
}

impl AgentTask {
    pub(super) fn history(&self) -> Vec<crate::types::Message> {
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
    pub(super) async fn handle_loop_check(
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

    pub(super) async fn emit(&self, event: AgentEvent) {
        if let Err(e) = self.event_tx.send(event).await {
            tracing::error!("failed to send agent event: {e}");
        }
    }

    pub(super) fn refresh_tools_and_prompt(&mut self) {
        self.tools = tools_for_depth(&self.all_tools, self.depth, self.flat, self.mode);
        let idx = memory::build_memory_index(&self.working_dir);
        self.system_prompt = system_prompt_for_depth(
            self.depth,
            &self.working_dir,
            &idx,
            &self.mcp_tools_context,
            self.flat,
        );
        match self.mode {
            AgentMode::Plan => self.system_prompt.push_str(PLAN_PROMPT_APPENDIX),
            AgentMode::Thorough => self.system_prompt.push_str(THOROUGH_PROMPT_APPENDIX),
            AgentMode::Oneshot => {}
        }
    }

    pub(super) async fn save_or_emit_error(&mut self) {
        if self.depth > 0 {
            return; // subtasks don't persist their isolated sessions
        }
        match self.session.save(&self.working_dir) {
            Ok(()) => debug!("session saved"),
            Err(e) => self.emit(AgentEvent::Error(e.to_string())).await,
        }
    }

    pub(super) async fn execute_turn(&mut self) -> TurnPhaseResult {
        let history = self.history();
        let Some(mut rx) = self.action_rx.take() else {
            return match self
                .ollama
                .stream_turn(&history, &self.tools, self.event_tx.clone(), true)
                .await
            {
                Err(e) => TurnPhaseResult::Error(e),
                Ok(TurnOutcome::Text(content)) => TurnPhaseResult::Text(content),
                Ok(TurnOutcome::ToolCalls(text, calls)) => TurnPhaseResult::ToolCalls(text, calls),
            };
        };
        let mut deferred: Vec<super::UserAction> = Vec::new();
        let result = {
            let outcome_fut = self.ollama.stream_turn(
                &history,
                &self.tools,
                self.event_tx.clone(),
                self.depth == 0,
            );
            tokio::pin!(outcome_fut);
            loop {
                tokio::select! {
                    action = rx.recv() => {
                        match action {
                            Some(super::UserAction::Cancel) | None => break TurnPhaseResult::Cancelled,
                            Some(super::UserAction::Quit) => break TurnPhaseResult::Quit,
                            Some(other) => deferred.push(other),
                        }
                    }
                    outcome = &mut outcome_fut => {
                        break match outcome {
                            Err(e) => TurnPhaseResult::Error(e),
                            Ok(TurnOutcome::Text(content)) => TurnPhaseResult::Text(content),
                            Ok(TurnOutcome::ToolCalls(text, calls)) => TurnPhaseResult::ToolCalls(text, calls),
                        };
                    }
                }
            }
        };
        self.action_rx = Some(rx);
        for action in deferred {
            self.pending_actions.push_back(action);
        }
        result
    }

    pub(super) async fn handle_text_turn(&mut self, content: String) {
        self.session.append_message(SessionMessage::Text {
            role: Role::Assistant,
            content,
            images: vec![],
        });
        self.save_or_emit_error().await;
        self.emit(AgentEvent::TurnDone).await;
    }

    pub(super) async fn handle_tool_calls(&mut self, text: String, calls: Vec<ToolCall>) {
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
            } else if call.name == "update_plan" {
                self.handle_update_plan(&call).await
            } else {
                Self::dispatch_tool(&call, &self.tools, &self.working_dir, &self.mcp, self.mode)
                    .await
            };
            debug!(tool = %call.name, is_error = result.is_error, "tool dispatch done");

            // Subtask findings flow into the orchestrator's context (truncated below)
            // but are not dumped to the TUI — keeps the terminal readable while the
            // orchestrator still has the full analysis to reason over.
            let display_result = if call.name == "delegate_task" {
                ToolResult {
                    output: format!(
                        "[subtask done: {} chars passed to orchestrator context]",
                        result.output.chars().count()
                    ),
                    ..result.clone()
                }
            } else {
                result.clone()
            };
            self.emit(AgentEvent::ToolCompleted(display_result)).await;
            let stored_content = if call.name == "delegate_task" && self.depth == 0 {
                truncate_subtask_result(result.output.clone(), 6000)
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
                self.refresh_tools_and_prompt();
            }
        }
        self.save_or_emit_error().await;
    }

    pub(super) async fn handle_update_plan(&mut self, call: &ToolCall) -> ToolResult {
        if self.depth > 0 {
            return ToolResult {
                call_id: call.id.clone(),
                output: "update_plan is only available to the root agent. \
                         Complete your current task and return results; \
                         the root will update the plan."
                    .into(),
                is_error: true,
                images: vec![],
            };
        }
        match call.arguments.get("items").and_then(|v| v.as_array()) {
            None => ToolResult {
                call_id: call.id.clone(),
                output: "error: 'items' array is required".into(),
                is_error: true,
                images: vec![],
            },
            Some(arr) if arr.is_empty() => ToolResult {
                call_id: call.id.clone(),
                output: "error: 'items' must not be empty".into(),
                is_error: true,
                images: vec![],
            },
            Some(arr) => {
                match serde_json::from_value::<Vec<PlanItem>>(serde_json::Value::Array(arr.clone()))
                {
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        output: format!("error: invalid plan items: {e}"),
                        is_error: true,
                        images: vec![],
                    },
                    Ok(items) => {
                        let n = items.len();
                        let pending = items
                            .iter()
                            .filter(|i| i.status == crate::types::PlanStatus::Pending)
                            .count();
                        let in_progress = items
                            .iter()
                            .filter(|i| i.status == crate::types::PlanStatus::InProgress)
                            .count();
                        let completed = items
                            .iter()
                            .filter(|i| i.status == crate::types::PlanStatus::Completed)
                            .count();
                        self.session.plan = items.clone();
                        self.emit(AgentEvent::PlanUpdated(items)).await;
                        ToolResult {
                            call_id: call.id.clone(),
                            output: format!(
                                "plan updated: {n} items ({pending} pending, \
                                 {in_progress} in_progress, {completed} completed)"
                            ),
                            is_error: false,
                            images: vec![],
                        }
                    }
                }
            }
        }
    }

    pub(super) async fn dispatch_tool(
        call: &ToolCall,
        tools: &[ToolDefinition],
        working_dir: &std::path::Path,
        mcp: &McpRegistry,
        mode: AgentMode,
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
                ToolSource::BuiltIn => execute_built_in_with_mode(call, working_dir, mode).await,
                ToolSource::Mcp => mcp.execute(call).await,
            },
        }
    }
}
