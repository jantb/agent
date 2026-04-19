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

/// Drain the first event plus any additional pending events from the receiver.
/// Returns `(continue_loop, streaming_transitioned_to_idle)`.
/// - `continue_loop = false` when the channel has closed.
/// - `streaming_transitioned_to_idle = true` if any event in the batch
///   transitioned `app.streaming` from true to false.
pub async fn drain_agent_events(
    first: Option<AgentEvent>,
    rx: &mut mpsc::Receiver<AgentEvent>,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
    current_depth: &mut usize,
) -> (bool, bool) {
    let mut streaming_to_idle = false;
    let was_streaming = app.streaming;
    if !handle_agent_event(first, app, action_tx, current_depth).await {
        return (false, was_streaming && !app.streaming);
    }
    if was_streaming && !app.streaming {
        streaming_to_idle = true;
    }
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                let was = app.streaming;
                if !handle_agent_event(Some(ev), app, action_tx, current_depth).await {
                    return (false, streaming_to_idle || (was && !app.streaming));
                }
                if was && !app.streaming {
                    streaming_to_idle = true;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return (false, streaming_to_idle);
            }
        }
    }
    (true, streaming_to_idle)
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::drain_agent_events;
    use crate::agent::UserAction;
    use crate::app::App;
    use crate::types::AgentEvent;

    fn make_app() -> App {
        App::new("test".into(), std::path::PathBuf::from("/tmp"))
    }

    #[tokio::test]
    async fn drain_appends_multiple_text_deltas() {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
        let (action_tx, _action_rx) = mpsc::channel::<UserAction>(16);
        // Send b and c; a is passed as first — keep tx alive so channel stays open
        tx.send(AgentEvent::TextDelta("b".into())).await.unwrap();
        tx.send(AgentEvent::TextDelta("c".into())).await.unwrap();

        let mut app = make_app();
        let mut depth = 0;
        let result = drain_agent_events(
            Some(AgentEvent::TextDelta("a".into())),
            &mut rx,
            &mut app,
            &action_tx,
            &mut depth,
        )
        .await;

        assert_eq!(app.current_streaming_text, "abc");
        assert_eq!(result, (true, false));
        drop(tx); // drop after assertion so channel stays open during drain
    }

    #[tokio::test]
    async fn drain_stops_when_empty() {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
        let (action_tx, _action_rx) = mpsc::channel::<UserAction>(16);
        tx.send(AgentEvent::TextDelta("x".into())).await.unwrap();
        // don't drop tx — channel stays open but empty after first recv

        let mut app = make_app();
        let mut depth = 0;
        let result = drain_agent_events(
            Some(AgentEvent::TextDelta("x".into())),
            &mut rx,
            &mut app,
            &action_tx,
            &mut depth,
        )
        .await;

        assert_eq!(result, (true, false));
        // The one event we put in the channel should have been drained too
        assert_eq!(app.current_streaming_text, "xx");
        // Channel is now empty (tx still open)
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn drain_detects_channel_closed_via_none_first() {
        let (_tx, mut rx) = mpsc::channel::<AgentEvent>(16);
        let (action_tx, _action_rx) = mpsc::channel::<UserAction>(16);

        let mut app = make_app();
        let mut depth = 0;
        let (cont, _) = drain_agent_events(None, &mut rx, &mut app, &action_tx, &mut depth).await;
        assert!(!cont);
    }

    #[tokio::test]
    async fn drain_detects_channel_closed_via_disconnect() {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
        let (action_tx, _action_rx) = mpsc::channel::<UserAction>(16);
        // Send one event, then drop sender
        tx.send(AgentEvent::TextDelta("first".into()))
            .await
            .unwrap();
        drop(tx); // sender dropped — channel will be Disconnected after first item

        let mut app = make_app();
        let mut depth = 0;
        // Pass the first event manually; drain loop will hit Disconnected
        let (cont, _) = drain_agent_events(
            Some(AgentEvent::TextDelta("first".into())),
            &mut rx,
            &mut app,
            &action_tx,
            &mut depth,
        )
        .await;
        assert!(!cont);
    }

    #[tokio::test]
    async fn drain_reports_streaming_transition() {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
        let (action_tx, _action_rx) = mpsc::channel::<UserAction>(16);
        tx.send(AgentEvent::TurnDone).await.unwrap();

        let mut app = make_app();
        app.start_assistant_turn(); // sets app.streaming = true
        let mut depth = 0;

        // Pass TurnDone as first event; channel also has another TurnDone
        let result = drain_agent_events(
            Some(AgentEvent::TurnDone),
            &mut rx,
            &mut app,
            &action_tx,
            &mut depth,
        )
        .await;

        assert_eq!(result, (true, true));
        assert!(!app.streaming);
    }
}
