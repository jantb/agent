use std::sync::Arc;

use tracing::warn;

use crate::{
    prompts::{system_prompt_for_depth, PLAN_SUBTASK_APPENDIX},
    session::{Session, SessionMessage},
    tools::selection::tools_for_depth,
    types::{AgentEvent, AgentMode, Role},
};

use super::{loop_detect::truncate_subtask_result, AgentTask};

pub(super) static SUBTASK_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// RAII guard: emits SubtaskExit via try_send in Drop, ensuring the event
/// is always delivered even if the subtask future is cancelled or panics.
/// Disarm before the normal explicit emit to avoid double-emission.
pub(super) struct SubtaskExitGuard {
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
    depth: usize,
    armed: bool,
}

impl SubtaskExitGuard {
    pub(super) fn new(tx: tokio::sync::mpsc::Sender<AgentEvent>, depth: usize) -> Self {
        Self {
            tx,
            depth,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SubtaskExitGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self
                .tx
                .try_send(AgentEvent::SubtaskExit { depth: self.depth });
        }
    }
}

impl AgentTask {
    /// Run an isolated subtask: fresh session, depth-filtered tools, depth+1.
    /// The parent session is never touched during child execution.
    pub(super) async fn run_subtask(
        &self,
        prompt: String,
        custom_system: Option<String>,
    ) -> String {
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

        // Only Plan mode propagates — Thorough's interview_question is root-only;
        // Oneshot is the neutral child default.
        let child_mode = if self.mode == AgentMode::Plan {
            AgentMode::Plan
        } else {
            AgentMode::Oneshot
        };
        let child_tools = tools_for_depth(&self.all_tools, child_depth, self.flat, child_mode);
        let mut child_system = system_prompt_for_depth(
            child_depth,
            &self.working_dir,
            "",
            &self.mcp_tools_context,
            self.flat,
        );
        if let Some(extra) = custom_system {
            child_system.push_str("\n\n## Instructions from orchestrator\n");
            child_system.push_str(&extra);
        }
        if child_mode == AgentMode::Plan {
            child_system.push_str(PLAN_SUBTASK_APPENDIX);
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
            pending_actions: std::collections::VecDeque::new(),
            session: child_session,
            system_prompt: child_system,
            depth: child_depth,
            flat: self.flat,
            mode: child_mode,
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
    /// Boxed to break the async recursion cycle
    /// (run_single_task → handle_tool_calls → run_subtask → run_single_task).
    pub(super) fn run_single_task(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send + '_>> {
        Box::pin(async move {
            use super::turn::TurnPhaseResult;

            let mut round = 0u32;
            let mut recent_fps: std::collections::VecDeque<u64> =
                std::collections::VecDeque::with_capacity(20);
            let mut nudged = false;
            let mut nudge_msg_idx: usize = 0;

            loop {
                match self.execute_turn().await {
                    TurnPhaseResult::Text(content) => {
                        round += 1;
                        if round > super::MAX_TOOL_ROUNDS {
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
                        self.session
                            .append_message(crate::session::SessionMessage::Text {
                                role: crate::types::Role::Assistant,
                                content: content.clone(),
                                images: vec![],
                            });
                        return content;
                    }
                    TurnPhaseResult::ToolCalls(text, calls) => {
                        round += 1;
                        if round > super::MAX_TOOL_ROUNDS {
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
        })
    }
}
