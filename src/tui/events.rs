use tokio::sync::mpsc;

use crate::agent::UserAction;
use crate::app::{App, InterviewPickerState};
use crate::types::AgentEvent;

// NOTE: AgentEvent handling is intentionally duplicated across headless.rs and tui/events.rs.
// When adding a new AgentEvent variant, update BOTH.

/// Handle one `AgentEvent`.  Returns `false` when the channel closed and the
/// loop should exit.
pub async fn handle_agent_event(
    event: Option<AgentEvent>,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
    current_depth: &mut usize,
) -> bool {
    match event {
        Some(AgentEvent::ThinkingStarted) => app.set_thinking(true),
        Some(AgentEvent::ThinkingDelta(d)) => app.append_thinking_text(&d),
        Some(AgentEvent::ThinkingDone) => app.flush_thinking(),
        Some(AgentEvent::TextDelta(d)) => app.append_streaming_text(&d),
        Some(AgentEvent::ToolRequested(c)) => app.add_tool_call(&c),
        Some(AgentEvent::ToolCompleted(r)) => app.add_tool_result(&r),
        Some(AgentEvent::TurnStats {
            eval_count,
            eval_duration_ns,
            prompt_eval_count,
        }) => {
            app.update_turn_stats(
                eval_count,
                eval_duration_ns,
                prompt_eval_count,
                *current_depth,
            );
        }
        Some(AgentEvent::TurnDone) => {
            app.finish_assistant_turn();
            if let Some((text, images)) = app.dequeue_message() {
                app.start_assistant_turn();
                if let Err(e) = action_tx
                    .send(UserAction::SendMessage {
                        text,
                        images,
                        mode: app.mode,
                    })
                    .await
                {
                    tracing::error!("failed to send queued message: {e}");
                }
            }
        }
        Some(AgentEvent::Error(e)) => {
            app.fail_active_node();
            app.finish_assistant_turn();
            app.message_queue.clear();
            app.set_error(e);
        }
        Some(AgentEvent::LoopDetected) => {
            app.fail_active_node();
            app.finish_assistant_turn();
            app.message_queue.clear();
            app.set_error("Loop detected — model was repeating itself".into());
        }
        Some(AgentEvent::SubtaskEnter { depth, label }) => {
            *current_depth = depth;
            app.enter_subtask(depth, label);
        }
        Some(AgentEvent::SubtaskExit { depth }) => {
            *current_depth = depth.saturating_sub(1);
            app.exit_subtask(depth);
        }
        Some(AgentEvent::InterviewQuestion {
            question,
            suggestions,
            answer_tx,
        }) => {
            app.interview_picker = Some(InterviewPickerState {
                question,
                suggestions,
                selected: 0,
                custom_input: String::new(),
                custom_mode: false,
                answer_tx: Some(answer_tx.0),
            });
        }
        Some(AgentEvent::PlanUpdated(items)) => {
            app.apply_plan_update(items);
        }
        None => return false,
    }
    true
}
